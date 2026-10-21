#!/usr/bin/env python3
"""`tools/align_lyrics.py`'s pure parts (MV9-g): which lines are sung, how a
line becomes alignment tokens, and how word times become cue spans.

The alignment itself needs the MMS runtime and a GPU-sized model; the part a
treatment depends on — the file shape and the span rules — is pinned here.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import align_lyrics  # noqa: E402


class SungLines(unittest.TestCase):
    def test_section_headers_and_blank_lines_are_not_sung(self) -> None:
        text = "[Intro: spoken]\nI thought the heat\n\n[Chorus]\nLet the state persist\n  \n"
        self.assertEqual(
            align_lyrics.sung_lines(text),
            ["I thought the heat", "Let the state persist"],
        )

    def test_words_drop_punctuation_and_split_on_dashes(self) -> None:
        self.assertEqual(align_lyrics.words_of("Still—"), ["still"])
        self.assertEqual(
            align_lyrics.words_of("“Do not mistake the wanting"),
            ["do", "not", "mistake", "the", "wanting"],
        )
        self.assertEqual(align_lyrics.words_of("don’t-stop"), ["don't", "stop"])
        self.assertEqual(align_lyrics.words_of("..."), [])


class Spans(unittest.TestCase):
    def timed(self, *lines: tuple[float, float, float]) -> list[dict]:
        return [{"words": [{"start": start, "end": end, "score": score}]} for start, end, score in lines]

    def test_a_cue_leads_its_first_word_and_holds_until_the_next_line(self) -> None:
        placed = align_lyrics.build(
            ["one", "two", "three"],
            self.timed((10.0, 11.0, 0.9), (11.5, 12.0, 0.8), (30.0, 31.0, 0.2)),
            60.0,
        )
        self.assertEqual(placed[0]["at"], 9.9)
        # Held to just before the next line opens, which is sooner than the hold.
        self.assertEqual(placed[0]["until"], round(placed[1]["at"] - align_lyrics.GAP_SECONDS, 3))
        # A long gap after: held for the hold, not to the next line.
        self.assertEqual(placed[1]["until"], 12.0 + align_lyrics.HOLD_SECONDS)
        # The last line never outlives the track.
        self.assertLessEqual(placed[2]["until"], 60.0)
        self.assertEqual(placed[2]["score"], 0.2)

    def test_a_line_at_zero_does_not_lead_into_negative_time(self) -> None:
        placed = align_lyrics.build(["first"], self.timed((0.02, 1.0, 1.0)), 10.0)
        self.assertEqual(placed[0]["at"], 0.0)


if __name__ == "__main__":
    unittest.main()
