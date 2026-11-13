#!/usr/bin/env python3
"""Assertions for tools/caption_styles_check.sh (MV2).

Every claim is checked against something the application did not produce
for the claim itself:

- the saved project against the *treatment text*;
- where a caption landed by counting pixels of its colour — the base style is
  magenta and `shout` yellow, colours no scene in the fixture draws at the
  frame's edges, which the control stills (same video, no styles) confirm;
- a title card against the same moment of the same video with no card, and
  its position against the anchor rule restated here;
- "drew the title it was given" by the card's words *as laid out* (the
  `title cards:` line prints the layout's lines, not the entry's fields)
  against the words in the treatment, with the project's artist in capitals
  and the handle after it.

The negative controls run when this gate was written are recorded in
docs/MUSIC_VIDEO_CLI.md.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from treatment_check import FAILURES, check, read, report_line, rgb  # noqa: E402

SEPARATOR = "  ·  "


def load(path: Path) -> dict:
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as error:
        check(False, f"{path.name} could not be read: {error}")
        return {}


def count(image, band: tuple[float, float], test) -> int:
    width, height, raw = image
    top, bottom = int(band[0] * height), int(band[1] * height)
    total = 0
    for y in range(top, bottom):
        row = raw[y * width * 3 : (y + 1) * width * 3]
        for x in range(0, len(row), 3):
            if test(row[x], row[x + 1], row[x + 2]):
                total += 1
    return total


def magenta(r: int, g: int, b: int) -> bool:
    return r >= 170 and g <= 90 and b >= 140 and r - g >= 100


def yellow(r: int, g: int, b: int) -> bool:
    return r >= 200 and g >= 170 and b <= 90


TOP = (0.0, 0.25)
BOTTOM = (0.75, 1.0)
INK = 40  # pixels of a 640x360 caption's colour: a line is several hundred
STRAY = 10


def region_diff(a, b, rect: tuple[int, int, int, int]) -> float:
    width, height, raw_a = a
    _, _, raw_b = b
    left, top, w, h = rect
    left, top = max(0, left), max(0, top)
    right, bottom = min(width, left + w), min(height, top + h)
    total, pixels = 0, 0
    for y in range(top, bottom):
        start, end = (y * width + left) * 3, (y * width + right) * 3
        total += sum(abs(p - q) for p, q in zip(raw_a[start:end], raw_b[start:end]))
        pixels += (end - start)
    return total / max(1, pixels)


def card_rect(line: str) -> tuple[int, int, int, int] | None:
    import re

    match = re.search(r" at (-?\d+),(-?\d+) (\d+)x(\d+) ", line)
    return tuple(int(value) for value in match.groups()) if match else None


def rgba(text: str) -> str:
    digits = text.lstrip("#").lower()
    return digits + "ff" if len(digits) == 6 else digits


def check_vocabulary(out: Path) -> None:
    vocabulary = load(out / "vocabulary.json")
    check(vocabulary.get("caption_styles", {}).get("maximum") == 16,
          "--vocabulary does not describe caption_styles")
    check(vocabulary.get("titles", {}).get("maximum") == 16,
          "--vocabulary does not describe titles")


def check_saved(out: Path) -> None:
    treatment = load(out / "styled.treatment.json")
    project = load(out / "styled.musi")
    styles = {entry["name"]: entry for entry in project.get("caption_style", {}).get("styles", [])}
    check(sorted(styles) == sorted(treatment["caption_styles"]),
          f"saved styles {sorted(styles)} are not the treatment's {sorted(treatment['caption_styles'])}")
    for name, wanted in treatment["caption_styles"].items():
        saved = styles.get(name, {})
        stated = {key: value for key, value in saved.items() if key != "name"}
        expected = {key: (rgba(value) if key.endswith("_rgba") else value)
                    for key, value in wanted.items()}
        check(stated == expected, f"style {name}: saved {stated}, treatment {expected}")
    cues = project.get("lyrics", {}).get("cues", [])
    check([cue.get("style") for cue in cues] == [line.get("style") for line in treatment["lyrics"]],
          f"line styles {[cue.get('style') for cue in cues]} are not the treatment's")
    sections = project.get("scene_switches", {}).get("cues", [])
    check([cue.get("caption_style") for cue in sections]
          == [segment.get("caption_style") for segment in treatment["segments"]],
          "section styles are not the treatment's")
    titles = project.get("scene_switches", {}).get("titles", [])
    check(len(titles) == len(treatment["titles"]), f"{len(titles)} titles saved")
    for saved, wanted in zip(titles, treatment["titles"]):
        for member in ("title", "artist", "handle", "style"):
            check(saved.get(member) == wanted.get(member),
                  f"title {wanted['title']!r}: {member} saved {saved.get(member)!r}, "
                  f"treatment {wanted.get(member)!r}")
        check((saved.get("start_seconds"), saved.get("end_seconds")) == (wanted["at"], wanted["until"]),
              f"title {wanted['title']!r} spans {saved.get('start_seconds')}..{saved.get('end_seconds')}")
    check(not titles or "artist" not in titles[0],
          "the project's artist was copied onto a card instead of being taken when drawn")

    plain = read(out / "plain.musi")
    check(bool(plain), "the control project was not saved")
    for member in ('"styles"', '"titles"', '"caption_style":"', '"style":"'):
        check(member not in plain, f"a project with no named styles or cards writes {member}")

    again = load(out / "resaved.musi")
    for path in (("caption_style",), ("scene_switches",)):
        check(again.get(path[0]) == project.get(path[0]),
              f"reopening and re-saving changed {path[0]}")
    check([cue.get("style") for cue in again.get("lyrics", {}).get("cues", [])]
          == [cue.get("style") for cue in cues], "reopening and re-saving changed the line styles")

    kept = load(out / "kept.musi")
    check(kept.get("caption_style", {}).get("styles") == project.get("caption_style", {}).get("styles"),
          "a treatment with no caption_styles dropped the project's")
    check(kept.get("scene_switches", {}).get("titles") == titles,
          "a treatment with no titles dropped the project's cards")
    check([cue.get("style") for cue in kept.get("lyrics", {}).get("cues", [])]
          == [cue.get("style") for cue in cues], "a treatment with no lyrics dropped the line styles")
    check(read(out / "dropped.status").strip() == "1",
          "a table dropping a style a kept line uses was not refused")
    check(not (out / "dropped.musi").exists(), "a refused treatment saved a project")
    dropped = read(out / "dropped.log")
    check("shout" in dropped and "kept" in dropped,
          "the refusal of a dropped style does not name the style and the kept line")
    check(read(out / "unknown.status").strip() == "1", "an unknown style name was not refused")
    check(not (out / "unknown.png").exists() and not (out / "unknown.musi").exists(),
          "a refused treatment still rendered or saved")
    check("segments[0] (0:00.000).caption_style" in read(out / "unknown.log")
          and '"tpo"' in read(out / "unknown.log"),
          "the unknown-style refusal does not name the place and the name")


def check_reports(out: Path) -> None:
    treatment = load(out / "styled.treatment.json")
    log = read(out / "styled.log")
    lines = sum(1 for line in treatment["lyrics"] if "style" in line)
    sections = sum(1 for segment in treatment["segments"] if "caption_style" in segment)
    claim = (f"caption styles {len(treatment['caption_styles'])} "
             f"({lines} line{'s' if lines != 1 else ''}, "
             f"{sections} section{'s' if sections != 1 else ''}), {len(treatment['titles'])} titles")
    check(claim in report_line(log, "treatment"), f"treatment line lacks {claim!r}")
    check("no artist" not in report_line(log, "treatment"), "a card with an artist was called bare")
    # Stills at 2.0 (spectrum, base), 5.7 (clawd, its section's), 7.2 (its
    # own over its section's), 9.5 (Cadence draws its own line: none).
    check(report_line(log, "caption styles") == "base 1 line, top 1 line, top+shout 1 line",
          f"caption styles line: {report_line(log, 'caption styles')!r}")
    titles = report_line(log, "title cards")
    check(titles.startswith("drew 2 cards over 2 frames"), f"title cards line: {titles!r}")
    second = treatment["titles"][1]
    check(f'last "{second["title"]}" / "{second["artist"].upper()}" style=card' in titles,
          f"the card over Cadence is not the one it was given: {titles!r}")
    first = treatment["titles"][0]
    alone = report_line(read(out / "title-alone.log"), "title cards")
    wanted = f'last "{first["title"]}" / "{treatment["artist"].upper()}{SEPARATOR}{first["handle"]}" style=title'
    check(wanted in alone, f"the first card is not the one it was given: {alone!r} (wanted {wanted!r})")
    check("NO ARTIST" not in alone, "a card taking the project's artist was called bare")
    bare = read(out / "bare.log")
    check("1 with no artist" in report_line(bare, "treatment"),
          "a card with no artist anywhere is not said on the treatment line")
    check("NO ARTIST" in report_line(bare, "title cards"),
          "a card with no artist anywhere is not said on the title cards line")


def check_pictures(out: Path) -> None:
    styled = {name: rgb(out / f"styled-{name}.png") for name in ("title", "top", "loud", "cadence")}
    plain = {name: rgb(out / f"plain-{name}.png") for name in ("title", "top", "loud", "cadence")}
    width, height, _ = styled["title"]

    # The base line in the base section: magenta at the bottom.
    check(count(styled["title"], BOTTOM, magenta) > INK, "the base line is not at the bottom")
    # The section's style moved the line to the top; the control did not.
    top_line = count(styled["top"], TOP, magenta)
    check(top_line > INK and count(styled["top"], BOTTOM, magenta) < STRAY,
          f"the line in the top section is not at the top ({top_line} magenta pixels there)")
    check(count(plain["top"], TOP, magenta) < STRAY and count(plain["top"], BOTTOM, magenta) > INK,
          "the control line is not at the bottom: the scene, not the style, may be magenta")
    # Stacking: the shouted line is yellow (its own) *and* at the top (its
    # section's), and nothing of it is magenta.
    loud = count(styled["loud"], TOP, yellow)
    check(loud > INK, f"the shouted line is not yellow at the top ({loud} pixels)")
    check(count(styled["loud"], BOTTOM, yellow) < STRAY and count(styled["loud"], TOP, magenta) < STRAY,
          "the shouted line is not wholly its own colour in its section's place")
    check(count(plain["loud"], TOP, yellow) < STRAY, "the control has yellow at the top: the scene does")

    # The first card: ink where the control has none, centred, and only there.
    alone_line = report_line(read(out / "title-alone.log"), "title cards")
    rect = card_rect(alone_line)
    check(rect is not None, f"no card rect on {alone_line!r}")
    if rect:
        left, top, w, h = rect
        check(abs(left + w / 2 - width / 2) <= 2 and abs(top + h / 2 - height / 2) <= 3,
              f"the card at {rect} is not centred in {width}x{height}")
        drawn = region_diff(styled["title"], plain["title"], rect)
        check(drawn > 12, f"the card's rect differs from the control by only {drawn:.1f}")
        far = region_diff(styled["title"], plain["title"], (0, 0, width, max(1, top - 70)))
        check(far < 1.5, f"the frame far above the card differs from the control by {far:.1f}")
    check((out / "title-alone.png").read_bytes() == (out / "styled-title.png").read_bytes(),
          "the card's still alone is not the bytes of the same still from the four-still pass")

    # The second card, over Cadence, which draws its own line and no caption.
    cadence_rect = card_rect(report_line(read(out / "styled.log"), "title cards"))
    if cadence_rect:
        drawn = region_diff(styled["cadence"], plain["cadence"], cadence_rect)
        check(drawn > 10, f"the card over Cadence differs from the control by only {drawn:.1f}")


def check_column(out: Path) -> None:
    def field(name: str, key: str) -> str:
        line = report_line(read(out / f"{name}.log"), "lyrics column")
        for token in line.split():
            if token.startswith(f"{key}="):
                return token[len(key) + 1 :]
        return ""

    before = field("column", "cue_style")
    check(before.startswith("shout:pickable@"), f"the cue card shows {before!r}, not the line's own style")
    titles = field("column", "titles")
    check(titles.startswith("2@"), f"the Lyrics view lists {titles!r} title cards, not 2")
    # The table in name order is card, shout, top: a press steps to the next.
    pressed = field("column-press", "cue_style")
    check(pressed.startswith("top:pickable@"), f"a press on the style chip left {pressed!r}")
    gap = field("column-gap", "cue_style")
    check(gap.startswith("shout:pickable@"), f"a press beside the chip changed it to {gap!r}")


def main() -> int:
    out = Path(sys.argv[1])
    check_vocabulary(out)
    check_saved(out)
    check_reports(out)
    check_pictures(out)
    check_column(out)
    if FAILURES:
        print(f"{len(FAILURES)} check(s) failed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
