"""
ASCII DREAMSCAPE
================
A generative psychedelic ASCII-art screensaver, rendered to video.

Everything is computed as a grid of characters + colors, then drawn through a
fake-CRT pipeline (bloom, chromatic aberration, scanlines, vignette) so it looks
like it escaped from a 2003 Windows screensaver .scr file.

Usage:
    python dreamscape.py                 # full render -> dreamscape.mp4
    python dreamscape.py --preview       # fast, small, short
    python dreamscape.py --scene tunnel  # solo one scene for 10s
"""

import argparse
import math
import multiprocessing as mp
import os
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent

# ---------------------------------------------------------------- geometry ---
COLS, ROWS = 160, 54
CELL_W, CELL_H = 12, 20          # -> 1920 x 1080
FPS = 30
FADE = 2.6                       # crossfade seconds between scenes
FONT_PATH = r"C:\Windows\Fonts\consola.ttf"

TAU = math.pi * 2

# --------------------------------------------------------------- character ---
# Ramps run dark -> bright. Each scene picks the alphabet that suits its mood.
RAMPS = {
    "soft":   " .`',:;~-+=*oO0#%@",
    "blocks": "  ...:::░░▒▒▓▓██",
    "tech":   " .:-=+*#%@$&8B@",
    "matrix": " .:;+=xXÆ#$@10",
    "stars":  " ..``''+*x×※✦●",
    "wave":   " ~-_=≈≡+*#%@",
    "glyph":  " .·:∘○◦●◆◇▪▫■□▩▨",
}


def build_atlas():
    """Render every character we might use into one float32 alpha atlas."""
    chars = []
    for ramp in RAMPS.values():
        for ch in ramp:
            if ch not in chars:
                chars.append(ch)

    # Find the biggest font size whose glyphs still fit the cell.
    font = None
    for size in range(CELL_H + 4, 6, -1):
        f = ImageFont.truetype(FONT_PATH, size)
        w = f.getbbox("M")[2] - f.getbbox("M")[0]
        h = f.getbbox("Mg")[3] - f.getbbox("Mg")[1]
        if w <= CELL_W and h <= CELL_H:
            font = f
            break
    if font is None:
        font = ImageFont.truetype(FONT_PATH, 12)

    atlas = np.zeros((len(chars), CELL_H, CELL_W), np.float32)
    for i, ch in enumerate(chars):
        img = Image.new("L", (CELL_W, CELL_H), 0)
        d = ImageDraw.Draw(img)
        bb = d.textbbox((0, 0), ch, font=font)
        ox = (CELL_W - (bb[2] - bb[0])) / 2 - bb[0]
        oy = (CELL_H - (bb[3] - bb[1])) / 2 - bb[1]
        d.text((ox, oy), ch, fill=255, font=font)
        atlas[i] = np.asarray(img, np.float32) / 255.0

    # Space must be empty even if the font disagrees.
    atlas[chars.index(" ")] = 0.0

    index = {name: np.array([chars.index(c) for c in ramp], np.int32)
             for name, ramp in RAMPS.items()}
    return atlas, index


ATLAS, RAMP_IDX = build_atlas()


# ------------------------------------------------------------------- color ---
def hsv2rgb(h, s, v):
    h = (h % 1.0) * 6.0
    i = np.floor(h).astype(np.int32) % 6
    f = h - np.floor(h)
    p, q, t = v * (1 - s), v * (1 - s * f), v * (1 - s * (1 - f))
    conds = [i == 0, i == 1, i == 2, i == 3, i == 4, i == 5]
    r = np.select(conds, [v, q, p, p, t, v])
    g = np.select(conds, [t, v, v, q, p, p])
    b = np.select(conds, [p, p, t, v, v, q])
    return np.stack([r, g, b], -1).astype(np.float32)


# ------------------------------------------------------------- coordinates ---
_cx, _cy = (COLS - 1) / 2.0, (ROWS - 1) / 2.0
_ar = CELL_W / CELL_H
_col = np.arange(COLS, dtype=np.float32)
_row = np.arange(ROWS, dtype=np.float32)
_scale = ROWS / 2.0
X0 = ((_col - _cx) * _ar / _scale)[None, :] * np.ones((ROWS, 1), np.float32)
Y0 = ((_row - _cy) / _scale)[:, None] * np.ones((1, COLS), np.float32)


def breathe(t):
    """A slow drifting rotate / zoom / ripple applied to the whole world."""
    _, bass = env_at(t)
    rot = 0.28 * math.sin(t * 0.083) + 0.11 * math.sin(t * 0.031)
    zoom = (1.0 + 0.30 * math.sin(t * 0.137) + 0.12 * math.sin(t * 0.061 + 1.3)
            - 0.055 * bass)                            # kick punches the camera in
    c, s = math.cos(rot), math.sin(rot)
    x = (X0 * c - Y0 * s) * zoom
    y = (X0 * s + Y0 * c) * zoom
    xw = x + 0.14 * np.sin(y * 2.3 + t * 0.55)
    yw = y + 0.14 * np.sin(x * 2.1 - t * 0.47)
    return xw, yw


# ------------------------------------------------------------------ scenes ---
# Every scene returns (value, hue) as float arrays shaped (ROWS, COLS),
# value in [0,1] = brightness/density, hue in [0,1) = color wheel position.

def sc_plasma(x, y, t):
    r = np.sqrt(x * x + y * y)
    v = (np.sin(x * 3.1 + t * 0.7)
         + np.sin(y * 2.6 - t * 0.53)
         + np.sin((x + y) * 2.2 + t * 0.91)
         + np.sin(r * 4.4 - t * 1.27)) * 0.25
    val = 0.5 + 0.5 * v
    hue = 0.62 + val * 0.45 + t * 0.035
    return val, hue


def sc_warpnoise(x, y, t):
    """Domain warping: plasma fed back through itself. Peak trip."""
    q1 = np.sin(x * 1.9 + t * 0.31)
    q2 = np.cos(y * 1.7 - t * 0.27)
    p1 = np.sin((x + q1 * 1.2) * 2.7 + t * 0.44)
    p2 = np.cos((y + q2 * 1.2) * 2.5 - t * 0.39)
    w = np.sin((x + p1 * 1.6) * 2.0 + (y + p2 * 1.6) * 2.0 + t * 0.62)
    w += 0.6 * np.sin((x - p2) * 4.5 - (y + p1) * 4.1 - t * 0.8)
    val = 0.5 + 0.34 * w
    hue = 0.05 + 0.5 * p1 + 0.3 * p2 + t * 0.045
    return np.clip(val, 0, 1), hue


def sc_tunnel(x, y, t):
    r = np.sqrt(x * x + y * y) + 1e-3
    a = np.arctan2(y, x)
    d = 1.0 / r
    u = d * 1.15 + t * 0.85
    w = a / TAU * 8.0 + 0.35 * np.sin(d * 0.8 + t * 0.5)
    val = 0.5 + 0.5 * np.sin(u * TAU) * np.sin(w * TAU)
    val *= np.clip(r * 2.4, 0, 1) * np.clip(2.2 - r, 0, 1)
    hue = u * 0.11 + w * 0.03 + t * 0.04
    return np.clip(val, 0, 1), hue


def sc_kaleido(x, y, t):
    a = np.arctan2(y, x) + t * 0.13
    r = np.sqrt(x * x + y * y)
    n = 6.0
    wedge = TAU / n
    a = np.abs((a % wedge) - wedge * 0.5) * n
    xx, yy = r * np.cos(a), r * np.sin(a)
    v = (np.sin(xx * 4.0 + t * 0.8)
         + np.sin(yy * 3.4 - t * 0.6)
         + np.sin(r * 6.0 - t * 1.1)) / 3.0
    val = np.clip(0.5 + 0.55 * v, 0, 1) * np.clip(2.0 - r * 0.9, 0, 1)
    hue = 0.3 + r * 0.22 + t * 0.05 + v * 0.15
    return val, hue


_BLOBS = np.random.default_rng(7).uniform(0, TAU, (7, 4)).astype(np.float32)


def sc_lava(x, y, t):
    field = np.zeros_like(x)
    hacc = np.zeros_like(x)
    for i, (pa, pb, pc, pd) in enumerate(_BLOBS):
        bx = 1.35 * math.sin(t * 0.23 + pa) * math.cos(t * 0.11 + pb)
        by = 0.80 * math.sin(t * 0.19 + pc)
        rad = 0.16 + 0.09 * math.sin(t * 0.4 + pd)
        d2 = (x - bx) ** 2 + (y - by) ** 2 + 1e-3
        g = rad / d2
        field += g
        hacc += g * (i / len(_BLOBS))
    hue = 0.02 + hacc / (field + 1e-3) * 0.8 + t * 0.03
    val = np.clip(field * 0.30, 0, 1) ** 0.75
    return val, hue


_rng = np.random.default_rng(1312)
_RAIN_SPD = _rng.uniform(6.0, 22.0, COLS).astype(np.float32)
_RAIN_OFF = _rng.uniform(0, 200, COLS).astype(np.float32)
_RAIN_LEN = _rng.uniform(8.0, 26.0, COLS).astype(np.float32)


def sc_rain(x, y, t):
    head = (t * _RAIN_SPD + _RAIN_OFF) % (ROWS + 40.0) - 20.0
    dist = head[None, :] - _row[:, None]
    val = np.where(dist >= 0, np.exp(-dist / _RAIN_LEN[None, :]), 0.0) * 1.35
    val += 0.75 * np.exp(-np.abs(dist) * 1.1)          # hot head of the trail
    flick = 0.80 + 0.20 * np.sin(_row[:, None] * 12.7 + _col[None, :] * 4.3 + t * 9.0)
    val = np.clip(val * flick, 0, 1) + 0.05
    hue = 0.33 + 0.06 * np.sin(_col[None, :] * 0.2 + t * 0.3) + 0.10 * (1 - val)
    return val, hue


_NS = 1100
_ST_A = _rng.uniform(0, TAU, _NS).astype(np.float32)
_ST_Z = _rng.uniform(0, 1, _NS).astype(np.float32)
_ST_S = _rng.uniform(0.10, 0.30, _NS).astype(np.float32)
_ST_H = _rng.uniform(0, 1, _NS).astype(np.float32)


def sc_stars(x, y, t):
    z = 1.0 - ((t * _ST_S + _ST_Z) % 1.0)
    rr = (1.0 - z) ** 2.3 * 2.6
    spin = t * 0.09
    sx = rr * np.cos(_ST_A + spin)
    sy = rr * np.sin(_ST_A + spin)
    c = np.rint(_cx + sx * _scale / _ar).astype(np.int32)
    r_ = np.rint(_cy + sy * _scale).astype(np.int32)
    ok = (c >= 0) & (c < COLS) & (r_ >= 0) & (r_ < ROWS)
    val = np.zeros((ROWS, COLS), np.float32)
    hue = np.zeros((ROWS, COLS), np.float32)
    b = ((1.0 - z) ** 1.4).astype(np.float32)
    np.add.at(val, (r_[ok], c[ok]), b[ok])
    np.add.at(hue, (r_[ok], c[ok]), (_ST_H * b)[ok])
    hue = hue / np.maximum(val, 1e-4) * 0.35 + 0.55 + t * 0.02
    # faint background haze so the frame never goes fully dead
    haze = 0.10 + 0.06 * np.sin(np.hypot(x, y) * 5 - t * 1.5)
    return np.clip(val, 0, 1) + haze * 0.5, hue


def sc_vortex(x, y, t):
    r = np.sqrt(x * x + y * y) + 1e-3
    a = np.arctan2(y, x)
    spiral = a * 3.0 + np.log(r) * 5.0 - t * 1.5
    val = 0.5 + 0.5 * np.sin(spiral)
    val *= np.clip(r * 3.0, 0, 1)
    val = val ** 1.3
    hue = 0.75 + np.sin(spiral * 0.5) * 0.18 + t * 0.04 + r * 0.1
    return val, hue


def sc_ripple(x, y, t):
    v = np.zeros_like(x)
    for i in range(4):
        px = 1.25 * math.sin(t * 0.21 + i * 1.9)
        py = 0.75 * math.cos(t * 0.17 + i * 2.4)
        d = np.sqrt((x - px) ** 2 + (y - py) ** 2)
        v += np.sin(d * 11.0 - t * 3.0 + i) / (1.0 + d * 1.6)
    val = np.clip(0.5 + v * 0.42, 0, 1)
    hue = 0.48 + v * 0.22 + t * 0.03
    return val, hue


def sc_moire(x, y, t):
    g1 = np.sin(x * 9.0 + t * 0.4) * np.sin(y * 9.0 - t * 0.3)
    ang = t * 0.11
    xr = x * math.cos(ang) - y * math.sin(ang)
    yr = x * math.sin(ang) + y * math.cos(ang)
    s = 1.0 + 0.35 * math.sin(t * 0.23)
    g2 = np.sin(xr * 9.0 * s - t * 0.5) * np.sin(yr * 9.0 * s + t * 0.45)
    v = (g1 + g2) * 0.5
    val = np.clip(0.5 + v * 0.7, 0, 1)
    hue = 0.14 + v * 0.3 + t * 0.05
    return val, hue


# ------------------------------------------------------------------- words ---
def word_mask(text):
    """Rasterize a word onto the character grid as a soft 0..1 mask.

    Drawn into a proper 16:9 buffer first, then squashed down to the grid —
    character cells are 12x20, so text drawn straight into a 160x54 buffer
    would come out stretched almost twice as tall as it should be.
    """
    SS = 4
    bw, bh = COLS * SS, int(round(ROWS * SS * CELL_H / CELL_W))
    img = Image.new("L", (bw, bh), 0)
    d = ImageDraw.Draw(img)
    size = bh
    while size > 8:
        f = ImageFont.truetype(FONT_PATH, size)
        bb = d.textbbox((0, 0), text, font=f)
        if bb[2] - bb[0] <= bw * 0.86 and bb[3] - bb[1] <= bh * 0.5:
            break
        size = int(size * 0.9)
    d.text(((bw - (bb[2] - bb[0])) / 2 - bb[0],
            (bh - (bb[3] - bb[1])) / 2 - bb[1]), text, fill=255, font=f)
    m = np.asarray(img.resize((COLS, ROWS), Image.LANCZOS), np.float32) / 255.0
    return np.clip(m * 1.35, 0, 1) ** 0.7


# ---------------------------------------------------------------- sequence ---
BASE_SEQ = [
    ("warpnoise", sc_warpnoise, "soft",   11.0),
    ("plasma",    sc_plasma,    "blocks", 10.0),
    ("tunnel",    sc_tunnel,    "tech",   11.0),
    ("kaleido",   sc_kaleido,   "glyph",  10.0),
    ("lava",      sc_lava,      "blocks",  9.0),
    ("rain",      sc_rain,      "matrix", 10.0),
    ("stars",     sc_stars,     "stars",  10.0),
    ("vortex",    sc_vortex,    "wave",   10.0),
    ("moire",     sc_moire,     "tech",    9.0),
    ("ripple",    sc_ripple,    "wave",   10.0),
    ("warpnoise2", sc_warpnoise, "glyph", 11.0),
    ("plasma2",   sc_plasma,    "soft",   10.0),
]
BASE_TOTAL = float(sum(s[3] for s in BASE_SEQ))


def build_seq(target):
    """Grow the sequence to fill `target` seconds.

    Past one pass through the scenes it keeps going, reshuffled and with the
    alphabets rotated, so a longer render is more material rather than the same
    material slowed down.
    """
    if target is None or abs(target - BASE_TOTAL) < 0.5:
        return list(BASE_SEQ)
    names = list(RAMPS.keys())
    rng = np.random.default_rng(99)
    seq, total, rep = [], 0.0, 0
    while total < target:
        block = list(BASE_SEQ)
        if rep:
            block = [block[i] for i in rng.permutation(len(block))]
        for name, fn, ramp, dur in block:
            if rep:
                ramp = names[(names.index(ramp) + rep) % len(names)]
                dur = dur * float(rng.uniform(0.85, 1.15))
            seq.append((f"{name}/{rep}", fn, ramp, dur))
            total += dur
            if total >= target:
                break
        rep += 1
    name, fn, ramp, dur = seq[-1]                      # land exactly on target
    seq[-1] = (name, fn, ramp, max(FADE + 1.0, dur - (total - target)))
    return seq


def build_titles(total):
    return [
        ("ASCII", 3.0, 6.6),
        ("DREAMSCAPE", 6.2, 10.4),
        ("~ let go ~", total * 0.29, total * 0.29 + 4.5),
        ("BREATHE", total * 0.53, total * 0.53 + 4.5),
        ("DRIFT", total * 0.77, total * 0.77 + 4.5),
        ("* * *", total - 8.5, total - 4.5),
    ]


SEQ, BOUNDS, TOTAL, TITLES = None, None, None, None
ENV = None          # (amp, bass) envelopes sampled per output frame
ENV_FPS = FPS
_TITLE_CACHE = {}


def configure(target=None, env=None, fps=FPS):
    global SEQ, BOUNDS, TOTAL, TITLES, ENV, ENV_FPS
    SEQ = build_seq(target)
    BOUNDS = np.cumsum([0.0] + [s[3] for s in SEQ])
    TOTAL = float(BOUNDS[-1])
    TITLES = build_titles(TOTAL)
    ENV, ENV_FPS = env, fps


def env_at(t):
    """(amplitude, bass) at time t, both 0..1. Zero when there is no audio."""
    if ENV is None:
        return 0.0, 0.0
    k = min(int(t * ENV_FPS), ENV.shape[1] - 1)
    return float(ENV[0, k]), float(ENV[1, k])


configure()


def title_layer(t):
    """Returns (mask, strength) for any word currently materializing."""
    for text, t0, t1 in TITLES:
        if t0 - 0.6 <= t <= t1 + 0.6:
            if text not in _TITLE_CACHE:
                _TITLE_CACHE[text] = word_mask(text)
            u = (t - t0) / (t1 - t0)
            s = math.sin(np.clip(u, 0, 1) * math.pi) ** 0.6
            return _TITLE_CACHE[text], float(s)
    return None, 0.0


def evaluate(t):
    """Blend the sequence at time t into (value, hue, ramp_a, ramp_b, mix)."""
    x, y = breathe(t)
    i = int(np.searchsorted(BOUNDS, t, "right")) - 1
    i = max(0, min(i, len(SEQ) - 1))
    _, fn, ramp, _ = SEQ[i]
    val, hue = fn(x, y, t)

    mix = 0.0
    ramp_prev = ramp
    since = t - BOUNDS[i]
    if i > 0 and since < FADE:
        u = since / FADE
        pfn, pramp = SEQ[i - 1][1], SEQ[i - 1][2]
        pv, ph = pfn(x, y, t)
        w = u * u * (3 - 2 * u)                    # smoothstep
        val = pv * (1 - w) + val * w
        # circular hue blend so we sweep the wheel instead of washing to grey
        ca = np.cos(ph * TAU) * (1 - w) + np.cos(hue * TAU) * w
        sa = np.sin(ph * TAU) * (1 - w) + np.sin(hue * TAU) * w
        hue = np.arctan2(sa, ca) / TAU
        mix, ramp_prev = w, pramp
    return val, hue, ramp_prev, ramp, mix


# ------------------------------------------------------------------ render ---
_dither = _rng.random((ROWS, COLS)).astype(np.float32)
VIGNETTE = np.clip(1.15 - 0.42 * (X0 ** 2 * 0.55 + Y0 ** 2), 0.25, 1.0)


def cell_frame(t):
    val, hue, ramp_a, ramp_b, mix = evaluate(t)

    # brightness shaping: punchy, with a slow global pulse. When there is a
    # track underneath, the music rides on top of that — kept deliberately
    # gentle (~12%), because a hard full-frame strobe on every kick is both
    # ugly and genuinely unpleasant to sit in front of.
    amp, bass = env_at(t)
    pulse = 0.86 + 0.14 * math.sin(t * 0.29) + 0.06 * math.sin(t * 0.77)
    pulse *= 1.0 + 0.07 * (amp - 0.5) + 0.12 * (bass - 0.5)
    v = np.clip(val, 0, 1) ** 1.10 * pulse * VIGNETTE
    v = np.clip(v, 0, 1)
    sat = np.clip(0.72 + 0.30 * np.sin(hue * 9.0 + t * 0.4), 0.45, 1.0)
    hue = hue + 0.13 * math.sin(t * 0.037)

    # words surface out of the field: everything else dims, the letters go white
    mask, strength = title_layer(t)
    if strength > 0:
        m = mask * strength
        v = v * (1.0 - 0.80 * strength * (1.0 - mask)) * (1.0 - 0.45 * m)
        v = np.clip(v + m * 0.95, 0, 1)
        sat = sat * (1.0 - 0.85 * m)

    rgb = hsv2rgb(hue, sat, np.clip(v * 1.25, 0, 1))

    # character choice, dither-blended between the two active alphabets
    ia, ib = RAMP_IDX[ramp_a], RAMP_IDX[ramp_b]
    ja = np.clip((v * (len(ia) - 1) + 0.5).astype(np.int32), 0, len(ia) - 1)
    jb = np.clip((v * (len(ib) - 1) + 0.5).astype(np.int32), 0, len(ib) - 1)
    idx = np.where(_dither < mix, ib[jb], ia[ja])
    return idx, rgb, v


def rasterize(idx, rgb, v):
    a = ATLAS[idx]                                    # (R, C, CH, CW)
    a = a.transpose(0, 2, 1, 3).reshape(ROWS * CELL_H, COLS * CELL_W)
    col = np.repeat(np.repeat(rgb, CELL_H, 0), CELL_W, 1)
    return a[..., None] * col


def box_blur(img, passes=3):
    for _ in range(passes):
        p = np.pad(img, ((1, 1), (1, 1), (0, 0)), mode="edge")
        img = (p[:-2, 1:-1] + p[2:, 1:-1] + p[1:-1, :-2] + p[1:-1, 2:]
               + img * 2.0) * np.float32(1 / 6)
    return img


def upscale(img, f):
    return np.repeat(np.repeat(img, f, 0), f, 1)


H, W = ROWS * CELL_H, COLS * CELL_W
_scan = np.ones((H, 1, 1), np.float32)
_scan[::2] = 0.72
_scan[1::2] = 1.04


def crt(frame, t):
    # two-tier bloom: tight halo + wide atmospheric haze.
    # The haze is blurred at 1/16 scale — nobody can tell, and it is 16x cheaper.
    small = frame.reshape(H // 4, 4, W // 4, 4, 3).mean((1, 3))
    tight = box_blur(small, 2)
    tiny = tight.reshape(H // 8, 2, W // 8, 2, 3).mean((1, 3))
    wide = upscale(box_blur(tiny, 4), 2)
    glow = upscale(tight * 0.85 + wide * 1.5, 4)

    # chromatic aberration on the glow only — keeps glyphs readable
    g = glow.copy()
    g[..., 0] = np.roll(glow[..., 0], 3, axis=1)
    g[..., 2] = np.roll(glow[..., 2], -3, axis=1)

    out = frame * 1.0 + g * 0.9
    out *= _scan

    # rolling refresh band, the tell-tale sign of a CRT filmed on a camcorder
    band = 0.055 * np.sin((np.arange(H, dtype=np.float32) / H) * TAU * 1.2
                          - t * 2.1)[:, None, None]
    out *= (1.0 + band)

    out = out / (1.0 + out * 0.55)                     # soft filmic rolloff
    out = np.clip(out, 0, 1) ** (1 / 1.08)
    return (out * 255.0).astype(np.uint8)


def audio_envelope(path, fps, dur):
    """Decode the track and reduce it to per-frame (amplitude, bass) curves."""
    sr, win = 22050, 2048
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1",
         "-ar", str(sr), "-f", "f32le", "-"],
        stdout=subprocess.PIPE, check=True).stdout
    x = np.frombuffer(raw, np.float32)

    n = int(dur * fps)
    hop = sr / fps
    hann = np.hanning(win).astype(np.float32)
    freqs = np.fft.rfftfreq(win, 1.0 / sr)
    low = freqs < 160.0                                # kick + bassline
    amp = np.zeros(n, np.float32)
    bass = np.zeros(n, np.float32)
    for k in range(n):
        i = int(k * hop)
        seg = x[i:i + win]
        if seg.size < win:
            seg = np.pad(seg, (0, win - seg.size))
        s = np.abs(np.fft.rfft(seg * hann))
        amp[k] = s.sum()
        bass[k] = s[low].sum()

    def shape(v):
        v = v / max(np.percentile(v, 99), 1e-6)
        v = np.clip(v, 0, 1) ** 0.7
        # fast attack, slow release — the classic pump, instead of a jitter
        out = np.empty_like(v)
        acc = 0.0
        for i, s in enumerate(v):
            acc = s if s > acc else acc * 0.86 + s * 0.14
            out[i] = acc
        return out

    return np.stack([shape(amp), shape(bass)]).astype(np.float32)


def verify(path):
    """Decode the whole file and refuse to call it finished if it errors.

    A duration or frame-count check is not enough: a file can report the right
    length and still be full of broken NAL units. Only a full decode catches
    that, and it is cheap next to the render that produced it.
    """
    print("  verifying (full decode) ...", end="", flush=True)
    r = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-f", "null", "-"],
                       stderr=subprocess.PIPE)
    errs = [l for l in r.stderr.decode("utf8", "replace").splitlines() if l.strip()]
    if r.returncode != 0 or errs:
        print(f" FAILED — {len(errs)} decode errors")
        for line in errs[:5]:
            print(f"    {line}")
        raise SystemExit("  file is corrupt, do not ship it")
    print(" clean")


_JOB = {}


def _init(t0, fps, target, env):
    _JOB["t0"], _JOB["fps"] = t0, fps
    configure(target, env, fps)


def _render(k):
    t = _JOB["t0"] + k / _JOB["fps"]
    idx, rgb, v = cell_frame(t)
    return crt(rasterize(idx, rgb, v), t).tobytes()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--preview", action="store_true")
    ap.add_argument("--scene")
    ap.add_argument("--seconds", type=float)
    ap.add_argument("--still", type=float, help="write a single PNG at time T")
    ap.add_argument("--audio", help="soundtrack; visuals stretch to fit it")
    ap.add_argument("--no-mux", action="store_true",
                    help="use --audio for timing and reactivity but leave the "
                         "track out; mux it in as a separate pass afterwards")
    ap.add_argument("--jobs", type=int, default=min(8, os.cpu_count() or 4))
    ap.add_argument("--out", default=str(HERE / "dreamscape.mp4"))
    args = ap.parse_args()

    # An audio track sets the length of the piece, and drives it.
    env, adur = None, None
    if args.audio:
        adur = float(subprocess.run(
            ["ffprobe", "-v", "error", "-show_entries", "format=duration",
             "-of", "csv=p=0", args.audio],
            stdout=subprocess.PIPE, check=True).stdout.strip())
        fps_a = 15 if args.preview else FPS
        print(f"  analysing audio ({adur:.1f}s) ...")
        env = audio_envelope(args.audio, fps_a, adur)
        configure(adur, env, fps_a)

    if args.still is not None:
        idx, rgb, v = cell_frame(args.still)
        Image.fromarray(crt(rasterize(idx, rgb, v), args.still)).save(args.out)
        print(f"  -> {args.out}")
        return

    t0, dur = 0.0, TOTAL
    # build_seq() lands on the target but can only trim its last scene so far,
    # so TOTAL may overshoot by a second or two. The audio is the authority on
    # length: without this the fades get scheduled past where -shortest cuts,
    # and the piece ends on a hard chop instead of a fade.
    if args.audio:
        dur = min(TOTAL, adur)
    if args.scene:
        for i, (name, *_r) in enumerate(SEQ):
            if name == args.scene:
                t0, dur = float(BOUNDS[i]) + FADE, SEQ[i][3] - FADE
                break
    if args.seconds:
        dur = args.seconds
    if args.preview and not args.seconds:
        dur = min(dur, 12.0)

    fps = 15 if args.preview else FPS
    n = int(dur * fps)
    out = Path(args.out)

    cmd = ["ffmpeg", "-y", "-loglevel", "error",
           "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
           "-r", str(fps), "-i", "-"]
    if args.audio and not args.no_mux:
        cmd += ["-i", args.audio, "-map", "0:v", "-map", "1:a",
                "-c:a", "aac", "-b:a", "192k",
                "-af", "afade=t=out:st=%.2f:d=3" % max(0.0, dur - 3.0),
                "-shortest"]
    cmd += ["-vf", "fade=in:0:%d,fade=out:%d:%d" % (
                fps, max(0, n - int(fps * 1.6)), int(fps * 1.6)),
            "-c:v", "libx264", "-preset", "medium" if args.preview else "slow",
            "-crf", "18", "-pix_fmt", "yuv420p"]
    # +faststart rewrites the whole file once encoding finishes. On a long
    # render that is a quarter-gigabyte shuffle; leave it to the mux pass,
    # which is cheap enough to redo if it goes wrong.
    if not args.no_mux:
        cmd += ["-movflags", "+faststart"]
    cmd += [str(out)]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)

    # Rendered in batches: a plain imap queues finished frames faster than
    # ffmpeg drains them, and 6 MB apiece that eats all the RAM in the machine.
    started = time.time()
    batch = args.jobs * 2
    with mp.Pool(args.jobs, initializer=_init,
                 initargs=(t0, fps, adur, env)) as pool:
        for base in range(0, n, batch):
            chunk = range(base, min(base + batch, n))
            for buf in pool.map(_render, chunk, chunksize=1):
                proc.stdin.write(buf)
            k = min(base + batch, n)
            el = time.time() - started
            sys.stdout.write(f"\r  frame {k}/{n}  {el:6.1f}s elapsed  "
                             f"~{el / k * (n - k):5.1f}s left ")
            sys.stdout.flush()
    proc.stdin.close()
    rc = proc.wait()
    if rc != 0:
        raise SystemExit(f"\n  ffmpeg failed with exit code {rc} — "
                         f"{out} is not trustworthy, do not ship it")
    print(f"\n  -> {out}  ({dur:.1f}s @ {fps}fps, {W}x{H})")
    verify(out)


if __name__ == "__main__":
    main()
