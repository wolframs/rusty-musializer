#!/usr/bin/env python3
"""The MiMo lane's request, its failed runs, its estimate and its scales.

Offline: a fake transport stands in for OpenRouter and returns recorded
response shapes (the two runaways of the 2026-11-14 benchmark, a null answer,
an off-scale number, a well-formed answer). No socket, no audio device, no
credential: the key is a sentinel set for the duration of a test.
"""

from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import external_analysis  # noqa: E402
import mimo_openrouter  # noqa: E402


SENTINEL_KEY = "sk-or-v1-test-sentinel-not-a-key"
DURATION = 8.0


def creative(**overrides: object) -> dict:
    value = {
        "summary": "a summary", "moods": ["calm"], "energy": 0.5,
        "tension": 0.4, "valence": 0.1, "motion": ["drift"],
        "textures": ["warm"], "imagery": ["dusk"], "palette": ["#101020"],
        "scene_cues": ["spectrum"], "confidence": 0.7,
    }
    value.update(overrides)
    return value


def answer(**segment_overrides: object) -> str:
    return json.dumps({
        "global": creative(),
        "segments": [dict(creative(**segment_overrides),
                          start_seconds=0.0, end_seconds=DURATION)],
    })


def response(content: object, *, finish: object = "stop",
             completion_tokens: int = 5110, cost: float | None = 0.010596) -> dict:
    """An OpenRouter chat completion in the shape the benchmark recorded."""
    usage: dict = {
        "prompt_tokens": 940, "completion_tokens": completion_tokens,
        "completion_tokens_details": {"reasoning_tokens": completion_tokens - 1000},
        "total_tokens": 940 + completion_tokens,
    }
    if cost is not None:
        usage["cost"] = cost
    return {
        "id": "gen-test", "created": 1794682429, "model": "xiaomi/mimo-v2.5",
        "provider": "Venice",
        "choices": [{"finish_reason": finish,
                     "message": {"role": "assistant", "content": content}}],
        "usage": usage,
    }


# The two runaways and the null answer, as the benchmark saw them.
CUT_OFF = response(" " * 70_000, finish="length", completion_tokens=65_536, cost=0.1312484)
NULL_CONTENT = response(None, completion_tokens=2_000, cost=0.004)
EMPTY_CONTENT = response("   \n ", completion_tokens=1_800, cost=0.0036)


class FakeHTTPResponse(io.BytesIO):
    def __enter__(self) -> "FakeHTTPResponse":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()


class FakeOpenRouter:
    """Returns the queued bodies in order and records every request sent."""

    def __init__(self, *bodies: object) -> None:
        self.bodies = list(bodies)
        self.sent: list[dict] = []

    def __call__(self, request, timeout: float):  # noqa: ANN001
        self.sent.append(json.loads(request.data.decode("utf-8")))
        body = self.bodies.pop(0)
        if isinstance(body, BaseException):
            raise body
        return FakeHTTPResponse(json.dumps(body).encode("utf-8"))


class Workspace:
    def __init__(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.root = Path(self._dir.name)
        self.audio = self.root / "song.wav"
        self.audio.write_bytes(b"RIFF-not-really-audio")
        self.cache = self.root / "semantic.cache.json"
        self.receipt = mimo_openrouter.failure_receipt_path(self.cache)

    def close(self) -> None:
        self._dir.cleanup()


class LaneTestCase(unittest.TestCase):
    def setUp(self) -> None:
        self.ws = Workspace()
        self.addCleanup(self.ws.close)
        patcher = patch.dict(os.environ, {"OPENROUTER_API_KEY": SENTINEL_KEY})
        patcher.start()
        self.addCleanup(patcher.stop)

    def run_lane(self, *bodies: object) -> tuple[FakeOpenRouter, object]:
        fake = FakeOpenRouter(*bodies)
        outcome = mimo_openrouter.analyze_to_cache(
            self.ws.audio, self.ws.cache, audio_duration=DURATION,
            transport=fake, sleeper=lambda _seconds: None)
        return fake, outcome


class CompletionCap(LaneTestCase):
    def test_the_request_caps_the_completion_and_the_cap_is_in_its_identity(self) -> None:
        payload, settings = mimo_openrouter.build_request(
            b"\x00", audio_format="wav", audio_duration=DURATION)
        self.assertEqual(payload["max_tokens"], 16_000)
        self.assertEqual(settings["max_tokens"], 16_000)
        self.assertEqual(mimo_openrouter.MAX_COMPLETION_TOKENS, 16_000)
        # The cap must leave room for the largest legitimate answer the
        # benchmark measured (10 942) and cut both runaways (43 578, 65 536).
        self.assertGreater(mimo_openrouter.MAX_COMPLETION_TOKENS, 10_942)
        self.assertLess(mimo_openrouter.MAX_COMPLETION_TOKENS, 43_578)

    def test_the_cap_is_what_the_helper_actually_sends(self) -> None:
        fake, _ = self.run_lane(response(answer()))
        self.assertEqual(fake.sent[0]["max_tokens"], 16_000)
        self.assertNotIn("reasoning", fake.sent[0])


class FailedRuns(LaneTestCase):
    def assert_failed(self, body: dict, reason_fragment: str, cost_text: str) -> dict:
        with self.assertRaises(mimo_openrouter.FailedRun) as raised:
            self.run_lane(body)
        message = str(raised.exception)
        self.assertIn(reason_fragment, message)
        self.assertIn(cost_text, message)
        self.assertIn("Nothing was saved", message)
        # The user reads this behind a 26-byte prefix, clipped at 160 bytes:
        # the reason and the cost must both survive.
        self.assertLessEqual(len(message.encode("utf-8")), 160, message)
        self.assertFalse(self.ws.cache.exists(), "a failed run is never cached")
        receipt = json.loads(self.ws.receipt.read_text())
        self.assertEqual(receipt["schema_version"], "musializer.mimo-failed-run/v1")
        self.assertEqual(receipt["message"], message)
        self.assertEqual(receipt["generation"]["usage"], body["usage"])
        self.assertNotIn(SENTINEL_KEY, self.ws.receipt.read_text())
        return receipt

    def test_a_length_cut_off_is_a_failed_run_and_names_its_cost(self) -> None:
        receipt = self.assert_failed(
            CUT_OFF, "cut off at the 16000-token cap", "billed 0.1312 USD")
        self.assertEqual(receipt["finish_reason"], "length")
        self.assertEqual(receipt["reported_cost_usd"], 0.1312484)
        self.assertEqual(receipt["content_characters"], 70_000)
        self.assertEqual(len(receipt["content_start"]), mimo_openrouter.FAILED_CONTENT_KEEP)

    def test_a_cut_off_that_happens_to_be_valid_json_is_still_a_failed_run(self) -> None:
        # A truncated answer can parse: the first segments of a longer list.
        self.assert_failed(response(answer(), finish="length"),
                           "cut off", "billed 0.0106 USD")

    def test_null_content_is_a_failed_run(self) -> None:
        receipt = self.assert_failed(NULL_CONTENT, "no answer text", "billed 0.0040 USD")
        self.assertIsNone(receipt["content_characters"])

    def test_empty_content_is_a_failed_run(self) -> None:
        self.assert_failed(EMPTY_CONTENT, "empty answer", "billed 0.0036 USD")

    def test_an_unfinished_answer_is_a_failed_run(self) -> None:
        self.assert_failed(response(answer(), finish="content_filter"),
                           "finish reason: content_filter", "billed")
        self.ws.receipt.unlink()
        self.assert_failed(response(answer(), finish=None),
                           "finish reason: none", "billed")

    def test_an_answer_that_is_not_json_is_a_failed_run(self) -> None:
        self.assert_failed(response("The track feels warm and slow."),
                           "not valid JSON", "billed")

    def test_an_unreported_cost_is_said_to_be_unreported(self) -> None:
        self.assert_failed(response(None, cost=None), "no answer text",
                           "OpenRouter reported no cost for it")

    def test_a_billed_failure_is_sent_once_not_retried(self) -> None:
        fake = FakeOpenRouter(NULL_CONTENT, response(answer()), response(answer()))
        with self.assertRaises(mimo_openrouter.FailedRun):
            mimo_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION,
                transport=fake, sleeper=lambda _seconds: None)
        self.assertEqual(len(fake.sent), 1)

    def test_a_transient_refusal_is_still_retried(self) -> None:
        busy = urllib.error.HTTPError(
            mimo_openrouter.OPENROUTER_URL, 503, "busy", {}, io.BytesIO(b""))
        fake, _ = self.run_lane(busy, response(answer()))
        self.assertEqual(len(fake.sent), 2)
        self.assertTrue(self.ws.cache.exists())

    def test_a_well_formed_answer_is_cached_and_clears_an_old_receipt(self) -> None:
        self.ws.receipt.write_text("{}")
        _, envelope = self.run_lane(response(answer()))
        self.assertFalse(self.ws.receipt.exists())
        cached = json.loads(self.ws.cache.read_text())
        self.assertEqual(cached["cache_key"], envelope["cache_key"])
        self.assertEqual(cached["normalized"]["segments"][0]["energy"], 0.5)
        self.assertEqual(cached["request"]["max_tokens"], 16_000)


class CostEstimate(LaneTestCase):
    def test_a_whole_track_is_priced_from_its_answer_not_its_audio(self) -> None:
        estimate = mimo_openrouter.estimate_cost("xiaomi/mimo-v2.5", 200.0)
        self.assertEqual(estimate["input_tokens"], 1_480 + 900)
        self.assertEqual(estimate["completion_tokens_typical"], [4_000, 11_000])
        self.assertEqual(estimate["completion_tokens_max"], 16_000)
        self.assertEqual(estimate["usd_typical"], [0.008952, 0.022952])
        self.assertEqual(estimate["usd_max"], 0.032952)
        # The audio alone is a twentieth of the low end.
        self.assertLess(1_480 * 0.40 / 1e6 * 10, estimate["usd_typical"][0])

    def test_the_benchmarks_recorded_call_lands_inside_the_range(self) -> None:
        # 100 s, 5 110 completion tokens, reported 0.010596 USD.
        low, high = mimo_openrouter.estimate_cost("xiaomi/mimo-v2.5", 100.0)["usd_typical"]
        self.assertLessEqual(low, 0.010596)
        self.assertLessEqual(0.010596, high)

    def test_an_unlisted_model_gets_tokens_and_no_dollars(self) -> None:
        estimate = mimo_openrouter.estimate_cost("acme/never-priced", 200.0)
        self.assertEqual(estimate["input_tokens"], 2_380)
        self.assertIsNone(estimate["usd_typical"])
        self.assertIsNone(estimate["usd_max"])

    def test_the_dry_run_shows_the_estimate(self) -> None:
        payload, settings = mimo_openrouter.build_request(
            b"\x00", audio_format="wav", audio_duration=200.0)
        dump = mimo_openrouter.redacted_request_dump(payload, settings, "a" * 64)
        self.assertEqual(dump["cost_estimate"]["usd_max"], 0.032952)

    def test_the_cached_answer_records_the_estimate_beside_the_reported_cost(self) -> None:
        _, envelope = self.run_lane(response(answer(), cost=0.010596))
        generation = envelope["normalized"]["provenance"]["generation"]
        self.assertEqual(generation["estimate"]["usd_max"],
                         mimo_openrouter.estimate_cost("xiaomi/mimo-v2.5", DURATION)["usd_max"])
        self.assertEqual(generation["reported_cost_usd"], 0.010596)
        self.assertEqual(generation["usage"]["completion_tokens"], 5110)
        manifest = external_analysis.build_assist_manifest(
            mode="mimo", audio_sha="a" * 64, measured_duration=DURATION,
            cache_status={"semantic": "generated"}, paths={"manifest": Path("m.json")},
            plan={"sections": []}, lyrics=None, semantic=envelope["normalized"])
        self.assertEqual(manifest["semantic_cost"]["reported_cost_usd"], 0.010596)
        self.assertEqual(manifest["semantic_cost"]["completion_tokens"], 5110)
        self.assertTrue(manifest["semantic_cost"]["billed_this_run"])

    def test_a_failed_run_without_a_reported_cost_names_the_ceiling(self) -> None:
        with self.assertRaises(mimo_openrouter.FailedRun) as raised:
            self.run_lane(response(None, cost=None))
        ceiling = mimo_openrouter.estimate_cost("xiaomi/mimo-v2.5", DURATION)["usd_max"]
        self.assertIn(f"at most {ceiling:.3f} USD", str(raised.exception))


class FailedRunReachesTheUser(LaneTestCase):
    """Through `run_assist`, the sentence the job log's last line carries."""

    def runner_for(self, body: dict):
        def runner(argv, **_kwargs):  # noqa: ANN001
            self.assertTrue(argv[1].endswith("mimo_openrouter.py"), argv)
            try:
                mimo_openrouter.analyze_to_cache(
                    Path(argv[2]), Path(argv[3]), audio_duration=float(argv[5]),
                    transport=FakeOpenRouter(body), sleeper=lambda _s: None)
            except mimo_openrouter.FailedRun as error:
                return subprocess.CompletedProcess(argv, 1, "", f"MiMo analysis failed: {error}\n")
            return subprocess.CompletedProcess(argv, 0, "", "")
        return runner

    def run_assist(self, body: dict) -> None:
        def cache(path, *_args, **kwargs):  # noqa: ANN001
            if path.name == "measured.json":
                return {"audio": {"duration_seconds": DURATION}}
            if path.exists():
                value = json.loads(path.read_text())
                return value if kwargs["accept"](value) else None
            return None

        with patch.object(external_analysis, "_cache_matches", side_effect=cache), \
             patch.object(external_analysis, "build_scene_plan",
                          side_effect=RuntimeError("reached the planner")):
            external_analysis.run_assist(
                self.ws.audio, self.ws.root, audio_duration=DURATION, mode="mimo",
                semantic_cache=self.ws.cache, allow_dotenv=False,
                runner=self.runner_for(body))

    def test_the_named_reason_and_cost_replace_exited_with_code_1(self) -> None:
        with self.assertRaises(RuntimeError) as raised:
            self.run_assist(CUT_OFF)
        message = str(raised.exception)
        self.assertTrue(message.startswith("MiMo's answer was cut off"), message)
        self.assertIn("0.1312 USD", message)
        self.assertNotIn("exited with code", message)

    def test_a_well_formed_answer_reaches_the_planner(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "reached the planner"):
            self.run_assist(response(answer()))

    def test_a_receipt_for_other_audio_is_not_this_runs_reason(self) -> None:
        self.ws.receipt.write_text(json.dumps({
            "schema_version": mimo_openrouter.FAILED_RUN_SCHEMA_VERSION,
            "audio_sha256": "f" * 64, "message": "stale"}))
        self.assertIsNone(external_analysis._mimo_failure(self.ws.cache, "a" * 64))


if __name__ == "__main__":
    unittest.main()
