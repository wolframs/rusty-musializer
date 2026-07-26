#!/usr/bin/env python3
"""Derives the Rusty Musializer logo from the C project's logo.

Why a script rather than a committed one-off: the derivation is the interesting
part. Anyone can regenerate the icon after the upstream logo changes, and the
transform is stated as three numbers instead of being buried in a binary.

    tools/make_rust_logo.py

Reads ``../musializer/resources/logo/`` (read-only, like everything in that tree)
and writes ``resources/logo/`` here.

## The transform

The C logo is a navy sphere, ``#2e3f6e`` at hue 224 degrees, with a grey rim and
two grey highlights. The transform is:

* rotate hue by **+154 degrees**, landing the navy on hue 18;
* multiply saturation by **1.5**;
* lift value by **35% of the pixel's own saturation**.

Net effect on the sphere: ``#2e3f6e`` -> ``#843411``, an oxide rust.

Every step is chosen so that **unsaturated pixels do not move**. A hue rotation
cannot shift a grey; multiplying a saturation of zero leaves zero; and the value
lift is scaled by saturation, so it is zero for a grey. The rim and the highlights
come out byte-comparable to upstream, and only the thing that carries colour
changes. That is why this is a transform rather than a repaint: it stays
recognisably the same logo, rusted.

A flat value lift would have been the easy mistake -- it would wash out the whole
icon rather than warming the sphere.
"""

import colorsys
import pathlib
import sys

try:
    from PIL import Image
except ImportError:
    sys.exit("This script needs Pillow: python3 -m pip install --user Pillow")

# Chosen so the source navy (#2e3f6e, hue 224) lands on hue 18.
HUE_ROTATION_DEGREES = 154.0
# Hue rotation alone gives a dark chocolate, because the source navy is only
# moderately saturated (0.58) and fairly dark (0.43). Rust is a *saturated* oxide,
# so saturation is multiplied and value is lifted as well.
SATURATION_GAIN = 1.5
# The value lift is deliberately proportional to each pixel's own saturation, so an
# unsaturated pixel is left exactly alone. That is what keeps the grey rim and the
# two highlights identical to upstream instead of washing them out -- a flat value
# lift would brighten the whole logo, not just the part that carries colour.
VALUE_LIFT_PER_SATURATION = 0.35

REPO = pathlib.Path(__file__).resolve().parent.parent
ORACLE_LOGO = pathlib.Path("/home/wolfram/Projects/musializer/resources/logo")
OUT = REPO / "resources" / "logo"


def rust_png(source: pathlib.Path, destination: pathlib.Path) -> None:
    """Applies [`rust`] to every pixel, preserving alpha exactly."""
    image = Image.open(source).convert("RGBA")
    pixels = list(image.get_flattened_data())

    recoloured = []
    for r, g, b, a in pixels:
        nr, ng, nb = rust(r / 255.0, g / 255.0, b / 255.0)
        recoloured.append((round(nr * 255), round(ng * 255), round(nb * 255), a))

    image.putdata(recoloured)
    destination.parent.mkdir(parents=True, exist_ok=True)
    image.save(destination, "PNG")
    print(f"wrote {destination.relative_to(REPO)} ({image.width}x{image.height})")


def rust(r: float, g: float, b: float) -> tuple[float, float, float]:
    """The whole transform, on one pixel in 0..1 floats.

    An unsaturated input is returned unchanged, which is the property the greys
    depend on.
    """
    h, s, v = colorsys.rgb_to_hsv(r, g, b)
    h = (h + (HUE_ROTATION_DEGREES % 360.0) / 360.0) % 1.0
    v = min(1.0, v * (1.0 + VALUE_LIFT_PER_SATURATION * s))
    s = min(1.0, s * SATURATION_GAIN)
    return colorsys.hsv_to_rgb(h, s, v)


def rust_hex(value: str) -> str:
    """Applies the same transform to a `#rrggbb` literal, for the SVG."""
    raw = value.lstrip("#")
    r, g, b = (int(raw[i : i + 2], 16) / 255.0 for i in (0, 2, 4))
    nr, ng, nb = rust(r, g, b)
    return "#{:02x}{:02x}{:02x}".format(round(nr * 255), round(ng * 255), round(nb * 255))


def rust_svg(source: pathlib.Path, destination: pathlib.Path) -> None:
    """Rewrites the SVG's colour literals through [`rust`].

    Only saturated colours move, so the greys stay byte-identical and the diff
    against upstream is one line.
    """
    import re

    text = source.read_text(encoding="utf-8")
    changed: dict[str, str] = {}

    def replace(match: "re.Match[str]") -> str:
        original = match.group(0)
        recoloured = rust_hex(original)
        if recoloured.lower() != original.lower():
            changed[original] = recoloured
        return recoloured

    text = re.sub(r"#[0-9a-fA-F]{6}\b", replace, text)
    header = (
        "<!-- Derived from ../musializer/resources/logo/logo.svg by "
        f"tools/make_rust_logo.py: hue +{HUE_ROTATION_DEGREES:.0f} deg, saturation x{SATURATION_GAIN},\n"
        f"     value +{VALUE_LIFT_PER_SATURATION:.0%} of saturation, so the navy sphere reads as rust.\n"
        "     Greys are unchanged by construction: every step is a no-op at zero saturation.\n"
        "     Do not hand-edit -- regenerate. -->\n"
    )
    if text.startswith("<?xml"):
        end = text.index("?>") + 2
        text = text[:end] + "\n" + header + text[end:]
    else:
        text = header + text

    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(text, encoding="utf-8")
    for original, recoloured in sorted(changed.items()):
        print(f"  {original} -> {recoloured}")
    print(f"wrote {destination.relative_to(REPO)}")


def main() -> None:
    if not ORACLE_LOGO.is_dir():
        sys.exit(f"source logo directory not found: {ORACLE_LOGO}")

    rust_png(ORACLE_LOGO / "logo-256.png", OUT / "logo-256.png")
    rust_svg(ORACLE_LOGO / "logo.svg", OUT / "logo.svg")

    # The desktop icon theme wants a few sizes; downscaling from 256 is enough for
    # a menu entry and avoids depending on an SVG rasterizer, which this machine
    # does not have.
    master = Image.open(OUT / "logo-256.png")
    for size in (128, 64, 48, 32, 16):
        resized = master.resize((size, size), Image.LANCZOS)
        path = OUT / f"logo-{size}.png"
        resized.save(path, "PNG")
        print(f"wrote {path.relative_to(REPO)} ({size}x{size})")


if __name__ == "__main__":
    main()
