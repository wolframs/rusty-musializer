#!/usr/bin/env python3
"""Offline tests for the MiMo description benchmark harness.

Every fixture response in this file is **synthetic** — written by hand to
exercise a scoring rule, never copied from a model. No test opens a socket,
reads a credential, or requires the operator's music to be present.

Where a rule could pass for the wrong reason, the test pairs the positive case
with the perturbation that must fail it. The lyric scorer, the tempo octave
rule and the concreteness counter all carry one, because each of them would
otherwise be satisfied by a scorer that returned a constant.
"""

from __future__ import annotations

import base64
import json
import math
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

from mimo_bench import (  # noqa: E402
    bench_io,
    cost,
    ground_truth,
    matrix,
    prompts,
    report,
    request_build,
    schema,
    scorers,
)
from mimo_bench import run as bench_run  # noqa: E402


# ---------------------------------------------------------------------------
# Synthetic fixtures
# ---------------------------------------------------------------------------

def synthetic_document(**overrides: object) -> dict[str, object]:
    """A schema-valid completion. SYNTHETIC: hand-written, not model output."""
    document: dict[str, object] = {
        "summary": "A mid-tempo synth-pop excerpt built on a drum machine and a warm pad.",
        "instruments": [
            {"name": "drum machine", "timbre": "tight, clipped 808 kick with a short tail"},
            {"name": "synth bass", "timbre": "round sine with a slow portamento"},
            {"name": "synth pad", "timbre": "warm, heavily chorused"},
            {"name": "lead vocal", "timbre": "close-mic'd, breathy, doubled in the chorus"},
        ],
        "tempo_bpm": 112.0,
        "meter": "4/4",
        "key": "F#",
        "mode": "minor",
        "sections": [
            {"start_seconds": 30.0, "end_seconds": 46.0, "label": "verse"},
            {"start_seconds": 46.0, "end_seconds": 62.0, "label": "chorus"},
        ],
        "lyric_moments": [
            {"seconds": 44.5, "phrase": "the queen of the sky"},
            {"seconds": 70.2, "phrase": "just came through"},
        ],
        "harmony_notes": "i - VI - III - VII, with a suspended fourth over the chorus.",
        "production_notes": "Sidechain compression on the pad; a long plate reverb on the vocal.",
        "texture": ["glassy", "wide stereo", "dense in the chorus"],
        "feel": ["restless", "wistful", "propulsive"],
        "energy": 0.62,
        "tension": 0.4,
        "valence": 0.1,
        "uncertain": [],
    }
    document.update(overrides)
    return document


SYNTHETIC_PROSE = """\
The excerpt opens at 30 s with a drum machine pattern: a clipped 808 kick and a \
closed hi-hat on the offbeat. A synth bass enters at 34 s, round and slightly \
detuned. The tempo sits at 112 bpm in 4/4, and the key is F# minor throughout.
At 46 s a chorus arrives and the lead vocal doubles. "the queen of the sky" is \
sung around 44.5 s, and "just came through" follows at 70.2 s.
It is a nice track with a lovely, uplifting vibe.
This paragraph mentions nothing checkable at all whatsoever."""


def synthetic_record(
    *,
    cell: str,
    call_index: int = 0,
    structured: bool = True,
    text: str | None = None,
    chunk_start: float = 30.0,
    document: dict[str, object] | None = None,
) -> dict[str, object]:
    """One stored call. SYNTHETIC: no network response was ever involved."""
    body = text if text is not None else json.dumps(
        document if document is not None else synthetic_document())
    return {
        "cell": cell,
        "repeat": 1,
        "call_index": call_index,
        "kind": "audio",
        "turn": 1,
        "probe_run": 0,
        "structured": structured,
        "audio_seconds": 100.0,
        "chunk_index": call_index,
        "chunk_start_seconds": chunk_start,
        "chunk_end_seconds": chunk_start + 100.0,
        "estimated_text_input_tokens": 300,
        "identity": {"prompt_sha256": "synthetic", "schema_version": schema.SCHEMA_VERSION},
        "text": body,
        "status": "ok",
        "model_served": "xiaomi/mimo-v2.5",
        "provider_served": "synthetic-provider",
        "usage": {"prompt_tokens": 5300, "completion_tokens": 700},
    }


def synthetic_truth(**overrides: object) -> ground_truth.TrackTruth:
    truth = ground_truth.TrackTruth(
        slug="synthetic",
        excerpt_start=30.0,
        excerpt_end=130.0,
        measured_bpm=56.25,
        measured_bpm_confidence=0.4,
        excerpt_bpm=56.25,
        excerpt_bpm_confidence=0.41,
        excerpt_bpm_candidates=[56.25],
        sections=[(30.0, 46.0), (46.0, 63.0), (63.0, 130.0)],
        lyrics=[
            ground_truth.LyricTruth(18, "The queen of the sky just came through",
                                    70.0, 74.2, "adjudicated"),
            ground_truth.LyricTruth(23, "I make a wish when the sun drops low",
                                    93.0, 96.5, "adjudicated"),
        ],
    )
    for name, value in overrides.items():
        setattr(truth, name, value)
    return truth


# ---------------------------------------------------------------------------
# Chunking arithmetic
# ---------------------------------------------------------------------------


class ChunkArithmeticTests(unittest.TestCase):
    def test_every_chunking_covers_exactly_the_excerpt(self) -> None:
        for track in bench_io.TRACKS:
            for chunking in matrix.CHUNKINGS:
                spans = matrix.chunk_spans(track, chunking)
                self.assertEqual(len(spans), chunking.count, chunking.id)
                self.assertAlmostEqual(spans[0].start_seconds,
                                       track.excerpt_start_seconds)
                self.assertAlmostEqual(
                    spans[-1].end_seconds,
                    track.excerpt_start_seconds + bench_io.EXCERPT_SECONDS)
                total = sum(span.seconds for span in spans)
                self.assertAlmostEqual(total, bench_io.EXCERPT_SECONDS, places=6)

    def test_chunks_are_contiguous_and_non_overlapping(self) -> None:
        track = bench_io.primary_track()
        for chunking in matrix.CHUNKINGS:
            spans = matrix.chunk_spans(track, chunking)
            for previous, following in zip(spans, spans[1:]):
                self.assertAlmostEqual(previous.end_seconds, following.start_seconds)

    def test_each_span_knows_the_whole_count(self) -> None:
        track = bench_io.primary_track()
        spans = matrix.chunk_spans(track, matrix.CHUNKINGS_BY_ID["c10x10"])
        self.assertTrue(all(span.count == 10 for span in spans))

    def test_a_ragged_chunking_clips_rather_than_overruns(self) -> None:
        # 3 x 40 s does not divide 100 s: the axis is granularity at fixed
        # total duration, so the last chunk must be short, not the excerpt long.
        ragged = matrix.Chunking("c03x40", 3, 40.0)
        spans = matrix.chunk_spans(bench_io.primary_track(), ragged)
        self.assertEqual(len(spans), 3)
        self.assertAlmostEqual(spans[-1].seconds, 20.0)
        self.assertAlmostEqual(sum(span.seconds for span in spans), 100.0)


# ---------------------------------------------------------------------------
# The matrix
# ---------------------------------------------------------------------------


class MatrixTests(unittest.TestCase):
    def test_cells_are_deduplicated_across_blocks(self) -> None:
        cells = matrix.cells()
        self.assertEqual(len(cells), len({cell.id for cell in cells}))
        shared = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S1")
        self.assertEqual(set(shared.blocks), {"chunking", "shaping"})
        turn_one = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S0")
        self.assertEqual(set(turn_one.blocks), {"prompt", "shaping"})

    def test_every_block_is_represented(self) -> None:
        blocks: set[str] = set()
        for cell in matrix.cells():
            blocks.update(cell.blocks)
        self.assertEqual(blocks, {"chunking", "prompt", "shaping", "replication"})

    def test_dependent_arms_name_the_free_text_cell(self) -> None:
        for arm in ("S2a", "S2b", "S3"):
            cell = matrix.cell_by_id(f"shut-up-cat/c01x100/strict-checklist/{arm}")
            self.assertEqual(cell.depends_on,
                             "shut-up-cat/c01x100/strict-checklist/S0")

    def test_a_dependency_is_ordered_before_its_dependents(self) -> None:
        seen: set[str] = set()
        for cell, repeat in matrix.units(2):
            if cell.depends_on:
                self.assertIn((cell.depends_on, repeat), seen,
                              f"{cell.id} runs before its dependency")
            seen.add((cell.id, repeat))

    def test_call_counts_match_the_chunkings(self) -> None:
        track = bench_io.primary_track()
        for chunking in matrix.CHUNKINGS:
            cell = matrix.cell_by_id(
                f"{track.slug}/{chunking.id}/strict-checklist/S1")
            self.assertEqual(len(matrix.calls_for(cell, track)), chunking.count)

    def test_the_determinism_probe_is_the_only_multi_call_text_arm(self) -> None:
        track = bench_io.primary_track()
        probe = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S3")
        calls = matrix.calls_for(probe, track)
        self.assertEqual(len(calls), matrix.REFORMATTER_PROBE_RUNS)
        self.assertTrue(all(call.model == matrix.REFORMATTER_MODEL for call in calls))
        self.assertTrue(all(call.audio_seconds == 0.0 for call in calls))

    def test_only_s2a_resends_the_audio(self) -> None:
        track = bench_io.primary_track()
        resent = matrix.calls_for(
            matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S2a"), track)
        elided = matrix.calls_for(
            matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S2b"), track)
        self.assertEqual(resent[0].kind, "audio")
        self.assertEqual(elided[0].kind, "text")

    def test_summary_counts_are_self_consistent(self) -> None:
        summary = matrix.matrix_summary(3)
        self.assertEqual(summary["units"], summary["cells"] * 3)
        self.assertEqual(summary["total_calls"],
                         summary["audio_calls"] + summary["text_calls"])
        self.assertEqual(
            summary["total_calls"],
            sum(row["calls_per_repeat"] for row in summary["rows"]) * 3)


# ---------------------------------------------------------------------------
# Request construction
# ---------------------------------------------------------------------------


class RequestConstructionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.track = bench_io.primary_track()
        self.cell = matrix.cell_by_id("shut-up-cat/c05x20/strict-checklist/S1")
        self.call = matrix.calls_for(self.cell, self.track)[2]

    def test_audio_is_base64_and_round_trips(self) -> None:
        payload, _ = request_build.build_listen_request(
            self.cell, self.call, b"synthetic-audio-bytes")
        part = payload["messages"][0]["content"][1]
        self.assertEqual(part["type"], "input_audio")
        self.assertEqual(base64.b64decode(part["input_audio"]["data"]),
                         b"synthetic-audio-bytes")
        self.assertEqual(part["input_audio"]["format"], "mp3")

    def test_the_span_header_states_the_absolute_offset(self) -> None:
        payload, identity = request_build.build_listen_request(
            self.cell, self.call, b"x")
        text = payload["messages"][0]["content"][0]["text"]
        self.assertIn("chunk 3 of 5", text)
        self.assertIn("70.00 s", text)          # 30 + 2 * 20
        self.assertIn("90.00 s", text)
        self.assertEqual(identity["chunk_start_seconds"], 70.0)
        self.assertEqual(identity["time_frame_policy"], "absolute-offset-declared")

    def test_the_schema_is_attached_only_to_structured_arms(self) -> None:
        structured, _ = request_build.build_listen_request(
            self.cell, self.call, b"x")
        self.assertEqual(
            structured["response_format"]["json_schema"]["name"], schema.SCHEMA_NAME)
        free = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S0")
        payload, identity = request_build.build_listen_request(
            free, matrix.calls_for(free, self.track)[0], b"x")
        self.assertNotIn("response_format", payload)
        self.assertIsNone(identity["schema_version"])

    def test_the_second_turn_replays_the_conversation(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S2a")
        call = matrix.calls_for(cell, self.track)[0]
        payload, identity = request_build.build_second_turn_request(
            cell, call, "a synthetic description", b"audio")
        roles = [message["role"] for message in payload["messages"]]
        self.assertEqual(roles, ["user", "assistant", "user"])
        self.assertTrue(identity["audio_resent"])
        self.assertEqual(payload["messages"][1]["content"], "a synthetic description")

    def test_the_elided_second_turn_carries_no_audio(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S2b")
        call = matrix.calls_for(cell, self.track)[0]
        payload, identity = request_build.build_second_turn_request(
            cell, call, "a synthetic description", None)
        serialized = json.dumps(payload)
        self.assertNotIn("input_audio", serialized)
        self.assertFalse(identity["audio_resent"])
        self.assertIsNone(identity["audio_sha256"])

    def test_the_reformatter_sees_only_the_description(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S3")
        call = matrix.calls_for(cell, self.track)[0]
        payload, identity = request_build.build_reformatter_request(
            cell, call, "a synthetic description")
        self.assertEqual(payload["model"], matrix.REFORMATTER_MODEL)
        self.assertEqual(len(payload["messages"]), 1)
        self.assertIn("a synthetic description", payload["messages"][0]["content"])
        self.assertNotIn("input_audio", json.dumps(payload))
        self.assertIsNone(identity["audio_sha256"])

    def test_redaction_removes_the_base64_and_keeps_the_hash(self) -> None:
        payload, identity = request_build.build_listen_request(
            self.cell, self.call, b"secret-sounding-audio")
        redacted = request_build.redact(payload)
        data = redacted["messages"][0]["content"][1]["input_audio"]["data"]
        self.assertNotIn("c2VjcmV0", data)
        self.assertIn(identity["audio_sha256"], data)
        # The original payload must be untouched: it is what gets sent.
        self.assertNotIn("omitted",
                         payload["messages"][0]["content"][1]["input_audio"]["data"])

    def test_a_dry_run_dump_never_carries_a_credential(self) -> None:
        payload, identity = request_build.build_listen_request(
            self.cell, self.call, b"x")
        dump = json.dumps(request_build.request_dump(payload, identity))
        self.assertIn("<redacted", dump)
        self.assertNotIn("sk-or-v1", dump)

    def test_editing_a_prompt_changes_its_identity(self) -> None:
        first = prompts.prompt("strict-checklist").sha256
        second = bench_io.canonical_sha256(
            prompts.prompt("strict-checklist").text + " ")
        self.assertNotEqual(first, second)

    def test_the_two_registers_request_the_same_content(self) -> None:
        for item in prompts.CHECKLIST_ITEMS:
            self.assertIn(item, prompts.STRICT_CHECKLIST)
            self.assertIn(item, prompts.CASUAL_CHECKLIST)
        self.assertIn(":)", prompts.CASUAL_CHECKLIST)
        self.assertIn(":)", prompts.CASUAL_OPEN)
        self.assertNotIn(":)", prompts.STRICT_CHECKLIST)
        self.assertNotIn(":)", prompts.STRICT_OPEN)


# ---------------------------------------------------------------------------
# Cost projection
# ---------------------------------------------------------------------------


class CostTests(unittest.TestCase):
    def test_the_bracket_is_ordered_and_positive(self) -> None:
        projection = cost.project(3)
        totals = projection["totals"]
        self.assertGreater(totals["usd_low"], 0.0)
        self.assertGreaterEqual(totals["usd_high"], totals["usd_low"])
        for row in projection["rows"]:
            self.assertGreaterEqual(row["usd_high"], row["usd_low"])

    def test_cost_scales_with_repeats(self) -> None:
        one = cost.project(1)["totals"]
        three = cost.project(3)["totals"]
        self.assertAlmostEqual(three["usd_low"], one["usd_low"] * 3, places=9)
        self.assertEqual(three["calls"], one["calls"] * 3)

    def test_text_only_arms_are_billed_no_audio(self) -> None:
        rows = {row["cell"]: row for row in cost.project(1)["rows"]}
        self.assertEqual(rows["shut-up-cat/c01x100/strict-checklist/S2b"]["audio_seconds"], 0.0)
        self.assertEqual(rows["shut-up-cat/c01x100/strict-checklist/S3"]["audio_seconds"], 0.0)
        self.assertGreater(rows["shut-up-cat/c01x100/strict-checklist/S2a"]["audio_seconds"], 0.0)

    def test_every_chunking_bills_the_same_audio_duration(self) -> None:
        rows = {row["cell"]: row for row in cost.project(1)["rows"]}
        seconds = {
            rows[f"shut-up-cat/{chunking.id}/strict-checklist/S1"]["audio_seconds"]
            for chunking in matrix.CHUNKINGS
        }
        self.assertEqual(seconds, {100.0})

    def test_finer_chunking_costs_more_despite_equal_audio(self) -> None:
        # The whole point of the chunking axis: the audio is identical, so any
        # extra cost is the per-call prompt and completion overhead.
        rows = {row["cell"]: row for row in cost.project(1)["rows"]}
        coarse = rows["shut-up-cat/c01x100/strict-checklist/S1"]["usd_low"]
        fine = rows["shut-up-cat/c20x05/strict-checklist/S1"]["usd_low"]
        self.assertGreater(fine, coarse * 5)

    def test_usage_calibration_recovers_a_planted_rate(self) -> None:
        records = [
            {"kind": "audio", "audio_seconds": 100.0,
             "estimated_text_input_tokens": 300,
             "usage": {"prompt_tokens": 300 + 100 * 37}},
            {"kind": "audio", "audio_seconds": 20.0,
             "estimated_text_input_tokens": 300,
             "usage": {"prompt_tokens": 300 + 20 * 37}},
            {"kind": "text", "audio_seconds": 0.0, "usage": {"prompt_tokens": 900}},
        ]
        calibrated = cost.calibrate_from_usage(records)
        self.assertEqual(calibrated["samples"], 2)
        self.assertAlmostEqual(calibrated["audio_tokens_per_second"], 37.0, places=6)


# ---------------------------------------------------------------------------
# Schema validation
# ---------------------------------------------------------------------------


class SchemaTests(unittest.TestCase):
    def test_a_synthetic_document_validates(self) -> None:
        self.assertIsInstance(schema.validate(synthetic_document()), dict)

    def test_a_missing_field_is_refused(self) -> None:
        document = synthetic_document()
        del document["tempo_bpm"]
        with self.assertRaises(schema.SchemaViolation):
            schema.validate(document)

    def test_an_extra_field_is_refused(self) -> None:
        with self.assertRaises(schema.SchemaViolation):
            schema.validate(synthetic_document(genre="synth-pop"))

    def test_a_null_tempo_is_allowed_but_a_null_energy_is_not(self) -> None:
        schema.validate(synthetic_document(tempo_bpm=None))
        with self.assertRaises(schema.SchemaViolation):
            schema.validate(synthetic_document(energy=None))

    def test_strict_mode_requires_every_property(self) -> None:
        properties = set(schema.DESCRIPTION_SCHEMA["properties"])
        self.assertEqual(set(schema.DESCRIPTION_SCHEMA["required"]), properties)
        self.assertFalse(schema.DESCRIPTION_SCHEMA["additionalProperties"])


# ---------------------------------------------------------------------------
# Scorers
# ---------------------------------------------------------------------------


class TempoScoringTests(unittest.TestCase):
    def test_a_doubled_claim_is_octave_equivalent(self) -> None:
        claims = scorers.Claims(source="structured", tempo_bpm=[112.5])
        scored = scorers.score_tempo(claims, synthetic_truth())
        self.assertEqual(scored["octave"], 1)
        self.assertEqual(scored["exact"], 0)
        self.assertAlmostEqual(scored["accept_rate"], 1.0)

    def test_an_exact_claim_is_not_reported_as_an_octave(self) -> None:
        claims = scorers.Claims(source="structured", tempo_bpm=[56.0])
        scored = scorers.score_tempo(claims, synthetic_truth())
        self.assertEqual(scored["exact"], 1)

    def test_an_unrelated_tempo_is_wrong(self) -> None:
        claims = scorers.Claims(source="structured", tempo_bpm=[97.0])
        scored = scorers.score_tempo(claims, synthetic_truth())
        self.assertEqual(scored["wrong"], 1)
        self.assertEqual(scored["accept_rate"], 0.0)

    def test_no_claim_is_absent_not_wrong(self) -> None:
        scored = scorers.score_tempo(scorers.Claims(source="structured"),
                                     synthetic_truth())
        self.assertEqual(scored["status"], "absent")

    def test_the_matched_reference_is_named(self) -> None:
        truth = synthetic_truth(excerpt_bpm_candidates=[56.25, 175.78])
        scored = scorers.score_tempo(
            scorers.Claims(source="s", tempo_bpm=[176.0]), truth)
        self.assertEqual(scored["matched_against"], ["excerpt#2"])
        self.assertAlmostEqual(scored["verdicts"][0]["ratio"], 176.0 / 175.78,
                               places=3)

    def test_an_exact_match_wins_over_an_octave_match(self) -> None:
        # 112.5 is an octave of the measured 56.25 and exact against a
        # candidate; the verdict must be the stronger of the two.
        truth = synthetic_truth(excerpt_bpm_candidates=[112.5])
        scored = scorers.score_tempo(
            scorers.Claims(source="s", tempo_bpm=[112.5]), truth)
        self.assertEqual(scored["verdicts"][0]["verdict"], "exact")
        self.assertEqual(scored["verdicts"][0]["against"], "excerpt#1")

    def test_the_scorer_abstains_without_a_measurement(self) -> None:
        truth = synthetic_truth(measured_bpm=None, excerpt_bpm=None,
                                excerpt_bpm_candidates=[])
        scored = scorers.score_tempo(
            scorers.Claims(source="structured", tempo_bpm=[112.0]), truth)
        self.assertEqual(scored["status"], "abstain")


class KeyAndMeterScoringTests(unittest.TestCase):
    def test_key_abstains_until_the_operator_authors_it(self) -> None:
        claims = scorers.claims_from_structured(synthetic_document())
        self.assertEqual(scorers.score_key(claims, synthetic_truth())["status"],
                         "abstain")

    def test_exact_parallel_and_relative_are_distinguished(self) -> None:
        truth = synthetic_truth(key_status="adjudicated", key_tonic="F#",
                                key_mode="minor")
        exact = scorers.score_key(
            scorers.Claims(source="s", keys=[("F#", "minor")]), truth)
        parallel = scorers.score_key(
            scorers.Claims(source="s", keys=[("F#", "major")]), truth)
        relative = scorers.score_key(
            scorers.Claims(source="s", keys=[("A", "major")]), truth)
        wrong = scorers.score_key(
            scorers.Claims(source="s", keys=[("C", "major")]), truth)
        self.assertEqual(exact["verdict"], "exact")
        self.assertEqual(parallel["verdict"], "parallel")
        self.assertEqual(relative["verdict"], "relative")
        self.assertEqual(wrong["verdict"], "wrong")

    def test_enharmonic_spellings_are_the_same_key(self) -> None:
        self.assertEqual(scorers.normalize_pitch("Gb"), scorers.normalize_pitch("F#"))
        self.assertEqual(scorers.normalize_pitch("D♭"), scorers.normalize_pitch("C#"))

    def test_meter_matches_after_normalization(self) -> None:
        truth = synthetic_truth(meter_status="adjudicated", meter="4/4")
        scored = scorers.score_meter(
            scorers.Claims(source="s", meters=["common time"]), truth)
        self.assertEqual(scored["verdict"], "correct")
        wrong = scorers.score_meter(
            scorers.Claims(source="s", meters=["3/4"]), truth)
        self.assertEqual(wrong["verdict"], "wrong")


class InstrumentScoringTests(unittest.TestCase):
    def truth(self) -> ground_truth.TrackTruth:
        return synthetic_truth(
            instruments_status="adjudicated",
            instruments_present=["drum machine", "synth bass", "lead vocal", "piano"],
            instruments_allowed_extra=["synth pad"],
            instruments_absent=["saxophone", "banjo"],
        )

    def test_precision_recall_and_the_neutral_list(self) -> None:
        claims = scorers.claims_from_structured(synthetic_document())
        scored = scorers.score_instruments(claims, self.truth())
        self.assertEqual(scored["matched"], ["drum machine", "lead vocal", "synth bass"])
        self.assertEqual(scored["missed"], ["piano"])
        self.assertEqual(scored["neutral_claimed"], ["synth pad"])
        self.assertEqual(scored["false_positives"], [])
        self.assertAlmostEqual(scored["recall"], 0.75)
        self.assertAlmostEqual(scored["precision"], 1.0)

    def test_a_canary_instrument_is_reported_separately(self) -> None:
        document = synthetic_document()
        document["instruments"] = list(document["instruments"]) + [
            {"name": "tenor sax", "timbre": "breathy"}]
        scored = scorers.score_instruments(
            scorers.claims_from_structured(document), self.truth())
        self.assertEqual(scored["canaries_claimed"], ["saxophone"])
        self.assertIn("saxophone", scored["false_positives"])
        self.assertLess(scored["precision"], 1.0)

    def test_prose_naming_finds_the_same_instruments(self) -> None:
        found, _ = scorers.instruments_in_text(SYNTHETIC_PROSE)
        self.assertIn("drum machine", found)
        self.assertIn("synth bass", found)
        self.assertIn("hi-hat", found)

    def test_an_unknown_name_is_reported_not_silently_dropped(self) -> None:
        document = synthetic_document()
        document["instruments"] = [{"name": "glass armonica", "timbre": "eerie"}]
        claims = scorers.claims_from_structured(document)
        self.assertEqual(claims.instruments, [])
        self.assertEqual(claims.unknown_instrument_terms, ["glass armonica"])

    def test_scoring_abstains_while_the_list_is_unauthored(self) -> None:
        scored = scorers.score_instruments(
            scorers.claims_from_structured(synthetic_document()), synthetic_truth())
        self.assertEqual(scored["status"], "abstain")


class LyricScoringTests(unittest.TestCase):
    def test_a_quoted_phrase_matches_its_aligned_line(self) -> None:
        claims = scorers.Claims(
            source="s", lyric_moments=[(70.4, "the queen of the sky")])
        scored = scorers.score_lyric_moments(claims, synthetic_truth())
        self.assertEqual(scored["matched"], 1)
        self.assertEqual(scored["fabricated"], 0)
        self.assertAlmostEqual(scored["median_error_seconds"], 0.4, places=6)
        self.assertEqual(scored["within_2s"], 1.0)

    def test_shifting_the_claim_ten_seconds_breaks_the_tolerance(self) -> None:
        # The negative control: a scorer that returned a constant would pass
        # the test above and fail this one.
        claims = scorers.Claims(
            source="s", lyric_moments=[(80.4, "the queen of the sky")])
        scored = scorers.score_lyric_moments(claims, synthetic_truth())
        self.assertAlmostEqual(scored["median_error_seconds"], 10.4, places=6)
        self.assertEqual(scored["within_2s"], 0.0)
        self.assertEqual(scored["within_5s"], 0.0)

    def test_an_invented_phrase_counts_as_fabrication(self) -> None:
        claims = scorers.Claims(
            source="s", lyric_moments=[(70.0, "burning down the pier tonight")])
        scored = scorers.score_lyric_moments(claims, synthetic_truth())
        self.assertEqual(scored["fabricated"], 1)
        self.assertEqual(scored["fabrication_rate"], 1.0)

    def test_a_phrase_with_no_time_is_untimed_not_wrong(self) -> None:
        claims = scorers.Claims(
            source="s", lyric_moments=[(float("nan"), "the queen of the sky")])
        scored = scorers.score_lyric_moments(claims, synthetic_truth())
        self.assertEqual(scored["untimed"], 1)
        self.assertIsNone(scored["median_error_seconds"])

    def test_a_one_word_claim_is_unscoreable_rather_than_fabricated(self) -> None:
        claims = scorers.Claims(source="s", lyric_moments=[(70.0, "free")])
        scored = scorers.score_lyric_moments(claims, synthetic_truth())
        self.assertEqual(scored["too_short"], 1)
        self.assertEqual(scored["fabricated"], 0)

    def test_coverage_counts_distinct_truth_lines(self) -> None:
        claims = scorers.Claims(source="s", lyric_moments=[
            (70.0, "the queen of the sky"),
            (70.1, "queen of the sky just came"),
        ])
        scored = scorers.score_lyric_moments(claims, synthetic_truth())
        self.assertAlmostEqual(scored["coverage"], 0.5)

    def test_containment_similarity_ignores_the_truth_line_being_longer(self) -> None:
        self.assertAlmostEqual(
            scorers.phrase_similarity("queen of the sky",
                                      "The queen of the sky just came through"),
            1.0)
        self.assertLess(
            scorers.phrase_similarity("nothing like this line at all",
                                      "The queen of the sky just came through"),
            scorers.LYRIC_MATCH_THRESHOLD)

    def test_prose_extraction_pairs_a_quote_with_its_nearby_time(self) -> None:
        claims = scorers.claims_from_text(SYNTHETIC_PROSE)
        phrases = {phrase for _, phrase in claims.lyric_moments}
        self.assertIn("the queen of the sky", phrases)
        times = {phrase: seconds for seconds, phrase in claims.lyric_moments}
        self.assertAlmostEqual(times["the queen of the sky"], 44.5)


class FormScoringTests(unittest.TestCase):
    def test_boundaries_inside_the_tolerance_agree(self) -> None:
        claims = scorers.Claims(source="s", sections=[(30.0, "verse"), (47.5, "chorus")])
        scored = scorers.score_sections(claims, synthetic_truth())
        self.assertEqual(scored["claimed_boundaries"], 2)
        self.assertAlmostEqual(scored["agreement_precision"], 1.0)

    def test_boundaries_outside_the_tolerance_do_not(self) -> None:
        claims = scorers.Claims(source="s", sections=[(38.0, "verse"), (55.0, "chorus")])
        scored = scorers.score_sections(claims, synthetic_truth())
        self.assertAlmostEqual(scored["agreement_precision"], 0.0)


class ConcretenessTests(unittest.TestCase):
    def test_a_measured_sentence_is_concrete(self) -> None:
        scored = scorers.concreteness(
            "The tempo sits at 112 bpm in 4/4. A synth bass enters at 34 s.")
        self.assertEqual(scored["concrete"], 2)
        self.assertEqual(scored["generic"], 0)

    def test_a_sentence_of_adjectives_is_generic(self) -> None:
        scored = scorers.concreteness(
            "It is a nice track. The vibe is lovely and uplifting.")
        self.assertEqual(scored["concrete"], 0)
        self.assertEqual(scored["generic"], 2)

    def test_padding_does_not_improve_the_share(self) -> None:
        # A sentence that is neither concrete nor generic is excluded from the
        # ratio, so adding filler cannot raise concrete_share.
        base = scorers.concreteness("The tempo sits at 112 bpm.")
        padded = scorers.concreteness(
            "The tempo sits at 112 bpm. Something happens next.")
        self.assertEqual(base["concrete_share"], padded["concrete_share"])
        self.assertEqual(padded["neutral"], 1)

    def test_the_mixed_fixture_reports_both_kinds(self) -> None:
        scored = scorers.concreteness(SYNTHETIC_PROSE)
        self.assertGreater(scored["concrete"], 0)
        self.assertGreater(scored["generic"], 0)
        self.assertGreater(scored["concrete_per_100_words"], 0.0)

    def test_the_lexicon_hash_travels_with_the_number(self) -> None:
        self.assertEqual(scorers.concreteness("x")["lexicon_sha256"],
                         scorers.LEXICON_SHA256)


class ConsistencyTests(unittest.TestCase):
    def test_identical_runs_agree_completely(self) -> None:
        run = scorers.claims_from_structured(synthetic_document())
        agreement = scorers.inter_run_agreement([run, run, run])
        self.assertAlmostEqual(agreement["descriptor_jaccard_mean"], 1.0)
        self.assertAlmostEqual(
            agreement["numeric_stability"]["energy"]["stdev"], 0.0)

    def test_disjoint_vocabularies_score_zero(self) -> None:
        first = scorers.Claims(source="s", descriptors=["glassy", "wide"])
        second = scorers.Claims(source="s", descriptors=["gritty", "narrow"])
        agreement = scorers.inter_run_agreement([first, second])
        self.assertEqual(agreement["descriptor_jaccard_mean"], 0.0)

    def test_a_single_run_abstains(self) -> None:
        run = scorers.claims_from_structured(synthetic_document())
        self.assertEqual(scorers.inter_run_agreement([run])["status"], "abstain")


class DeterminismTests(unittest.TestCase):
    def test_identical_reformats_disagree_nowhere(self) -> None:
        documents = [synthetic_document() for _ in range(5)]
        scored = scorers.field_disagreement(documents, schema.DETERMINISM_FIELDS)
        self.assertEqual(scored["field_disagreement_rate"], 0.0)
        self.assertEqual(scored["identical_output_rate"], 1.0)

    def test_one_drifting_field_is_localized(self) -> None:
        documents = [synthetic_document() for _ in range(5)]
        documents[3]["tempo_bpm"] = 113.0
        documents[4]["tempo_bpm"] = 111.0
        scored = scorers.field_disagreement(documents, schema.DETERMINISM_FIELDS)
        self.assertEqual(scored["worst_field"], "tempo_bpm")
        self.assertAlmostEqual(scored["fields"]["tempo_bpm"], 0.4)
        self.assertAlmostEqual(scored["fields"]["meter"], 0.0)
        self.assertAlmostEqual(scored["identical_output_rate"], 0.6)

    def test_list_order_is_not_a_disagreement(self) -> None:
        first = synthetic_document()
        second = synthetic_document(feel=["propulsive", "wistful", "restless"])
        scored = scorers.field_disagreement([first, second], ("feel",))
        self.assertEqual(scored["field_disagreement_rate"], 0.0)

    def test_two_runs_are_the_minimum(self) -> None:
        self.assertEqual(
            scorers.field_disagreement([synthetic_document()],
                                       schema.DETERMINISM_FIELDS)["status"],
            "abstain")


# ---------------------------------------------------------------------------
# Ground truth
# ---------------------------------------------------------------------------


class GroundTruthTests(unittest.TestCase):
    #: A 42.7 ms analysis hop quantizes the recoverable tempo: at 150 BPM the
    #: neighbouring integer lags are 140.6 and 156.25 BPM, so no estimator can
    #: do better than about 5 % there. The tolerance is that bound, not slack.
    QUANTIZATION_TOLERANCE = 0.06

    def synthetic_measured(self, bpm: float, seconds: float = 60.0) -> dict:
        """A planted pulse. SYNTHETIC: a Gaussian onset bump on the beat grid.

        Smeared rather than a one-frame impulse, because a bare impulse train
        whose period is not a whole number of frames has its true correlation
        peak at the least common multiple, which tests the fixture rather than
        the estimator.
        """
        hop, rate = 1024, 24000
        step = hop / rate
        period = 60.0 / bpm
        sigma = 0.05
        frames = []
        for index in range(int(seconds / step)):
            time = index * step
            distance = time % period
            distance = min(distance, period - distance)
            frames.append({"onset_strength": math.exp(-0.5 * (distance / sigma) ** 2)})
        return {
            "analysis": {"hop_size": hop, "sample_rate": rate},
            "frames": frames,
            "summary": {"sections": [
                {"start_seconds": 0.0, "end_seconds": 20.0},
                {"start_seconds": 20.0, "end_seconds": 40.0},
                {"start_seconds": 40.0, "end_seconds": 60.0},
            ]},
        }

    def test_the_excerpt_tempo_estimate_recovers_a_planted_pulse(self) -> None:
        measured = self.synthetic_measured(120.0)
        estimate = ground_truth.excerpt_tempo_bpm(measured, 0.0, 60.0)
        self.assertIsNotNone(estimate)
        self.assertLess(abs(estimate - 120.0) / 120.0, self.QUANTIZATION_TOLERANCE)

    def test_the_argmax_alone_can_be_a_submultiple(self) -> None:
        # This is why the estimate returns a candidate set rather than one
        # number. The repository's normalization inflates long lags, so a
        # planted 150 BPM comes back as 74 — a factor no octave equivalence
        # in the scorer would forgive if 74 were the only reference.
        estimate = ground_truth.excerpt_tempo_bpm(
            self.synthetic_measured(150.0), 0.0, 60.0)
        self.assertGreater(abs(estimate - 150.0) / 150.0,
                           self.QUANTIZATION_TOLERANCE)

    def test_every_planted_tempo_appears_among_the_candidates(self) -> None:
        for bpm in (60.0, 75.0, 90.0, 100.0, 120.0, 128.0, 140.0, 150.0, 180.0):
            pulse = ground_truth.excerpt_pulse(
                self.synthetic_measured(bpm), 0.0, 60.0)
            candidates = [candidate["bpm"] for candidate in pulse["candidates"]]
            self.assertTrue(
                any(abs(candidate - bpm) / bpm < self.QUANTIZATION_TOLERANCE
                    for candidate in candidates),
                f"{bpm} not in {candidates}")

    def test_weak_candidates_are_dropped_rather_than_ranked(self) -> None:
        pulse = ground_truth.excerpt_pulse(self.synthetic_measured(120.0), 0.0, 60.0)
        scores = [candidate["score"] for candidate in pulse["candidates"]]
        self.assertTrue(all(score > 0.0 for score in scores))
        self.assertTrue(all(score >= scores[0] * ground_truth.CANDIDATE_SCORE_FLOOR
                            for score in scores))

    def test_the_chance_baseline_is_reported_and_below_a_half(self) -> None:
        # An accept rate has to be read against what a random guess scores.
        truth = synthetic_truth(excerpt_bpm_candidates=[56.25, 175.78, 82.72])
        chance = scorers.tempo_chance_accept_rate(truth)
        self.assertGreater(chance, 0.0)
        self.assertLess(chance, 0.5)

    def test_silence_yields_no_estimate(self) -> None:
        measured = {"analysis": {"hop_size": 1024, "sample_rate": 24000},
                    "frames": [{"onset_strength": 0.0} for _ in range(2000)]}
        self.assertIsNone(ground_truth.excerpt_tempo_bpm(measured, 0.0, 60.0))

    def test_only_sections_touching_the_excerpt_are_kept_and_clipped(self) -> None:
        sections = ground_truth.measured_sections(
            self.synthetic_measured(120.0), 25.0, 45.0)
        self.assertEqual(sections, [(25.0, 40.0), (40.0, 45.0)])

    def test_an_adjudicated_onset_overrides_the_aligned_one(self) -> None:
        aligned = {"lines": [
            {"reference_line_index": 18, "start_seconds": 66.78,
             "end_seconds": 74.0, "text": "The queen of the sky just came through"},
        ]}
        adjudication = {18: {"line": 18, "true_start_seconds": 70.0}}
        truth = ground_truth.lyric_truth(aligned, adjudication, 30.0, 130.0)
        self.assertEqual(len(truth), 1)
        self.assertAlmostEqual(truth[0].start_seconds, 70.0)
        self.assertEqual(truth[0].source, "adjudicated")

    def test_an_unlocatable_line_is_dropped_rather_than_trusted(self) -> None:
        aligned = {"lines": [
            {"reference_line_index": 26, "start_seconds": 106.8,
             "end_seconds": 110.0, "text": "but you know I do"},
        ]}
        adjudication = {26: {"line": 26, "true_start_seconds": None}}
        self.assertEqual(
            ground_truth.lyric_truth(aligned, adjudication, 30.0, 130.0), [])

    def test_lines_outside_the_excerpt_are_excluded(self) -> None:
        aligned = {"lines": [
            {"reference_line_index": 0, "start_seconds": 7.5, "end_seconds": 10.0,
             "text": "I make a wish when the sun drops low"},
            {"reference_line_index": 1, "start_seconds": 44.8, "end_seconds": 48.0,
             "text": "Shut up Cat I already know"},
        ]}
        truth = ground_truth.lyric_truth(aligned, {}, 30.0, 130.0)
        self.assertEqual([line.reference_line_index for line in truth], [1])

    def test_the_shipped_ground_truth_file_is_well_formed_and_abstaining(self) -> None:
        document = ground_truth.authored()
        self.assertEqual(document["schema"], ground_truth.GROUND_TRUTH_VERSION)
        for track in bench_io.TRACKS:
            entry = document["tracks"][track.slug]
            self.assertIn(entry["key"]["status"], ("unadjudicated", "adjudicated", "none"))
            self.assertAlmostEqual(entry["excerpt"]["start_seconds"],
                                   track.excerpt_start_seconds)
            self.assertAlmostEqual(entry["excerpt"]["seconds"],
                                   bench_io.EXCERPT_SECONDS)


# ---------------------------------------------------------------------------
# Reporting over stored records
# ---------------------------------------------------------------------------


class ReportTests(unittest.TestCase):
    def test_a_structured_record_scores_end_to_end(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S1")
        record = synthetic_record(cell=cell.id)
        scored = report.score_unit(cell, 1, synthetic_truth(), [record])
        self.assertEqual(scored["status"], "scored")
        self.assertEqual(scored["conformance"]["schema_ok"], 1)
        self.assertEqual(scored["tempo"]["octave"], 1)
        self.assertEqual(scored["lyric_position"]["status"], "scored")
        self.assertEqual(scored["provenance"]["models_served"], ["xiaomi/mimo-v2.5"])

    def test_malformed_json_is_a_conformance_failure_not_a_crash(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S1")
        record = synthetic_record(cell=cell.id, text="Here is your description!")
        scored = report.score_unit(cell, 1, synthetic_truth(), [record])
        self.assertEqual(scored["conformance"]["schema_ok"], 0)
        self.assertEqual(scored["conformance"]["parse_ok"], 0)
        self.assertTrue(scored["conformance"]["errors"])

    def test_a_free_text_record_is_mined_for_claims(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S0")
        record = synthetic_record(cell=cell.id, structured=False, text=SYNTHETIC_PROSE)
        scored = report.score_unit(cell, 1, synthetic_truth(), [record])
        self.assertEqual(scored["tempo"]["octave"], 1)
        self.assertGreater(scored["concreteness"]["concrete"], 0)

    def test_chunk_local_timestamps_are_detected(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c05x20/strict-checklist/S1")
        # Two chunks, each answering on its own clock: the truth line at 70.0 s
        # lands 10 s into the chunk that starts at 60.0 s.
        first = synthetic_record(
            cell=cell.id, call_index=0, chunk_start=30.0,
            document=synthetic_document(lyric_moments=[], sections=[]))
        second = synthetic_record(
            cell=cell.id, call_index=1, chunk_start=60.0,
            document=synthetic_document(
                lyric_moments=[{"seconds": 10.0, "phrase": "the queen of the sky"}],
                sections=[]))
        scored = report.score_unit(cell, 1, synthetic_truth(), [first, second])
        self.assertEqual(scored["time_frame"]["frame_used"], "chunk-local")
        self.assertFalse(scored["time_frame"]["obeyed"])

    def test_absolute_timestamps_are_reported_as_obeyed(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c05x20/strict-checklist/S1")
        first = synthetic_record(
            cell=cell.id, call_index=0, chunk_start=30.0,
            document=synthetic_document(lyric_moments=[], sections=[]))
        second = synthetic_record(
            cell=cell.id, call_index=1, chunk_start=60.0,
            document=synthetic_document(
                lyric_moments=[{"seconds": 70.0, "phrase": "the queen of the sky"}],
                sections=[]))
        scored = report.score_unit(cell, 1, synthetic_truth(), [first, second])
        self.assertEqual(scored["time_frame"]["frame_used"], "absolute")
        self.assertTrue(scored["time_frame"]["obeyed"])

    def test_merging_chunks_unions_instruments_and_sorts_times(self) -> None:
        first = scorers.claims_from_structured(synthetic_document(
            instruments=[{"name": "piano", "timbre": "bright"}],
            lyric_moments=[{"seconds": 90.0, "phrase": "a later line here"}]))
        second = scorers.claims_from_structured(synthetic_document(
            instruments=[{"name": "banjo", "timbre": "plinky"}],
            lyric_moments=[{"seconds": 40.0, "phrase": "an earlier line here"}]))
        merged = report.merge_claims([first, second])
        self.assertEqual(merged.instruments, ["piano", "banjo"])
        self.assertEqual([seconds for seconds, _ in merged.lyric_moments], [40.0, 90.0])

    def test_the_probe_arm_is_not_merged_but_compared(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S3")
        records = [synthetic_record(cell=cell.id, call_index=index)
                   for index in range(matrix.REFORMATTER_PROBE_RUNS)]
        records[2]["text"] = json.dumps(synthetic_document(meter="3/4"))
        scored = report.score_unit(cell, 1, synthetic_truth(), records)
        self.assertEqual(scored["determinism"]["status"], "scored")
        self.assertEqual(scored["determinism"]["worst_field"], "meter")
        self.assertAlmostEqual(scored["determinism"]["identical_output_rate"], 0.8)

    def test_a_missing_unit_says_so(self) -> None:
        cell = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S1")
        self.assertEqual(
            report.score_unit(cell, 1, synthetic_truth(), [])["status"], "missing")


# ---------------------------------------------------------------------------
# Resume logic and the live gates
# ---------------------------------------------------------------------------


class ResumeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.results = Path(self.temporary.name) / "results"
        patcher = mock.patch.object(bench_io, "RESULTS_ROOT", self.results)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(self.temporary.cleanup)
        self.cell = matrix.cell_by_id("shut-up-cat/c05x20/strict-checklist/S1")

    def test_a_unit_with_no_marker_is_pending(self) -> None:
        queue = bench_run.pending(1, only_cell=self.cell.id)
        self.assertEqual([(cell.id, repeat) for cell, repeat in queue],
                         [(self.cell.id, 1)])

    def test_a_done_marker_removes_it_from_the_queue(self) -> None:
        bench_io.write_done(self.cell.id, 1, status="ok")
        self.assertEqual(bench_run.pending(1, only_cell=self.cell.id), [])

    def test_a_failed_unit_returns_only_with_retry_failed(self) -> None:
        bench_io.write_done(self.cell.id, 1, status="failed")
        self.assertEqual(bench_run.pending(1, only_cell=self.cell.id), [])
        self.assertEqual(
            len(bench_run.pending(1, only_cell=self.cell.id, retry_failed=True)), 1)

    def test_stored_calls_are_skipped_on_resume(self) -> None:
        bench_io.atomic_write_json(
            bench_io.call_path(self.cell.id, 1, 0),
            synthetic_record(cell=self.cell.id, call_index=0))
        bench_io.atomic_write_json(
            bench_io.call_path(self.cell.id, 1, 3),
            synthetic_record(cell=self.cell.id, call_index=3))
        self.assertEqual(bench_io.completed_calls(self.cell.id, 1), {0, 3})

    def test_a_partial_unit_is_still_pending(self) -> None:
        bench_io.atomic_write_json(
            bench_io.call_path(self.cell.id, 1, 0),
            synthetic_record(cell=self.cell.id, call_index=0))
        self.assertEqual(len(bench_run.pending(1, only_cell=self.cell.id)), 1)

    def test_execution_resumes_at_the_first_missing_call(self) -> None:
        sent: list[dict] = []

        def fake_submit(payload, **_kwargs):
            sent.append(payload)
            return {
                "id": "synthetic", "model": "xiaomi/mimo-v2.5",
                "usage": {"prompt_tokens": 1, "completion_tokens": 1},
                "choices": [{"message": {"content": json.dumps(synthetic_document())},
                             "finish_reason": "stop"}],
            }

        for index in (0, 1, 2):
            bench_io.atomic_write_json(
                bench_io.call_path(self.cell.id, 1, index),
                synthetic_record(cell=self.cell.id, call_index=index))
        with mock.patch.object(bench_run, "submit", fake_submit), \
                mock.patch.object(bench_run, "audio_bytes_for",
                                  lambda *args, **kwargs: b"synthetic"):
            result = bench_run.execute_unit(self.cell, 1)
        self.assertEqual(result["status"], "ok")
        self.assertEqual(result["calls_sent"], 2)
        self.assertEqual(len(sent), 2)
        self.assertEqual(bench_io.completed_calls(self.cell.id, 1), {0, 1, 2, 3, 4})
        self.assertEqual(bench_io.cell_state(self.cell.id, 1)["status"], "ok")

    def test_a_failure_leaves_the_earlier_calls_stored(self) -> None:
        calls: list[int] = []

        def failing_submit(payload, **_kwargs):
            calls.append(1)
            if len(calls) > 2:
                raise bench_run.BenchFailure("synthetic transport failure")
            return {
                "id": "synthetic", "model": "xiaomi/mimo-v2.5",
                "choices": [{"message": {"content": json.dumps(synthetic_document())}}],
            }

        with mock.patch.object(bench_run, "submit", failing_submit), \
                mock.patch.object(bench_run, "audio_bytes_for",
                                  lambda *args, **kwargs: b"synthetic"):
            result = bench_run.execute_unit(self.cell, 1)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(bench_io.completed_calls(self.cell.id, 1), {0, 1})
        self.assertTrue(bench_io.cell_state(self.cell.id, 1)["errors"])

    def test_a_dependent_arm_reads_the_stored_description(self) -> None:
        dependent = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S3")
        bench_io.atomic_write_json(
            bench_io.call_path(dependent.depends_on, 1, 0),
            synthetic_record(cell=dependent.depends_on, structured=False,
                             text=SYNTHETIC_PROSE))
        self.assertEqual(bench_run.description_for(dependent, 1), SYNTHETIC_PROSE)

    def test_a_missing_dependency_is_a_named_failure(self) -> None:
        dependent = matrix.cell_by_id("shut-up-cat/c01x100/strict-checklist/S3")
        with self.assertRaises(bench_run.BenchFailure):
            bench_run.description_for(dependent, 1)


class LiveGateTests(unittest.TestCase):
    def test_the_default_is_a_dry_run(self) -> None:
        permitted, reason = bench_run.live_permitted(False, {})
        self.assertFalse(permitted)
        self.assertIn("dry run", reason)

    def test_the_flag_alone_is_not_enough(self) -> None:
        permitted, reason = bench_run.live_permitted(
            True, {"OPENROUTER_API_KEY": "synthetic"})
        self.assertFalse(permitted)
        self.assertIn(bench_run.LIVE_CONFIRMATION_VARIABLE, reason)

    def test_a_wrong_confirmation_value_is_refused(self) -> None:
        permitted, _ = bench_run.live_permitted(True, {
            bench_run.LIVE_CONFIRMATION_VARIABLE: "yes",
            "OPENROUTER_API_KEY": "synthetic",
        })
        self.assertFalse(permitted)

    def test_both_gates_plus_a_key_permit_it(self) -> None:
        permitted, reason = bench_run.live_permitted(True, {
            bench_run.LIVE_CONFIRMATION_VARIABLE: bench_run.LIVE_CONFIRMATION_VALUE,
            "OPENROUTER_API_KEY": "synthetic",
        })
        self.assertTrue(permitted)
        self.assertEqual(reason, "live")

    def test_a_missing_key_is_refused_even_with_both_gates(self) -> None:
        permitted, reason = bench_run.live_permitted(True, {
            bench_run.LIVE_CONFIRMATION_VARIABLE: bench_run.LIVE_CONFIRMATION_VALUE,
        })
        self.assertFalse(permitted)
        self.assertIn("OPENROUTER_API_KEY", reason)

    def test_the_plan_command_never_reaches_the_transport(self) -> None:
        import urllib.request

        parser = bench_run.build_parser()
        options = parser.parse_args(["plan", "--repeats", "1"])
        with mock.patch.object(urllib.request, "urlopen",
                               side_effect=AssertionError("the dry run opened a socket")), \
                mock.patch("sys.stdout"):
            self.assertEqual(bench_run.command_plan(options), 0)

    def test_run_commands_refuse_without_the_gates(self) -> None:
        parser = bench_run.build_parser()
        for command in ("next", "all"):
            options = parser.parse_args([command])
            with mock.patch.dict("os.environ", {}, clear=True), \
                    mock.patch("sys.stderr"):
                self.assertEqual(bench_run.main([command]), 2)
            self.assertFalse(options.live)


class NaNHandlingTests(unittest.TestCase):
    """`nan` compares false against everything, which has hidden a bug here before."""

    def test_an_untimed_moment_sorts_last_rather_than_corrupting_the_order(self) -> None:
        first = scorers.Claims(source="s", lyric_moments=[(float("nan"), "no time given")])
        second = scorers.Claims(source="s", lyric_moments=[(40.0, "a timed line")])
        merged = report.merge_claims([first, second])
        self.assertEqual(merged.lyric_moments[0][1], "a timed line")
        self.assertTrue(math.isnan(merged.lyric_moments[1][0]))

    def test_an_untimed_moment_never_enters_the_error_statistics(self) -> None:
        claims = scorers.Claims(source="s", lyric_moments=[
            (float("nan"), "the queen of the sky"),
            (70.5, "the queen of the sky"),
        ])
        scored = scorers.score_lyric_moments(claims, synthetic_truth())
        self.assertEqual(scored["untimed"], 1)
        self.assertAlmostEqual(scored["median_error_seconds"], 0.5, places=6)


if __name__ == "__main__":
    unittest.main()
