#!/usr/bin/env python3
"""Pure regression tests for anchor→block lyric localization (tranche LT1).

Everything here runs without torch, a GPU or audio: the acoustic pass in
``tools/anchor_block_align.py`` only supplies per-line decisions, and every
decision *about* those decisions is made in ``tools/lyric_anchor_block.py``.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import external_analysis  # noqa: E402
import lyric_align  # noqa: E402
import lyric_anchor_block as anchor_block  # noqa: E402
from analysis_io import AnalysisValidationError  # noqa: E402


def _lines(*texts: str) -> list[dict[str, object]]:
    """Authored lines in the shape ``classify_reference_lines`` produces."""
    return [
        {"index": index, "kind": "lyric", "display": text,
         "tokens": lyric_align.normalize_tokens(text)}
        for index, text in enumerate(texts)
    ]


def _plan(lines: list[dict[str, object]]) -> dict[str, object]:
    return {
        "lines": lines,
        "anchors": [],
        "blocks": [],
        "unreliable_evidence": [],
        "evidence_token_count": 0,
        "authored_token_count": sum(len(line["tokens"]) for line in lines),
    }


def _decision(start: float, end: float, score: float = 0.4) -> dict[str, object]:
    return {
        "status": "aligned" if score >= anchor_block.MINIMUM_ALIGNMENT_SCORE
        else "weak",
        "score": score,
        "acoustic_start_seconds": start,
        "acoustic_end_seconds": end,
        "word_alignments": [],
        "first_word_score": score,
        "last_word_score": score,
    }


class CoverageInvariantTests(unittest.TestCase):
    """Invariant 1: an unlocatable line is unresolved, never absent."""

    def test_a_line_the_acoustics_could_not_place_becomes_unresolved(self) -> None:
        lines = _lines("first line here", "second line here", "third line here")
        document = anchor_block.assemble_document(
            _plan(lines),
            {0: _decision(1.0, 3.0), 2: _decision(9.0, 11.0)},
            {0: 0, 2: 0}, {}, audio_duration=30.0,
        )
        self.assertEqual([cue["reference_line_index"] for cue in document["lines"]],
                         [0, 2])
        self.assertEqual([entry["reference_line_index"]
                          for entry in document["unresolved"]], [1])
        self.assertEqual(document["unresolved"][0]["reason"], "no_alignment")
        # The count that matters is cues + unresolved, not cues.
        self.assertEqual(
            len(document["lines"]) + len(document["unresolved"]), len(lines))

    def test_every_unresolved_line_is_also_a_review_flag(self) -> None:
        lines = _lines("first line here", "second line here")
        document = anchor_block.assemble_document(
            _plan(lines), {0: _decision(1.0, 3.0)}, {0: 0}, {},
            audio_duration=30.0)
        self.assertEqual(
            [(flag["reference_line_index"], flag["flag"])
             for flag in document["review_flags"]],
            [(1, "unresolved")])

    def test_a_dropped_line_is_refused_rather_than_shipped(self) -> None:
        """Negative control for the bug this tranche replaced.

        The previous pipeline omitted authored lines *silently*, so the
        interesting failure is one nothing complains about. Doctoring a
        finished document to drop one line must fail the guard even though the
        document is otherwise perfectly well formed.
        """
        lines = _lines("first line here", "second line here", "third line here")
        document = anchor_block.assemble_document(
            _plan(lines),
            {0: _decision(1.0, 3.0), 1: _decision(4.0, 6.0),
             2: _decision(9.0, 11.0)},
            {0: 0, 1: 0, 2: 0}, {}, audio_duration=30.0,
        )
        # Assembly itself is clean, so the control is not passing by accident.
        anchor_block.validate_full_coverage(document, lines)
        doctored = dict(document)
        doctored["lines"] = [cue for cue in document["lines"]
                             if cue["reference_line_index"] != 1]
        with self.assertRaises(AnalysisValidationError) as caught:
            anchor_block.validate_full_coverage(doctored, lines)
        self.assertIn("lost authored lines", str(caught.exception))

    def test_a_line_too_long_for_a_caption_is_unresolved_by_name(self) -> None:
        lines = _lines("short line here",
                       "x" * (lyric_align.MAX_CUE_TEXT_BYTES + 1))
        document = anchor_block.assemble_document(
            _plan(lines), {0: _decision(1.0, 3.0), 1: _decision(4.0, 6.0)},
            {0: 0, 1: 0}, {}, audio_duration=30.0)
        self.assertEqual([entry["reason"] for entry in document["unresolved"]],
                         ["line_too_long"])


class ShippedSchemaTests(unittest.TestCase):
    """The lane the helper writes must match the schema the bundle ships.

    Nothing validates `schemas/lyric-sync-v1.schema.json` at runtime, which is
    exactly why it can drift: the previous aligned artifact already carried
    fields the schema closed against. This keeps the additive LT1 fields
    honest without adding a jsonschema dependency.
    """

    SCHEMA = json.loads(
        (ROOT / "schemas" / "lyric-sync-v1.schema.json").read_text("utf-8"))

    def _document(self) -> dict[str, object]:
        lines = _lines("first line here", "second line here")
        plan = _plan(lines)
        plan["structure"] = [{"reference_line_index": 9, "kind": "section",
                              "text": "[Chorus]"}]
        plan["performed_candidates"] = [{
            "start_seconds": 30.0, "end_seconds": 31.0, "text": "yeah",
            "confidence": 0.7, "source": "whisper-unmatched",
            "uncertain": True,
            "reason": "heard outside every authored lyric placement",
        }]
        document = anchor_block.assemble_document(
            plan, {0: _decision(10.0, 12.0)}, {0: 0}, {0: (9.5, 11.5)},
            audio_duration=60.0)
        # The fields external_analysis adds around the aligner's output.
        document["audio"] = {"sha256": "0" * 64, "duration_seconds": 60.0}
        document["reference"] = {"source": "embedded:lyrics-eng",
                                 "sha256": "1" * 64}
        document["provenance"] = {}
        document["generation"] = {}
        document["timing_refinement"] = {}
        return document

    def test_performed_candidates_survive_acoustic_refinement_separately(self) -> None:
        document = self._document()
        self.assertEqual(document["performed_candidates"][0]["text"], "yeah")
        self.assertEqual(document["statistics"]["performed_candidates"], 1)

    def test_no_field_falls_outside_the_shipped_schema(self) -> None:
        document = self._document()
        self.assertEqual(set(document) - set(self.SCHEMA["properties"]), set())
        self.assertEqual(set(self.SCHEMA["required"]) - set(document), set())

    def test_cue_and_statistic_fields_are_all_declared(self) -> None:
        document = self._document()
        declared = set(self.SCHEMA["$defs"]["line"]["properties"])
        for cue in document["lines"]:
            self.assertEqual(set(cue) - declared, set())
            self.assertEqual(
                set(self.SCHEMA["$defs"]["line"]["required"]) - set(cue), set())
        statistics = self.SCHEMA["properties"]["statistics"]
        self.assertEqual(
            set(document["statistics"]) - set(statistics["properties"]), set())
        self.assertEqual(
            set(statistics["required"]) - set(document["statistics"]), set())


class RepeatedPhraseAbstentionTests(unittest.TestCase):
    """Invariant 2: an undecidable repeated occurrence abstains."""

    # Recorded from the pre-LT1 artifacts of `shut-up-cat`, the pinned example
    # in `docs/LYRICS_TIMING_BENCHMARK_RESULTS.md`. "The queen of the sky just
    # came through" is authored six times; the anchor lane placed reference
    # line 26 at 106.78 s and the coarse lane at 122.27 s, and the operator
    # adjudicated both wrong with the true occurrence unlocated. 122.27 s is
    # nearer the *next* occurrence's 132.81 s than its own, which is the shape
    # of "the global ordering did not decide this".
    QUEEN_ANCHOR = {3: 24.16, 16: 58.65, 18: 70.03, 20: 81.46,
                    26: 106.78, 33: 132.81}
    QUEEN_COARSE = {3: 23.56, 16: 56.53, 18: 66.87, 20: 84.01,
                    26: 122.27, 33: 130.32}

    def _queen(self, anchor: dict[int, float], coarse: dict[int, float]):
        indices = sorted(anchor)
        lines = [{"index": index, "kind": "lyric",
                  "display": "The queen of the sky just came through",
                  "tokens": lyric_align.normalize_tokens(
                      "The queen of the sky just came through")}
                 for index in indices]
        placements = {position: (anchor[index], anchor[index] + 3.5)
                      for position, index in enumerate(indices)}
        proposals = {index: (coarse[index], coarse[index] + 3.5)
                     for index in indices}
        return lines, placements, proposals

    def test_the_pinned_repeated_chorus_line_abstains(self) -> None:
        lines, placements, coarse = self._queen(
            self.QUEEN_ANCHOR, self.QUEEN_COARSE)
        abstentions = anchor_block.repeated_phrase_abstentions(
            lines, placements, coarse)
        self.assertEqual(
            {lines[position]["index"]: reason
             for position, reason in abstentions.items()},
            {26: "repeated_occurrence_ambiguous"})

    def test_agreeing_views_keep_every_occurrence(self) -> None:
        """The same six lines after the Whisper repetition loop was contained.

        `--max-context 0` moved the coarse proposal for line 26 from 122.27 s
        to 108.27 s, which is nearer its own placement than any sibling's. The
        two views then agree about the *occurrence*, so nothing abstains — the
        criterion must not fire merely because the text repeats.
        """
        coarse = {**self.QUEEN_COARSE, 16: 58.13, 18: 66.78, 20: 78.22,
                  26: 108.27, 33: 131.24}
        anchor = {**self.QUEEN_ANCHOR, 26: 110.05}
        lines, placements, proposals = self._queen(anchor, coarse)
        self.assertEqual(
            anchor_block.repeated_phrase_abstentions(
                lines, placements, proposals), {})

    def test_a_unique_line_never_abstains_however_far_the_views_disagree(self) -> None:
        lines = _lines("a completely unique authored line")
        abstentions = anchor_block.repeated_phrase_abstentions(
            lines, {0: (10.0, 13.0)}, {0: (90.0, 93.0)})
        self.assertEqual(abstentions, {})

    def test_two_occurrences_on_one_acoustic_phrase_both_abstain(self) -> None:
        lines = _lines("shut up cat", "shut up cat")
        abstentions = anchor_block.repeated_phrase_abstentions(
            lines, {0: (40.0, 42.0), 1: (40.1, 42.1)}, {})
        self.assertEqual(abstentions,
                         {0: "repeated_phrase_collapsed",
                          1: "repeated_phrase_collapsed"})

    def test_an_abstention_is_unresolved_and_flagged_not_a_cue(self) -> None:
        lines = _lines("shut up cat", "shut up cat")
        document = anchor_block.assemble_document(
            _plan(lines), {0: _decision(40.0, 42.0), 1: _decision(40.1, 42.1)},
            {0: 0, 1: 0}, {}, audio_duration=160.0)
        self.assertEqual(document["lines"], [])
        self.assertEqual(len(document["unresolved"]), 2)
        self.assertTrue(all(entry["abstained"] for entry in document["unresolved"]))
        self.assertEqual({flag["flag"] for flag in document["review_flags"]},
                         {"unresolved"})


class ReviewFlagTests(unittest.TestCase):
    """Invariant 4: a flag is disagreement, never the aligner's own score."""

    def test_cross_view_disagreement_beyond_three_seconds_is_flagged(self) -> None:
        lines = _lines("first line here", "second line here")
        document = anchor_block.assemble_document(
            _plan(lines), {0: _decision(10.0, 12.0), 1: _decision(20.0, 22.0)},
            {0: 0, 1: 0},
            {0: (9.5, 11.5), 1: (14.0, 16.0)}, audio_duration=60.0)
        flags = {flag["reference_line_index"]: flag
                 for flag in document["review_flags"]}
        self.assertNotIn(0, flags)
        self.assertEqual(flags[1]["flag"], "coarse_disagreement")
        self.assertAlmostEqual(flags[1]["delta_seconds"], 5.9, places=6)
        self.assertTrue(document["lines"][1]["uncertain"])
        self.assertFalse(document["lines"][0]["uncertain"])

    def test_a_low_score_alone_neither_flags_nor_removes_a_cue(self) -> None:
        """The adjudication measured the score at 0.139 right vs 0.142 wrong.

        A `weak` line is therefore still a cue, and a strong one is still
        flagged when the views disagree. Confidence stays null rather than
        publishing a number the evidence does not support.
        """
        lines = _lines("first line here", "second line here")
        document = anchor_block.assemble_document(
            _plan(lines),
            {0: _decision(10.0, 12.0, score=0.02),
             1: _decision(20.0, 22.0, score=0.95)},
            {0: 0, 1: 0}, {0: (10.1, 12.1), 1: (14.0, 16.0)},
            audio_duration=60.0)
        self.assertEqual(len(document["lines"]), 2)
        self.assertEqual([cue["confidence"] for cue in document["lines"]],
                         [None, None])
        self.assertEqual([flag["reference_line_index"]
                          for flag in document["review_flags"]], [1])

    def test_an_occurrence_scale_disagreement_never_becomes_a_cue(self) -> None:
        """Negative control for the Groyper Idol +40 s chorus jump."""
        lines = _lines("a completely unique authored line")
        document = anchor_block.assemble_document(
            _plan(lines), {0: _decision(50.0, 54.0, score=0.95)}, {0: 0},
            {0: (10.0, 14.0)}, audio_duration=90.0)
        self.assertEqual(document["lines"], [])
        self.assertEqual(len(document["unresolved"]), 1)
        unresolved = document["unresolved"][0]
        self.assertEqual(
            unresolved["reason"], "cross_view_occurrence_ambiguous")
        self.assertAlmostEqual(unresolved["coarse_start_seconds"], 10.0)
        self.assertAlmostEqual(unresolved["acoustic_start_seconds"], 49.9)
        self.assertEqual(document["review_flags"][0]["flag"], "unresolved")

    def test_groyper_incident_chorus_is_four_abstentions_not_four_cues(self) -> None:
        """Pin the four measured v1 chorus jumps without distributing audio."""
        lines = _lines(
            "started like a now im just an idol",
            "room gets wild pass me the flare gun",
            "texas in july sweat through the denim",
            "dont go soft on me dont go soft on me",
        )
        fine_starts = (79.73, 83.83, 86.53, 88.92)
        coarse_starts = (37.30, 45.88, 51.28, 66.34)
        decisions = {
            position: _decision(start, start + 2.5, score=0.95)
            for position, start in enumerate(fine_starts)
        }
        coarse = {
            position: (start, start + 2.5)
            for position, start in enumerate(coarse_starts)
        }
        document = anchor_block.assemble_document(
            _plan(lines), decisions, {position: 0 for position in range(4)},
            coarse, audio_duration=247.36)
        self.assertEqual(document["lines"], [])
        self.assertEqual(len(document["unresolved"]), 4)
        self.assertEqual(
            {entry["reason"] for entry in document["unresolved"]},
            {"cross_view_occurrence_ambiguous"})
        self.assertTrue(all(
            abs(entry["acoustic_start_seconds"]
                - entry["coarse_start_seconds"])
            > anchor_block.MAXIMUM_ACCEPTED_DISAGREEMENT_SECONDS
            for entry in document["unresolved"]))

    def test_a_backwards_authored_pair_abstains_instead_of_being_sorted(self) -> None:
        lines = _lines("first line here", "second line here")
        document = anchor_block.assemble_document(
            _plan(lines), {0: _decision(30.0, 32.0), 1: _decision(10.0, 12.0)},
            {0: 0, 1: 0}, {}, audio_duration=60.0)
        self.assertEqual(document["lines"], [])
        self.assertEqual(
            [entry["reason"] for entry in document["unresolved"]],
            ["authored_order_ambiguous", "authored_order_ambiguous"])
        self.assertEqual(document["order_violations"], [])

    def test_equal_coarse_support_cannot_choose_a_survivor_from_reversed_lines(
            self) -> None:
        lines = _lines("first line here", "second line here")
        document = anchor_block.assemble_document(
            _plan(lines), {0: _decision(30.0, 32.0), 1: _decision(10.0, 12.0)},
            {0: 0, 1: 0}, {0: (29.9, 32.0), 1: (9.9, 12.0)},
            audio_duration=60.0)
        self.assertEqual(document["lines"], [])
        self.assertEqual(len(document["unresolved"]), 2)

    def test_a_coarse_local_refinement_needs_the_independent_block_view(
            self) -> None:
        proposal_start = 10.0
        self.assertFalse(anchor_block.coarse_local_refinement_allowed(
            None, proposal_start))
        self.assertFalse(anchor_block.coarse_local_refinement_allowed(
            _decision(50.0, 54.0, score=0.95), proposal_start))
        self.assertTrue(anchor_block.coarse_local_refinement_allowed(
            _decision(14.0, 18.0, score=0.02), proposal_start))

    def test_a_coarse_conditioned_path_disputed_by_global_search_abstains(
            self) -> None:
        lines = _lines("a unique but wrongly localized line")
        decision = _decision(50.0, 54.0, score=0.95)
        decision["occurrence_disputed"] = True
        decision["independent_global_candidate"] = _decision(
            10.0, 14.0, score=0.95)
        document = anchor_block.assemble_document(
            _plan(lines), {0: decision}, {0: 0}, {}, audio_duration=90.0)
        self.assertEqual(document["lines"], [])
        self.assertEqual(
            document["unresolved"][0]["reason"],
            "global_localization_disagreement")
        self.assertEqual(
            document["unresolved"][0]["independent_global_candidate"]
            ["acoustic_start_seconds"],
            10.0)

    def test_a_conditioned_miss_retains_its_successful_global_challenger(
            self) -> None:
        lines = _lines("a line missed by the conditioned path")
        decision = {"status": "no_alignment", "score": 0.0,
                    "independent_global_candidate": _decision(10.0, 14.0)}
        document = anchor_block.assemble_document(
            _plan(lines), {0: decision}, {}, {}, audio_duration=90.0)
        self.assertEqual(document["unresolved"][0]["reason"], "no_alignment")
        self.assertEqual(
            document["unresolved"][0]["independent_global_candidate"]
            ["acoustic_start_seconds"],
            10.0)

    def test_a_rare_anchor_can_resolve_global_occurrence_but_stays_flagged(
            self) -> None:
        lines = _lines("a rare exact anchored line")
        decision = _decision(50.0, 54.0, score=0.95)
        decision["global_disagreement_anchor_resolved"] = True
        decision["independent_global_candidate"] = _decision(
            10.0, 14.0, score=0.95)
        document = anchor_block.assemble_document(
            _plan(lines), {0: decision}, {0: 0}, {}, audio_duration=90.0)
        self.assertEqual(len(document["lines"]), 1)
        self.assertTrue(document["lines"][0]["uncertain"])
        self.assertEqual(
            document["review_flags"][0]["flag"],
            "global_disagreement_anchor_resolved")

    def test_an_anchor_at_another_time_cannot_validate_a_coarse_section(
            self) -> None:
        anchor = {"start_seconds": 10.0, "end_seconds": 12.0}
        candidate = _decision(100.0, 104.0, score=0.95)
        block = {"window_start": 94.0, "window_end": 113.0}
        self.assertFalse(anchor_block.anchor_supports_section_occurrence(
            anchor, candidate, block))
        anchored_candidate = _decision(11.0, 14.0, score=0.02)
        anchored_block = {"window_start": 5.0, "window_end": 30.0}
        self.assertTrue(anchor_block.anchor_supports_section_occurrence(
            anchor, anchored_candidate, anchored_block))


class BlockPartitionTests(unittest.TestCase):
    """Invariant 3: initial and terminal blocks are first-class."""

    def test_lines_after_the_last_anchor_get_a_terminal_block(self) -> None:
        lines = _lines(*[f"authored line number {n}" for n in range(6)])
        anchors = [{"authored_token_index": 4, "length": 4, "line_position": 1,
                    "start_seconds": 20.0, "end_seconds": 24.0}]
        blocks = anchor_block.build_blocks(
            lines, anchors, audio_duration=80.0)
        kinds = [block["kind"] for block in blocks]
        self.assertEqual(kinds, ["initial", "terminal"])
        self.assertEqual(blocks[0]["window_start"], 0.0)
        self.assertEqual(blocks[-1]["last_line"], len(lines) - 1)
        self.assertEqual(blocks[-1]["window_end"], 80.0)

    def test_no_anchors_is_one_searched_block_not_an_omission(self) -> None:
        lines = _lines("only line here", "and another one")
        blocks = anchor_block.build_blocks(lines, [], audio_duration=42.0)
        self.assertEqual(len(blocks), 1)
        self.assertEqual(blocks[0]["kind"], "unanchored")
        self.assertEqual((blocks[0]["first_line"], blocks[0]["last_line"]),
                         (0, len(lines) - 1))

    def test_section_coarse_evidence_bounds_a_sparse_anchor_tail(self) -> None:
        """The Groyper shape: evidence after the intro beats token splitting."""
        lines = _lines(
            "verse one a", "verse one b", "chorus one a", "chorus one b",
            "verse two a", "verse two b")
        for position, line in enumerate(lines):
            line["section_position"] = position // 2
        coarse = {
            0: (20.0, 23.0), 1: (24.0, 27.0),
            2: (40.0, 44.0), 3: (48.0, 52.0),
            4: (100.0, 104.0), 5: (108.0, 112.0),
        }
        blocks = anchor_block.build_blocks(
            lines, [], coarse=coarse, audio_duration=180.0)
        self.assertGreaterEqual(len(blocks), 3)
        self.assertTrue(all(block["kind"] == "section-evidence"
                            for block in blocks))
        chorus = next(block for block in blocks
                       if block["first_line"] == 2)
        self.assertEqual((chorus["first_line"], chorus["last_line"]), (2, 3))
        self.assertLess(chorus["window_start"], 40.0)
        self.assertGreater(chorus["window_end"], 52.0)
        # Crucially, its search cannot drift into verse two at 100 s.
        self.assertLess(chorus["window_end"], 90.0)

    def test_groyper_chorus_window_excludes_the_measured_eighty_second_jump(
            self) -> None:
        """Pin the real section density, not a conveniently spaced toy."""
        lines = _lines(*[f"line {position}" for position in range(18)])
        section_sizes = (4, 4, 4, 4, 1, 1)
        position = 0
        for section, size in enumerate(section_sizes):
            for _ in range(size):
                lines[position]["section_position"] = section
                position += 1
        coarse = {
            0: (8.86, 19.23), 1: (19.96, 26.04),
            2: (26.04, 29.60), 3: (29.60, 36.86),
            4: (37.30, 45.88), 5: (45.88, 51.16),
            6: (51.28, 58.41), 7: (66.34, 73.65),
            10: (120.05, 121.64), 11: (122.08, 127.51),
            14: (168.40, 171.13), 15: (171.13, 172.32),
            16: (172.32, 174.33), 17: (174.33, 194.68),
        }
        blocks = anchor_block.build_blocks(
            lines, [], coarse=coarse, audio_duration=247.36)
        chorus = next(block for block in blocks
                       if block["first_line"] == 4)
        self.assertEqual((chorus["first_line"], chorus["last_line"]), (4, 7))
        self.assertTrue(chorus["fully_coarse_evidenced"])
        self.assertLess(chorus["window_start"], 37.30)
        self.assertGreater(chorus["window_end"], 73.65)
        self.assertLess(chorus["window_end"], 80.58)

    def test_estimated_coarse_rows_are_review_only_not_window_evidence(
            self) -> None:
        sync = {
            "lines": [
                {"reference_line_index": 2, "start_seconds": 10.0,
                 "end_seconds": 12.0, "confidence": 1.0,
                 "estimated": False},
                {"reference_line_index": 3, "start_seconds": 40.0,
                 "end_seconds": 42.0, "confidence": None,
                 "estimated": True},
                {"reference_line_index": 4, "start_seconds": 70.0,
                 "end_seconds": 72.0, "confidence": 0.3,
                 "estimated": False},
            ]
        }
        self.assertEqual(set(anchor_block.coarse_proposals(sync)), {2, 3, 4})
        self.assertEqual(
            set(anchor_block.coarse_proposals(sync, trusted_only=True)), {2})

    def test_every_authored_line_belongs_to_some_block(self) -> None:
        lines = _lines(*[f"authored line number {n}" for n in range(9)])
        anchors = [
            {"authored_token_index": 4, "length": 4, "line_position": 1,
             "start_seconds": 10.0, "end_seconds": 14.0},
            {"authored_token_index": 24, "length": 4, "line_position": 6,
             "start_seconds": 60.0, "end_seconds": 64.0},
        ]
        blocks = anchor_block.build_blocks(lines, anchors, audio_duration=90.0)
        covered = {position for block in blocks
                   for position in range(block["first_line"],
                                         block["last_line"] + 1)}
        self.assertEqual(covered, set(range(len(lines))))

    def test_an_anchor_that_goes_backwards_is_dropped(self) -> None:
        candidates = [
            {"length": 4, "start_seconds": 10.0, "end_seconds": 14.0},
            {"length": 3, "start_seconds": 5.0, "end_seconds": 7.0},
            {"length": 4, "start_seconds": 40.0, "end_seconds": 44.0},
        ]
        kept = anchor_block.monotonic_subset(candidates)
        self.assertEqual([item["start_seconds"] for item in kept], [10.0, 40.0])


class WhisperEvidencePassTests(unittest.TestCase):
    """whisper-cli 1.8.6: `--max-context 0`, and VAD only when configured."""

    def _argv(self, **environment: str) -> list[str]:
        previous = {key: os.environ.get(key) for key in environment}
        os.environ.update(environment)
        try:
            # The degraded-VAD path prints an explanation; the test asserts the
            # argv, not the console.
            with contextlib.redirect_stderr(io.StringIO()):
                vad = external_analysis.whisper_vad_model()
            _, whisper = external_analysis.whisper_request(
                Path("song.mp3"), whisper_bin=Path("whisper-cli"),
                model=Path("ggml-large-v3-turbo.bin"), language="en",
                dtw_model=None, ffmpeg="ffmpeg",
                output_prefix=Path("/tmp/x"), vad_model=vad,
            )
            return whisper
        finally:
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value

    def test_text_conditioning_is_disabled_on_every_run(self) -> None:
        argv = self._argv(MUSIALIZER_WHISPER_VAD_MODEL="")
        self.assertIn("-mc", argv)
        self.assertEqual(argv[argv.index("-mc") + 1], "0")

    def test_vad_is_off_unless_a_model_is_named(self) -> None:
        argv = self._argv(MUSIALIZER_WHISPER_VAD_MODEL="")
        self.assertNotIn("--vad", argv)

    def test_a_missing_vad_model_degrades_instead_of_failing(self) -> None:
        argv = self._argv(
            MUSIALIZER_WHISPER_VAD_MODEL="/nonexistent/musializer/silero.bin")
        self.assertNotIn("--vad", argv)
        self.assertIn("-mc", argv)

    def test_the_conditioning_policy_is_part_of_the_cache_identity(self) -> None:
        settings = external_analysis._whisper_request_settings(
            language="en", dtw_model=None, model_sha256="0" * 64,
            vad_model=None)
        self.assertEqual(settings["text_conditioning"],
                         external_analysis.WHISPER_TEXT_CONDITIONING)
        self.assertIsNone(settings["vad_model_sha256"])


class LocalizationCacheIdentityTests(unittest.TestCase):
    """A lane from the old localization policy must not be reused."""

    def _document(self, **overrides: object) -> dict[str, object]:
        document = {
            "localization_policy": anchor_block.LOCALIZATION_POLICY,
            "localization_policy_version": anchor_block.LOCALIZATION_POLICY_VERSION,
            "timing_refinement": {
                "adapter": "tools/anchor_block_align.py",
                "alignment_version": anchor_block.ALIGNMENT_VERSION,
                "model": "torchaudio.pipelines.MMS_FA",
                "localization_policy": anchor_block.LOCALIZATION_POLICY,
                "localization_policy_version": (
                    anchor_block.LOCALIZATION_POLICY_VERSION),
            },
            "generation": {
                "whisper_sha256": "a" * 64,
                "coarse_sha256": "b" * 64,
                "reference_sha256": "c" * 64,
            },
        }
        document.update(overrides)
        return document

    def _accepts(self, document: dict[str, object]) -> bool:
        return external_analysis._anchor_block_cache_accepts(
            document, whisper_sha256="a" * 64, coarse_sha256="b" * 64,
            reference_sha256="c" * 64)

    def test_a_matching_lane_is_reused(self) -> None:
        self.assertTrue(self._accepts(self._document()))

    def test_an_older_policy_version_is_regenerated(self) -> None:
        self.assertFalse(self._accepts(self._document(
            localization_policy_version="0")))

    def test_an_older_acoustic_request_is_regenerated(self) -> None:
        document = self._document()
        document["timing_refinement"]["alignment_version"] = "0"
        self.assertFalse(self._accepts(document))

    def test_the_previous_per_cue_lane_is_not_accepted(self) -> None:
        legacy = {
            "timing_refinement": {
                "adapter": "tools/force_align_lyrics.py",
                "alignment_version": "13",
                "model": "torchaudio.pipelines.MMS_FA",
                "source_sha256": "b" * 64,
            },
        }
        self.assertFalse(self._accepts(legacy))

    def test_a_different_authored_text_is_regenerated(self) -> None:
        document = self._document()
        document["generation"]["reference_sha256"] = "d" * 64
        self.assertFalse(self._accepts(document))

    def test_the_coarse_lane_records_its_demoted_role(self) -> None:
        self.assertEqual(external_analysis._sync_request_settings()["role"],
                         "coarse_proposal")


class AssistManifestLaneKeys(unittest.TestCase):
    """The manifest's LT1 review keys belong to a run that had a lyrics lane.

    Review LT1-R, R1. The cache folder is keyed by audio, not by mode, so a
    Sections run's manifest sits beside the ``lyrics.aligned.json`` an earlier
    full run wrote. ``lyrics_unresolved: 0`` in that manifest is enough for the
    panel to treat the folder as an LT1 job and read the stale document.
    """

    def _manifest(self, mode: str, lyrics: dict[str, object] | None) -> dict[str, object]:
        return external_analysis.build_assist_manifest(
            mode=mode, audio_sha="a" * 64, measured_duration=60.0,
            cache_status={}, paths={"manifest": Path("/job/assist-manifest.json"),
                                    "aligned": Path("/job/lyrics.aligned.json")},
            plan={"sections": [{}, {}]}, lyrics=lyrics, semantic=None,
        )

    def _lyrics(self) -> dict[str, object]:
        return {
            "lane": "lyric_sync", "lines": [{}, {}], "unmatched": [],
            "unresolved": [{}, {}], "review_flags": [{}, {}, {}],
            "localization_policy": "anchor-block-mms",
            "localization_policy_version": "3",
            "reference": {"source": "embedded:lyrics-eng",
                          "sha256": "8" * 64},
            "statistics": {"reference_lines": 5},
        }

    def test_a_run_without_a_lyrics_lane_writes_no_review_keys(self) -> None:
        manifest = self._manifest("sections", None)
        counts = manifest["result_counts"]
        self.assertNotIn("lyrics_unresolved", counts)
        self.assertNotIn("lyrics_review_flags", counts)
        self.assertNotIn("lyric_localization", manifest)
        # And it stays legible as a manifest: the lane-independent counts are
        # still there, so nothing else about the folder changes.
        self.assertEqual(counts["sections"], 2)
        self.assertEqual(counts["lyrics"], 0)
        self.assertNotIn("lyrics", manifest["provenance_streams"])

    def test_a_lyrics_run_writes_all_three_markers(self) -> None:
        manifest = self._manifest("lyrics", self._lyrics())
        self.assertEqual(manifest["result_counts"]["lyrics_unresolved"], 2)
        self.assertEqual(manifest["result_counts"]["lyrics_review_flags"], 3)
        self.assertEqual(manifest["lyric_localization"],
                         {"policy": "anchor-block-mms", "policy_version": "3"})
        self.assertEqual(manifest["lyric_reference"], {
            "source": "embedded:lyrics-eng", "sha256": "8" * 64,
            "alignable_lines": 5,
        })
        self.assertIn("lyrics", manifest["provenance_streams"])

    def test_a_lyrics_run_that_placed_everything_still_says_zero(self) -> None:
        # Zero is a claim only a run with the lane may make, and it must keep
        # making it: "every line placed" is a different answer from "no review".
        lyrics = self._lyrics()
        lyrics["unresolved"] = []
        lyrics["review_flags"] = []
        counts = self._manifest("all", lyrics)["result_counts"]
        self.assertEqual(counts["lyrics_unresolved"], 0)
        self.assertEqual(counts["lyrics_review_flags"], 0)


if __name__ == "__main__":
    unittest.main()
