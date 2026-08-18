#!/usr/bin/env python3
"""Regression tests for the AP2 discovery tranche's Python-side subset.

Covers: `tools/musializer_doctor.py`'s AP2-a runtime inventory extension and
its AP2-b models-directory resolution, `tools/codex_model_discovery.py`
(AP2-c), and `tools/provider_catalog.py` (AP2-d), plus the shared
`tools/atomic_cache.py` write path they both use.
Matches `tests/test_lyrics_timing.py`'s pattern: plain `unittest`, `tools/`
pushed onto `sys.path`, one `TestCase` per module/concern, picked up by
`tools/support_bundle_check.sh`'s `python3 -m unittest discover -s tests`.
"""

from __future__ import annotations

import builtins
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import atomic_cache  # noqa: E402
import codex_model_discovery  # noqa: E402
import musializer_doctor  # noqa: E402
import provider_catalog  # noqa: E402
import runtime_inventory  # noqa: E402


def _completed(stdout: str = "", returncode: int = 0) -> "subprocess.CompletedProcess[str]":
    return subprocess.CompletedProcess(args=[], returncode=returncode, stdout=stdout, stderr="")


class AtomicCacheTests(unittest.TestCase):
    def test_cache_dir_prefers_xdg_cache_home(self) -> None:
        self.assertEqual(
            atomic_cache.cache_dir({"XDG_CACHE_HOME": "/tmp/xdg-example"}),
            Path("/tmp/xdg-example/musializer"),
        )

    def test_cache_dir_falls_back_to_home_cache(self) -> None:
        resolved = atomic_cache.cache_dir({})
        self.assertEqual(resolved, Path.home() / ".cache/musializer")

    def test_write_then_read_round_trips(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "sub" / "doc.json"
            atomic_cache.write_json_atomic(path, {"a": 1})
            self.assertEqual(atomic_cache.read_json(path), {"a": 1})

    def test_no_temp_file_survives_a_successful_write(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "doc.json"
            atomic_cache.write_json_atomic(path, {"a": 1})
            self.assertEqual(os.listdir(tmp), ["doc.json"])

    def test_truncated_file_reads_as_no_cache_rather_than_raising(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "doc.json"
            path.write_text('{"a": 1, "b": [1, 2,')  # deliberately cut mid-array
            self.assertIsNone(atomic_cache.read_json(path))

    def test_non_object_top_level_is_not_a_cache(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "doc.json"
            path.write_text("[1, 2, 3]")
            self.assertIsNone(atomic_cache.read_json(path))


class RuntimeInventoryTests(unittest.TestCase):
    def test_missing_whisper_binary_is_unavailable_not_an_exception(self) -> None:
        result = runtime_inventory.whisper_identity(
            binary=None, model=None, which=lambda name: None,
            runner=lambda *a, **k: _completed(), sha256_file=lambda p: "unused",
        )
        self.assertEqual(result["state"], "unavailable")
        self.assertIn("MUSIALIZER_WHISPER_BIN", result["remediation"])
        self.assertIsNone(result["path"])

    def test_whisper_binary_present_model_missing_still_unavailable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "whisper-cli"
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            result = runtime_inventory.whisper_identity(
                binary=binary, model=None, which=lambda name: None,
                runner=lambda *a, **k: _completed(), sha256_file=lambda p: "unused",
            )
            self.assertEqual(result["state"], "unavailable")
            self.assertEqual(result["path"], str(binary))
            self.assertIn("MUSIALIZER_WHISPER_MODEL", result["remediation"])

    def test_whisper_ok_reports_language_support_from_model_filename(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            install = Path(tmp) / "whisper.cpp"
            binary = install / "build/bin/whisper-cli"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            model = install / "ggml-medium.en.bin"
            model.write_bytes(b"fake-model-bytes")

            result = runtime_inventory.whisper_identity(
                binary=binary, model=model, which=lambda name: None,
                runner=lambda *a, **k: _completed(), sha256_file=lambda p: "deadbeef",
            )
            self.assertEqual(result["state"], "ok")
            self.assertEqual(result["language_support"], "en-only")
            self.assertEqual(result["model_sha256"], "deadbeef")
            self.assertIsNone(result["version"])  # no .git directory in the fixture

    def test_whisper_version_comes_from_install_directory_git_describe(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            install = Path(tmp) / "whisper.cpp"
            binary = install / "build/bin/whisper-cli"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            model = install / "ggml-large-v3.bin"
            model.write_bytes(b"fake")
            (install / ".git").mkdir()

            calls = []

            def runner(argv, **kwargs):
                calls.append(argv)
                if argv[:2] == ["git", "-C"]:
                    return _completed(stdout="v1.8.6\n")
                return _completed()

            result = runtime_inventory.whisper_identity(
                binary=binary, model=model, which=lambda name: None,
                runner=runner, sha256_file=lambda p: "irrelevant",
            )
            self.assertEqual(result["version"], "v1.8.6")
            self.assertEqual(result["language_support"],
                              "multilingual (whisper.cpp auto-detect / --language)")

    def test_gpu_ready_reflects_ldd_output(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "whisper-cli"
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            model = Path(tmp) / "ggml-large-v3.bin"
            model.write_bytes(b"fake")

            def runner(argv, **kwargs):
                if Path(argv[0]).name == "ldd":
                    return _completed(stdout="libcudart.so.12 => /usr/lib/libcudart.so.12\n")
                return _completed()

            result = runtime_inventory.whisper_identity(
                binary=binary, model=model, which=lambda name: "/usr/bin/ldd" if name == "ldd" else None,
                runner=runner, sha256_file=lambda p: "x",
            )
            self.assertTrue(result["gpu_ready"])

    def test_gpu_ready_is_none_when_ldd_is_unavailable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "whisper-cli"
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            model = Path(tmp) / "ggml-large-v3.bin"
            model.write_bytes(b"fake")

            result = runtime_inventory.whisper_identity(
                binary=binary, model=model, which=lambda name: None,
                runner=lambda *a, **k: _completed(), sha256_file=lambda p: "x",
            )
            self.assertIsNone(result["gpu_ready"])

    def test_large_model_skips_hashing_rather_than_stalling(self) -> None:
        original_stat = Path.stat

        def oversized_stat(self):
            result = original_stat(self)
            return SimpleNamespace(st_size=runtime_inventory.HASH_MAX_BYTES + 1)

        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "whisper-cli"
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            model = Path(tmp) / "ggml-large-v3.bin"
            model.write_bytes(b"fake")

            called = []

            def sha256_file(path):
                called.append(path)
                return "should-not-be-called"

            Path.stat = oversized_stat
            try:
                result = runtime_inventory.whisper_identity(
                    binary=binary, model=model, which=lambda name: None,
                    runner=lambda *a, **k: _completed(), sha256_file=sha256_file,
                )
            finally:
                Path.stat = original_stat
            self.assertIsNone(result["model_sha256"])
            self.assertEqual(called, [])

    def test_stem_separator_is_honest_about_being_absent_and_unused(self) -> None:
        result = runtime_inventory.stem_separator_identity(
            which=lambda name: None, runner=lambda *a, **k: _completed(), environ={})
        self.assertEqual(result["state"], "not installed (optional; unused)")
        self.assertIn("always analyzes the full mix", result["remediation"])

    def test_stem_separator_is_detected_but_not_claimed_as_wired(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "demucs"
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            result = runtime_inventory.stem_separator_identity(
                which=lambda name: None,
                runner=lambda *a, **k: _completed(stdout="usage: demucs"),
                environ={"MUSIALIZER_STEM_SEPARATOR_BIN": str(binary)},
            )
            self.assertEqual(result["state"], "detected (unused)")
            self.assertEqual(result["path"], str(binary))
            self.assertIn("does not invoke", result["remediation"])

    def test_stale_executable_shim_is_broken_not_installed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "demucs"
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            result = runtime_inventory.stem_separator_identity(
                which=lambda name: str(binary),
                runner=lambda *a, **k: subprocess.CompletedProcess(
                    args=[], returncode=1, stdout="",
                    stderr="ModuleNotFoundError: No module named demucs"),
                environ={},
            )
            self.assertEqual(result["state"], "broken (unused)")
            self.assertIn("ModuleNotFoundError", result["remediation"])

    def test_collect_never_raises_when_everything_is_absent(self) -> None:
        runtimes = runtime_inventory.collect(
            whisper_binary=None, whisper_model=None, align_python=None, align_model=None,
            which=lambda name: None, runner=lambda *a, **k: _completed(), environ={},
            sha256_file=lambda p: "unused",
        )
        self.assertEqual(set(runtimes), {"whisper", "mms_ctc_aligner", "stem_separator"})
        self.assertEqual(runtimes["whisper"]["state"], "unavailable")
        self.assertEqual(runtimes["mms_ctc_aligner"]["state"], "unavailable")
        self.assertEqual(
            runtimes["stem_separator"]["state"],
            "not installed (optional; unused)")
        for runtime in runtimes.values():
            self.assertIsInstance(runtime["remediation"], str)


class DoctorExecutableDiscoveryTests(unittest.TestCase):
    def test_explicit_codex_path_survives_a_desktop_path_that_cannot_find_it(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            codex = Path(tmp) / "codex"
            codex.write_text("#!/bin/sh\n")
            codex.chmod(0o755)
            found, detail = musializer_doctor._codex_executable(
                codex, lambda _name: None)
            self.assertEqual(found, str(codex))
            self.assertEqual(detail, str(codex))

    def test_invalid_explicit_codex_path_is_loud_and_never_falls_back(self) -> None:
        missing = Path("/definitely/missing/codex")
        found, detail = musializer_doctor._codex_executable(
            missing, lambda _name: "/usr/bin/codex")

        self.assertIsNone(found)
        self.assertIn(str(missing), detail)


# --- Models directory resolution (AP2-b) ------------------------------------

def _assist_settings(models_dir: str, **extra) -> str:
    document = {"schema": musializer_doctor.ASSIST_SETTINGS_SCHEMA,
                "active_profile": "recommended",
                "local_runtimes": {"models_dir": models_dir}}
    document.update(extra)
    return json.dumps(document)


class ModelsDirectoryTests(unittest.TestCase):
    """The operator rule from AGENTS.md, as the doctor reports it.

    The same ladder is implemented in `musializer_core::assist::models_dir`
    and probed by `musializer_runtime::assist::models`; these tests pin the
    doctor's copy, which is what a user actually reads.
    """

    def _locked(self, path: Path) -> bool:
        """chmod 0o500 and report whether it really blocked this process."""
        path.chmod(0o500)

        def restore() -> None:
            # The temporary directory may already be gone: Python's own
            # cleanup chmods and retries on a locked directory.
            try:
                path.chmod(0o700)
            except OSError:
                pass

        self.addCleanup(restore)
        probe = path / "musializer-writability-check"
        try:
            probe.write_text("x")
        except OSError:
            return True
        probe.unlink()
        return False

    def test_writable_install_directory_is_the_default(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            root.mkdir()
            section = musializer_doctor._models_directory(
                root=root, application=None, environ={"HOME": str(Path(tmp) / "home")})
            self.assertEqual(section["source"], "install-default")
            self.assertEqual(section["resolved"], str(root / "models"))
            self.assertTrue(section["writable"])
            self.assertEqual(section["state"], "ok")
            self.assertIsNone(section["override"])

    def test_install_directory_is_the_applications_own_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            application = root / "dist/musializer"
            application.parent.mkdir(parents=True)
            application.write_text("#!/bin/sh\n")
            section = musializer_doctor._models_directory(
                root=root, application=application, environ={})
            self.assertEqual(section["install_default"], str(root / "dist/models"))
            self.assertEqual(section["resolved"], str(root / "dist/models"))

    def test_unwritable_install_directory_falls_back_to_the_home_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            home = Path(tmp) / "home"
            root.mkdir()
            home.mkdir()
            if not self._locked(root):
                self.skipTest("this process can write to a 0500 directory (running as root?)")
            section = musializer_doctor._models_directory(
                root=root, application=None, environ={"HOME": str(home)})
            self.assertEqual(section["source"], "home-fallback")
            self.assertEqual(section["resolved"], str(home / "musializer/models"))
            self.assertFalse(section["install_default_writable"])
            # The default that lost is still named, per "never a location the
            # user was not shown".
            self.assertEqual(section["install_default"], str(root / "models"))

    def test_an_explicit_override_wins_over_a_writable_default(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            chosen = Path(tmp) / "elsewhere/models"
            settings = Path(tmp) / "assist.json"
            root.mkdir()
            settings.write_text(_assist_settings(str(chosen)))
            section = musializer_doctor._models_directory(
                root=root, application=None,
                environ={"HOME": str(Path(tmp) / "home"),
                         "MUSIALIZER_ASSIST_SETTINGS": str(settings)})
            self.assertEqual(section["source"], "settings-override")
            self.assertEqual(section["resolved"], str(chosen))
            self.assertEqual(section["override"], str(chosen))
            self.assertTrue(section["install_default_writable"])
            self.assertEqual(section["install_default"], str(root / "models"))
            self.assertIsNone(section["settings_error"])

    def test_an_unwritable_override_still_wins_and_says_so(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            chosen = Path(tmp) / "locked/models"
            settings = Path(tmp) / "assist.json"
            root.mkdir()
            chosen.parent.mkdir()
            if not self._locked(chosen.parent):
                self.skipTest("this process can write to a 0500 directory (running as root?)")
            settings.write_text(_assist_settings(str(chosen)))
            section = musializer_doctor._models_directory(
                root=root, application=None,
                environ={"MUSIALIZER_ASSIST_SETTINGS": str(settings)})
            self.assertEqual(section["source"], "settings-override")
            self.assertFalse(section["writable"])
            self.assertEqual(section["state"], "unavailable")

    def test_settings_path_follows_the_xdg_ladder(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            xdg = Path(tmp) / "config"
            (xdg / "musializer").mkdir(parents=True)
            (xdg / "musializer/assist.json").write_text(_assist_settings("/srv/weights"))
            section = musializer_doctor._models_directory(
                root=Path(tmp), application=None,
                environ={"XDG_CONFIG_HOME": str(xdg), "HOME": str(Path(tmp) / "home")})
            self.assertEqual(section["settings_path"], str(xdg / "musializer/assist.json"))
            self.assertEqual(section["resolved"], "/srv/weights")

            home_only = musializer_doctor._assist_settings_path({"HOME": "/home/example"})
            self.assertEqual(home_only, Path("/home/example/.config/musializer/assist.json"))
            self.assertIsNone(musializer_doctor._assist_settings_path({}))

    def test_a_corrupt_settings_file_is_reported_and_the_default_still_resolves(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            root.mkdir()
            settings = Path(tmp) / "assist.json"
            settings.write_text("{ broken")
            section = musializer_doctor._models_directory(
                root=root, application=None,
                environ={"MUSIALIZER_ASSIST_SETTINGS": str(settings)})
            self.assertIsNotNone(section["settings_error"])
            self.assertEqual(section["source"], "install-default")
            # Reported, never repaired.
            self.assertEqual(settings.read_text(), "{ broken")

    def test_a_foreign_schema_is_not_applied(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            root.mkdir()
            settings = Path(tmp) / "assist.json"
            settings.write_text(json.dumps({"schema": "musializer.assist-settings/v2",
                                            "local_runtimes": {"models_dir": "/srv/weights"}}))
            section = musializer_doctor._models_directory(
                root=root, application=None,
                environ={"MUSIALIZER_ASSIST_SETTINGS": str(settings)})
            self.assertIn("musializer.assist-settings/v2", section["settings_error"])
            self.assertIsNone(section["override"])
            self.assertEqual(section["source"], "install-default")

    def test_an_oversized_settings_file_is_refused_before_it_is_parsed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            root.mkdir()
            settings = Path(tmp) / "assist.json"
            padding = " " * (musializer_doctor.ASSIST_SETTINGS_MAX_BYTES + 1)
            settings.write_text(_assist_settings("/srv/weights") + padding)
            section = musializer_doctor._models_directory(
                root=root, application=None,
                environ={"MUSIALIZER_ASSIST_SETTINGS": str(settings)})
            self.assertIn("cap", section["settings_error"])
            self.assertIsNone(section["override"])

    def test_only_the_models_dir_field_reaches_the_report(self) -> None:
        # E11: assist.json carries a credential mode and fingerprint. The
        # doctor reads one field out of this file and must not echo the rest.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            root.mkdir()
            settings = Path(tmp) / "assist.json"
            settings.write_text(_assist_settings(
                str(Path(tmp) / "weights"),
                credentials={"openrouter": {"mode": "file", "lookup_id": "MUSICANARY",
                                            "fingerprint": "0a1b2c3d"}}))
            section = musializer_doctor._models_directory(
                root=root, application=None,
                environ={"MUSIALIZER_ASSIST_SETTINGS": str(settings)})
            rendered = json.dumps(section)
            self.assertNotIn("MUSICANARY", rendered)
            self.assertNotIn("0a1b2c3d", rendered)
            self.assertNotIn("fingerprint", rendered)

    def test_no_home_and_an_unwritable_default_resolves_to_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            root.mkdir()
            if not self._locked(root):
                self.skipTest("this process can write to a 0500 directory (running as root?)")
            section = musializer_doctor._models_directory(
                root=root, application=None, environ={})
            self.assertIsNone(section["resolved"])
            self.assertIsNone(section["source"])
            self.assertEqual(section["state"], "unavailable")
            self.assertIsNone(section["home_fallback"])

    def test_the_human_report_shows_the_default_and_that_an_override_would_win(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "install"
            root.mkdir()
            section = musializer_doctor._models_directory(
                root=root, application=None,
                environ={"HOME": str(Path(tmp) / "home")})
            runtimes = runtime_inventory.collect(
                whisper_binary=None, whisper_model=None, align_python=None,
                align_model=None, which=lambda name: None,
                runner=lambda *a, **k: _completed(), environ={},
                sha256_file=lambda path: "unused")
            report = {
                "root": str(root), "checks": [], "runtimes": runtimes,
                "capabilities": {name: {"ready": True, "missing": []}
                                 for name in musializer_doctor.CAPABILITIES},
                "models_directory": section,
                "gpu": {"kind": "none", "available": False, "devices": []},
            }
            rendered = musializer_doctor.render_human(report)
            self.assertIn("Models directory:", rendered)
            self.assertIn(f"Resolved: {root / 'models'} (install-default)", rendered)
            self.assertIn(f"Install default: {root / 'models'} (writable)", rendered)
            self.assertIn(f"Home fallback: {Path(tmp) / 'home/musializer/models'}", rendered)
            self.assertIn("Override: none", rendered)
            self.assertIn("it wins over the default above", rendered)


# --- Codex model discovery (AP2-c) -----------------------------------------

_STUB_PREAMBLE = """
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\\n")
    sys.stdout.flush()

for raw_line in iter(sys.stdin.readline, ""):
    line = raw_line.strip()
    if not line:
        continue
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        send({"id": message["id"], "result": {"userAgent": "stub/0.1"}})
"""

_OLD_CODEX_TAIL = """
    elif method == "model/list":
        send({"id": message["id"],
              "error": {"code": -32601, "message": "Method not found: model/list"}})
"""

_NEW_CODEX_TAIL = """
    elif method == "model/list":
        send({"id": message["id"], "result": {"data": [
            {"id": "gpt-5.6-sol", "model": "gpt-5.6-sol", "displayName": "GPT-5.6-Sol",
             "description": "flagship", "hidden": False, "isDefault": True,
             "defaultReasoningEffort": "low",
             "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": "fast"}],
             "inputModalities": ["text", "image"]},
            {"id": "gpt-5.6-luna", "model": "gpt-5.6-luna", "displayName": "GPT-5.6-Luna",
             "description": "fast+cheap", "hidden": False, "isDefault": False,
             "defaultReasoningEffort": "medium",
             "supportedReasoningEfforts": [{"reasoningEffort": "medium", "description": "balanced"}],
             "inputModalities": ["text"]},
        ]}})
"""

_UNRESPONSIVE_TAIL = """
    # deliberately does not answer model/list, to exercise the timeout path
"""


class CodexModelDiscoveryTests(unittest.TestCase):
    def _write_stub(self, tail: str) -> Path:
        handle = tempfile.NamedTemporaryFile(
            mode="w", suffix=".py", delete=False, dir=tempfile.gettempdir())
        handle.write(_STUB_PREAMBLE + tail)
        handle.close()
        self.addCleanup(os.unlink, handle.name)
        return Path(handle.name)

    def test_missing_codex_binary_falls_back_to_default(self) -> None:
        result = codex_model_discovery.discover_models(
            codex_bin="musializer-test-definitely-not-a-real-binary", timeout=2.0)
        self.assertFalse(result.supported)
        self.assertEqual(result.models, [])
        self.assertEqual(result.default_label, "Codex default")
        self.assertIsNotNone(result.error)

    def test_old_codex_without_model_list_yields_exactly_codex_default(self) -> None:
        stub = self._write_stub(_OLD_CODEX_TAIL)
        result = codex_model_discovery.discover_models(
            codex_bin=[sys.executable, "-u", str(stub)], timeout=5.0)
        self.assertFalse(result.supported)
        self.assertEqual(result.models, [])
        self.assertEqual(result.default_label, "Codex default")
        self.assertIn("model/list", result.error or "")

    def test_unresponsive_codex_times_out_to_default(self) -> None:
        stub = self._write_stub(_UNRESPONSIVE_TAIL)
        result = codex_model_discovery.discover_models(
            codex_bin=[sys.executable, "-u", str(stub)], timeout=1.0)
        self.assertFalse(result.supported)
        self.assertEqual(result.models, [])
        self.assertEqual(result.default_label, "Codex default")

    def test_new_codex_reports_a_normalized_catalog(self) -> None:
        stub = self._write_stub(_NEW_CODEX_TAIL)
        result = codex_model_discovery.discover_models(
            codex_bin=[sys.executable, "-u", str(stub)], timeout=5.0)
        self.assertTrue(result.supported)
        self.assertIsNone(result.error)
        ids = [model["id"] for model in result.models]
        self.assertEqual(ids, ["gpt-5.6-sol", "gpt-5.6-luna"])
        self.assertTrue(result.models[0]["is_default"])
        self.assertFalse(result.models[1]["is_default"])
        self.assertEqual(result.models[0]["supported_reasoning_efforts"],
                          [{"reasoning_effort": "low", "description": "fast"}])

    def test_desktop_launch_finds_node_for_an_npm_style_codex_wrapper(self) -> None:
        """The absolute Codex shim is not a complete launch recipe.

        Plasma omits both user-local directories here: Codex lives in the npm
        one and its ``/usr/bin/env node`` interpreter lives in ``.local/bin``.
        Discovery must add the latter for the child instead of timing out after
        the wrapper exits 127.
        """
        stub = self._write_stub(_NEW_CODEX_TAIL)
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp) / "home"
            node_dir = home / ".local/bin"
            codex_dir = home / ".local/npm-global/bin"
            node_dir.mkdir(parents=True)
            codex_dir.mkdir(parents=True)
            node = node_dir / "node"
            node.write_text(f'#!/bin/sh\nexec {sys.executable} "$@"\n')
            node.chmod(0o755)
            codex = codex_dir / "codex"
            codex.write_text(f'#!/usr/bin/env node\n{stub.read_text()}')
            codex.chmod(0o755)

            result = codex_model_discovery.discover_models(
                codex_bin=str(codex), timeout=2.0,
                environ={"HOME": str(home), "PATH": "/usr/bin:/bin"},
            )
            self.assertTrue(result.supported, result.error)
            self.assertEqual(len(result.models), 2)

    def test_early_codex_exit_reports_stderr_without_waiting_for_timeout(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            broken = Path(tmp) / "codex"
            broken.write_text("#!/bin/sh\necho missing runtime >&2\nexit 127\n")
            broken.chmod(0o755)
            started = __import__("time").monotonic()
            result = codex_model_discovery.discover_models(
                codex_bin=str(broken), timeout=2.0,
                environ={"HOME": tmp, "PATH": "/usr/bin:/bin"},
            )
            elapsed = __import__("time").monotonic() - started
            self.assertFalse(result.supported)
            self.assertIn("missing runtime", result.error or "")
            self.assertLess(elapsed, 1.0)

    def test_spawned_process_is_always_terminated(self) -> None:
        stub = self._write_stub(_NEW_CODEX_TAIL)
        spawned: list[subprocess.Popen] = []
        real_popen = subprocess.Popen

        def tracking_popen(*args, **kwargs):
            proc = real_popen(*args, **kwargs)
            spawned.append(proc)
            return proc

        codex_model_discovery.discover_models(
            codex_bin=[sys.executable, "-u", str(stub)], timeout=5.0, popen=tracking_popen)
        self.assertEqual(len(spawned), 1)
        self.assertIsNotNone(spawned[0].poll(), "discovery must terminate the process it spawned")

    def test_discovery_never_opens_the_codex_auth_file_itself(self) -> None:
        stub = self._write_stub(_OLD_CODEX_TAIL)
        with tempfile.TemporaryDirectory() as tmp:
            codex_home = Path(tmp) / "codex-home"
            codex_home.mkdir()
            sentinel_auth = codex_home / "auth.json"
            sentinel_auth.write_text('{"OPENAI_API_KEY": "sk-sentinel-should-not-be-touched"}')

            opened_paths: list[str] = []
            real_open = builtins.open

            def tracking_open(file, *args, **kwargs):
                try:
                    opened_paths.append(os.fspath(file))
                except TypeError:
                    pass
                return real_open(file, *args, **kwargs)

            builtins.open = tracking_open
            try:
                result = codex_model_discovery.discover_models(
                    codex_bin=[sys.executable, "-u", str(stub)], timeout=5.0,
                    environ={"CODEX_HOME": str(codex_home), "PATH": os.environ.get("PATH", "")},
                )
            finally:
                builtins.open = real_open

            self.assertFalse(result.supported)  # sanity: the stub really is the old-Codex case
            self.assertNotIn(str(sentinel_auth), opened_paths)

    def test_refresh_and_cache_writes_only_on_full_success(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "codex-models-v1.json"

            old_stub = self._write_stub(_OLD_CODEX_TAIL)
            failed = codex_model_discovery.refresh_and_cache(
                codex_bin=[sys.executable, "-u", str(old_stub)], timeout=5.0, cache_file=cache_file)
            self.assertFalse(failed.supported)
            self.assertFalse(cache_file.exists())

            new_stub = self._write_stub(_NEW_CODEX_TAIL)
            ok = codex_model_discovery.refresh_and_cache(
                codex_bin=[sys.executable, "-u", str(new_stub)], timeout=5.0, cache_file=cache_file)
            self.assertTrue(ok.supported)
            self.assertTrue(cache_file.exists())
            cached = codex_model_discovery.read_cache(cache_file)
            assert cached is not None
            self.assertEqual(cached["model_count"], 2)
            self.assertEqual(cached["schema_version"], codex_model_discovery.SCHEMA_VERSION)

            # A later failed refresh must not disturb the cache written above.
            mtime_before = cache_file.stat().st_mtime_ns
            bytes_before = cache_file.read_bytes()
            failed_again = codex_model_discovery.refresh_and_cache(
                codex_bin=[sys.executable, "-u", str(old_stub)], timeout=5.0, cache_file=cache_file)
            self.assertFalse(failed_again.supported)
            self.assertEqual(cache_file.stat().st_mtime_ns, mtime_before)
            self.assertEqual(cache_file.read_bytes(), bytes_before)

    def test_the_command_line_honours_an_explicit_codex_binary(self) -> None:
        """Defect C. The AI settings dialog's Refresh button runs this module as
        a program, and until now the module had no ``__main__`` at all: the
        button spawned ``python3 codex_model_discovery.py refresh``, which
        imported the module, did nothing, exited 0, and reported success.

        ``--codex-bin`` is the other half. The dialog resolves ``codex`` through
        the four-rung ladder in ``musializer_runtime::assist::discover`` and
        hands the answer down, because this tool's own ``PATH`` is the parent's
        ``PATH`` -- the exact environment that failed to find it.
        """
        stub = self._write_stub(_NEW_CODEX_TAIL)
        with tempfile.TemporaryDirectory() as tmp:
            cache_dir = Path(tmp)
            # A shell wrapper, so the argument really is a single path to an
            # executable rather than a list this test constructed.
            wrapper = cache_dir / "codex-stub"
            wrapper.write_text(f'#!/bin/sh\nexec {sys.executable} -u {stub} "$@"\n')
            wrapper.chmod(0o755)

            status = codex_model_discovery.main(
                ["refresh", "--codex-bin", str(wrapper), "--cache-dir", str(cache_dir),
                 "--timeout", "5"])
            self.assertEqual(status, 0)
            cached = codex_model_discovery.read_cache(
                cache_dir / codex_model_discovery.CACHE_FILENAME)
            assert cached is not None
            self.assertEqual(cached["model_count"], 2)
            self.assertEqual(cached["codex_bin"], str(wrapper))

    def test_a_failed_refresh_exits_non_zero_rather_than_claiming_success(self) -> None:
        """The dialog reads the exit status to decide whether to say "refreshed"."""
        with tempfile.TemporaryDirectory() as tmp:
            status = codex_model_discovery.main(
                ["refresh", "--codex-bin", "musializer-test-definitely-not-a-real-binary",
                 "--cache-dir", tmp, "--timeout", "2"])
            self.assertEqual(status, 1)
            self.assertFalse((Path(tmp) / codex_model_discovery.CACHE_FILENAME).exists())


# --- OpenRouter provider catalog (AP2-d) ------------------------------------

def _model_entry(model_id: str, *, input_modalities=("text",), output_modalities=("text",),
                  description: str = "a model") -> dict:
    return {
        "id": model_id, "canonical_slug": f"{model_id}-20260101", "name": model_id,
        "description": description, "context_length": 128000,
        "architecture": {"input_modalities": list(input_modalities),
                          "output_modalities": list(output_modalities), "tokenizer": "GPT"},
        "pricing": {"prompt": "0.000002", "completion": "0.000006"},
        "top_provider": {"context_length": 128000, "max_completion_tokens": 8192,
                          "is_moderated": False},
    }


def _catalog_bytes(models: list[dict]) -> bytes:
    return json.dumps({"data": models}).encode("utf-8")


class ProviderCatalogTests(unittest.TestCase):
    def test_normalize_model_keeps_only_the_allowlisted_fields(self) -> None:
        normalized = provider_catalog.normalize_model(_model_entry(
            "acme/model", input_modalities=("text", "audio"), output_modalities=("text",)))
        self.assertEqual(normalized["id"], "acme/model")
        self.assertEqual(normalized["input_modalities"], ["text", "audio"])
        self.assertEqual(normalized["pricing"]["prompt"], "0.000002")
        self.assertNotIn("hugging_face_id", normalized)
        self.assertNotIn("benchmarks", normalized)

    def test_missing_id_refuses_the_model(self) -> None:
        with self.assertRaises(provider_catalog.CatalogValidationError):
            provider_catalog.normalize_model({"name": "no id here"})

    def test_bad_price_type_degrades_to_none_not_a_refusal(self) -> None:
        entry = _model_entry("acme/model")
        entry["pricing"]["prompt"] = 0.000002  # a real client would never do this: number, not string
        normalized = provider_catalog.normalize_model(entry)
        self.assertIsNone(normalized["pricing"]["prompt"])

    def test_filters_select_by_modality(self) -> None:
        catalog = provider_catalog.normalize_catalog(
            _catalog_bytes([
                _model_entry("text-only", input_modalities=("text",)),
                _model_entry("audio-in", input_modalities=("text", "audio")),
            ]),
            filters={"input_modalities": "audio"}, source_url="https://example/models",
        )
        self.assertEqual([m["id"] for m in catalog["models"]], ["audio-in"])
        self.assertEqual(catalog["unfiltered_model_count"], 2)
        self.assertEqual(catalog["filters"], {"input_modalities": "audio"})

    def test_malformed_catalog_is_refused_and_prior_cache_survives(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "openrouter-models-v1.json"
            good = _catalog_bytes([_model_entry("acme/good")])
            first = provider_catalog.refresh(
                cache_file=cache_file, filters={}, fetch=lambda url, timeout: good)
            self.assertTrue(first.ok)

            bad = json.dumps({"data": [{"name": "missing the id field"}]}).encode("utf-8")
            second = provider_catalog.refresh(
                cache_file=cache_file, filters={}, fetch=lambda url, timeout: bad)
            self.assertFalse(second.ok)
            self.assertIsNotNone(second.document)
            self.assertEqual(second.document["models"][0]["id"], "acme/good")
            self.assertEqual(provider_catalog.read_cache(cache_file)["models"][0]["id"], "acme/good")

    def test_oversized_response_is_refused_and_prior_cache_survives(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "openrouter-models-v1.json"
            good = _catalog_bytes([_model_entry("acme/good")])
            provider_catalog.refresh(cache_file=cache_file, filters={}, fetch=lambda url, timeout: good)

            oversized_entry = _model_entry("acme/huge",
                                            description="x" * (provider_catalog.MAX_RESPONSE_BYTES + 1))
            oversized = _catalog_bytes([oversized_entry])
            self.assertGreater(len(oversized), provider_catalog.MAX_RESPONSE_BYTES)
            result = provider_catalog.refresh(
                cache_file=cache_file, filters={}, fetch=lambda url, timeout: oversized)
            self.assertFalse(result.ok)
            self.assertEqual(provider_catalog.read_cache(cache_file)["models"][0]["id"], "acme/good")

    def test_oversized_model_count_is_refused_and_prior_cache_survives(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "openrouter-models-v1.json"
            good = _catalog_bytes([_model_entry("acme/good")])
            provider_catalog.refresh(cache_file=cache_file, filters={}, fetch=lambda url, timeout: good)

            too_many = _catalog_bytes([
                _model_entry(f"acme/model-{i}") for i in range(provider_catalog.MAX_MODEL_COUNT + 1)
            ])
            result = provider_catalog.refresh(
                cache_file=cache_file, filters={}, fetch=lambda url, timeout: too_many)
            self.assertFalse(result.ok)
            self.assertEqual(provider_catalog.read_cache(cache_file)["models"][0]["id"], "acme/good")

    def test_duplicated_id_is_refused_and_prior_cache_survives(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "openrouter-models-v1.json"
            good = _catalog_bytes([_model_entry("acme/good")])
            provider_catalog.refresh(cache_file=cache_file, filters={}, fetch=lambda url, timeout: good)

            duplicated = _catalog_bytes([_model_entry("acme/dup"), _model_entry("acme/dup")])
            result = provider_catalog.refresh(
                cache_file=cache_file, filters={}, fetch=lambda url, timeout: duplicated)
            self.assertFalse(result.ok)
            self.assertIn("duplicate", (result.error or "").lower())
            self.assertEqual(provider_catalog.read_cache(cache_file)["models"][0]["id"], "acme/good")

    def test_truncated_partial_write_catalog_is_refused_and_prior_cache_survives(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "openrouter-models-v1.json"
            good = _catalog_bytes([_model_entry("acme/good")])
            provider_catalog.refresh(cache_file=cache_file, filters={}, fetch=lambda url, timeout: good)

            # Simulate a fetch that was cut off mid-stream: valid-looking JSON
            # sliced before it closes, exactly what a truncated write/download
            # looks like on the wire.
            full = _catalog_bytes([_model_entry("acme/new"), _model_entry("acme/other")])
            truncated = full[: len(full) - 20]
            result = provider_catalog.refresh(
                cache_file=cache_file, filters={}, fetch=lambda url, timeout: truncated)
            self.assertFalse(result.ok)
            self.assertEqual(provider_catalog.read_cache(cache_file)["models"][0]["id"], "acme/good")
            # And the cache file on disk itself must be untouched, not merely
            # "read back the same" by coincidence.
            self.assertEqual(
                json.loads(cache_file.read_text())["models"][0]["id"], "acme/good")

    def test_successful_refresh_writes_atomically_and_is_reloadable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "openrouter-models-v1.json"
            payload = _catalog_bytes([_model_entry("acme/a"), _model_entry("acme/b")])
            result = provider_catalog.refresh(
                cache_file=cache_file, filters={"output_modalities": "text"},
                url="https://openrouter.ai/api/v1/models", fetch=lambda url, timeout: payload)
            self.assertTrue(result.ok)
            self.assertEqual(os.listdir(tmp), [cache_file.name])  # no leftover temp file
            reloaded = provider_catalog.read_cache(cache_file)
            self.assertEqual(reloaded["schema_version"], provider_catalog.SCHEMA_VERSION)
            self.assertEqual(reloaded["source_url"], "https://openrouter.ai/api/v1/models")
            self.assertEqual(reloaded["filters"], {"output_modalities": "text"})
            self.assertEqual(len(reloaded["models"]), 2)

    def test_live_fetch_normalizes_and_caches_the_real_catalog(self) -> None:
        # The task brief permits one live call to verify shape; every
        # refusal-path test above uses local fixtures instead.
        with tempfile.TemporaryDirectory() as tmp:
            cache_file = Path(tmp) / "openrouter-models-v1.json"
            result = provider_catalog.refresh(
                cache_file=cache_file, filters={"input_modalities": "text"}, timeout=15.0)
            if not result.ok:
                self.skipTest(f"no network reachable for the live OpenRouter check: {result.error}")
            self.assertGreater(result.document["unfiltered_model_count"], 0)
            reloaded = provider_catalog.read_cache(cache_file)
            self.assertEqual(reloaded, result.document)


if __name__ == "__main__":
    unittest.main()
