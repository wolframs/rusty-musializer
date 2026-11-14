"""Read the alpha a still's frame had before it was written opaque (ST).

Since ST every still is an opaque PNG: r, g and b are the frame's, alpha is
255. The frame's own alpha is not thrown away unseen. The application counts it
first and prints, per still,

    still alpha:     translucent=N/M min=A PATH

(and `sheet alpha:     tile=I t=T translucent=N/M min=A` per contact-sheet
tile). That count is the detector for a scene that "eats destination alpha"
(the `BLEND_ALPHA` trap in docs/TRAPS.md), which gates used to read off the
PNG's alpha plane. A gate that asserted a still's alpha was 255 now asserts
that the still's line says `translucent=0`, through `require_opaque` here.

Standard library only: tools/treatment_check.py promises that.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Iterable, NamedTuple

LINE = re.compile(r"^still alpha: +translucent=(\d+)/(\d+) min=(\d+) (.+)$")


class Census(NamedTuple):
    translucent: int
    pixels: int
    min: int

    def describe(self) -> str:
        """The line's own fields: `translucent=N/M min=A`."""
        return f"translucent={self.translucent}/{self.pixels} min={self.min}"


def censuses(logs: Iterable[Path]) -> dict[str, Census]:
    """Every `still alpha:` line in `logs`, keyed by the still's file name.

    By name, because the line prints the path as the command line gave it,
    which may be relative to a directory the reader is not in. A name that
    two lines claim is refused rather than guessed between.
    """
    found: dict[str, Census] = {}
    for log in logs:
        if not log.is_file():
            continue
        for line in log.read_text(errors="replace").splitlines():
            match = LINE.match(line)
            if not match:
                continue
            name = Path(match[4]).name
            census = Census(int(match[1]), int(match[2]), int(match[3]))
            if name in found and found[name] != census:
                raise AssertionError(f"two `still alpha:` lines name {name}: {found[name]} and {census}")
            found[name] = census
    return found


def require_opaque(logs: Iterable[Path], names: Iterable[str]) -> dict[str, Census]:
    """Asserts each named still's frame left no pixel below alpha 255.

    A still with no line fails too: a missing report is not an opaque frame.
    """
    found = censuses(logs)
    for name in names:
        census = found.get(name)
        if census is None:
            raise AssertionError(f"{name}: no `still alpha:` report line")
        if census.translucent:
            raise AssertionError(
                f"{name}: the frame's blends left {census.translucent} of {census.pixels} pixels "
                f"below alpha 255 (min {census.min}); a scene eats destination alpha (docs/TRAPS.md)"
            )
    return found
