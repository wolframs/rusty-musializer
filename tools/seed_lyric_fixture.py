#!/usr/bin/env python3
"""Seed a generated `.musi` with synthetic frame-boundary evidence.

The project itself is first written by Musializer from the synthetic sweep WAV,
so this helper only fills the authored lanes and copies a first-party-shipped
font into the generated asset bundle. No user media or durable fixture enters the
repository. Asset digests cover bytes, not the JSON document, so these edits are
safe and project open verifies every copied file before a `Track` exists.
"""

import argparse
import hashlib
import json
import pathlib
import shutil


LONG_UTF8 = (
    "Καλημέρα κόσμε — мир продолжает звучать; the seeded caption keeps wrapping "
    "until the third line ends honestly with an ellipsis instead of losing this final clause"
)

LINES = [
    LONG_UTF8,
    "Every band a step above",
    "Count the pulse, it never lies",
    "Down again to where we started",
    "And the spectrum keeps its shape",
    "One more turn before it ends",
]


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bundle_font(project_path: pathlib.Path) -> dict:
    root = pathlib.Path(__file__).resolve().parents[1]
    # Alegreya's shipped file carries the Greek and Cyrillic used by LONG_UTF8.
    # Loading the same family through the imported slot still exercises the
    # verified project-asset path without turning missing glyphs into evidence.
    source = root / "resources/fonts/Alegreya-Regular.ttf"
    licence_source = root / "resources/fonts/OFL.txt"
    asset_root = project_path.with_suffix("").with_name(project_path.stem + ".assets")
    font_dir = asset_root / "fonts"
    font_dir.mkdir(parents=True, exist_ok=True)

    font_sha = digest(source)
    licence_sha = digest(licence_source)
    bundled_font = font_dir / f"{font_sha}.otf"
    bundled_licence = font_dir / f"{licence_sha}.txt"
    shutil.copyfile(source, bundled_font)
    shutil.copyfile(licence_source, bundled_licence)
    relative_root = asset_root.name
    return {
        "path": f"{relative_root}/fonts/{bundled_font.name}",
        "sha256": font_sha,
        "family": "Seeded Alegreya",
        "licence_path": f"{relative_root}/fonts/{bundled_licence.name}",
        "licence_sha256": licence_sha,
        "licence_name": "OFL-1.1",
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("project", type=pathlib.Path)
    parser.add_argument(
        "--drop",
        choices=("lyrics", "semantic", "manual"),
        help="negative control: omit exactly one project lane",
    )
    parser.add_argument("--box", choices=("none", "shadow", "plate"), default="plate")
    parser.add_argument(
        "--scene",
        choices=("spectrum", "cadence", "loom", "constellation"),
        default="constellation",
    )
    parser.add_argument(
        "--scene-plan",
        action="store_true",
        help="seed a disabled two-cue automatic scene plan",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    project = json.loads(args.project.read_text())
    duration = float(project["audio"]["duration_seconds"])

    cues = []
    for index, text in enumerate(LINES):
        start = 0.4 + index * 1.25
        end = start + 1.0
        if end > duration:
            break
        cues.append(
            {
                "id": index + 1,
                "start_seconds": round(start, 3),
                "end_seconds": round(end, 3),
                "text": text,
            }
        )
    if not cues:
        raise SystemExit("the fixture is too short to hold a cue")

    project["lyrics"] = (
        {"next_id": 1, "cues": []}
        if args.drop == "lyrics"
        else {"next_id": len(cues) + 1, "cues": cues}
    )
    project["semantic_events"] = (
        []
        if args.drop == "semantic"
        else [
            {
                "timestamp_seconds": 0.0,
                "id": 11,
                "type": "semantic",
                "values": [0.82, 0.91, -0.72, 0.97],
            },
            {
                "timestamp_seconds": 2.0,
                "id": 12,
                "type": "semantic",
                "values": [0.18, 0.23, 0.78, 0.88],
            },
        ]
    )
    project["manual_events"] = (
        []
        if args.drop == "manual"
        else [
            {
                "timestamp_seconds": 0.5,
                "id": 21,
                "type": "cue",
                "values": [0.33],
            },
            {
                "timestamp_seconds": 1.0,
                "id": 22,
                "type": "custom",
                "values": [0.66],
            },
        ]
    )
    project["caption_style"] = {
        "face": "imported",
        "box": args.box,
        "anchor": "top_left",
        "size_scale": 0.062,
        "margin_scale": 0.055,
        "width_scale": 0.34,
        "text_rgba": "f8f2ffff",
        "box_rgba": "120d2bd8",
        "font": bundle_font(args.project),
    }
    if args.scene_plan:
        midpoint = round(duration / 2.0, 3)
        project["scene_switches"] = {
            "enabled": False,
            "cues": [
                {
                    "id": 101,
                    "start_seconds": 0.0,
                    "end_seconds": midpoint,
                    "scene_name": "spectrum",
                    "strength": 1.0,
                    "settings": [],
                },
                {
                    "id": 102,
                    "start_seconds": midpoint,
                    "end_seconds": duration,
                    "scene_name": "loom",
                    "strength": 1.0,
                    "settings": [],
                },
            ],
        }
    project["scenes"][0]["scene_type"] = args.scene
    project["output"].update(
        {
            "width": 640,
            "height": 360,
            "fps_numerator": 30,
            "fps_denominator": 1,
            "quality": "balanced",
        }
    )
    args.project.write_text(json.dumps(project, indent=2) + "\n")
    print(
        f"seeded {len(project['lyrics']['cues'])} lyrics, "
        f"{len(project['semantic_events'])} semantic events, "
        f"{len(project['manual_events'])} manual events, "
        f"box={args.box}, scene={args.scene}, drop={args.drop or 'none'}, "
        f"scene_plan={'2 cues' if args.scene_plan else 'unchanged'}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
