#!/usr/bin/env python3
"""The MiMo lane's request, its failed runs, its estimate and its scales.

Offline: a fake transport stands in for OpenRouter and returns recorded
response shapes (the two runaways of the 2026-11-14 benchmark, a null answer,
an off-scale number, a well-formed answer). No socket, no audio device, no
credential: the key is a sentinel set for the duration of a test.
"""

from __future__ import annotations

import base64
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
import semantic_openrouter  # noqa: E402


SENTINEL_KEY = "sk-or-v1-test-sentinel-not-a-key"
DURATION = 8.0
# The classes below up to `GeminiLane` pin the lane as MiMo ran it, numbers
# and sentences included; MiMo stays selectable, and its request must not move.
MIMO = "xiaomi/mimo-v2.5"
GEMINI = "google/gemini-3.8-flash"


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


def fake_encoder(audio_bytes: bytes, source_format: str, encoding, **_kwargs) -> bytes:  # noqa: ANN001
    """Stands in for FFmpeg: deterministic bytes that name what was asked."""
    return b"ID3-free " + encoding.identity().encode() + b" of " + audio_bytes


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
        self.receipt = semantic_openrouter.failure_receipt_path(self.cache)

    def close(self) -> None:
        self._dir.cleanup()


class LaneTestCase(unittest.TestCase):
    def setUp(self) -> None:
        self.ws = Workspace()
        self.addCleanup(self.ws.close)
        patcher = patch.dict(os.environ, {"OPENROUTER_API_KEY": SENTINEL_KEY})
        patcher.start()
        self.addCleanup(patcher.stop)
        # An unrouted `run_assist` asks for the helper's default model; these
        # classes pin MiMo's lane, so its default is MiMo here.
        default = patch.object(semantic_openrouter, "MODEL", self.MODEL)
        default.start()
        self.addCleanup(default.stop)

    MODEL = MIMO

    def run_lane(self, *bodies: object) -> tuple[FakeOpenRouter, object]:
        fake = FakeOpenRouter(*bodies)
        outcome = semantic_openrouter.analyze_to_cache(
            self.ws.audio, self.ws.cache, audio_duration=DURATION, model=self.MODEL,
            transport=fake, sleeper=lambda _seconds: None, encoder=fake_encoder)
        return fake, outcome


class CompletionCap(LaneTestCase):
    def test_the_request_caps_the_completion_and_the_cap_is_in_its_identity(self) -> None:
        payload, settings = semantic_openrouter.build_request(
            b"\x00", audio_format="wav", audio_duration=DURATION, model=MIMO)
        self.assertEqual(payload["max_tokens"], 16_000)
        self.assertEqual(settings["max_tokens"], 16_000)
        cap = semantic_openrouter.max_completion_tokens(MIMO)
        self.assertEqual(cap, 16_000)
        # The cap must leave room for the largest legitimate answer the
        # benchmark measured (10 942) and cut both runaways (43 578, 65 536).
        self.assertGreater(cap, 10_942)
        self.assertLess(cap, 43_578)

    def test_the_cap_is_what_the_helper_actually_sends(self) -> None:
        fake, _ = self.run_lane(response(answer()))
        self.assertEqual(fake.sent[0]["max_tokens"], 16_000)
        self.assertNotIn("reasoning", fake.sent[0])


class FailedRuns(LaneTestCase):
    def assert_failed(self, body: dict, reason_fragment: str, cost_text: str) -> dict:
        with self.assertRaises(semantic_openrouter.FailedRun) as raised:
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
        self.assertEqual(receipt["schema_version"], "musializer.semantic-failed-run/v1")
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
        self.assertEqual(len(receipt["content_start"]), semantic_openrouter.FAILED_CONTENT_KEEP)

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
        with self.assertRaises(semantic_openrouter.FailedRun):
            semantic_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                transport=fake, sleeper=lambda _seconds: None)
        self.assertEqual(len(fake.sent), 1)

    def test_a_rate_limit_is_retried_because_it_was_not_processed(self) -> None:
        limited = urllib.error.HTTPError(
            semantic_openrouter.OPENROUTER_URL, 429, "slow down", {"Retry-After": "0"},
            io.BytesIO(b""))
        fake, _ = self.run_lane(limited, response(answer()))
        self.assertEqual(len(fake.sent), 2)
        self.assertTrue(self.ws.cache.exists())

    def test_a_request_that_never_left_is_retried(self) -> None:
        refused = urllib.error.URLError(ConnectionRefusedError(111, "Connection refused"))
        fake, _ = self.run_lane(refused, response(answer()))
        self.assertEqual(len(fake.sent), 2)
        self.assertTrue(self.ws.cache.exists())

    def test_a_gateway_failure_after_sending_is_one_call_that_may_be_billed(self) -> None:
        # H4c: a 502 or 503 was retried up to three times, while the
        # confirmation budgets one call; three capped calls on 100 s are
        # about 0.097 USD against "at most 0.033".
        # H4d: every status that is not a clear refusal, not only the five
        # the H4c list named (a Cloudflare 524, a 418, a 409, a redirect).
        for status in (502, 503, 504, 408, 500, 524, 520, 418, 409, 307):
            with self.subTest(status=status):
                gateway = urllib.error.HTTPError(
                    semantic_openrouter.OPENROUTER_URL, status, "gateway", {}, io.BytesIO(b""))
                fake = FakeOpenRouter(gateway, response(answer()), response(answer()))
                with self.assertRaises(semantic_openrouter.FailedRun) as raised:
                    semantic_openrouter.analyze_to_cache(
                        self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                        transport=fake, sleeper=lambda _seconds: None)
                self.assertEqual(len(fake.sent), 1, "sent once, never again by itself")
                message = str(raised.exception)
                self.assertEqual(
                    message,
                    f"OpenRouter answered HTTP {status} after the request was sent; it may "
                    "have been billed (at most 0.033 USD). Nothing was saved or sent again.")
                self.assertLess(len(message.encode()), 160)
                self.assertFalse(self.ws.cache.exists())
                receipt = json.loads(self.ws.receipt.read_text())
                self.assertTrue(receipt["may_be_billed"])
                self.assertEqual(receipt["message"], message)
                self.assertIsNone(receipt["reported_cost_usd"])

    def test_a_clear_refusal_is_not_billed_and_not_resent(self) -> None:
        for status in sorted(semantic_openrouter.REFUSED_STATUSES):
            with self.subTest(status=status):
                refused = urllib.error.HTTPError(
                    semantic_openrouter.OPENROUTER_URL, status, "no", {}, io.BytesIO(b""))
                fake = FakeOpenRouter(refused, response(answer()))
                with self.assertRaises(RuntimeError) as raised:
                    semantic_openrouter.analyze_to_cache(
                        self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                        transport=fake, sleeper=lambda _seconds: None)
                self.assertNotIsInstance(raised.exception, semantic_openrouter.FailedRun)
                self.assertEqual(str(raised.exception),
                                 f"OpenRouter request failed with HTTP {status}")
                self.assertEqual(len(fake.sent), 1)
                self.assertFalse(self.ws.receipt.exists())

    def test_a_timeout_or_a_dropped_answer_may_be_billed_and_is_not_resent(self) -> None:
        import http.client
        for error, reason in (
                (TimeoutError("timed out"), "OpenRouter's answer did not arrive within 600 s"),
                (http.client.RemoteDisconnected("closed"),
                 "the connection dropped before OpenRouter answered (RemoteDisconnected)"),
                (http.client.IncompleteRead(b"{"),
                 "OpenRouter's answer was cut short (IncompleteRead)")):
            with self.subTest(error=type(error).__name__):
                fake = FakeOpenRouter(error, response(answer()))
                with self.assertRaises(semantic_openrouter.FailedRun) as raised:
                    semantic_openrouter.analyze_to_cache(
                        self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                        transport=fake, sleeper=lambda _seconds: None)
                self.assertEqual(len(fake.sent), 1)
                self.assertTrue(str(raised.exception).startswith(reason), str(raised.exception))
                self.assertIn("it may have been billed (at most 0.033 USD)", str(raised.exception))
                self.assertTrue(json.loads(self.ws.receipt.read_text())["may_be_billed"])

    def test_an_unreadable_answer_may_be_billed(self) -> None:
        class Garbled(FakeOpenRouter):
            def __call__(self, request, timeout: float):  # noqa: ANN001
                self.sent.append(json.loads(request.data.decode("utf-8")))
                return FakeHTTPResponse(b"<html>bad gateway</html>")
        fake = Garbled()
        with self.assertRaises(semantic_openrouter.FailedRun) as raised:
            semantic_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                transport=fake, sleeper=lambda _seconds: None)
        self.assertEqual(len(fake.sent), 1)
        self.assertTrue(str(raised.exception).startswith("OpenRouter's answer could not be read"))

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
        estimate = semantic_openrouter.estimate_cost("xiaomi/mimo-v2.5", 200.0)
        self.assertEqual(estimate["input_tokens"], 1_480 + 200)
        self.assertEqual(estimate["completion_tokens_typical"], [1_500, 11_000])
        self.assertEqual(estimate["completion_tokens_max"], 16_000)
        self.assertEqual(estimate["usd_typical"], [0.003672, 0.022672])
        self.assertEqual(estimate["usd_max"], 0.032672)
        # The audio alone is under a sixth of the low end.
        self.assertLess(1_480 * 0.40 / 1e6 * 6, estimate["usd_typical"][0])

    def test_the_benchmarks_recorded_call_lands_inside_the_range(self) -> None:
        # 100 s: the benchmark's 5 110 completion tokens (0.010596 USD) and
        # this lane's two capped calls, 2 407 and 2 044 (0.0051852, 0.00417248).
        low, high = semantic_openrouter.estimate_cost("xiaomi/mimo-v2.5", 100.0)["usd_typical"]
        for reported in (0.010596, 0.0051852, 0.00417248):
            self.assertLessEqual(low, reported)
            self.assertLessEqual(reported, high)

    def test_an_unlisted_model_gets_tokens_and_no_dollars(self) -> None:
        estimate = semantic_openrouter.estimate_cost("acme/never-priced", 200.0)
        self.assertEqual(estimate["input_tokens"], 1_680)
        self.assertIsNone(estimate["usd_typical"])
        self.assertIsNone(estimate["usd_max"])

    def test_the_dry_run_shows_the_estimate(self) -> None:
        payload, settings = semantic_openrouter.build_request(
            b"\x00", audio_format="wav", audio_duration=200.0, model=MIMO)
        dump = semantic_openrouter.redacted_request_dump(payload, settings, "a" * 64)
        self.assertEqual(dump["cost_estimate"]["usd_max"], 0.032672)

    def test_the_cached_answer_records_the_estimate_beside_the_reported_cost(self) -> None:
        _, envelope = self.run_lane(response(answer(), cost=0.010596))
        generation = envelope["normalized"]["provenance"]["generation"]
        self.assertEqual(generation["estimate"]["usd_max"],
                         semantic_openrouter.estimate_cost("xiaomi/mimo-v2.5", DURATION)["usd_max"])
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
        with self.assertRaises(semantic_openrouter.FailedRun) as raised:
            self.run_lane(response(None, cost=None))
        # 8 s is 260 prompt tokens: the capped call is 0.032104 USD, a bound
        # that reads 0.033 (rounded up), never 0.032.
        ceiling = semantic_openrouter.estimate_cost("xiaomi/mimo-v2.5", DURATION)["usd_max"]
        self.assertEqual(ceiling, 0.032104)
        self.assertIn("(at most 0.033 USD)", str(raised.exception))

    def test_at_most_is_rounded_up_as_the_rust_side_rounds_it(self) -> None:
        # The figure core::assist::cost pins at 100 s: 940 prompt tokens,
        # 0.032376 USD, shown as at most 0.033 USD on both sides.
        estimate = semantic_openrouter.estimate_cost("xiaomi/mimo-v2.5", 100.0)
        self.assertEqual(estimate["input_tokens"], 940)
        self.assertEqual(estimate["usd_max"], 0.032376)
        self.assertEqual(semantic_openrouter._cost_clause({}, estimate),
                         "OpenRouter reported no cost for it (at most 0.033 USD)")
        self.assertEqual(semantic_openrouter.ceil_usd(0.033, 3), 0.033)
        self.assertEqual(semantic_openrouter.ceil_usd(0.0320000000001, 3), 0.032)
        self.assertEqual(semantic_openrouter.ceil_usd(0.032001, 3), 0.033)


class NumericScales(LaneTestCase):
    def test_every_scale_is_stated_in_the_prompt_and_the_schema(self) -> None:
        expected = {"energy": (0, 1), "tension": (0, 1), "valence": (-1, 1),
                    "confidence": (0, 1)}
        self.assertEqual({name: (low, high) for name, (low, high, _) in
                          semantic_openrouter.NUMERIC_SCALES.items()}, expected)
        for where in ("global", "segments"):
            schema = semantic_openrouter.MODEL_OUTPUT_SCHEMA["properties"][where]
            properties = (schema["items"] if where == "segments" else schema)["properties"]
            for name, (low, high) in expected.items():
                with self.subTest(where=where, field=name):
                    field = properties[name]
                    self.assertEqual((field["minimum"], field["maximum"]), (low, high))
                    self.assertIn(f"from {low:g} to {high:g}", field["description"])
        for name, (low, high) in expected.items():
            self.assertIn(f"- {name}: a number from {low:g} to {high:g};",
                          semantic_openrouter.SYSTEM_PROMPT)
        self.assertIn("Never use percentages", semantic_openrouter.SYSTEM_PROMPT)

    def test_the_benchmarks_off_scale_energy_fails_the_run(self) -> None:
        # 78, 8 and 5: the three chunks of one excerpt, unprompted.
        for energy in (78, 8, 5):
            with self.subTest(energy=energy):
                with self.assertRaises(semantic_openrouter.FailedRun) as raised:
                    self.run_lane(response(answer(energy=energy)))
                message = str(raised.exception)
                self.assertIn(f"segments[0].energy is {energy}, outside 0 to 1", message)
                self.assertFalse(self.ws.cache.exists(), "never clamped into a result")
                self.ws.receipt.unlink()

    def test_valence_has_its_own_range(self) -> None:
        _, envelope = self.run_lane(response(answer(valence=-1.0)))
        self.assertEqual(envelope["normalized"]["segments"][0]["valence"], -1.0)
        self.ws.cache.unlink()
        with self.assertRaises(semantic_openrouter.FailedRun) as raised:
            self.run_lane(response(answer(valence=-1.5)))
        self.assertIn("valence is -1.5, outside -1 to 1", str(raised.exception))

    def test_float_noise_at_a_bound_is_the_bound(self) -> None:
        _, envelope = self.run_lane(response(answer(confidence=1.0000000002)))
        self.assertEqual(envelope["normalized"]["segments"][0]["confidence"], 1.0)

    def test_an_old_cache_is_not_read_as_a_new_answer_but_still_opens(self) -> None:
        v2 = external_analysis._semantic_request_identity(
            Path("song.wav"), audio_sha="a" * 64, measured_duration=DURATION, zdr=False)
        self.assertEqual(v2["prompt_version"], "musializer-semantic-score/v2")
        # A v1 cache as the previous helper wrote it: its identity named v1, a
        # different prompt and schema hash, and no cap.
        v1_identity = dict(v2, prompt_version="musializer-semantic-score/v1",
                           prompt_sha256="0" * 64, model_output_schema_sha256="1" * 64)
        v1_identity.pop("max_tokens")
        normalized = semantic_openrouter.normalize_semantic_score(
            json.loads(answer()), audio_sha256="a" * 64, audio_duration=DURATION,
            request_settings={k: v for k, v in v1_identity.items() if k != "audio_sha256"})
        normalized["provenance"]["prompt_version"] = "musializer-semantic-score/v1"
        old = {"schema_version": "musializer.analysis-cache/v1",
               "cache_key": external_analysis.canonical_sha256(v1_identity),
               "request": v1_identity, "raw_response": {}, "normalized": normalized}
        self.assertFalse(external_analysis._semantic_cache_accepts(old, request_identity=v2))
        # The document itself is still a semantic-score/v1 and still reads.
        self.assertEqual(external_analysis._semantic_document(old)["segments"][0]["energy"], 0.5)


class AFailedRegenerationIsSaid(LaneTestCase):
    """H4c: an earlier answer staged after a later attempt failed says so.

    A success, then a regeneration of the same request that fails (its
    receipt beside the earlier cache), then an Assist run that reuses the
    cache: the manifest the panel reads names the failure, when, why and what
    it may have cost, and which answer is staged.
    """

    def run_assist_to_manifest(self, runner) -> dict:  # noqa: ANN001
        def cache(path, *_args, **kwargs):  # noqa: ANN001
            if path.name == "measured.json":
                return {"audio": {"duration_seconds": DURATION}}
            if path.exists():
                value = json.loads(path.read_text())
                return value if kwargs["accept"](value) else None
            return None

        with patch.object(external_analysis, "_cache_matches", side_effect=cache), \
             patch.object(external_analysis, "build_scene_plan",
                          return_value={"sections": []}), \
             patch.object(external_analysis, "build_bridge", return_value=""):
            return external_analysis.run_assist(
                self.ws.audio, self.ws.root, audio_duration=DURATION, mode="mimo",
                semantic_cache=self.ws.cache, allow_dotenv=False, runner=runner)

    def helper(self, body: object):
        def runner(argv, **_kwargs):  # noqa: ANN001
            try:
                semantic_openrouter.analyze_to_cache(
                    Path(argv[2]), Path(argv[3]), audio_duration=float(argv[5]), model=argv[argv.index('--model') + 1],
                    transport=FakeOpenRouter(body), sleeper=lambda _s: None)
            except semantic_openrouter.FailedRun as error:
                return subprocess.CompletedProcess(argv, 1, "", f"Semantic analysis failed: {error}\n")
            return subprocess.CompletedProcess(argv, 0, "", "")
        return runner

    def never(self, argv, **_kwargs):  # noqa: ANN001
        self.fail(f"the cached answer was not reused: {argv}")

    def test_the_earlier_answer_is_staged_with_the_failure_named(self) -> None:
        first = self.run_assist_to_manifest(self.helper(response(answer())))
        self.assertEqual(first["cache_status"]["semantic"], "generated")
        self.assertNotIn("semantic_latest_failure", first)
        # A regeneration of exactly this request and cache fails, billed.
        with self.assertRaises(semantic_openrouter.FailedRun):
            semantic_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                transport=FakeOpenRouter(CUT_OFF), sleeper=lambda _s: None)
        self.assertTrue(self.ws.receipt.exists())
        failed_utc = json.loads(self.ws.receipt.read_text())["failed_utc"]
        manifest = self.run_assist_to_manifest(self.never)
        self.assertEqual(manifest["cache_status"]["semantic"], "reused")
        failure = manifest["semantic_latest_failure"]
        self.assertTrue(failure["same_request"])
        self.assertEqual(failure["reported_cost_usd"], 0.1312484)
        self.assertEqual(failure["answer_in_use"]["generation_id"], "gen-test")
        when = f"{failed_utc[:10]} {failed_utc[11:16]} UTC"
        self.assertEqual(
            failure["words"],
            f"The latest MiMo attempt failed on {when}: MiMo's answer was cut off at the "
            "16000-token cap. Nothing was saved; OpenRouter billed 0.1312 USD for it. The "
            "feelings staged here are the earlier answer, made on 2026-11-14 18:53 UTC, which "
            "was kept.")
        # Said on every reuse until an attempt succeeds.
        again = self.run_assist_to_manifest(self.never)
        self.assertEqual(again["semantic_latest_failure"]["words"], failure["words"])

    def test_a_gateway_failure_on_regeneration_says_it_may_have_been_billed(self) -> None:
        self.run_assist_to_manifest(self.helper(response(answer())))
        gateway = urllib.error.HTTPError(
            semantic_openrouter.OPENROUTER_URL, 502, "gateway", {}, io.BytesIO(b""))
        with self.assertRaises(semantic_openrouter.FailedRun):
            semantic_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                transport=FakeOpenRouter(gateway), sleeper=lambda _s: None)
        failure = self.run_assist_to_manifest(self.never)["semantic_latest_failure"]
        self.assertTrue(failure["may_be_billed"])
        self.assertIn("it may have been billed (at most 0.033 USD)", failure["words"])

    def test_a_failure_older_than_the_answer_is_not_its_word(self) -> None:
        # H4d: two helpers on one cache. Helper B starts (its start removes
        # any receipt), helper A then fails and writes its receipt, and B
        # succeeds after it. The answer is newer than the failure: B's success
        # removes the receipt, and a receipt left by anyone older than the
        # answer is never read as "the latest attempt failed".
        failing = FakeOpenRouter(CUT_OFF)

        class FailsAnotherFirst(FakeOpenRouter):
            def __call__(inner, request, timeout: float):  # noqa: ANN001, N805
                with self.assertRaises(semantic_openrouter.FailedRun):
                    semantic_openrouter.analyze_to_cache(
                        self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
                        transport=failing, sleeper=lambda _s: None)
                self.assertTrue(self.ws.receipt.exists(), "helper A's receipt")
                return super().__call__(request, timeout)

        semantic_openrouter.analyze_to_cache(
            self.ws.audio, self.ws.cache, audio_duration=DURATION, model=MIMO,
            transport=FailsAnotherFirst(response(answer())), sleeper=lambda _s: None)
        self.assertFalse(self.ws.receipt.exists(), "B's success removed A's older receipt")
        manifest = self.run_assist_to_manifest(self.never)
        self.assertNotIn("semantic_latest_failure", manifest)
        # A receipt older than the answer that is still there (written by a
        # helper of before H4d, say) is not the answer's word either.
        envelope = json.loads(self.ws.cache.read_text())
        stale = {"schema_version": semantic_openrouter.FAILED_RUN_SCHEMA_VERSION,
                 "failed_utc": "2026-11-14T18:00:00Z",
                 "failed_ns": envelope["answered_ns"] - 1,
                 "audio_sha256": envelope["request"]["audio_sha256"],
                 "message": "an older failure"}
        self.ws.receipt.write_text(json.dumps(stale))
        self.assertNotIn("semantic_latest_failure", self.run_assist_to_manifest(self.never))
        # One that failed after the answer is.
        stale["failed_ns"] = envelope["answered_ns"] + 1
        self.ws.receipt.write_text(json.dumps(stale))
        failure = self.run_assist_to_manifest(self.never)["semantic_latest_failure"]
        self.assertEqual(failure["message"], "an older failure")
        # A success removes only a receipt older than itself.
        self.assertTrue(self.ws.receipt.exists())
        semantic_openrouter._drop_older_receipt(self.ws.cache, envelope["answered_ns"])
        self.assertTrue(self.ws.receipt.exists(), "a later failure's receipt stays")

    def test_no_receipt_no_word(self) -> None:
        self.run_assist_to_manifest(self.helper(response(answer())))
        manifest = self.run_assist_to_manifest(self.never)
        self.assertNotIn("semantic_latest_failure", manifest)


class FailedRunReachesTheUser(LaneTestCase):
    """Through `run_assist`, the sentence the job log's last line carries."""

    def runner_for(self, body: dict):
        def runner(argv, **_kwargs):  # noqa: ANN001
            self.assertTrue(argv[1].endswith("semantic_openrouter.py"), argv)
            try:
                semantic_openrouter.analyze_to_cache(
                    Path(argv[2]), Path(argv[3]), audio_duration=float(argv[5]), model=argv[argv.index('--model') + 1],
                    transport=FakeOpenRouter(body), sleeper=lambda _s: None)
            except semantic_openrouter.FailedRun as error:
                return subprocess.CompletedProcess(argv, 1, "", f"Semantic analysis failed: {error}\n")
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
            "schema_version": semantic_openrouter.FAILED_RUN_SCHEMA_VERSION,
            "audio_sha256": "f" * 64, "message": "stale"}))
        self.assertIsNone(external_analysis._semantic_failure(self.ws.cache, "a" * 64))


class WrittenByTheOldName(LaneTestCase):
    """Files written while this helper was tools/mimo_openrouter.py still read."""

    def test_a_cache_made_under_the_old_name_is_this_requests_answer(self) -> None:
        _, envelope = self.run_lane(response(answer()))
        identity = external_analysis._semantic_request_identity(
            self.ws.audio, audio_sha=envelope["request"]["audio_sha256"],
            measured_duration=DURATION, zdr=False)
        self.assertEqual(envelope["normalized"]["provenance"]["adapter"],
                         "tools/semantic_openrouter.py")
        self.assertEqual(envelope["normalized"]["provenance"]["source_kind"],
                         "openrouter_semantic")
        self.assertTrue(external_analysis._semantic_cache_accepts(
            envelope, request_identity=identity))
        old = json.loads(json.dumps(envelope))
        old["normalized"]["provenance"].update(
            adapter="tools/mimo_openrouter.py", source_kind="mimo_openrouter")
        self.assertTrue(external_analysis._semantic_cache_accepts(
            old, request_identity=identity), "a MiMo answer is not billed again for a rename")
        # Any other maker is not.
        old["normalized"]["provenance"].update(adapter="tools/other.py")
        self.assertFalse(external_analysis._semantic_cache_accepts(
            old, request_identity=identity))

    def test_a_receipt_under_the_old_schema_is_still_read(self) -> None:
        self.ws.receipt.write_text(json.dumps({
            "schema_version": "musializer.mimo-failed-run/v1",
            "audio_sha256": "a" * 64, "message": "MiMo returned no answer text."}))
        self.assertEqual(external_analysis._semantic_failure(self.ws.cache, "a" * 64),
                         "MiMo returned no answer text.")
        self.ws.receipt.write_text(json.dumps({
            "schema_version": "musializer.other-receipt/v1",
            "audio_sha256": "a" * 64, "message": "not ours"}))
        self.assertIsNone(external_analysis._semantic_failure(self.ws.cache, "a" * 64))


def gemini_response(content: object, *, finish: object = "stop", completion_tokens: int = 2231,
                    reasoning_tokens: int = 641, cost: float | None = 0.01148025) -> dict:
    """A chat completion in the shape OpenRouter returned for Gemini 3.8 Flash
    on 2026-11-15 (usage figures from the first real call; the answer text is
    synthetic). Thinking is reported under `completion_tokens_details`."""
    usage: dict = {
        "prompt_tokens": 4152, "completion_tokens": completion_tokens,
        "completion_tokens_details": {"reasoning_tokens": reasoning_tokens,
                                      "audio_tokens": 0, "image_tokens": 0},
        "prompt_tokens_details": {"audio_tokens": 2502, "cached_tokens": 0},
        "total_tokens": 4152 + completion_tokens, "is_byok": False,
    }
    if cost is not None:
        usage["cost"] = cost
    return {
        "id": "gen-gemini-test", "created": 1794758836, "model": GEMINI,
        "provider": "Google", "object": "chat.completion",
        "choices": [{"finish_reason": finish, "native_finish_reason": "STOP",
                     "message": {"role": "assistant", "content": content,
                                 "reasoning": "short thought", "refusal": None}}],
        "usage": usage,
    }


class GeminiLane(LaneTestCase):
    """The lane on its model since 2026-11-15: the same guards, its own request."""

    MODEL = GEMINI

    def test_the_request_is_shaped_for_gemini(self) -> None:
        fake, _ = self.run_lane(gemini_response(answer()))
        sent = fake.sent[0]
        self.assertEqual(sent["model"], GEMINI)
        self.assertEqual(sent["max_tokens"], 16_000)
        # Not sent: Gemini 3 wants its default, and the ZDR (Vertex) endpoints
        # do not list the parameter, so `require_parameters` would leave none.
        self.assertNotIn("temperature", sent)
        self.assertNotIn("reasoning", sent)
        audio = sent["messages"][0]["content"][1]["input_audio"]
        self.assertEqual(audio["format"], "mp3", "the source was a .wav; mp3 is what is sent")
        self.assertEqual(base64.b64decode(audio["data"]),
                         b"ID3-free mp3 libmp3lame 44100 Hz 2 ch 128k of RIFF-not-really-audio")
        self.assertEqual(sent["response_format"]["type"], "json_schema")
        self.assertTrue(sent["provider"]["require_parameters"])

    def test_the_identity_carries_the_shape_and_the_cap(self) -> None:
        _, settings = semantic_openrouter.build_request(
            b"x", audio_format="wav", audio_duration=DURATION, model=GEMINI)
        self.assertEqual(settings["audio_format"], "mp3")
        self.assertEqual(settings["audio_encoding"], "mp3 libmp3lame 44100 Hz 2 ch 128k")
        self.assertIsNone(settings["temperature"])
        self.assertEqual(settings["max_tokens"], 16_000)
        identity = external_analysis._semantic_request_identity(
            Path("song.flac"), audio_sha="a" * 64, measured_duration=DURATION, zdr=True)
        self.assertEqual(identity["model"], GEMINI, "the unrouted default is Gemini")
        self.assertEqual({k: v for k, v in identity.items() if k != "audio_sha256"},
                         dict(settings, zero_data_retention=True))

    def test_a_mimo_answer_is_not_served_as_geminis(self) -> None:
        mimo = external_analysis._semantic_request_identity(
            Path("song.wav"), audio_sha="a" * 64, measured_duration=DURATION, zdr=False,
            semantic=dict(external_analysis._semantic_route(None), model=MIMO))
        gemini = external_analysis._semantic_request_identity(
            Path("song.wav"), audio_sha="a" * 64, measured_duration=DURATION, zdr=False)
        self.assertNotEqual(external_analysis.canonical_sha256(mimo),
                            external_analysis.canonical_sha256(gemini))
        _, envelope = self.run_lane_as(MIMO, response(answer()))
        audio_sha = envelope["request"]["audio_sha256"]
        mimo["audio_sha256"] = gemini["audio_sha256"] = audio_sha
        self.assertTrue(external_analysis._semantic_cache_accepts(envelope, request_identity=mimo))
        self.assertFalse(external_analysis._semantic_cache_accepts(envelope, request_identity=gemini))

    def run_lane_as(self, model: str, *bodies: object):  # noqa: ANN201
        fake = FakeOpenRouter(*bodies)
        outcome = semantic_openrouter.analyze_to_cache(
            self.ws.audio, self.ws.cache, audio_duration=DURATION, model=model,
            transport=fake, sleeper=lambda _seconds: None, encoder=fake_encoder)
        return fake, outcome

    def test_mimos_request_is_byte_for_byte_what_it_was(self) -> None:
        # Hashes of the identity and the body as tools/mimo_openrouter.py built
        # them at 2d5d1b6, for the same inputs: a user who stays on MiMo keeps
        # every cached answer, and MiMo is sent exactly what it was sent.
        kwargs = dict(audio_duration=200.0, audio_format="mp3", provider_order=["venice"],
                      allow_fallbacks=False, zero_data_retention=True,
                      max_price={"prompt": 1.0})
        settings = semantic_openrouter.request_settings(model=MIMO, **kwargs)
        payload, _ = semantic_openrouter.build_request(b"abc", model=MIMO, **kwargs)
        self.assertEqual(
            semantic_openrouter.canonical_sha256(settings),
            "18163520c97f2ee718c5760e14a47c78ac3563865dc72f05091ee8cc69221dd4")
        self.assertEqual(
            semantic_openrouter.canonical_sha256(payload),
            "8fae05410bfe4b042805bf33ee899abafe5c904bd6999ecb40e08488732eebac")

    def test_a_well_formed_answer_is_cached_with_its_estimate_and_cost(self) -> None:
        _, envelope = self.run_lane(gemini_response(answer()))
        cached = json.loads(self.ws.cache.read_text())
        self.assertEqual(cached["cache_key"], envelope["cache_key"])
        provenance = cached["normalized"]["provenance"]
        self.assertEqual(provenance["model"], GEMINI)
        self.assertEqual(provenance["provider"], "Google")
        generation = provenance["generation"]
        self.assertEqual(generation["reported_cost_usd"], 0.01148025)
        self.assertEqual(generation["usage"]["completion_tokens_details"]["reasoning_tokens"], 641)
        self.assertEqual(generation["estimate"]["version"], "musializer.semantic-cost-estimate/v1")
        self.assertEqual(generation["estimate"]["prices_usd_per_million"],
                         {"prompt": 0.825, "audio": 0.825, "completion": 4.125})

    def assert_failed(self, body: dict, words: str) -> dict:
        with self.assertRaises(semantic_openrouter.FailedRun) as raised:
            self.run_lane(body)
        message = str(raised.exception)
        self.assertEqual(message, words)
        self.assertLessEqual(len(message.encode("utf-8")), 160, message)
        self.assertFalse(self.ws.cache.exists(), "a failed run is never cached")
        receipt = json.loads(self.ws.receipt.read_text())
        self.assertEqual(receipt["message"], message)
        self.ws.receipt.unlink()
        return receipt

    def test_a_cut_off_answer_is_a_failed_run(self) -> None:
        receipt = self.assert_failed(
            gemini_response('{"global": {"summary": "an', finish="length",
                            completion_tokens=16_000, reasoning_tokens=14_000, cost=0.0631),
            "Gemini's answer was cut off at the 16000-token cap. Nothing was saved; "
            "OpenRouter billed 0.0631 USD for it.")
        self.assertEqual(receipt["finish_reason"], "length")
        self.assertFalse(receipt["may_be_billed"])
        self.assertEqual(receipt["reported_cost_usd"], 0.0631)

    def test_empty_or_missing_content_is_a_failed_run(self) -> None:
        self.assert_failed(gemini_response("", cost=0.004),
                           "Gemini returned an empty answer. Nothing was saved; "
                           "OpenRouter billed 0.0040 USD for it.")
        # Unreported cost: the ceiling for 8 s is 0.06757245, at most 0.068.
        self.assert_failed(gemini_response(None, cost=None),
                           "Gemini returned no answer text. Nothing was saved; "
                           "OpenRouter reported no cost for it (at most 0.068 USD).")

    def test_an_off_scale_number_fails_the_run(self) -> None:
        self.assert_failed(
            gemini_response(answer(tension=7)),
            "Gemini's answer does not fit the schema (segments[0].tension is 7, outside 0 to 1). "
            "Nothing was saved; OpenRouter billed 0.0115 USD for it.")

    def test_a_refusal_is_free_and_an_unclear_status_may_be_billed(self) -> None:
        refused = urllib.error.HTTPError(semantic_openrouter.OPENROUTER_URL, 404,
                                         "no endpoint", {}, io.BytesIO(b""))
        fake = FakeOpenRouter(refused, gemini_response(answer()))
        with self.assertRaises(RuntimeError) as raised:
            self.run_lane_as(GEMINI, refused)
        self.assertNotIsInstance(raised.exception, semantic_openrouter.FailedRun)
        self.assertEqual(str(raised.exception), "OpenRouter request failed with HTTP 404")
        self.assertFalse(self.ws.receipt.exists())
        del fake
        unclear = urllib.error.HTTPError(semantic_openrouter.OPENROUTER_URL, 524,
                                         "timeout", {}, io.BytesIO(b""))
        fake = FakeOpenRouter(unclear, gemini_response(answer()))
        with self.assertRaises(semantic_openrouter.FailedRun) as raised:
            semantic_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION, model=GEMINI,
                transport=fake, sleeper=lambda _s: None, encoder=fake_encoder)
        self.assertEqual(len(fake.sent), 1, "sent once, never again by itself")
        self.assertEqual(str(raised.exception),
                         "OpenRouter answered HTTP 524 after the request was sent; it may "
                         "have been billed (at most 0.068 USD). Nothing was saved or sent again.")
        self.assertTrue(json.loads(self.ws.receipt.read_text())["may_be_billed"])

    def test_a_request_too_large_for_the_model_is_never_sent(self) -> None:
        fake = FakeOpenRouter(gemini_response(answer()))
        with self.assertRaises(RuntimeError) as raised:
            semantic_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION, model=GEMINI,
                transport=fake, sleeper=lambda _s: None,
                encoder=lambda *_args, **_kw: b"\x00" * 15_000_000)
        self.assertNotIsInstance(raised.exception, semantic_openrouter.FailedRun)
        self.assertEqual(fake.sent, [], "nothing left the machine")
        self.assertIn("past the 19.5 MB one Gemini request may carry; nothing was sent",
                      str(raised.exception))
        receipt = json.loads(self.ws.receipt.read_text())
        self.assertFalse(receipt["may_be_billed"])
        self.assertEqual(external_analysis._semantic_failure(self.ws.cache,
                                                             receipt["audio_sha256"]),
                         receipt["message"])
        # Just under the limit is sent.
        self.ws.receipt.unlink()
        fake = FakeOpenRouter(gemini_response(answer()))
        semantic_openrouter.analyze_to_cache(
            self.ws.audio, self.ws.cache, audio_duration=DURATION, model=GEMINI,
            transport=fake, sleeper=lambda _s: None,
            encoder=lambda *_args, **_kw: b"\x00" * 14_000_000)
        self.assertEqual(len(fake.sent), 1)

    def test_an_encoder_failure_says_why_and_that_nothing_was_sent(self) -> None:
        def broken(*_args, **_kwargs):  # noqa: ANN002, ANN003
            raise RuntimeError("FFmpeg was not found, and it is needed to encode the track; "
                               "nothing was sent")
        fake = FakeOpenRouter(gemini_response(answer()))
        with self.assertRaises(RuntimeError):
            semantic_openrouter.analyze_to_cache(
                self.ws.audio, self.ws.cache, audio_duration=DURATION, model=GEMINI,
                transport=fake, sleeper=lambda _s: None, encoder=broken)
        self.assertEqual(fake.sent, [])
        receipt = json.loads(self.ws.receipt.read_text())
        self.assertEqual(receipt["message"], "FFmpeg was not found, and it is needed to "
                         "encode the track; nothing was sent.")
        self.assertFalse(receipt["may_be_billed"])

    def test_an_unrouted_assist_run_spawns_the_helper_for_gemini(self) -> None:
        seen: list[list[str]] = []

        def runner(argv, **_kwargs):  # noqa: ANN001
            seen.append(list(argv))
            semantic_openrouter.analyze_to_cache(
                Path(argv[2]), Path(argv[3]), audio_duration=float(argv[5]),
                model=argv[argv.index("--model") + 1],
                transport=FakeOpenRouter(gemini_response(answer())),
                sleeper=lambda _s: None, encoder=fake_encoder)
            return subprocess.CompletedProcess(argv, 0, "", "")

        def cache(path, *_args, **kwargs):  # noqa: ANN001
            if path.name == "measured.json":
                return {"audio": {"duration_seconds": DURATION}}
            if path.exists():
                value = json.loads(path.read_text())
                return value if kwargs["accept"](value) else None
            return None

        with patch.object(external_analysis, "_cache_matches", side_effect=cache), \
             patch.object(external_analysis, "build_scene_plan", return_value={"sections": []}), \
             patch.object(external_analysis, "build_bridge", return_value=""):
            manifest = external_analysis.run_assist(
                self.ws.audio, self.ws.root, audio_duration=DURATION, mode="mimo",
                semantic_cache=self.ws.cache, allow_dotenv=False, runner=runner)
        self.assertTrue(seen[0][1].endswith("tools/semantic_openrouter.py"), seen[0])
        self.assertEqual(seen[0][seen[0].index("--model") + 1], GEMINI)
        self.assertEqual(manifest["semantic_cost"]["reported_cost_usd"], 0.01148025)


class RealEncoding(unittest.TestCase):
    """FFmpeg itself, on a synthetic tone: what leaves is MP3 with no tags."""

    def test_tags_never_leave_with_the_audio(self) -> None:
        try:
            ffmpeg = semantic_openrouter.ffmpeg_executable()
        except RuntimeError:
            self.skipTest("FFmpeg is not installed")
        with tempfile.TemporaryDirectory() as folder:
            tagged = Path(folder) / "tone.flac"
            subprocess.run(
                [ffmpeg, "-nostdin", "-v", "error", "-f", "lavfi", "-i",
                 "sine=frequency=440:duration=2", "-metadata", "title=PRIVATE-TITLE-CANARY",
                 "-metadata", "artist=PRIVATE-ARTIST-CANARY", str(tagged)],
                check=True, timeout=60)
            encoding = semantic_openrouter.model_profile(GEMINI).audio_encoding
            sent = semantic_openrouter.encode_audio(tagged.read_bytes(), "flac", encoding)
        self.assertNotIn(b"PRIVATE-TITLE-CANARY", sent)
        self.assertNotIn(b"PRIVATE-ARTIST-CANARY", sent)
        self.assertFalse(sent.startswith(b"ID3"), "no ID3 header at all")
        self.assertEqual(sent[0], 0xFF, "an MPEG audio frame comes first")
        self.assertGreater(len(sent), 10_000)

    def _tone(self, folder: Path, name: str, seconds: int, *extra: str) -> Path:
        ffmpeg = semantic_openrouter.ffmpeg_executable()
        path = folder / name
        subprocess.run(
            [ffmpeg, "-nostdin", "-v", "error", "-f", "lavfi", "-i",
             f"sine=frequency=440:duration={seconds}", *extra, str(path)],
            check=True, timeout=120)
        return path

    def test_an_encoding_past_the_limit_stops_and_says_nothing_was_sent(self) -> None:
        try:
            semantic_openrouter.ffmpeg_executable()
        except RuntimeError:
            self.skipTest("FFmpeg is not installed")
        encoding = semantic_openrouter.model_profile(GEMINI).audio_encoding
        with tempfile.TemporaryDirectory() as folder:
            tone = self._tone(Path(folder), "long.flac", 120)
            # 120 s at 128 kbit/s is about 1.9 MB of MP3.
            with self.assertRaises(RuntimeError) as caught:
                semantic_openrouter.encode_audio(
                    tone.read_bytes(), "flac", encoding, max_bytes=200_000)
            self.assertIn("encodes to more than 0.2 MB", str(caught.exception))
            self.assertIn("nothing was sent", str(caught.exception))
            whole = semantic_openrouter.encode_audio(
                tone.read_bytes(), "flac", encoding, max_bytes=4_000_000)
            self.assertGreater(len(whole), 1_500_000)

    def test_an_index_at_the_end_is_still_read_and_no_copy_is_left(self) -> None:
        try:
            semantic_openrouter.ffmpeg_executable()
        except RuntimeError:
            self.skipTest("FFmpeg is not installed")
        encoding = semantic_openrouter.model_profile(GEMINI).audio_encoding
        with tempfile.TemporaryDirectory() as folder, \
                tempfile.TemporaryDirectory() as workdir:
            # FFmpeg writes an M4A's index last unless told otherwise: a pipe
            # could not read this file, a seekable descriptor can.
            tone = self._tone(Path(folder), "tone.m4a", 180, "-c:a", "aac")
            sent = semantic_openrouter.encode_audio(
                tone.read_bytes(), "m4a", encoding, workdir=Path(workdir))
            self.assertEqual(sent[0], 0xFF)
            # Long enough that a reader which cannot seek back from the index
            # to the audio fails: a three-second file fits FFmpeg's read-ahead.
            self.assertGreater(len(sent), 2_500_000)
            self.assertEqual(list(Path(workdir).iterdir()), [], "no private copy on disk")

    def test_the_encoder_never_sees_the_key(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            seen = Path(folder) / "environment.txt"
            stand_in = Path(folder) / "ffmpeg"
            stand_in.write_text('#!/bin/sh\nenv > "$TMPDIR/environment.txt"\nprintf "\\377encoded"\n')
            stand_in.chmod(0o755)
            encoding = semantic_openrouter.model_profile(GEMINI).audio_encoding
            with patch.dict(os.environ, {
                    "OPENROUTER_API_KEY": "sk-or-CANARY-MUST-NOT-LEAK",
                    "SOME_OTHER_TOKEN": "CANARY-TOO", "TMPDIR": folder}), \
                    patch.object(semantic_openrouter, "ffmpeg_executable",
                                      return_value=str(stand_in)):
                sent = semantic_openrouter.encode_audio(b"audio", "flac", encoding)
            self.assertEqual(sent, b"\xffencoded")
            environment = seen.read_text()
            self.assertNotIn("CANARY", environment)
            self.assertIn("TMPDIR=", environment)


if __name__ == "__main__":
    unittest.main()
