#!/usr/bin/env python3
"""The execution snapshot, route wiring and cache identity (tranche P4).

Authority: ``docs/ASSIST_PROVIDER_CONTRACTS.md`` §5 and §6. Everything here is
pure: no subprocess, no network, no audio. The application resolves the route
graph and writes the record; what these tests pin is what the *helper* does with
it — which model reaches which flag, which artifact is regenerated, and which
identity ends up in provenance.
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import external_analysis  # noqa: E402
import mimo_openrouter  # noqa: E402
from analysis_io import AnalysisValidationError  # noqa: E402


def snapshot(**overrides: object) -> dict:
    """A well-formed `musializer.assist-execution/v1` record.

    Written out here rather than generated from the Rust side on purpose: this
    is the *reader's* fixture, and a fixture produced by the writer could hide
    exactly the disagreement these tests exist to catch.
    """
    contracts = [
        {
            "contract": "TC-MEASURED", "route_type": "builtin",
            "runtime_id": "builtin-analyzer", "runtime_version": None,
            "model_id": "builtin-analyzer", "model_sha256": None,
            "reasoning_effort": None, "boundary_applied": "local-only",
            "boundary_confirmed": False, "audio_scope": "none",
            "excerpt_spans": [], "provider_constraints": None,
            "provider_served": None, "prompt_version": None,
            "prompt_sha256": None, "schema_version": None,
            "fallback_policy": "none", "fallback_taken": False,
            "fallback_from": None,
        },
        {
            "contract": "TC-WORDING", "route_type": "codex",
            "runtime_id": "codex", "runtime_version": None,
            "model_id": "gpt-5.6-sol", "model_sha256": None,
            "reasoning_effort": "high", "boundary_applied": "text-leaves-machine",
            "boundary_confirmed": True, "audio_scope": "none",
            "excerpt_spans": [], "provider_constraints": None,
            "provider_served": None, "prompt_version": None,
            "prompt_sha256": None, "schema_version": None,
            "fallback_policy": "none", "fallback_taken": False,
            "fallback_from": None,
        },
        {
            "contract": "TC-SEMANTIC", "route_type": "openrouter",
            "runtime_id": "openrouter", "runtime_version": None,
            "model_id": "google/gemini-2.5-flash", "model_sha256": None,
            "reasoning_effort": None, "boundary_applied": "audio-leaves-machine",
            "boundary_confirmed": True, "audio_scope": "whole-track",
            "excerpt_spans": [], "provider_constraints": {
                "order": ["fireworks"], "only": [], "ignore": ["deepinfra"],
                "allow_fallbacks": False, "zdr_required": True,
                "max_price_audio": 12.5,
            },
            "provider_served": None, "prompt_version": None,
            "prompt_sha256": None, "schema_version": None,
            "fallback_policy": "none", "fallback_taken": False,
            "fallback_from": None,
        },
    ]
    document = {
        "snapshot_schema": "musializer.assist-execution/v1",
        "settings_schema": "musializer.assist-settings/v1",
        "profile_id": "studio",
        "resolved_at_utc": "2026-08-05T12:00:00Z",
        "contracts": contracts,
        "catalog_revision": None,
        "suitability_revision": "musializer.assist-suitability/2026-08-05",
        "credential_present": True,
        "credential_fingerprint": "0a1b2c3d",
    }
    document.update(overrides)
    return document


class ExecutionSnapshotReading(unittest.TestCase):
    def test_absent_path_is_an_unrouted_run_rather_than_an_error(self) -> None:
        self.assertIsNone(external_analysis.read_execution_snapshot(None))

    def test_a_foreign_schema_is_refused_rather_than_ignored(self) -> None:
        # A job that believes it is routed and is not would record provenance
        # nobody chose, so this is an error and not a fall back to defaults.
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "snapshot.json"
            path.write_text(json.dumps(
                snapshot(snapshot_schema="musializer.assist-execution/v2")))
            with self.assertRaises(AnalysisValidationError) as raised:
                external_analysis.read_execution_snapshot(path)
            self.assertIn("musializer.assist-execution/v1", str(raised.exception))

            path.write_text(json.dumps({"snapshot_schema":
                                        "musializer.assist-execution/v1"}))
            with self.assertRaises(AnalysisValidationError):
                external_analysis.read_execution_snapshot(path)

            with self.assertRaises(AnalysisValidationError):
                external_analysis.read_execution_snapshot(Path(scratch) / "absent")

    def test_an_implausibly_large_snapshot_is_refused_before_it_is_parsed(self) -> None:
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "snapshot.json"
            path.write_text(" " * (external_analysis.EXECUTION_SNAPSHOT_BYTE_LIMIT + 1))
            with self.assertRaises(AnalysisValidationError):
                external_analysis.read_execution_snapshot(path)

    def test_a_schema_legal_route_without_a_helper_adapter_is_refused(self) -> None:
        document = snapshot()
        document["contracts"].append({
            "contract": "TC-COARSE", "route_type": "openrouter",
            "runtime_id": "openrouter", "model_id": "google/gemini-test",
        })
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "snapshot.json"
            path.write_text(json.dumps(document))
            with self.assertRaises(AnalysisValidationError) as raised:
                external_analysis.read_execution_snapshot(path)
            self.assertIn("does not implement", str(raised.exception))


class RouteIdentity(unittest.TestCase):
    def test_the_identity_is_exactly_what_a_user_can_change(self) -> None:
        identity = external_analysis.route_identity(snapshot(), "TC-SEMANTIC")
        self.assertEqual(
            set(identity),
            set(external_analysis.EXECUTION_ROUTE_IDENTITY_FIELDS),
        )
        # Observed fields describe the run, not the route: including them would
        # regenerate every artifact whenever a provider rebalanced.
        for observed in ("provider_served", "runtime_version", "model_sha256"):
            self.assertNotIn(observed, identity)
        self.assertIsNone(external_analysis.route_identity(snapshot(), "TC-ALIGN"))

    def test_changing_the_model_or_the_constraints_changes_the_identity(self) -> None:
        base = external_analysis.route_identity(snapshot(), "TC-SEMANTIC")

        rerouted = snapshot()
        rerouted["contracts"][2]["model_id"] = "xiaomi/mimo-v2.5"
        self.assertNotEqual(
            external_analysis.route_identity(rerouted, "TC-SEMANTIC"), base)

        relaxed = snapshot()
        relaxed["contracts"][2]["provider_constraints"]["zdr_required"] = False
        self.assertNotEqual(
            external_analysis.route_identity(relaxed, "TC-SEMANTIC"), base)

        # And an observed field moving leaves it alone.
        served = snapshot()
        served["contracts"][2]["provider_served"] = "fireworks"
        self.assertEqual(
            external_analysis.route_identity(served, "TC-SEMANTIC"), base)

    def test_a_routed_run_refuses_an_artifact_with_no_identity(self) -> None:
        """§5 rule 7. An unknown route is not a matching route."""
        identity = external_analysis.route_identity(snapshot(), "TC-SEMANTIC")
        pre_p4 = {"provenance": {"adapter": "x"}}
        self.assertFalse(external_analysis._execution_route_accepts(pre_p4, identity))
        # An unrouted run compares nothing, so the command line is unchanged.
        self.assertTrue(external_analysis._execution_route_accepts(pre_p4, None))

        stamped = {"provenance": {"execution": {"route_identity": identity}}}
        self.assertTrue(external_analysis._execution_route_accepts(stamped, identity))
        other = dict(identity, model_id="something/else")
        self.assertFalse(external_analysis._execution_route_accepts(stamped, other))

    def test_the_provenance_stamp_names_the_snapshot_and_the_contract(self) -> None:
        stamp = external_analysis.execution_provenance(snapshot(), "TC-WORDING")
        self.assertEqual(stamp["contract"], "TC-WORDING")
        self.assertEqual(stamp["profile_id"], "studio")
        self.assertEqual(stamp["snapshot_schema"], "musializer.assist-execution/v1")
        self.assertEqual(stamp["route_identity"]["reasoning_effort"], "high")
        self.assertIsNone(external_analysis.execution_provenance(snapshot(), "TC-ALIGN"))


class SemanticRouteWiring(unittest.TestCase):
    def test_the_route_supplies_the_model_and_every_constraint(self) -> None:
        route = external_analysis._semantic_route(
            external_analysis.execution_route(snapshot(), "TC-SEMANTIC"))
        self.assertEqual(route["model"], "google/gemini-2.5-flash")
        self.assertEqual(route["order"], ["fireworks"])
        self.assertEqual(route["ignore"], ["deepinfra"])
        self.assertFalse(route["allow_fallbacks"])
        self.assertTrue(route["zdr_required"])
        self.assertEqual(route["max_price"], {"audio": 12.5})

    def test_an_unrouted_run_keeps_the_helpers_own_defaults(self) -> None:
        route = external_analysis._semantic_route(None)
        self.assertEqual(route["model"], mimo_openrouter.MODEL)
        self.assertTrue(route["allow_fallbacks"])
        self.assertFalse(route["zdr_required"])
        self.assertEqual(route["max_price"], {})

    def test_the_request_identity_carries_the_route(self) -> None:
        """Changing a constraint must not reuse another policy's answer."""
        route = external_analysis._semantic_route(
            external_analysis.execution_route(snapshot(), "TC-SEMANTIC"))
        identity = external_analysis._mimo_request_identity(
            Path("song.wav"), audio_sha="a" * 64, measured_duration=8.0,
            zdr=True, semantic=route)
        self.assertEqual(identity["model"], "google/gemini-2.5-flash")
        self.assertEqual(identity["provider_order"], ["fireworks"])
        self.assertEqual(identity["provider_ignore"], ["deepinfra"])
        self.assertFalse(identity["allow_fallbacks"])
        self.assertTrue(identity["zero_data_retention"])
        self.assertEqual(identity["max_price"], {"audio": 12.5})

        relaxed = dict(route, zdr_required=False)
        other = external_analysis._mimo_request_identity(
            Path("song.wav"), audio_sha="a" * 64, measured_duration=8.0,
            zdr=False, semantic=relaxed)
        self.assertNotEqual(identity, other)


class MimoRequest(unittest.TestCase):
    def test_the_model_is_a_flag_and_reaches_the_payload(self) -> None:
        payload, settings = mimo_openrouter.build_request(
            b"\x00\x01", audio_format="wav", audio_duration=8.0,
            model="google/gemini-2.5-flash",
            provider_order=["fireworks"], provider_only=["fireworks"],
            provider_ignore=["deepinfra"], allow_fallbacks=False,
            zero_data_retention=True, max_price={"audio": 12.5},
        )
        self.assertEqual(payload["model"], "google/gemini-2.5-flash")
        self.assertEqual(settings["model"], "google/gemini-2.5-flash")
        provider = payload["provider"]
        self.assertEqual(provider["order"], ["fireworks"])
        self.assertEqual(provider["only"], ["fireworks"])
        self.assertEqual(provider["ignore"], ["deepinfra"])
        self.assertFalse(provider["allow_fallbacks"])
        self.assertTrue(provider["zdr"])
        self.assertEqual(provider["max_price"], {"audio": 12.5})

    def test_an_unconstrained_request_is_what_it_always_was(self) -> None:
        payload, settings = mimo_openrouter.build_request(
            b"\x00", audio_format="wav", audio_duration=8.0)
        self.assertEqual(payload["model"], mimo_openrouter.MODEL)
        self.assertNotIn("only", payload["provider"])
        self.assertNotIn("ignore", payload["provider"])
        self.assertNotIn("zdr", payload["provider"])
        self.assertNotIn("max_price", payload["provider"])
        self.assertEqual(settings["provider_only"], [])
        self.assertEqual(settings["max_price"], {})

    def test_the_recorded_model_is_the_one_the_response_reported(self) -> None:
        """§6: `model_id` is observed, not inferred."""
        raw = {
            "global": _creative(), "segments": [dict(_creative(),
                                                     start_seconds=0.0,
                                                     end_seconds=8.0)],
        }
        served = mimo_openrouter.normalize_semantic_score(
            raw, audio_sha256="a" * 64, audio_duration=8.0,
            request_settings={"model": "xiaomi/mimo-v2.5"},
            response_metadata={"model": "xiaomi/mimo-v2.5:free",
                               "provider": "Fireworks"},
            requested_model="xiaomi/mimo-v2.5",
        )
        self.assertEqual(served["provenance"]["model"], "xiaomi/mimo-v2.5:free")
        self.assertEqual(served["provenance"]["requested_model"], "xiaomi/mimo-v2.5")
        self.assertEqual(served["provenance"]["provider"], "Fireworks")

        # A response that reports nothing falls back to what was requested,
        # never to the module constant — which is the whole reason the parameter
        # exists.
        silent = mimo_openrouter.normalize_semantic_score(
            raw, audio_sha256="a" * 64, audio_duration=8.0,
            request_settings={"model": "google/gemini-2.5-flash"},
            response_metadata={}, requested_model="google/gemini-2.5-flash",
        )
        self.assertEqual(silent["provenance"]["model"], "google/gemini-2.5-flash")


class CodexRouteWiring(unittest.TestCase):
    def test_the_reasoning_effort_joins_the_request_identity(self) -> None:
        plain = external_analysis._codex_request_settings(None)
        self.assertEqual(plain, {"sandbox": "read-only", "ephemeral": True})
        high = external_analysis._codex_request_settings("high")
        self.assertEqual(high["reasoning_effort"], "high")
        self.assertNotEqual(plain, high)

    def test_a_review_at_another_effort_is_not_reused(self) -> None:
        document = {
            "source": {"sha256": "b" * 64},
            "provenance": {
                "adapter": "tools/external_analysis.py",
                "adapter_version": external_analysis.ADAPTER_VERSION,
                "source_kind": "codex_lyric_review",
                "model": "gpt-5.6-sol",
                "prompt_version": external_analysis.LYRIC_PROMPT_VERSION,
                "prompt_sha256": external_analysis.sha256_file(
                    external_analysis.LYRIC_PROMPT),
                "request_settings": external_analysis._codex_request_settings("high"),
            },
        }
        accepts = lambda effort: external_analysis._review_cache_accepts(  # noqa: E731
            document, source_sha256="b" * 64, model="gpt-5.6-sol",
            reasoning_effort=effort)
        self.assertTrue(accepts("high"))
        self.assertFalse(accepts("low"))
        self.assertFalse(accepts(None))


class ObservedSnapshot(unittest.TestCase):
    def test_the_manifest_records_what_ran_not_what_was_asked_for(self) -> None:
        observed = external_analysis.observe_execution(
            snapshot(),
            review={"provenance": {"model": "gpt-5.6-sol-mini"}},
            semantic={
                "schema_version": "musializer.semantic-score/v1",
                "provenance": {"model": "google/gemini-2.5-flash-preview",
                               "provider": "Fireworks",
                               "prompt_version": "musializer-semantic-score/v1"},
            },
        )
        wording = observed["contracts"][1]
        semantic = observed["contracts"][2]
        self.assertEqual(wording["model_id"], "gpt-5.6-sol-mini")
        self.assertEqual(semantic["model_id"], "google/gemini-2.5-flash-preview")
        self.assertEqual(semantic["provider_served"], "Fireworks")
        self.assertEqual(semantic["prompt_version"], "musializer-semantic-score/v1")
        self.assertEqual(semantic["schema_version"], "musializer.semantic-score/v1")
        # A stage that produced nothing keeps the resolved identity: there is
        # nothing to have observed.
        self.assertEqual(observed["contracts"][0]["model_id"], "builtin-analyzer")
        # And the record it was built from is untouched — re-resolving is what
        # §5 invariant 3 forbids; recording what happened is a different copy.
        self.assertEqual(snapshot()["contracts"][2]["model_id"],
                         "google/gemini-2.5-flash")

    def test_an_acoustic_lane_reports_its_pipeline_from_timing_refinement(self) -> None:
        """The aligner's identity lives in its own block, not in `provenance`."""
        routed = snapshot()
        routed["contracts"][1]["contract"] = "TC-ALIGN"
        routed["contracts"][1]["route_type"] = "local-proc"
        routed["contracts"][1]["runtime_id"] = "mms-ctc"
        routed["contracts"][1]["model_id"] = "mms-ctc"
        observed = external_analysis.observe_execution(
            routed,
            aligned={"timing_refinement": {"model": "torchaudio.pipelines.MMS_FA"},
                     "provenance": {"execution": {"contract": "TC-ALIGN"}}},
        )
        self.assertEqual(observed["contracts"][1]["model_id"],
                         "torchaudio.pipelines.MMS_FA")

    def test_an_unrouted_run_embeds_nothing(self) -> None:
        self.assertIsNone(external_analysis.observe_execution(None))
        manifest = external_analysis.build_assist_manifest(
            mode="sections", audio_sha="a" * 64, measured_duration=8.0,
            cache_status={}, paths={"manifest": Path("m.json")},
            plan={"sections": []}, lyrics=None, semantic=None,
        )
        self.assertNotIn("execution_snapshot", manifest,
                         "absent and null are different answers")


def _creative() -> dict:
    """The eleven fields `mimo_openrouter._creative` requires."""
    return {
        "summary": "a summary", "moods": ["calm"], "energy": 0.5,
        "tension": 0.4, "valence": 0.1, "motion": ["drift"],
        "textures": ["warm"], "imagery": ["dusk"], "palette": ["#101020"],
        "scene_cues": ["spectrum"], "confidence": 0.7,
    }


if __name__ == "__main__":
    unittest.main()
