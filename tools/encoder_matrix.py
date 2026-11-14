#!/usr/bin/env python3
"""Measure encoder settings on the exact frames an export hands to FFmpeg."""

# Two steps, so a matrix can be re-run without rendering again:
#
#   capture NAME QUALITY -- APP-ARGS...
#       Renders an export through tools/headless_run.sh (muted, isolated) with an
#       `ffmpeg` in front of the real one that also writes every frame it is
#       piped, losslessly (FFV1), to build/encoder-matrix/cap/NAME-QUALITY.mkv.
#       The application's own MP4 is kept beside it, so the matrix can prove it
#       reproduces the application's bytes.
#
#   encode [--only NAME,...] [--set SETNAME] [--reuse]
#       Encodes each capture with every candidate in the set (video only, the
#       application's argument list plus the candidate's additions), and prints
#       one row per cell: size, encode wall time, the gradient measure, and
#       PSNR/SSIM per plane against the yuv420p frames handed to the encoder.
#
#   crops CELL... --out PNG
#       The same frame from several cells, cropped to the measured region,
#       contrast-stretched and enlarged side by side, to judge by eye.
#
# The gradient measure is the gate's (`tools/headless_check.sh`, "a slow dark
# gradient"): RMS of a 7 px box blur minus a 31 px box blur, here of the decoded
# Y plane in Y codes, over a region. A staircase leaves its steps in it; a
# dithered gradient leaves almost nothing once blurred. The region per sampled
# frame is the darkest smooth region with a real gradient in it (picked on the
# reference, so every cell is measured on the same pixels); `corner` is the
# gate's fixed top-left rect. Under a film finish the gradient measure reads the
# grain, not contours, so `blotch` (the blurred encoding error) is the one to
# read there (`docs/TRAPS.md`, "The gate's gradient measure reads grain as
# detail"). The 2026-11 run and its decision: docs/archive/BALANCED_ENCODER_2026-11.md.
#
# --set names a candidate table below; `today` in it must reproduce the
# application's MP4, which `encode` checks against the capture's own export.

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
WORK = REPO / "build" / "encoder-matrix"
CAP = WORK / "cap"
OUT = WORK / "out"
SHM = Path("/dev/shm") / f"encoder-matrix-{os.getuid()}"
REAL_FFMPEG = shutil.which("ffmpeg") or "/usr/bin/ffmpeg"

# The application's arguments, `crates/musializer-runtime/src/process/ffmpeg.rs`
# (`Encoder::start_with_program`), video side only. Audio is left out so a size
# is the video stream's.
X264_TODAY = {
    "balanced": ["-preset", "fast", "-crf", "20"],
    "high": ["-preset", "slow", "-crf", "16"],
    "master": ["-preset", "slow", "-crf", "12"],
}
NVENC_TODAY = {
    "balanced": ["-preset", "p1", "-rc", "vbr", "-cq", "28", "-b:v", "0"],
    "high": ["-preset", "p4", "-rc", "vbr", "-cq", "23", "-b:v", "0"],
    "master": ["-preset", "p6", "-rc", "vbr", "-cq", "19", "-b:v", "0"],
}
COLOUR = "colorprim=bt709:transfer=bt709:colormatrix=bt709"
TAIL = [
    "-pix_fmt", "yuv420p", "-color_primaries", "bt709", "-color_trc", "bt709",
    "-colorspace", "bt709", "-movflags", "+faststart",
]


def x264(quality: str, extra: list[str] | None = None, params: str = "",
         crf: str | None = None) -> list[str]:
    base = list(X264_TODAY[quality])
    if crf is not None:
        base[3] = crf
    return (["-c:v", "libx264"] + base + ["-profile:v", "high"] + (extra or [])
            + ["-x264-params", COLOUR + (":" + params if params else "")])


def nvenc(quality: str, extra: list[str] | None = None) -> list[str]:
    return (["-c:v", "h264_nvenc"] + NVENC_TODAY[quality] + (extra or [])
            + ["-profile:v", "high"])


SETS = {
    "balanced": {
        "today": x264("balanced"),
        "aq3": x264("balanced", ["-aq-mode", "3"]),
        "aq3s0.8": x264("balanced", ["-aq-mode", "3", "-aq-strength", "0.8"]),
        "aq3s1.2": x264("balanced", ["-aq-mode", "3", "-aq-strength", "1.2"]),
        "aq2": x264("balanced", ["-aq-mode", "2"]),
        "aq1s1.4": x264("balanced", ["-aq-strength", "1.4"]),
        "aq1s1.2": x264("balanced", ["-aq-strength", "1.2"]),
        "aq1s1.3": x264("balanced", ["-aq-strength", "1.3"]),
        "aq3s1.4": x264("balanced", ["-aq-mode", "3", "-aq-strength", "1.4"]),
        "film": x264("balanced", ["-tune", "film"]),
        "film+s1.2": x264("balanced", ["-tune", "film", "-aq-strength", "1.2"]),
        "film+aq3": x264("balanced", ["-tune", "film", "-aq-mode", "3"]),
        "grain": x264("balanced", ["-tune", "grain"]),
        "nodz": x264("balanced", params="no-dct-decimate=1:deadzone-inter=6:deadzone-intra=6"),
        "aq3nodz": x264("balanced", ["-aq-mode", "3"],
                        params="no-dct-decimate=1:deadzone-inter=6:deadzone-intra=6"),
        "crf19": x264("balanced", crf="19"),
        "crf18": x264("balanced", crf="18"),
        "crf17": x264("balanced", crf="17"),
        "high-args": x264("high"),
    },
    # What else could flatten the dither at Balanced: the preset, mb-tree,
    # deblocking, and B-frames' coarser quantiser.
    "round2": {
        "today": x264("balanced"),
        "medium": x264("balanced", ["-preset", "medium"]),
        "slow": x264("balanced", ["-preset", "slow"]),
        "nombtree": x264("balanced", params="mbtree=0"),
        "deblock-2": x264("balanced", params="deblock=-2,-2"),
        "nodeblock": x264("balanced", params="no-deblock=1"),
        "pb1.1": x264("balanced", params="ipratio=1.1:pbratio=1.1"),
        "aq3nombtree": x264("balanced", ["-aq-mode", "3"], params="mbtree=0"),
        "film+pb1.1": x264("balanced", ["-tune", "film"], params="ipratio=1.1:pbratio=1.1"),
    },
    "deep": {
        "high": x264("high"),
        "high+aq3": x264("high", ["-aq-mode", "3"]),
        "master": x264("master"),
        "master+aq3": x264("master", ["-aq-mode", "3"]),
    },
    "nvenc": {
        "nv-today": nvenc("balanced"),
        "nv-saq": nvenc("balanced", ["-spatial-aq", "1"]),
        "nv-saq12": nvenc("balanced", ["-spatial-aq", "1", "-aq-strength", "12"]),
        "nv-saq-taq": nvenc("balanced", ["-spatial-aq", "1", "-temporal-aq", "1"]),
        "nv-high": nvenc("high"),
        "nv-high-saq": nvenc("high", ["-spatial-aq", "1"]),
    },
}

WRAPPER = """#!/bin/bash
# encoder_matrix.py: tee the frames the application pipes into FFmpeg.
if [ -n "$MZ_MATRIX_CAPTURE" ] && [[ " $* " == *" rawvideo "* ]]; then
    exec {real} "$@" -map 0:v:0 -c:v ffv1 -level 3 -slices 24 -threads 8 \\
        -pix_fmt bgr0 "$MZ_MATRIX_CAPTURE"
fi
exec {real} "$@"
"""


def capture(name: str, quality: str, app_args: list[str]) -> None:
    CAP.mkdir(parents=True, exist_ok=True)
    wrapbin = WORK / "wrapbin"
    wrapbin.mkdir(parents=True, exist_ok=True)
    wrapper = wrapbin / "ffmpeg"
    wrapper.write_text(WRAPPER.format(real=REAL_FFMPEG))
    wrapper.chmod(0o755)
    stem = CAP / f"{name}-{quality}"
    mkv, mp4 = stem.with_suffix(".mkv"), stem.with_suffix(".mp4")
    for path in (mkv, mp4):
        path.unlink(missing_ok=True)
    env = dict(os.environ)
    env["PATH"] = f"{wrapbin}:{env['PATH']}"
    env["MZ_MATRIX_CAPTURE"] = str(mkv)
    env["MZ_STATE_DIR"] = str(WORK / "state")
    cmd = [str(REPO / "tools" / "headless_run.sh"), *app_args,
           "--quality", quality, "--render", str(mp4)]
    started = time.monotonic()
    with open(stem.with_suffix(".log"), "w") as log:
        status = subprocess.call(cmd, env=env, stdout=log, stderr=subprocess.STDOUT)
    took = time.monotonic() - started
    meta = {"name": name, "quality": quality, "args": app_args, "status": status,
            "render_seconds": round(took, 1)}
    (stem.with_suffix(".json")).write_text(json.dumps(meta, indent=1))
    print(f"capture {name}-{quality}: exit {status}, {took:.0f} s, "
          f"{mkv.stat().st_size if mkv.exists() else 0} bytes of frames")
    if status != 0 or not mkv.exists():
        sys.exit(1)


def probe(path: Path) -> tuple[int, int, int]:
    out = subprocess.check_output([
        "ffprobe", "-v", "error", "-select_streams", "v:0", "-count_packets",
        "-show_entries", "stream=width,height,nb_read_packets", "-of", "csv=p=0",
        str(path)], text=True).strip().split(",")
    return int(out[0]), int(out[1]), int(out[2])


def box(a: np.ndarray, k: int) -> np.ndarray:
    c = np.cumsum(np.cumsum(np.pad(a, ((k // 2 + 1, k // 2), (k // 2 + 1, k // 2)),
                                   mode="edge"), 0), 1)
    return (c[k:, k:] - c[:-k, k:] - c[k:, :-k] + c[:-k, :-k]) / (k * k)


def residual(y: np.ndarray) -> np.ndarray:
    return box(y, 7) - box(y, 31)


def samples(n: int) -> tuple[int, ...]:
    """Five frames spread over the clip, away from its ends."""
    return tuple(int(n * (i + 0.5) / 5) for i in range(5))


def y_frames(path: Path, w: int, h: int, frames: tuple[int, ...]) -> np.ndarray:
    sel = "+".join(f"eq(n\\,{n})" for n in frames)
    raw = subprocess.check_output([
        REAL_FFMPEG, "-v", "error", "-i", str(path), "-vf",
        f"select='{sel}',extractplanes=y", "-vsync", "0", "-f", "rawvideo", "-"])
    return np.frombuffer(raw, np.uint8).reshape(-1, h, w).astype(np.float64)


TILE = (160, 256)  # rows, columns


def pick_regions(ref_y: np.ndarray) -> list[tuple[int, int, int, int]]:
    """Per sampled frame: the darkest tile that is smooth and holds a gradient.

    Smooth: the reference's own residual is low (no stars, edges, text).
    A gradient: the 31 px blur spans at least 3 Y codes across the tile, since
    a flat field has no contour to lose. Dark: mean Y under 80 and above 18
    (black at 16 has nothing to band). Among those, the lowest of
    mean Y + 40 * reference residual.
    """
    regions = []
    th, tw = TILE
    for y in ref_y:
        res = residual(y)
        smooth = box(y, 31)
        best = None
        for r in range(16, y.shape[0] - th - 16, th // 2):
            for c in range(16, y.shape[1] - tw - 16, tw // 2):
                tile = y[r:r + th, c:c + tw]
                mean = tile.mean()
                if not 18 <= mean <= 80:
                    continue
                inner = smooth[r + 16:r + th - 16, c + 16:c + tw - 16]
                if inner.max() - inner.min() < 3:
                    continue
                rr = float(np.sqrt((res[r + 16:r + th - 16, c + 16:c + tw - 16] ** 2).mean()))
                score = mean + 40 * rr
                if best is None or score < best[0]:
                    best = (score, r, c)
        regions.append((best[1], best[2], th, tw) if best else None)
    return regions


def gradient(cell_y: np.ndarray, regions) -> float:
    values = []
    for y, region in zip(cell_y, regions):
        if region is None:
            continue
        r, c, th, tw = region
        res = residual(y)[r + 16:r + th - 16, c + 16:c + tw - 16]
        values.append(float(np.sqrt((res ** 2).mean())))
    return float(np.mean(values)) if values else float("nan")


def blotch(cell_y: np.ndarray, ref_y: np.ndarray, regions) -> float:
    """RMS of the encoding error after a 7 px blur, over the same regions.

    The gradient measure assumes a smooth source. Under a film finish the
    source is grain, the encoder's flattening *lowers* that measure, and the
    visible defect is the grain turning into flat blocks a code or two off
    the picture. A blurred error keeps those blocks and contour steps and
    drops the per-pixel grain, so it reads the same defect on both kinds of
    material.
    """
    values = []
    for y, ref, region in zip(cell_y, ref_y, regions):
        if region is None:
            continue
        r, c, th, tw = region
        err = box(y - ref, 7)[r + 16:r + th - 16, c + 16:c + tw - 16]
        values.append(float(np.sqrt((err ** 2).mean())))
    return float(np.mean(values)) if values else float("nan")


def quality(cell: Path, ref: Path) -> dict:
    err = subprocess.run([
        REAL_FFMPEG, "-hide_banner", "-i", str(cell), "-i", str(ref), "-lavfi",
        "[0:v]split[a][b];[1:v]split[c][d];[a][c]ssim;[b][d]psnr",
        "-f", "null", "-"], capture_output=True, text=True).stderr
    s = re.search(r"SSIM Y:([\d.]+).*U:([\d.]+).*V:([\d.]+).*All:([\d.]+)", err)
    p = re.search(r"PSNR y:([\d.inf]+) u:([\d.inf]+) v:([\d.inf]+) average:([\d.inf]+)", err)
    if not s or not p:
        sys.exit(f"no SSIM/PSNR for {cell}:\n{err[-2000:]}")
    return {"ssim": [float(v) for v in s.groups()], "psnr": [float(v) for v in p.groups()]}


def stream_md5(path: Path) -> str:
    return subprocess.check_output([
        REAL_FFMPEG, "-v", "error", "-i", str(path), "-map", "0:v:0", "-c", "copy",
        "-f", "md5", "-"], text=True).strip()


def encode(only: list[str] | None, setname: str, reuse: bool) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    SHM.mkdir(parents=True, exist_ok=True)
    rows_path = WORK / f"rows-{setname}.jsonl"
    # --reuse measures an existing cell again (a new measure) and keeps the
    # encode time recorded when it was made.
    previous = {}
    if reuse and rows_path.exists():
        for line in rows_path.read_text().splitlines():
            row = json.loads(line)
            previous[(row["cap"], row["cell"])] = row["seconds"]
        rows_path.unlink()
    caps = sorted(CAP.glob("*.mkv"))
    if only:
        caps = [c for c in caps if c.stem in only]
    for cap in caps:
        meta = json.loads(cap.with_suffix(".json").read_text())
        w, h, n = probe(cap)
        raw = SHM / f"{cap.stem}.rgba"
        subprocess.check_call([REAL_FFMPEG, "-v", "error", "-y", "-i", str(cap),
                               "-pix_fmt", "rgba", "-f", "rawvideo", str(raw)])
        source = ["-f", "rawvideo", "-pix_fmt", "rgba", "-s", f"{w}x{h}",
                  "-framerate", "30", "-i", str(raw)]
        ref = OUT / f"{cap.stem}.ref.mkv"
        if not ref.exists():
            subprocess.check_call([REAL_FFMPEG, "-v", "error", "-y", *source,
                                   "-c:v", "ffv1", "-level", "3", "-slices", "24",
                                   *TAIL[:-2], str(ref)])
        frames = samples(n)
        ref_y = y_frames(ref, w, h, frames)
        regions = pick_regions(ref_y)
        corner = [(0, 0, 260, 420)] * len(ref_y)
        floor = gradient(ref_y, regions)
        print(f"== {cap.stem}: {w}x{h}, {n} frames; regions {regions}; "
              f"reference gradient {floor:.3f}, corner {gradient(ref_y, corner):.3f}")
        for label, args in SETS[setname].items():
            if setname == "deep" and meta["quality"] == "balanced":
                continue
            out = OUT / f"{cap.stem}.{label}.mp4"
            if reuse and out.exists() and (cap.stem, label) in previous:
                took = previous[(cap.stem, label)]
                run = subprocess.CompletedProcess([], 0)
            else:
                started = time.monotonic()
                run = subprocess.run([REAL_FFMPEG, "-v", "error", "-y", *source, *args,
                                      *TAIL, str(out)], capture_output=True, text=True)
                took = time.monotonic() - started
            if run.returncode != 0:
                print(f"  {label:10s} FAILED: {run.stderr.strip()[-300:]}")
                continue
            cell_y = y_frames(out, w, h, frames)
            q = quality(out, ref)
            row = {"cap": cap.stem, "set": setname, "cell": label,
                   "bytes": out.stat().st_size, "seconds": round(took, 2),
                   "gradient": round(gradient(cell_y, regions), 4),
                   "corner": round(gradient(cell_y, corner), 4),
                   "blotch": round(blotch(cell_y, ref_y, regions), 4),
                   "floor": round(floor, 4), **q, "regions": regions}
            with open(rows_path, "a") as f:
                f.write(json.dumps(row) + "\n")
            print(f"  {label:10s} {row['bytes'] / 1e6:7.2f} MB {took:6.1f} s  "
                  f"grad {row['gradient']:.3f} blotch {row['blotch']:.3f} corner {row['corner']:.3f}  "
                  f"SSIM Y {q['ssim'][0]:.5f} All {q['ssim'][3]:.5f}  "
                  f"PSNR y {q['psnr'][0]:.2f} avg {q['psnr'][3]:.2f}")
        app_mp4 = cap.with_suffix(".mp4")
        today = OUT / f"{cap.stem}.today.mp4"
        if setname == "balanced" and meta["quality"] == "balanced" and today.exists():
            same = stream_md5(app_mp4) == stream_md5(today)
            print(f"  the application's own MP4 and the 'today' cell: "
                  f"{'identical video stream' if same else 'DIFFERENT'}")
        raw.unlink()


def crops(cells: list[str], out: str, frame_index: int) -> None:
    from PIL import Image, ImageDraw
    tiles = []
    for cell in cells:
        path = OUT / (f"{cell}.mkv" if cell.endswith(".ref") else f"{cell}.mp4")
        cap_stem = cell.split(".")[0]
        rows = [json.loads(line) for p in WORK.glob("rows-*.jsonl")
                for line in p.read_text().splitlines()]
        regions = next(r["regions"] for r in rows if r["cap"] == cap_stem)
        w, h, n = probe(path)
        y = y_frames(path, w, h, (samples(n)[frame_index],))[0]
        r, c, th, tw = regions[frame_index]
        tiles.append((cell, y[r:r + th, c:c + tw]))
    lo = min(np.percentile(t, 1) for _, t in tiles)
    hi = max(np.percentile(t, 99) for _, t in tiles)
    scale = 3
    images = []
    for name, t in tiles:
        v = np.clip((t - lo) / max(hi - lo, 1) * 255, 0, 255).astype(np.uint8)
        # Nearest-neighbour by repetition: PIL's resize fails on this
        # machine's numpy/PIL pairing.
        v = np.repeat(np.repeat(v, scale, 0), scale, 1)
        im = Image.fromarray(np.ascontiguousarray(np.stack([v] * 3, -1)))
        ImageDraw.Draw(im).text((6, 6), name, fill=(255, 64, 64))
        images.append(im)
    sheet = Image.new("RGB", (images[0].width, sum(i.height + 4 for i in images)))
    yy = 0
    for im in images:
        sheet.paste(im, (0, yy))
        yy += im.height + 4
    sheet.save(out)
    print(f"{out}: Y {lo:.0f}..{hi:.0f} stretched to 0..255, x{scale}")


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("capture")
    c.add_argument("name")
    c.add_argument("quality", choices=["balanced", "high", "master"])
    c.add_argument("app_args", nargs=argparse.REMAINDER)
    e = sub.add_parser("encode")
    e.add_argument("--only", default="")
    e.add_argument("--set", default="balanced", choices=sorted(SETS))
    e.add_argument("--reuse", action="store_true")
    k = sub.add_parser("crops")
    k.add_argument("cells", nargs="+")
    k.add_argument("--out", required=True)
    k.add_argument("--frame", type=int, default=2)
    a = ap.parse_args()
    if a.cmd == "capture":
        args = a.app_args[1:] if a.app_args[:1] == ["--"] else a.app_args
        capture(a.name, a.quality, args)
    elif a.cmd == "encode":
        encode([s for s in a.only.split(",") if s] or None, a.set, a.reuse)
    else:
        crops(a.cells, a.out, a.frame)


if __name__ == "__main__":
    main()
