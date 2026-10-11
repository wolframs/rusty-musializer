#!/usr/bin/env python3
"""Assertions for tools/treatment_check.sh.

Reads the files that script left in its output directory and checks them
against the treatments it wrote. Standard library plus ffmpeg (already a
dependency of the application) for decoding PNGs.

Each check states what it compares against, because a number produced by the
thing under test and then compared with itself proves nothing:

- counts and values are compared with the *treatment text*, not with the
  report line that claims them;
- a contact sheet's geometry is recomputed here from the layout rule rather
  than read back from the application;
- "a still from a multi-still pass is the still taken alone" is byte equality
  between two separate processes;
- "a still is the frame the export encodes" is PSNR against a decoded MP4
  frame, judged against a neighbouring-moment control rather than a bare
  threshold.

Negative controls that were run when this gate was written are recorded in
docs/MUSIC_VIDEO_CLI.md, with what each one made fail.
"""

from __future__ import annotations

import json
import math
import re
import subprocess
import sys
from pathlib import Path

FAILURES: list[str] = []


def check(condition: bool, message: str) -> None:
    if not condition:
        FAILURES.append(message)
        print(f"FAIL: {message}", file=sys.stderr)


def read(path: Path) -> str:
    return path.read_text(errors="replace") if path.is_file() else ""


def report_line(log: str, key: str) -> str:
    """The value of the first `key:` report line, or ''."""
    for line in log.splitlines():
        if line.startswith(f"{key}:"):
            return line.split(":", 1)[1].strip()
    return ""


def png_size(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    return int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")


def rgb(path: Path) -> tuple[int, int, bytes]:
    width, height = png_size(path)
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        check=True,
        capture_output=True,
    ).stdout
    if len(raw) != width * height * 3:
        raise ValueError(f"{path}: decoded {len(raw)} bytes for {width}x{height}")
    return width, height, raw


def region_mean(image: tuple[int, int, bytes], left: int, top: int, width: int, height: int) -> float:
    image_width, _, raw = image
    total = 0
    for y in range(top, top + height):
        start = (y * image_width + left) * 3
        total += sum(raw[start : start + width * 3])
    return total / (width * height * 3)


def region_bytes(image: tuple[int, int, bytes], left: int, top: int, width: int, height: int) -> bytes:
    image_width, _, raw = image
    rows = []
    for y in range(top, top + height):
        start = (y * image_width + left) * 3
        rows.append(raw[start : start + width * 3])
    return b"".join(rows)


def psnr(a: tuple[int, int, bytes], b: tuple[int, int, bytes]) -> float:
    if (a[0], a[1]) != (b[0], b[1]):
        return 0.0
    squared = sum((x - y) * (x - y) for x, y in zip(a[2], b[2]))
    if squared == 0:
        return math.inf
    return 10.0 * math.log10(255.0 * 255.0 / (squared / len(a[2])))


# The layout rule, restated rather than imported: `contact_sheet.rs` is the thing
# under test. If these constants and that file disagree, one of them is wrong.
TILE_LONG_EDGE = 480
SHEET_MAX_WIDTH = 2000
GUTTER = 8
LABEL_HEIGHT = 48


def sheet_layout(tiles: int, frame_width: int, frame_height: int) -> dict:
    scale = TILE_LONG_EDGE / max(frame_width, frame_height)
    tile_width = round(frame_width * scale)
    tile_height = round(frame_height * scale)
    columns = min(max((SHEET_MAX_WIDTH - GUTTER) // (tile_width + GUTTER), 1), tiles)
    rows = -(-tiles // columns)
    return {
        "width": GUTTER + columns * (tile_width + GUTTER),
        "height": GUTTER + rows * (tile_height + LABEL_HEIGHT + GUTTER),
        "tile": (tile_width, tile_height),
        "columns": columns,
    }


def tile_origin(layout: dict, index: int) -> tuple[int, int]:
    tile_width, tile_height = layout["tile"]
    return (
        GUTTER + (index % layout["columns"]) * (tile_width + GUTTER),
        GUTTER + (index // layout["columns"]) * (tile_height + LABEL_HEIGHT + GUTTER),
    )


def check_vocabulary(out: Path) -> None:
    text = read(out / "vocabulary.json")
    try:
        vocabulary = json.loads(text)
    except json.JSONDecodeError as error:
        check(False, f"--vocabulary did not print JSON with no display: {error}")
        return
    names = [scene["name"] for scene in vocabulary.get("scenes", [])]
    for wanted in ("spectrum", "clawd", "loom", "knot"):
        check(wanted in names, f"vocabulary omits scene {wanted}")
    loom = next((scene for scene in vocabulary["scenes"] if scene["name"] == "loom"), {})
    weight = next((s for s in loom.get("settings", []) if s["name"] == "weight"), None)
    check(weight is not None, "vocabulary omits loom.weight")
    if weight:
        # The bounds the bad-range refusal must quote, so the two cannot drift.
        check(weight["key"] == "settings.loom.weight", "loom.weight has the wrong key")
        check(weight["minimum"] < 1.5 < weight["maximum"], "loom.weight bounds exclude 1.5")
        check(weight["maximum"] < 99, "loom.weight bounds admit 99; bad-range proves nothing")
    check("bass" in vocabulary["routes"]["sources"], "vocabulary omits the bass source")
    check(all(scene.get("look") for scene in vocabulary["scenes"]), "a scene has no `look`")


def check_applied(out: Path) -> None:
    treatment = json.loads(read(out / "full.treatment.json"))
    log = read(out / "full.log")
    line = report_line(log, "treatment")
    sections = len(treatment["segments"])
    scenes = len({segment["scene"] for segment in treatment["segments"]})
    routes = sum(len(block.get("routes", [])) for block in treatment["scenes"].values())
    for wanted in (
        f"{sections} sections over 0:12.000",
        f"({scenes} scenes)",
        f"{routes} routes",
        f"{len(treatment['lyrics'])} lyric lines",
        f"{len(treatment['events'])} events",
        "caption style set",
        "output 640x360@30 balanced",
        "seed 2012",
    ):
        check(wanted in line, f"treatment line lacks {wanted!r}: {line!r}")

    project_path = out / "full.musi"
    check(project_path.is_file(), "--save-project wrote no project")
    if not project_path.is_file():
        return
    project = json.loads(project_path.read_text())
    check(project["metadata"]["title"] == treatment["title"], "the title did not reach the project")
    check(project["deterministic_seed"] == 2012, "the seed did not reach the project")
    check(
        (project["output"]["width"], project["output"]["height"], project["output"]["quality"])
        == (640, 360, "balanced"),
        "the output block did not reach the project",
    )

    plan = project["scene_switches"]
    check(plan["enabled"] is True, "the plan was saved disabled")
    spans = [(cue["start_seconds"], cue["scene_name"]) for cue in plan["cues"]]
    check(
        spans == [(0, "spectrum"), (3, "clawd"), (7.5, "loom"), (10, "spectrum")],
        f"the plan is not the treatment's: {spans}",
    )
    check(
        abs(plan["cues"][-1]["end_seconds"] - project["audio"]["duration_seconds"]) < 1e-6,
        "the last section does not run to the end of the track",
    )
    for earlier, later in zip(plan["cues"], plan["cues"][1:]):
        check(earlier["end_seconds"] == later["start_seconds"], "the plan is not contiguous")

    vocabulary = json.loads(read(out / "vocabulary.json"))

    def setting_index(scene: str, name: str) -> int:
        controls = next(s for s in vocabulary["scenes"] if s["name"] == scene)["settings"]
        return next(i for i, control in enumerate(controls) if control["name"] == name)

    def setting(cue: dict, scene: str, name: str) -> float:
        return cue["settings"][setting_index(scene, name)]

    cues = plan["cues"]
    # Base (scenes.spectrum.amplitude) reaches the first spectrum section; the
    # section's own override wins in the last. Compared as f32: 1.6 is not exact.
    check(abs(setting(cues[0], "spectrum", "amplitude") - 1.6) < 1e-6, "scene base amplitude was not applied")
    check(setting(cues[3], "spectrum", "amplitude") == 0.5, "section override of amplitude was not applied")
    check(setting(cues[1], "clawd", "cats") == 5, "clawd cats was not applied")
    check(setting(cues[1], "clawd", "terminal") == 0, "a false toggle was not written as 0")
    check(setting(cues[2], "loom", "weight") == 1.5, "loom weight was not applied")

    mappings = {m["parameter"]: m for m in project["scenes"][0]["mappings"] if m["output_min"] != m["output_max"]}
    hue = mappings.get("settings.spectrum.hue_swing")
    check(
        hue is not None and (hue["source"], hue["output_min"], hue["output_max"]) == ("bass", 10, 110),
        f"the named route did not reach the project: {hue}",
    )
    weight = mappings.get("settings.loom.weight")
    check(
        weight is not None and (weight["source"], weight["interpolation"]) == ("rms", "smoothstep"),
        f"the spec-string route did not reach the project: {weight}",
    )

    lyrics = [(c["start_seconds"], c["end_seconds"], c["text"]) for c in project["lyrics"]["cues"]]
    check(
        lyrics == [(1, 2.5, "first line"), (4, 8, "second line"), (8, 12, "third line")],
        # 4 -> 8: the next line takes over before the 6 s hold. 8 -> 12: the
        # track ends before it.
        f"lyric spans are not the treatment's closed ends: {lyrics}",
    )
    events = [(e["timestamp_seconds"], e["type"], e["values"]) for e in project["manual_events"]]
    check(events == [(5, "cue", [1]), (9, "custom", [0.5])], f"events are not the treatment's: {events}")
    style = project["caption_style"]
    check(
        (style["face"], style["box"], style["anchor"], style["size_scale"], style["text_rgba"])
        == ("space_grotesk", "plate", "top_center", 0.09, "ffe9c2ff"),
        f"the caption style is not the treatment's: {style}",
    )
    # Unstated members are the shipped defaults, not zeros.
    check(style["width_scale"] == 0.82 and style["margin_scale"] == 0.065, "caption defaults were lost")

    # An ordinary project: opening and saving it again changes nothing — except
    # the one field that is *supposed* to move, the audio's path inside the
    # bundle named after the new file (`full.assets/` -> `full-again.assets/`).
    again = out / "full-again.musi"
    check(again.is_file(), "the saved project did not reopen")
    if again.is_file():
        reopened = json.loads(again.read_text())
        check(
            reopened["audio"]["path"].startswith("full-again.assets/"),
            f"the re-saved project's audio is not in its own bundle: {reopened['audio']['path']}",
        )
        reopened["audio"]["path"] = project["audio"]["path"]
        check(reopened == project, "reopening and re-saving the treatment's project changed it")


def check_relook(out: Path) -> None:
    original = json.loads(read(out / "full.musi") or "{}")
    relooked_path = out / "relooked.musi"
    check(relooked_path.is_file(), "a look-only treatment over a project wrote nothing")
    if not relooked_path.is_file() or not original:
        return
    relooked = json.loads(relooked_path.read_text())
    line = report_line(read(out / "relook.log"), "treatment")
    check("lyric lines kept" in line and "events kept" in line, f"relook line does not say kept: {line!r}")
    check(relooked["lyrics"] == original["lyrics"], "a treatment with no `lyrics` key replaced the lyrics")
    check(relooked["manual_events"] == original["manual_events"], "a treatment with no `events` key replaced the events")
    check(relooked["caption_style"] == original["caption_style"], "a treatment with no caption block changed the style")
    check(
        [cue["scene_name"] for cue in relooked["scene_switches"]["cues"]] == ["pulse", "knot"],
        "the look-only treatment's plan was not applied",
    )
    positional = out / "positional.musi"
    check(positional.is_file(), "a treatment with no audio block did not direct the positional track")
    if positional.is_file():
        project = json.loads(positional.read_text())
        check(len(project["scene_switches"]["cues"]) == 2, "positional track did not get the plan")
        check(project["lyrics"]["cues"] == [], "a fresh track grew lyrics from nowhere")


def check_sheets(out: Path) -> None:
    segments_path = out / "sheet-segments.png"
    check(segments_path.is_file(), "--contact-sheet wrote no file")
    if segments_path.is_file():
        layout = sheet_layout(4, 640, 360)
        size = png_size(segments_path)
        check(
            size == (layout["width"], layout["height"]),
            f"segments sheet is {size}, the layout rule says {(layout['width'], layout['height'])}",
        )
        line = report_line(read(out / "full.log"), "contact sheet")
        # Midpoints of [0,3) [3,7.5) [7.5,10) [10,12).
        check("4 tiles from 0:01.500 to 0:11.000" in line, f"segments sheet line: {line!r}")
        image = rgb(segments_path)
        tile_width, tile_height = layout["tile"]
        tiles = []
        for index in range(4):
            left, top = tile_origin(layout, index)
            tiles.append(region_bytes(image, left, top, tile_width, tile_height))
            # Ink, not brightness: Loom on a quiet fixture is fine threads on a
            # near-black ground and averages 17, a point above the sheet's own
            # ground. A tile that never drew has no bright pixel at all and is
            # one flat colour.
            tile = tiles[-1]
            check(max(tile) > 120, f"segments tile {index} has no bright pixel (max {max(tile)})")
            check(len(set(tile[::97])) > 8, f"segments tile {index} is a flat fill")
            label = region_bytes(image, left, top + tile_height, tile_width, LABEL_HEIGHT)
            check(max(label) > 200, f"segments tile {index} has no label ink")
        # Clawd's cream ground against Spectrum's dark one: the plan really did
        # switch scene, rather than four tiles of whatever the base scene is.
        left, top = tile_origin(layout, 1)
        clawd = region_mean(image, left, top, tile_width, tile_height)
        left, top = tile_origin(layout, 0)
        spectrum = region_mean(image, left, top, tile_width, tile_height)
        check(clawd > spectrum + 60.0, f"clawd tile ({clawd:.1f}) is not brighter than spectrum ({spectrum:.1f})")
        check(len({tile for tile in tiles}) == 4, "two sections produced the identical tile")

    window_path = out / "sheet-window.png"
    check(window_path.is_file(), "the windowed sheet wrote no file")
    if window_path.is_file():
        layout = sheet_layout(6, 640, 360)
        check(
            png_size(window_path) == (layout["width"], layout["height"]),
            f"windowed sheet is {png_size(window_path)}",
        )
        line = report_line(read(out / "window.log"), "contact sheet")
        # Six centres of six equal slices of [3, 7).
        check("6 tiles from 0:03.333 to 0:06.667" in line, f"windowed sheet line: {line!r}")


def check_stills(out: Path) -> None:
    log = read(out / "full.log")
    stills = [line for line in log.splitlines() if line.startswith("export still:")]
    check(len(stills) == 2, f"expected two `export still:` lines, got {len(stills)}")
    for name in ("still-a.png", "still-b.png", "still-b-alone.png", "still-neighbour.png", "at-seven.png"):
        check((out / name).is_file(), f"{name} was not written")
    if FAILURES:
        return
    for name in ("still-a.png", "still-b.png"):
        check(png_size(out / name) == (640, 360), f"{name} is {png_size(out / name)}, not the output size")
    frames = re.findall(r"t=([0-9.]+) frame (\d+) of (\d+)", "\n".join(stills))
    check(
        [(time, int(frame)) for time, frame, _ in frames] == [("3.600", 108), ("7.000", 210)],
        f"stills are not on the frames their times name at 30 fps: {frames}",
    )
    check(
        (out / "still-b.png").read_bytes() == (out / "still-b-alone.png").read_bytes(),
        "the second still of a two-still pass differs from the same still taken alone",
    )
    check(
        (out / "still-a.png").read_bytes() != (out / "still-b.png").read_bytes(),
        "two stills of different moments are the same picture",
    )
    still = rgb(out / "still-b.png")
    matched = psnr(still, rgb(out / "at-seven.png"))
    control = psnr(rgb(out / "still-neighbour.png"), rgb(out / "at-seven.png"))
    print(f"still vs export frame: {matched:.2f} dB (neighbouring moment: {control:.2f} dB)")
    check(matched >= 35.0, f"the still is not the frame the export encodes: {matched:.2f} dB")
    check(matched >= control + 6.0, f"matched {matched:.2f} dB is not clear of the control {control:.2f} dB")
    # The caption is in the still: the top-centre plate the treatment asked for,
    # with "second line" on screen at 7 s. A dark plate on a cream ground.
    plate = region_mean(still, 220, 20, 200, 30)
    ground = region_mean(still, 20, 20, 100, 30)
    check(plate < ground - 40.0, f"no caption plate at top centre (plate {plate:.1f}, ground {ground:.1f})")


def check_refusals(out: Path) -> None:
    expected = {
        "bad-setting": ["segments[0]", "loom has no setting \"wieght\"", "weight"],
        "bad-range": ["segments[0]", "99 is outside"],
        "past-end": ["segments[1] (1:00.000)", "end of the track (0:12.000)"],
        "bad-digest": ["audio.sha256", "the treatment expects 0000"],
    }
    for name, fragments in expected.items():
        status = read(out / f"{name}.status").strip()
        log = read(out / f"{name}.log")
        check(status == "1", f"{name}: exit status {status!r}, expected 1")
        for fragment in fragments:
            check(fragment in log, f"{name}: the refusal does not say {fragment!r}")
        check(not report_line(log, "treatment"), f"{name}: a refused treatment printed a `treatment:` line")
        # Later stages are gated on the error: nothing is written from a
        # treatment that did not apply.
        check(not (out / f"{name}.musi").exists(), f"{name}: a project was saved after the refusal")
        check(not (out / f"{name}.png").exists(), f"{name}: a sheet was rendered after the refusal")

    check(read(out / "late.status").strip() == "1", "a still past the end exited 0")
    check(
        "90.000 s is not on a frame of this track, which ends at 12.000 s" in read(out / "late.log"),
        "a still past the end is not named",
    )
    check(not (out / "late.png").exists(), "a still past the end wrote a file")

    check(read(out / "two-tracks.status").strip() == "1", "a treatment for another track exited 0")
    check("is already open" in read(out / "two-tracks.log"), "the two-track refusal does not explain itself")
    check(not (out / "two-tracks.musi").exists(), "a project was saved for the wrong track")


def main() -> int:
    out = Path(sys.argv[1])
    check_vocabulary(out)
    check_applied(out)
    check_relook(out)
    check_sheets(out)
    check_stills(out)
    check_refusals(out)
    if FAILURES:
        print(f"{len(FAILURES)} check(s) failed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
