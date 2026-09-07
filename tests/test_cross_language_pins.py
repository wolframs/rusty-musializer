#!/usr/bin/env python3
"""Every hand-duplicated literal that crosses the Rust/Python boundary.

`docs/archive/ASSIST_AUDIT_2026-08-29.md` §C ends on the sentence this file
exists to answer: "Every cross-language constant — `MUSIALIZER_BRIDGE\\t1`,
seven `TC-*` tokens, four `RouteType` tokens, `"Codex default"`, six schema
strings — is a hand-duplicated literal. No shared fixtures, no codegen: each
side is pinned to its own copy." Two copies pinned separately are two copies
that can drift, and every one of these drifts is silent: a bumped catalog
schema makes the model picker say "never fetched", a renamed `TC-*` token makes
a contract unroutable, a moved bridge header makes every assist run fail to
import.

The method is `tools/support_bundle_check.sh`'s: **read the Rust source as
text**. It is the same argument that script makes about the helper manifest —
a second hand-maintained copy is the defect, so the check has to consult the
first one rather than a transcription of it. Nothing here compiles or runs
Rust, so it belongs in the Python suite and costs nothing.

Pure: no subprocess, no network, no audio device.
"""

from __future__ import annotations

import inspect
import re
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import codex_model_discovery  # noqa: E402
import external_analysis  # noqa: E402
import musializer_doctor  # noqa: E402
import provider_catalog  # noqa: E402


CORE = ROOT / "crates" / "musializer-core" / "src"
APP = ROOT / "crates" / "musializer-app" / "src"

CONTRACTS_RS = CORE / "assist" / "contracts.rs"
EXECUTION_RS = CORE / "assist" / "execution.rs"
SETTINGS_RS = CORE / "assist" / "settings.rs"
BRIDGE_RS = CORE / "project" / "analysis_bridge.rs"
CANDIDATE_RS = CORE / "project" / "analysis_candidate.rs"
ASSIST_SETTINGS_RS = APP / "ui" / "assist_settings.rs"
DIAGNOSIS_RS = CORE / "assist" / "diagnosis.rs"


def rust_source(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def rust_str_const(path: Path, name: str) -> str:
    """The value of `pub const NAME: &str = "...";`.

    A missing constant fails the test rather than returning a default: the
    constant having been renamed or moved is exactly the drift this file is
    looking for, and a `None` that compares unequal would report it as the
    wrong thing.
    """
    match = re.search(
        rf'\bconst\s+{re.escape(name)}\s*:\s*&\s*str\s*=\s*"((?:[^"\\]|\\.)*)"\s*;',
        rust_source(path))
    if match is None:
        raise AssertionError(f"{path.name} no longer declares a `const {name}: &str`")
    return match.group(1).encode("ascii").decode("unicode_escape")


class BridgeHeaderConstants(unittest.TestCase):
    """`MUSIALIZER_BRIDGE\\t1` — protocol-map row 1's first line."""

    def test_the_header_line_is_assembled_from_the_readers_two_constants(self) -> None:
        tag = rust_str_const(BRIDGE_RS, "HEADER_TAG")
        version = rust_str_const(BRIDGE_RS, "SCHEMA_VERSION")
        self.assertEqual(external_analysis.BRIDGE_VERSION, f"{tag}\t{version}")

    def test_the_separator_is_a_tab_on_both_sides(self) -> None:
        # The reader splits on `\t`; a header written with a space would be one
        # field and refused as a header, which is a confusing way to say
        # "someone reformatted a constant".
        self.assertEqual(external_analysis.BRIDGE_VERSION.count("\t"), 1)
        self.assertNotIn(" ", external_analysis.BRIDGE_VERSION)


class ContractTokens(unittest.TestCase):
    """Audit finding A9: the implemented-route table, maintained twice.

    `ContractId::route_is_implemented` (`contracts.rs`) and the `implemented`
    dict inside `read_execution_snapshot` (`external_analysis.py`) are the same
    table written in two languages. The widget-id-namespace trap at protocol
    scale: nothing fails when they disagree, the job simply refuses a route the
    application composed, or accepts one it never would.
    """

    def rust_tokens(self) -> set[str]:
        tokens = set(re.findall(r'#\[serde\(rename = "(TC-[A-Z]+)"\)\]',
                                rust_source(CONTRACTS_RS)))
        self.assertEqual(len(tokens), 7, "contracts.rs no longer declares 7 TC-* tokens")
        return tokens

    def python_table(self) -> str:
        return inspect.getsource(external_analysis.read_execution_snapshot)

    def test_the_seven_tokens_are_the_same_seven(self) -> None:
        python_tokens = set(re.findall(r'"(TC-[A-Z]+)"', self.python_table()))
        self.assertEqual(python_tokens, self.rust_tokens())

    def test_the_tokens_the_reader_declares_are_the_tokens_it_writes(self) -> None:
        # `token()` is what reaches a file; the serde renames are what parses
        # one back. Two tables in one Rust file, so pin them to each other too.
        written = set(re.findall(r'=> "(TC-[A-Z]+)"', rust_source(CONTRACTS_RS)))
        self.assertEqual(written, self.rust_tokens())

    def test_route_type_tokens_agree(self) -> None:
        rust_tokens = set(re.findall(r'Self::\w+ => "([a-z-]+)"',
                                     rust_source(CONTRACTS_RS)))
        # `RouteType::token` is the only `&'static str` match in the file whose
        # arms are all lowercase words; the contract tokens above are upper.
        rust_tokens = {token for token in rust_tokens if "-" in token or token.isalpha()}
        self.assertEqual(
            rust_tokens & {"builtin", "local-proc", "codex", "openrouter", "antigravity"},
            {"builtin", "local-proc", "codex", "openrouter", "antigravity"},
            "contracts.rs no longer spells the four route types this way")
        python_tokens = set(re.findall(r'route_type == "([a-z-]+)"', self.python_table()))
        self.assertEqual(python_tokens, {"builtin", "local-proc", "codex", "openrouter", "antigravity"})

    def test_openrouter_is_one_word_on_both_sides(self) -> None:
        # Recorded in `RouteType`'s own doc comment: it is the provider's name
        # and it reaches a file, so `open-router` would be a format change.
        self.assertNotIn("open-router", self.python_table())
        self.assertIn('rename = "openrouter"', rust_source(CONTRACTS_RS))

    def test_each_contracts_runtime_pair_is_the_pair_the_helper_implements(self) -> None:
        """The rows themselves, not just the vocabulary.

        Extracted from `runtime_is_implemented`'s match arms rather than
        transcribed, because a transcription is a third copy of the table this
        test exists because there are two of.
        """
        body = rust_source(CONTRACTS_RS).split("pub fn runtime_is_implemented", 1)[1]
        body = body.split("\n    /// The input modality", 1)[0]
        arms = re.findall(r"Self::(\w+)\s*=>\s*(.*?)(?=\n\s*Self::|\n        \})", body, re.DOTALL)
        pairs = [(variant, route, runtime) for variant, arm in arms
                 for route, runtime in re.findall(
                     r'route_type == RouteType::(\w+)\s*&& runtime_id == "([^"]+)"', arm)]
        self.assertEqual(len(pairs), 7, "six contracts have seven implemented routes")
        self.assertEqual(len({variant for variant, _, _ in pairs}), 6)
        route_tokens = {"Builtin": "builtin", "LocalProc": "local-proc",
                        "Codex": "codex", "OpenRouter": "openrouter", "Antigravity": "antigravity"}
        table = self.python_table()
        for variant, route_variant, runtime_id in pairs:
            with self.subTest(contract=variant):
                token = re.search(rf'Self::{variant}\s*=> "(TC-[A-Z]+)"',
                                  rust_source(CONTRACTS_RS))
                self.assertIsNotNone(token, variant)
                # The Python dict spells the same row as one `and` expression;
                # both halves must be present in it, keyed by the same token.
                row = re.search(
                    rf'"{token.group(1)}": \(?(.*?)(?:,\n|\n\s*\}})',
                    table, re.DOTALL)
                self.assertIsNotNone(row, token.group(1))
                self.assertIn(f'route_type == "{route_tokens[route_variant]}"',
                              row.group(1))
                self.assertIn(f'runtime_id == "{runtime_id}"', row.group(1))

    def test_the_unimplemented_contract_is_unimplemented_on_both_sides(self) -> None:
        self.assertIn("Self::Verify => false", rust_source(CONTRACTS_RS))
        self.assertIn('"TC-VERIFY": False', self.python_table())

    def test_the_two_model_gated_contracts_name_the_same_models(self) -> None:
        rust = rust_source(CONTRACTS_RS)
        for contract, model in (("Coarse", "whisper.cpp"), ("Align", "mms-ctc")):
            with self.subTest(contract=contract):
                self.assertIn(f'None | Some("{model}") => true', rust)
                self.assertIn(f'"{model}")', self.python_table())

    def test_the_empty_model_id_is_a_known_asymmetry(self) -> None:
        """A named pair rather than a silent exclusion.

        The helper accepts `model_id` of `""` where Rust's
        `Some("") => false` does not. The direction is safe — the application
        resolves the route, so a row Rust refuses to compose never reaches the
        helper — and the helper is the more permissive reader of a field the
        writer may leave blank. Pinned so that a *change* to it is deliberate.
        """
        self.assertIn('(None, "", "whisper.cpp")', self.python_table())
        self.assertIn('None | Some("whisper.cpp") => true', rust_source(CONTRACTS_RS))


class CodexDefaultLabel(unittest.TestCase):
    """§5 rule 6: a documented fallback label, never a model id."""

    def test_the_label_matches(self) -> None:
        self.assertEqual(
            external_analysis.CODEX_DEFAULT_LABEL,
            rust_str_const(EXECUTION_RS, "CODEX_DEFAULT_LABEL"))

    def test_the_label_is_not_a_model_id_shape(self) -> None:
        # Both sides compare it to reject it; a label that looked like an id
        # would be passed to `codex exec --model` by anything that missed the
        # comparison.
        self.assertIn(" ", external_analysis.CODEX_DEFAULT_LABEL)


class SchemaStrings(unittest.TestCase):
    """The six schema versions, each pinned across its own boundary."""

    def test_assist_settings(self) -> None:
        self.assertEqual(musializer_doctor.ASSIST_SETTINGS_SCHEMA,
                         rust_str_const(SETTINGS_RS, "SCHEMA"))

    def test_execution_snapshot(self) -> None:
        self.assertEqual(external_analysis.EXECUTION_SNAPSHOT_SCHEMA,
                         rust_str_const(EXECUTION_RS, "SNAPSHOT_SCHEMA"))

    def test_assist_manifest(self) -> None:
        # Taken from a built manifest rather than from a constant: the helper
        # writes the string inline, and what the reader has to agree with is
        # the value in the file.
        manifest = external_analysis.build_assist_manifest(
            mode="sections", audio_sha="c" * 64, measured_duration=2.0,
            cache_status={}, paths={"manifest": Path("m.json")},
            plan={"sections": []}, lyrics=None, semantic=None)
        self.assertEqual(manifest["schema_version"],
                         rust_str_const(CANDIDATE_RS, "MANIFEST_SCHEMA_VERSION"))

    def test_openrouter_catalog_reaches_both_rust_readers(self) -> None:
        # Audit finding B4: two readers, each pinned only to its own copy.
        schema = provider_catalog.SCHEMA_VERSION
        self.assertIn(f'schema != "{schema}"', rust_source(EXECUTION_RS))
        self.assertEqual(schema,
                         rust_str_const(ASSIST_SETTINGS_RS, "OPENROUTER_CATALOG_SCHEMA"))

    def test_codex_catalog(self) -> None:
        # Protocol-map row 6. The dialog's two catalog readers used to spell
        # their schemas as inline literals inside a closure, which is why the
        # Codex one had no Rust test of its own for a wrong version.
        self.assertEqual(codex_model_discovery.SCHEMA_VERSION,
                         rust_str_const(ASSIST_SETTINGS_RS, "CODEX_CATALOG_SCHEMA"))

    def test_doctor_report(self) -> None:
        # Audit finding B5, now closed: both Rust readers check this string
        # rather than accepting any JSON object, so it is pinned here the same
        # way the five above are instead of only appearing in fixtures.
        schema = musializer_doctor.SCHEMA_VERSION
        self.assertEqual(schema, rust_str_const(EXECUTION_RS, "DOCTOR_SCHEMA"))
        # The dialog reads the core's constant rather than declaring a second
        # one; what it must not do is go back to spelling it inline.
        self.assertIn("execution::DOCTOR_SCHEMA", rust_source(ASSIST_SETTINGS_RS))

    def test_every_schema_string_is_versioned(self) -> None:
        for name, value in (
            ("assist-settings", musializer_doctor.ASSIST_SETTINGS_SCHEMA),
            ("execution", external_analysis.EXECUTION_SNAPSHOT_SCHEMA),
            ("openrouter-catalog", provider_catalog.SCHEMA_VERSION),
            ("codex-catalog", codex_model_discovery.SCHEMA_VERSION),
            ("doctor", musializer_doctor.SCHEMA_VERSION),
        ):
            with self.subTest(schema=name):
                self.assertRegex(value, r"^musializer\.[a-z-]+/v\d+$")


class HelperFailureSentence(unittest.TestCase):
    """Audit finding B2: the one diagnosis the helper prints, read back in Rust.

    `main()` prints `External analysis failed: {error}` to stderr and returns 1;
    the desktop reads that line out of the job log and turns it into a toast
    that names the cause. Two seams, not one. The prefix is the first. The
    second is the three sentences `_run` raises behind it, which
    `core::assist::diagnosis::classify` tells apart by substring — reword either
    side and every failure silently collapses back to the single generic toast
    B2 is about, with nothing failing anywhere.

    The sentences are obtained by **driving `_run`** with an injected runner
    rather than by matching its source, because each one is composed from two
    format strings in different places (`f"{name} {detail}"`), and a check that
    reads either half alone would pass while the composition drifted. The
    classifier's own fragments are read out of `diagnosis.rs` as text, the
    method this file already uses.
    """

    def python_prefix(self) -> str:
        source = inspect.getsource(external_analysis.main)
        match = re.search(r'print\(f"([^"{]*)\{error\}", file=sys\.stderr\)', source)
        if match is None:
            raise AssertionError("external_analysis.main no longer prints a failure line")
        return match.group(1)

    def helper_sentences(self) -> dict[str, str]:
        """The three `RuntimeError` messages `_run` can raise, actually raised."""

        def failing(exception: BaseException):
            def runner(*args, **kwargs):
                raise exception
            return runner

        completed = subprocess.CompletedProcess(["whisper-cli"], 2, "", "")
        cases = {
            "missing": failing(OSError(2, "No such file or directory")),
            "timeout": failing(subprocess.TimeoutExpired(["whisper-cli"], 30.0)),
            "exit": lambda *args, **kwargs: completed,
        }
        sentences = {}
        for name, runner in cases.items():
            with self.assertRaises(RuntimeError) as raised:
                external_analysis._run(["/opt/whisper-cli"], timeout=30.0, runner=runner)
            sentences[name] = str(raised.exception)
        return sentences

    def rust_classifier(self) -> str:
        source = rust_source(DIAGNOSIS_RS)
        match = re.search(r"fn classify\(reported: &str\) -> FailureCause \{(.*?)\n\}",
                          source, re.S)
        if match is None:
            raise AssertionError("diagnosis.rs no longer declares `fn classify`")
        return match.group(1)

    def test_the_prefix_is_the_one_rust_looks_for(self) -> None:
        # The Rust constant carries the trailing space, and the Python literal
        # ends immediately before the interpolation, so the two are equal
        # without either side trimming.
        self.assertEqual(self.python_prefix(),
                         rust_str_const(DIAGNOSIS_RS, "FAILURE_PREFIX"))

    def test_each_helper_sentence_matches_exactly_one_rust_arm(self) -> None:
        classifier = self.rust_classifier()
        arms = {
            "missing": [("starts", frag) for frag in
                        re.findall(r'starts_with\("([^"]+)"\)', classifier)],
            "timeout": [("contains", frag) for frag in
                        re.findall(r'contains\("([^"]+)"\)', classifier)][:2],
            "exit": [("contains", frag) for frag in
                     re.findall(r'contains\("([^"]+)"\)', classifier)][2:],
        }
        self.assertEqual([len(tests) for tests in arms.values()], [1, 2, 1],
                         "diagnosis.rs::classify no longer has the three arms this pins")
        sentences = self.helper_sentences()
        for name, sentence in sentences.items():
            with self.subTest(cause=name):
                for kind, fragment in arms[name]:
                    if kind == "starts":
                        self.assertTrue(sentence.startswith(fragment), sentence)
                    else:
                        self.assertIn(fragment, sentence)
        # And they are three different sentences, so three different toasts.
        self.assertEqual(len(set(sentences.values())), 3)

    def test_a_helper_sentence_survives_the_prefix_it_is_printed_behind(self) -> None:
        # What Rust actually reads is the whole log line. The prefix must not
        # swallow the leading fragment the `missing` arm keys on, which it would
        # if either side gained or lost the separating space.
        prefix = self.python_prefix()
        for sentence in self.helper_sentences().values():
            line = f"{prefix}{sentence}"
            self.assertEqual(line.split(prefix, 1)[1], sentence)


if __name__ == "__main__":
    unittest.main()
