#!/usr/bin/env python3
"""What OpenRouter's video models cost, per second and per clip, right now.

    tools/openrouter_video_prices.py            # 720p and 1080p, 10 s clip
    tools/openrouter_video_prices.py --clip 6
    tools/openrouter_video_prices.py --hd      # 1080p or better, generators only

Reads the public, keyless https://openrouter.ai/api/v1/videos/models (the
ordinary /models list prices video models at 0). Prices there are SKUs with
mixed units: `cents_per_*` is cents, `duration_seconds*` is dollars per
second, and Seedance's `video_tokens*` is dollars per token. Seedance's token
count is estimated as width x height x fps / 1024 at 24 fps; that formula is
ByteDance's published one as of 2026-10, not something OpenRouter states, so
those rows are marked "~".

Nothing is generated and no credit is spent.
"""
from __future__ import annotations

import argparse
import json
import urllib.request

URL = "https://openrouter.ai/api/v1/videos/models"
PIXELS = {"480p": (854, 480), "720p": (1280, 720), "1080p": (1920, 1080)}


def per_second(skus: dict, resolution: str) -> tuple[float | None, str]:
    """Dollars per output second at `resolution`, and a note."""
    def find(*names):
        for name in names:
            if name in skus:
                return float(skus[name])
        return None

    cents = find(f"cents_per_video_output_second_{resolution}", f"cents_per_second_output_{resolution}",
                 "cents_per_second_output")
    if cents is not None:
        return cents / 100, ""
    dollars = find(f"duration_seconds_without_audio_{resolution}", f"duration_seconds_{resolution}",
                   f"text_to_video_duration_seconds_{resolution}", "duration_seconds_without_audio",
                   "duration_seconds")
    if dollars is not None:
        return dollars, ""
    token = find(f"video_tokens_without_audio_{resolution}", f"video_tokens_{resolution}",
                 "video_tokens_without_audio", "video_tokens")
    if token is not None and resolution in PIXELS:
        width, height = PIXELS[resolution]
        return token * width * height * 24 / 1024, "~"
    return None, ""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--clip", type=float, default=10.0, help="clip length in seconds")
    parser.add_argument("--hd", action="store_true",
                        help="only models that output 1080p, 2K or 4K, without editors, upscalers and avatars")
    args = parser.parse_args()
    with urllib.request.urlopen(URL, timeout=30) as response:
        models = json.load(response)["data"]
    rows = []
    for model in models:
        skus = model.get("pricing_skus") or {}
        resolutions = model.get("supported_resolutions") or []
        if args.hd and (not {"1080p", "2K", "4K"} & set(resolutions)
                        or any(word in model["id"] for word in ("edit", "upscale", "avatar", "aleph"))):
            continue
        cells = []
        for resolution in ("720p", "1080p"):
            if resolutions and resolution not in resolutions and not any(
                key.endswith("duration_seconds") or key == "cents_per_second_output" for key in skus
            ):
                cells.append(None)
                continue
            price, note = per_second(skus, resolution)
            cells.append(None if price is None else (price, note))
        durations = model.get("supported_durations") or []
        span = f"{min(durations)}-{max(durations)} s" if durations else "-"
        audio = {True: "yes", False: "no"}.get(model.get("generate_audio"), "?")
        frames = model.get("supported_frame_images") or []
        text = (model.get("description") or "").lower()
        references = any("reference" in key for key in skus) or "reference" in text
        inputs = "+".join(
            [name for name, on in (("first", "first_frame" in frames), ("last", "last_frame" in frames),
                                   ("refs", references)) if on]) or "text"
        rows.append((model["id"], cells, span, audio, ",".join(resolutions) or "-", inputs))
    rows.sort(key=lambda row: (row[1][0] or row[1][1] or (99, ""))[0])
    print(f"{'model':38} {'720p $/s':>9} {'1080p $/s':>9} {f'{args.clip:g} s @720p':>11}  length   audio  {'images in':15} resolutions")
    for model, (p720, p1080), span, audio, resolutions, inputs in rows:
        def cell(value):
            return f"{value[1]}{value[0]:.3f}" if value else "-"
        clip = f"${p720[0] * args.clip:.2f}" if p720 else "-"
        print(f"{model:38} {cell(p720):>9} {cell(p1080):>9} {clip:>11}  {span:8} {audio:5}  {inputs:15} {resolutions}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
