#!/usr/bin/env python3
"""`.bridge.tsv`: the richest format crossing the helper/application boundary.

Protocol-map row 1 (`docs/archive/ASSIST_AUDIT_2026-08-29.md` §C): the bridge is
written by `tools/external_analysis.py` and read by
`crates/musializer-core/src/project/analysis_bridge.rs`, and until this file it
had **no Python unit test at all** — its only checks were `build_bridge`'s own
self-validation call and the shell gate's end-to-end import.

What is pinned here is the writer's side of the contract: the header bytes, the
field count of every record kind, the refusal of a kind the reader has no arm
for, and a build->parse round trip that reads the payload back out. The Rust
reader's own tests pin the same numbers from the other side; the point of two
independent pins on one format is that a drift has to move both to stay green.

Pure: no subprocess, no network, no audio device.
"""

from __future__ import annotations

import base64
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import external_analysis  # noqa: E402
from analysis_io import AnalysisValidationError  # noqa: E402


AUDIO_SHA = "b" * 64

# The audit's row-1 arity table, written here as data rather than as assertions
# spread through the tests: it is the thing that has to match the reader.
RECORD_ARITY = {"AUDIO": 3, "LYRIC": 7, "SECTION": 7, "SEMANTIC": 9, "SEMANTIC_NOTE": 3}


def plan(duration_seconds: float = 10.0) -> dict:
    """A scene plan `validate_scene_plan` accepts: full, gapless coverage."""
    half = duration_seconds / 2.0
    return {
        "schema_version": external_analysis.SCENE_PLAN_VERSION,
        "lane": "scene_plan",
        "audio": {"sha256": AUDIO_SHA, "duration_seconds": duration_seconds},
        "sections": [
            {
                "start_seconds": 0.0, "end_seconds": half,
                "recommended_scene": "spectrum", "transition_strength": 0.5,
                "reasons": ["opening"],
            },
            {
                "start_seconds": half, "end_seconds": duration_seconds,
                "recommended_scene": "clawd", "transition_strength": 0.25,
                "reasons": [],
            },
        ],
    }


def lyrics() -> dict:
    return {
        "lines": [
            {"start_seconds": 0.5, "end_seconds": 1.5, "text": "first line",
             "confidence": 0.875},
            # Non-ASCII on purpose: the payload is base64 of UTF-8, so a text
            # column that survives ASCII proves nothing about the encoding.
            {"start_seconds": 2.0, "end_seconds": 3.0, "text": "zweite Zeile — ja",
             "uncertain": True},
        ]
    }


def semantic_score() -> dict:
    return {
        "schema_version": "musializer.semantic-score/v1",
        "segments": [
            {"start_seconds": 0.0, "end_seconds": 4.0, "summary": "builds",
             "energy": 0.4, "tension": 0.6, "valence": -0.2, "confidence": 0.8},
            {"start_seconds": 4.0, "end_seconds": 9.0, "summary": "drops",
             "energy": 0.9, "tension": 0.3, "valence": 0.5, "confidence": 0.7},
        ],
    }


def semantic_notes() -> dict:
    return {"schema_version": external_analysis.SEMANTIC_NOTES_VERSION,
            "text": "a paragraph of listening notes"}


def rows(text: str) -> list[list[str]]:
    return [line.split("\t") for line in text.splitlines()]


def rebuild(record_rows: list[list[str]]) -> str:
    return "\n".join("\t".join(row) for row in record_rows) + "\n"


class BridgeHeader(unittest.TestCase):
    """The header is two fields, exactly, and version 1 exactly."""

    def test_the_first_line_is_the_module_constant(self) -> None:
        text = external_analysis.build_bridge(plan())
        self.assertEqual(text.splitlines()[0], external_analysis.BRIDGE_VERSION)
        self.assertEqual(text.splitlines()[0], "MUSIALIZER_BRIDGE\t1")

    def test_a_later_schema_version_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[0][1] = "2"
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_missing_header_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows[1:]))

    def test_a_header_with_a_third_field_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[0].append("1")
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_the_document_ends_in_a_newline(self) -> None:
        # The Rust reader refuses input whose last byte is not `\n`
        # (`analysis_bridge.rs`'s `InputSize` arm), so this is a contract, not
        # a formatting habit.
        self.assertTrue(external_analysis.build_bridge(plan()).endswith("\n"))


class BridgeArity(unittest.TestCase):
    """Every record kind's field count, and the refusal of any other count."""

    def built(self) -> str:
        return external_analysis.build_bridge(
            plan(), lyrics=lyrics(), semantic=semantic_score())

    def test_each_written_record_has_its_documented_arity(self) -> None:
        seen: set[str] = set()
        for row in rows(self.built())[1:]:
            self.assertIn(row[0], RECORD_ARITY)
            self.assertEqual(len(row), RECORD_ARITY[row[0]], row[0])
            seen.add(row[0])
        self.assertEqual(seen, {"AUDIO", "LYRIC", "SECTION", "SEMANTIC"})

    def test_the_note_record_has_its_documented_arity(self) -> None:
        # `SEMANTIC_NOTE` is the other branch of the same argument: a run has
        # either scored segments or free notes, never both.
        note_rows = rows(external_analysis.build_bridge(
            plan(), semantic=semantic_notes()))
        notes = [row for row in note_rows if row[0] == "SEMANTIC_NOTE"]
        self.assertEqual(len(notes), 1)
        self.assertEqual(len(notes[0]), RECORD_ARITY["SEMANTIC_NOTE"])

    def test_a_short_record_is_refused_for_every_kind(self) -> None:
        for kind in ("AUDIO", "LYRIC", "SECTION", "SEMANTIC"):
            with self.subTest(kind=kind):
                record_rows = rows(self.built())
                index = next(i for i, row in enumerate(record_rows) if row[0] == kind)
                record_rows[index] = record_rows[index][:-1]
                with self.assertRaises(AnalysisValidationError):
                    external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_long_record_is_refused_for_every_kind(self) -> None:
        for kind in ("AUDIO", "LYRIC", "SECTION", "SEMANTIC"):
            with self.subTest(kind=kind):
                record_rows = rows(self.built())
                index = next(i for i, row in enumerate(record_rows) if row[0] == kind)
                record_rows[index].append("extra")
                with self.assertRaises(AnalysisValidationError):
                    external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_short_semantic_note_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(
            plan(), semantic=semantic_notes()))
        index = next(i for i, row in enumerate(record_rows)
                     if row[0] == "SEMANTIC_NOTE")
        record_rows[index] = record_rows[index][:-1]
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))


class BridgeRecordKinds(unittest.TestCase):
    def test_an_unknown_record_kind_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows.append(["CHAPTER", "17", "0", "1000", "spectrum", "500",
                            base64.b64encode(b"[]").decode("ascii")])
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_lowercase_known_kind_is_still_unknown(self) -> None:
        # The tokens are bytes in a file, not names to be matched loosely.
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[-1][0] = "section"
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_the_audio_record_must_come_first(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[1], record_rows[2] = record_rows[2], record_rows[1]
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))


class BridgeRoundTrip(unittest.TestCase):
    """build -> parse, reading the payload back out of the wire form."""

    def test_a_full_bridge_round_trips(self) -> None:
        source_plan, source_lyrics = plan(), lyrics()
        source_semantic = semantic_score()
        text = external_analysis.build_bridge(
            source_plan, lyrics=source_lyrics, semantic=source_semantic)
        parsed = external_analysis.parse_bridge(text)

        self.assertEqual(parsed[0], ["MUSIALIZER_BRIDGE", "1"])
        self.assertEqual(parsed[1], ["AUDIO", AUDIO_SHA, "10000"])

        lyric_rows = [row for row in parsed if row[0] == "LYRIC"]
        self.assertEqual(len(lyric_rows), 2)
        self.assertEqual([row[2] for row in lyric_rows], ["500", "2000"])
        self.assertEqual([row[3] for row in lyric_rows], ["1500", "3000"])
        # A confidence of `None` is -1 on the wire; 0.875 is 875 milli-units.
        self.assertEqual([row[4] for row in lyric_rows], ["875", "-1"])
        self.assertEqual([row[5] for row in lyric_rows], ["none", "uncertain"])
        self.assertEqual(
            [base64.b64decode(row[6]).decode("utf-8") for row in lyric_rows],
            [line["text"] for line in source_lyrics["lines"]])

        section_rows = [row for row in parsed if row[0] == "SECTION"]
        self.assertEqual([row[4] for row in section_rows], ["spectrum", "clawd"])
        self.assertEqual([row[5] for row in section_rows], ["500", "250"])
        self.assertEqual(
            [json.loads(base64.b64decode(row[6]).decode("utf-8"))
             for row in section_rows],
            [section["reasons"] for section in source_plan["sections"]])

        semantic_rows = [row for row in parsed if row[0] == "SEMANTIC"]
        self.assertEqual([row[4:8] for row in semantic_rows],
                         [["400", "600", "-200", "800"],
                          ["900", "300", "500", "700"]])
        self.assertEqual(
            [base64.b64decode(row[8]).decode("utf-8") for row in semantic_rows],
            ["builds", "drops"])

    def test_a_sections_only_bridge_round_trips(self) -> None:
        # The `--mode sections` shape the shell gate exercises end to end.
        parsed = external_analysis.parse_bridge(
            external_analysis.build_bridge(plan()))
        self.assertEqual([row[0] for row in parsed],
                         ["MUSIALIZER_BRIDGE", "AUDIO", "SECTION", "SECTION"])

    def test_stable_ids_are_unique_and_nonzero(self) -> None:
        parsed = external_analysis.parse_bridge(external_analysis.build_bridge(
            plan(), lyrics=lyrics(), semantic=semantic_score()))
        ids = [int(row[1]) for row in parsed
               if row[0] in {"LYRIC", "SECTION", "SEMANTIC"}]
        self.assertEqual(len(ids), 6)
        self.assertNotIn(0, ids)
        self.assertEqual(len(set(ids)), len(ids))

    def test_the_same_input_builds_the_same_bytes(self) -> None:
        # The ids are a digest of (kind, index, start, text), so two runs of one
        # helper over one track must produce a byte-identical bridge.
        first = external_analysis.build_bridge(
            plan(), lyrics=lyrics(), semantic=semantic_score())
        second = external_analysis.build_bridge(
            plan(), lyrics=lyrics(), semantic=semantic_score())
        self.assertEqual(first, second)

    def test_a_semantic_document_of_an_unknown_schema_writes_nothing(self) -> None:
        # Neither branch matches, so the bridge carries sections alone rather
        # than a record shaped by a guess.
        text = external_analysis.build_bridge(
            plan(), semantic={"schema_version": "musializer.semantic-score/v2",
                              "segments": semantic_score()["segments"]})
        self.assertNotIn("SEMANTIC", text)


class BridgeFieldValidation(unittest.TestCase):
    """The value checks the arity table cannot express."""

    def test_a_non_hex_audio_digest_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[1][1] = "g" * 64
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_short_audio_digest_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[1][1] = "b" * 63
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_record_past_the_audio_duration_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[-1][3] = "10001"
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_out_of_order_records_of_one_kind_are_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan(), lyrics=lyrics()))
        first = next(i for i, row in enumerate(record_rows) if row[0] == "LYRIC")
        record_rows[first], record_rows[first + 1] = (
            record_rows[first + 1], record_rows[first])
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_duplicate_stable_id_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan(), lyrics=lyrics()))
        first = next(i for i, row in enumerate(record_rows) if row[0] == "LYRIC")
        record_rows[first + 1][1] = record_rows[first][1]
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_an_unknown_scene_token_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[2][4] = "kaleidoscope"
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_lyric_flag_outside_the_pair_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan(), lyrics=lyrics()))
        index = next(i for i, row in enumerate(record_rows) if row[0] == "LYRIC")
        record_rows[index][5] = "maybe"
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_payload_that_is_not_base64_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[2][6] = "not base64!"
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_non_numeric_field_is_refused_with_the_typed_error(self) -> None:
        # `AnalysisValidationError` is what every caller of this module catches;
        # a bare `ValueError` out of `int()` escapes as a traceback instead.
        for record, column in (("AUDIO", 2), ("SECTION", 2), ("SECTION", 5)):
            with self.subTest(record=record, column=column):
                record_rows = rows(external_analysis.build_bridge(plan()))
                index = next(i for i, row in enumerate(record_rows)
                             if row[0] == record)
                record_rows[index][column] = "soon"
                with self.assertRaises(AnalysisValidationError):
                    external_analysis.parse_bridge(rebuild(record_rows))

    def test_a_zero_duration_audio_record_is_refused(self) -> None:
        record_rows = rows(external_analysis.build_bridge(plan()))
        record_rows[1][2] = "0"
        with self.assertRaises(AnalysisValidationError):
            external_analysis.parse_bridge(rebuild(record_rows))


if __name__ == "__main__":
    unittest.main()
