#!/usr/bin/env python3
"""Checks what tools/clip_check.sh produced (VG, 2026-11-14).

Every expected clip frame is computed here from the clip's own frame times —
23.976 fps as 1001/24000 s a frame, the variable-rate clip as its two rates —
and every still is compared with the frame FFmpeg extracts from the clip
itself, never with what the report line claims. A frame match counts only when
it beats both neighbouring frames by a margin, so an off-by-one fails.

    tools/clip_check.py OUTPUT_DIR [gpu] [cpu]
"""

from __future__ import annotations

import math
import re
import subprocess
import sys
from pathlib import Path

FAILURES: list[str] = []
SIZE = (320, 180)
FULL = (640, 360)
# A still of the right frame against the frame FFmpeg extracts, both at the
# clip's own size: the same decoder and conversion, sampled texel for texel.
# Measured 2026-11-14 on both renderers; see docs/VIDEO_GENERATION.md.
MATCH_DB = 40.0
# How far the right frame must beat its neighbours.
MARGIN_DB = 3.0


def check(condition: bool, message: str) -> None:
    if not condition:
        FAILURES.append(message)
        print(f"FAIL: {message}", file=sys.stderr)


def read(path: Path) -> str:
    return path.read_text(errors="replace") if path.is_file() else ""


def report_line(log: str, key: str) -> str:
    for line in log.splitlines():
        if line.startswith(f"{key}:"):
            return line.split(":", 1)[1].strip()
    return ""


def decode(args: list[str], size: tuple[int, int]) -> bytes:
    raw = subprocess.run(
        ["ffmpeg", "-nostdin", "-v", "error", *args, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        check=True,
        capture_output=True,
    ).stdout
    if len(raw) != size[0] * size[1] * 3:
        raise ValueError(f"{args}: decoded {len(raw)} bytes for {size}")
    return raw


def still(path: Path, size: tuple[int, int] = SIZE) -> bytes:
    return decode(["-i", str(path)], size)


def clip_frame(clip: Path, index: int, filters: str, size: tuple[int, int]) -> bytes:
    """Frame `index` of `clip`, counted as the decoder returns them."""
    return decode(
        [
            "-i", str(clip), "-vf", f"select=eq(n\\,{index}),{filters}",
            "-fps_mode", "passthrough", "-frames:v", "1",
        ],
        size,
    )


def psnr(a: bytes, b: bytes) -> float:
    if len(a) != len(b):
        return 0.0
    squared = sum((x - y) * (x - y) for x, y in zip(a, b))
    if squared == 0:
        return math.inf
    return 10.0 * math.log10(255.0 * 255.0 / (squared / len(a)))


def crop(image: bytes, width: int, left: int, top: int, w: int, h: int) -> bytes:
    rows = []
    for y in range(top, top + h):
        start = (y * width + left) * 3
        rows.append(image[start : start + w * 3])
    return b"".join(rows)


def mean(data: bytes) -> float:
    return sum(data) / max(1, len(data))


# --- the arithmetic, restated -----------------------------------------------------
NTSC = 1001 / 24000
NTSC_FRAMES = 144
NTSC_LENGTH = NTSC_FRAMES * NTSC


def ntsc_frame(seconds: float) -> int:
    return max(0, min(NTSC_FRAMES - 1, math.floor(seconds / NTSC + 1e-9)))


def vfr_frame(seconds: float) -> int:
    if seconds < 2.0:
        return min(59, math.floor(seconds * 30 + 1e-9))
    return min(89, 60 + math.floor((seconds - 2.0) * 15 + 1e-9))


def exact_expectations() -> dict[str, tuple[str, int, int]]:
    """Still time -> (clip, frame, frame count), from the exact treatment."""
    expected = {}
    for text in "1.5 2.0 3.4 6.0 6.9 7.6 9.5 10.1".split():
        t = float(text)
        if t < 7.5:
            expected[text] = ("ntsc.mp4", ntsc_frame(t - 1.0), NTSC_FRAMES)
        else:
            expected[text] = ("vfr.mp4", vfr_frame(0.5 + t - 7.5), 90)
    return expected


def frame_match(out: Path, image: bytes, clip: str, index: int, count: int, label: str) -> None:
    filters = "null"
    matched = psnr(image, clip_frame(out / clip, index, filters, FULL))
    neighbours = [
        psnr(image, clip_frame(out / clip, other, filters, FULL))
        for other in (index - 1, index + 1)
        if 0 <= other < count
    ]
    best = max(neighbours) if neighbours else 0.0
    print(f"{label}: frame {index} of {clip} at {matched:.2f} dB, neighbours {best:.2f} dB")
    check(matched >= MATCH_DB, f"{label}: frame {index} only {matched:.2f} dB")
    check(matched >= best + MARGIN_DB, f"{label}: frame {index} {matched:.2f} dB does not beat a neighbour's {best:.2f} dB")


def check_exact(out: Path, renderers: list[str]) -> None:
    for renderer in renderers:
        log = read(out / f"exact-{renderer}.log")
        check("clip:" in log and "EXACT" in report_line(log, "clip"), f"{renderer}: the clip line says no exact frame: {report_line(log, 'clip')}")
        check("placeholder=0" in report_line(log, "clip"), f"{renderer}: placeholders drew: {report_line(log, 'clip')}")
        for text, (clip, index, count) in exact_expectations().items():
            image = still(out / f"exact-{renderer}-{text}.png", FULL)
            frame_match(out, image, clip, index, count, f"exact {renderer} t={text}")
    # The report line's own frame index, a still per process.
    for text in ("6.0", "9.5"):
        clip, index, count = exact_expectations()[text]
        line = report_line(read(out / f"alone-{text}.log"), "clip")
        check(f"drew frame {index}/{count} EXACT" in line, f"alone t={text}: expected frame {index}/{count}, the line says {line}")
    # A still taken alone is the same picture as the one from the pass.
    renderer = renderers[0]
    for text in ("6.0", "9.5"):
        alone = still(out / f"alone-{text}.png", FULL)
        passed = still(out / f"exact-{renderer}-{text}.png", FULL)
        check(alone == passed, f"t={text}: a still alone differs from the same still in a pass ({psnr(alone, passed):.2f} dB)")


def check_overrun(out: Path) -> None:
    local = 6.5  # 5 s in, 1.5 s past the section start
    hold = ntsc_frame(min(local, NTSC_LENGTH))
    loop = ntsc_frame(local % NTSC_LENGTH)
    bounce = ntsc_frame(2 * NTSC_LENGTH - local)
    late = ntsc_frame(2 * NTSC_LENGTH - 7.5)
    for name, index in (("hold", hold), ("loop", loop), ("bounce", bounce), ("bounce-late", late)):
        frame_match(out, still(out / f"{name}.png", FULL), "ntsc.mp4", index, NTSC_FRAMES, name)
    check((hold, loop, bounce, late) == (143, 11, 132, 108), f"the restated arithmetic moved: {(hold, loop, bounce, late)}")


def check_fit(out: Path) -> None:
    index = 50  # 2.0 s into a 25 fps clip
    cover = still(out / "cover.png")
    # Cover: the middle 360x202.5 of the square, to the frame. FFmpeg crops in
    # whole pixels, so the reference is a half-row off; judged against the
    # neighbours, and against the uncropped square as the geometry's control.
    reference = clip_frame(out / "square.mp4", index, "crop=360:202:0:79,scale=320:180:flags=area", SIZE)
    squashed = clip_frame(out / "square.mp4", index, "scale=320:180:flags=area", SIZE)
    got, control = psnr(cover, reference), psnr(cover, squashed)
    print(f"cover: {got:.2f} dB against the centre crop, {control:.2f} dB against the whole square squashed")
    check(got >= 22.0 and got >= control + MARGIN_DB, f"cover: not the centre crop ({got:.2f} vs {control:.2f} dB)")
    check(mean(crop(cover, 320, 0, 0, 40, 180)) > 20, "cover: the left edge is black, the clip does not fill the frame")
    contain = still(out / "contain.png")
    bars = crop(contain, 320, 0, 0, 66, 180) + crop(contain, 320, 254, 0, 66, 180)
    check(mean(bars) < 2.0, f"contain: the side bars are not black (mean {mean(bars):.2f})")
    centre = crop(contain, 320, 72, 2, 176, 176)
    reference = crop(clip_frame(out / "square.mp4", index, "scale=180:180:flags=area", (180, 180)), 180, 2, 2, 176, 176)
    got = psnr(centre, reference)
    print(f"contain: the middle 180x180 at {got:.2f} dB, bars mean {mean(bars):.2f}")
    check(got >= 28.0, f"contain: the clip is not the middle 180x180 ({got:.2f} dB)")


def check_turned(out: Path) -> None:
    """A clip whose display matrix turns it a quarter is drawn as a player
    shows it: 180x320 and upright, the frame FFmpeg itself shows (its
    autorotate), not the turned picture squeezed into the coded 320x180."""
    size = (180, 320)
    image = still(out / "turned.png", size)
    index = 25  # 1.0 s into a 25 fps clip
    shown = clip_frame(out / "turned.mp4", index, "null", size)
    neighbours = [psnr(image, clip_frame(out / "turned.mp4", other, "null", size)) for other in (24, 26)]
    # The control: the turned picture squeezed into the coded size and drawn
    # back at the frame's, which is what the old decode looked like.
    squeezed = clip_frame(out / "turned.mp4", index, "scale=320:180,scale=180:320", size)
    got, control = psnr(image, shown), psnr(image, squeezed)
    print(f"turned: frame {index} at {got:.2f} dB, neighbours {max(neighbours):.2f} dB, squeezed {control:.2f} dB")
    check(got >= MATCH_DB and got >= max(neighbours) + MARGIN_DB, f"turned: not FFmpeg's frame {index} ({got:.2f} dB)")
    check(got >= control + MARGIN_DB, f"turned: no better than the squeezed picture ({got:.2f} vs {control:.2f} dB)")
    line = report_line(read(out / "turned.log"), "clip")
    check(f"drew frame {index}/50 EXACT of 180x320" in line, f"turned: the clip line says {line}")


def check_response(out: Path) -> None:
    quiet = (out / "response-0-fixture.png").read_bytes()
    silent = (out / "response-0-silent.png").read_bytes()
    check(quiet == silent, "response 0: the still over music differs from the still over silence")
    loud = (out / "response-1-fixture.png").read_bytes()
    loud_silent = (out / "response-1-silent.png").read_bytes()
    check(loud != loud_silent, "response 1 (control): music and silence give the same still, so the music touches nothing")
    line = report_line(read(out / "response-1-fixture.log"), "clip")
    print(f"response 1 over music: {line[line.find('look'):line.find(';')]}")


def check_exports(out: Path) -> None:
    def frames(name: str) -> list[str]:
        return [line for line in read(out / name).splitlines() if line and not line.startswith("#")]

    a, b = frames("export-a.framemd5"), frames("export-b.framemd5")
    check(len(a) >= 30, f"the export has {len(a)} frames")
    check(a == b, "two exports of the same window differ frame by frame")
    print(f"exports: {len(a)} frames, identical: {a == b}")


def check_project(out: Path) -> None:
    saved = read(out / "project-save.log")
    check("project:" in saved, "the clip project was not saved")
    musi = read(out / "project" / "take.musi")
    check('"clips":[' in musi and "take.assets/clips/" in musi, "the saved project does not bundle the clip")
    line = report_line(read(out / "project-reopen.log"), "clip")
    check("drew frame 119/144 EXACT" in line, f"reopened from the bundle: {line}")
    before = still(out / "project-before.png", FULL)
    after = still(out / "project-reopened.png", FULL)
    check(before == after, f"the reopened project's still differs ({psnr(before, after):.2f} dB)")
    line = report_line(read(out / "project-missing.log"), "clip")
    check("PLACEHOLDER" in line and "the file is missing" in line and "take.mp4" in line,
          f"the missing clip is not named: {line}")
    missing = still(out / "project-missing.png", FULL)
    check(max(missing) > 150, "the missing-clip card has no ink")
    print(f"missing: {line}")


def check_refusals(out: Path) -> None:
    for name, reason in (
        ("truncated", "FFmpeg cannot read it"),
        ("huge", "16000x9000"),
        ("missing", "the file is missing"),
    ):
        log = read(out / f"bad-{name}.log")
        status = read(out / f"bad-{name}.status").strip()
        check(status == "1", f"bad-{name}: exit status {status}")
        check("segments[1]" in log and reason in log, f"bad-{name}: the refusal does not name the place and '{reason}'")
        check(not (out / f"bad-{name}.png").exists(), f"bad-{name}: a still was written after the refusal")
        for line in log.splitlines():
            if reason in line:
                print(f"bad-{name}: {line.strip()[:160]}")
                break


def main() -> int:
    out = Path(sys.argv[1])
    renderers = sys.argv[2:] or ["gpu"]
    for step in (check_exact, check_overrun, check_fit, check_turned, check_response, check_exports, check_project, check_refusals):
        if step is check_exact:
            step(out, renderers)
        else:
            step(out)
    if FAILURES:
        print(f"clip_check: {len(FAILURES)} failure(s)", file=sys.stderr)
        return 1
    print("clip_check: every check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
