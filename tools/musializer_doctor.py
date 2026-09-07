#!/usr/bin/env python3
"""Dependency-free product preflight for Musializer capabilities.

The doctor never invokes FFmpeg, Whisper, Codex, or OpenRouter. It reports
discovery and filesystem readiness only. Credential values are neither read by
this module nor included in its output; OpenRouter presence is delegated to the
same strict dotenv helper used by ``external_analysis.py``.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from typing import Any, Callable, Mapping, Optional, Sequence

# Every sibling helper is imported through the guard below, and none of them is
# imported at module scope any more.
#
# Audit B7: `external_analysis` imports `lyric_align` at *its* top level, so
# deleting one support file killed the doctor at import — empty stdout, and a
# dialog reading "The doctor report could not be read: EOF while parsing a value
# at line 1 column 0". The one instrument that exists to name a broken
# installation was the instrument the breakage silenced. A doctor that cannot
# survive its own missing imports cannot report them, so an import failure is
# now a named failing check (`support_imports`) inside a report that still
# parses.
_IMPORT_FAILURES: dict[str, str] = {}


def _import_helper(name: str) -> Any:
    """Import a sibling helper, or record why it could not be imported.

    Deliberately catches every exception, not only `ImportError`: a helper with
    a syntax error, a missing third-party dependency, or a module-level `raise`
    breaks the doctor in exactly the same way and needs the same sentence.
    """
    try:
        return importlib.import_module(name)
    except BaseException as error:  # noqa: BLE001 - see the docstring
        _IMPORT_FAILURES[name] = f"{type(error).__name__}: {error}"
        return None


analysis_io = _import_helper("analysis_io")
runtime_inventory = _import_helper("runtime_inventory")
antigravity_audio = _import_helper("antigravity_audio")

external_analysis = None
if sys.version_info >= (3, 10):
    # The adapters themselves require Python 3.10 syntax. Keep this import lazy
    # enough that an older interpreter can still run the doctor and explain the
    # version failure instead of dying while parsing an adapter.
    external_analysis = _import_helper("external_analysis")


SCHEMA_VERSION = "musializer.doctor/v1"
ROOT = Path(__file__).resolve().parents[1]
CAPABILITIES = ("preview", "export", "local_lyrics", "remote_mimo", "font_import")

# AP2-b. The same rule as musializer_core::assist::models_dir, restated here
# because the doctor must report what the application will do, not what a
# reader of the operator rule assumes it does. Keep the two in step:
#   1. local_runtimes.models_dir wins, writable or not;
#   2. else <directory of the application executable>/models/, when writable;
#   3. else <home>/musializer/models.
MODELS_DIRECTORY_NAME = "models"
HOME_FALLBACK_DIRECTORY = "musializer"
ASSIST_SETTINGS_SCHEMA = "musializer.assist-settings/v1"
ASSIST_SETTINGS_MAX_BYTES = 256 * 1024  # musializer_core::assist::settings::MAX_FILE_SIZE

Which = Callable[[str], Optional[str]]
FindSpec = Callable[[str], Any]
Runner = Callable[..., subprocess.CompletedProcess[str]]

# The three runtime keys a report always carries, so a doctor that lost
# `runtime_inventory` still answers the question the dialog asks rather than
# omitting the section.
RUNTIME_KEYS = ("whisper", "mms_ctc_aligner", "stem_separator", "antigravity_acp")


def _check(identifier: str, ok: bool, summary: str, *,
           required_for: Sequence[str] = (), detail: Optional[str] = None,
           warning: bool = False) -> dict[str, Any]:
    result: dict[str, Any] = {
        "id": identifier,
        "ok": bool(ok),
        "status": "warning" if warning else ("ok" if ok else "missing"),
        "summary": summary,
        "required_for": list(required_for),
    }
    if detail:
        result["detail"] = detail
    return result


def _existing_application(root: Path) -> Optional[Path]:
    candidates = (
        root / "target/release/musializer",
        root / "target/debug/musializer",
        root / "musializer",
        root / "musializer.exe",
    )
    for path in candidates:
        if path.is_file() and (os.name == "nt" or os.access(path, os.X_OK)):
            return path
    return None


def _missing_files(root: Path, relative_paths: Sequence[str]) -> list[str]:
    return [path for path in relative_paths if not (root / path).is_file()]


def _probe_directory(path: Path) -> tuple[bool, str]:
    candidate = path.expanduser()
    if candidate.exists() and not candidate.is_dir():
        return False, f"{candidate} exists but is not a directory"
    probe = candidate
    while not probe.exists() and probe != probe.parent:
        probe = probe.parent
    if not probe.is_dir():
        return False, f"no existing parent directory for {candidate}"
    try:
        with tempfile.NamedTemporaryFile(prefix=".musializer-doctor-", dir=probe):
            pass
    except OSError as error:
        return False, f"{probe} is not writable: {error.strerror or error}"
    if candidate.exists():
        return True, f"writable: {candidate}"
    return True, f"creatable below writable parent: {probe}"


def _assist_settings_path(environ: Mapping[str, str]) -> Optional[Path]:
    """Where `assist.json` lives, by the same ladder `assist::files` uses."""
    override = environ.get("MUSIALIZER_ASSIST_SETTINGS", "").strip()
    if override:
        return Path(override)
    xdg_config = environ.get("XDG_CONFIG_HOME", "").strip()
    if xdg_config:
        return Path(xdg_config) / "musializer/assist.json"
    home = environ.get("HOME", "").strip()
    if home:
        return Path(home) / ".config/musializer/assist.json"
    return None


def _configured_models_dir(path: Optional[Path]) -> tuple[str, Optional[str]]:
    """`local_runtimes.models_dir` from the settings file: (value, error).

    Only that one field is read, and nothing else from the file reaches the
    report. `assist.json` is non-secret by contract (E11 -- it carries a
    credential *fingerprint* and mode, never a key), and this keeps the
    doctor's "no credential values are read" promise true by construction
    rather than by inspection of the file it happens to find.

    A broken file is reported and then ignored, never repaired and never a
    raised exception: the doctor's job is to say what it found.
    """
    if path is None or not path.is_file():
        return "", None
    try:
        size = path.stat().st_size
        if size > ASSIST_SETTINGS_MAX_BYTES:
            return "", f"{path} is larger than the {ASSIST_SETTINGS_MAX_BYTES} byte cap"
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, UnicodeDecodeError) as error:
        return "", f"{path} could not be read as assist settings: {error}"
    if not isinstance(document, dict):
        return "", f"{path} is not an assist settings object"
    schema = document.get("schema")
    if schema != ASSIST_SETTINGS_SCHEMA:
        return "", f"{path} declares schema {schema!r}, not {ASSIST_SETTINGS_SCHEMA}"
    runtimes = document.get("local_runtimes", {})
    if not isinstance(runtimes, dict):
        return "", f"{path} has a local_runtimes block that is not an object"
    configured = runtimes.get("models_dir", "")
    if not isinstance(configured, str):
        return "", f"{path} has a local_runtimes.models_dir that is not a string"
    return configured.strip(), None


def _models_directory(*, root: Path, application: Optional[Path],
                      environ: Mapping[str, str]) -> dict[str, Any]:
    """Resolve the downloaded-weights directory and show the whole ladder.

    Every candidate is reported, not only the winner, because the operator
    rule is "never a location the user was not shown" -- so an override that
    beat a perfectly good default has to say what it beat.
    """
    install_directory = application.parent if application else root
    install_default = install_directory / MODELS_DIRECTORY_NAME
    home = environ.get("HOME", "").strip()
    home_fallback = (Path(home) / HOME_FALLBACK_DIRECTORY / MODELS_DIRECTORY_NAME
                     if home else None)

    settings_path = _assist_settings_path(environ)
    override, settings_error = _configured_models_dir(settings_path)
    install_writable, install_detail = _probe_directory(install_default)

    if override:
        resolved: Optional[Path] = Path(override)
        source: Optional[str] = "settings-override"
        writable, detail = _probe_directory(resolved)
    elif install_writable:
        resolved, source, writable, detail = (install_default, "install-default",
                                              True, install_detail)
    elif home_fallback is not None:
        resolved, source = home_fallback, "home-fallback"
        writable, detail = _probe_directory(resolved)
    else:
        resolved, source, writable = None, None, False
        detail = f"{install_detail}; and no home directory to fall back to"

    return {
        "state": "ok" if resolved is not None and writable else "unavailable",
        "resolved": str(resolved) if resolved else None,
        "source": source,
        "writable": writable,
        "detail": detail,
        "install_default": str(install_default),
        "install_default_writable": install_writable,
        "install_default_detail": install_detail,
        "home_fallback": str(home_fallback) if home_fallback else None,
        "override": override or None,
        "settings_path": str(settings_path) if settings_path else None,
        "settings_error": settings_error,
    }


def _gpu_hint(which: Which, runner: Runner,
              environ: Mapping[str, str]) -> dict[str, Any]:
    nvidia_smi = which("nvidia-smi")
    if nvidia_smi:
        try:
            sensitive = ("KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH")
            safe_environment = {
                key: value for key, value in environ.items()
                if not any(marker in key.upper() for marker in sensitive)
            }
            result = runner(
                [nvidia_smi, "--query-gpu=name,memory.total", "--format=csv,noheader"],
                text=True, capture_output=True, timeout=3, check=False,
                env=safe_environment,
            )
            devices = [line.strip() for line in result.stdout.splitlines() if line.strip()]
            if result.returncode == 0 and devices:
                return {"kind": "nvidia-smi", "available": True, "devices": devices}
        except (OSError, subprocess.SubprocessError):
            pass
        return {"kind": "nvidia-smi", "available": False, "devices": []}
    visible = environ.get("CUDA_VISIBLE_DEVICES", "").strip()
    if visible and visible != "-1":
        return {"kind": "cuda-environment", "available": True,
                "devices": ["CUDA_VISIBLE_DEVICES is configured"]}
    if Path("/dev/dri/renderD128").exists():
        return {"kind": "dri-render-node", "available": True,
                "devices": ["DRI render node detected"]}
    return {"kind": "none", "available": False, "devices": []}


def _codex_executable(explicit: Optional[Path], which: Which) -> tuple[Optional[str], str]:
    """Resolve Codex using the path the desktop dialog already discovered.

    A Plasma-launched process does not inherit the interactive shell's PATH.
    The Rust dialog has a deliberate four-rung discovery ladder for that case;
    making the doctor repeat only ``shutil.which`` produced two contradictory
    answers in the same window.  An explicit path is therefore authoritative
    and set-but-invalid is reported rather than silently falling back.
    """
    if explicit is not None:
        candidate = explicit.expanduser()
        if candidate.is_file() and (os.name == "nt" or os.access(candidate, os.X_OK)):
            return str(candidate), str(candidate)
        return None, f"configured Codex path is not executable: {candidate}"
    discovered = which("codex")
    return discovered, discovered or "install Codex or select its executable in AI settings"


def _unmeasured_runtimes(reason: str) -> dict[str, dict[str, Any]]:
    """The `runtimes` section when `runtime_inventory` could not be imported.

    "Unavailable, and here is why" rather than an omitted section: an absent key
    reads to the Rust side as *unmeasured*, which is deliberately not a refusal
    (`PreflightFacts::runtime`), so a doctor that lost its inventory module
    would clear every local lane it can no longer see.
    """
    return {
        key: {
            "state": "unavailable", "path": None, "version": None,
            "model_path": None, "model_sha256": None, "language_support": None,
            "gpu_ready": None, "remediation": reason,
        }
        for key in RUNTIME_KEYS
    }


def _resolved_path(configured: Optional[Path],
                   discovered: Optional[Path]) -> tuple[Optional[Path], str]:
    """A runtime path and where it came from, by the job's own precedence.

    `external_analysis.run_assist` is `configured or discovered` -- a flag beats
    everything, present or not. Mirrored exactly, expansion included (there is
    none on either side): the doctor's job is to measure the installation a job
    would use, so a `~` that the job would not expand must not be expanded here
    either. Audit B1.
    """
    if configured is not None:
        return configured, "configured"
    if discovered is not None:
        return discovered, "discovered"
    return None, "none"


def _path_fact(value: Optional[Path], source: str) -> dict[str, Any]:
    return {"value": str(value) if value else None, "source": source}


def _path_detail(value: Optional[Path], source: str, remedy: str) -> str:
    """A check detail that says which path was probed and who chose it."""
    return f"{value} ({source})" if value else remedy


def audit(*, root: Path = ROOT, analysis_dir: Optional[Path] = None,
          output_dir: Optional[Path] = None,
          codex_bin: Optional[Path] = None,
          whisper_bin: Optional[Path] = None,
          whisper_model: Optional[Path] = None,
          align_python: Optional[Path] = None,
          antigravity_server: Optional[Path] = None,
          antigravity_harness: Optional[Path] = None,
          antigravity_profile: Optional[Path] = None,
          allow_dotenv: bool = True,
          environ: Optional[Mapping[str, str]] = None,
          which: Which = shutil.which, find_spec: FindSpec = importlib.util.find_spec,
          runner: Runner = subprocess.run) -> dict[str, Any]:
    root = root.resolve()
    analysis_dir = (analysis_dir or root / "build/analysis").resolve()
    output_dir = (output_dir or Path.cwd()).resolve()
    environ = environ if environ is not None else os.environ
    checks: list[dict[str, Any]] = []

    application = _existing_application(root)
    checks.append(_check(
        "application", application is not None,
        "Musializer application executable",
        required_for=CAPABILITIES,
        detail=str(application) if application else "run cargo build --release",
    ))

    ffmpeg = which("ffmpeg")
    checks.append(_check(
        "ffmpeg", ffmpeg is not None, "FFmpeg executable",
        required_for=("export", "local_lyrics", "remote_mimo"),
        detail=ffmpeg or "install FFmpeg and add it to PATH",
    ))
    ffprobe = which("ffprobe")
    checks.append(_check(
        "ffprobe", ffprobe is not None, "ffprobe export validator",
        detail=ffprobe or "optional, but recommended for release smoke checks",
        warning=ffprobe is None,
    ))

    python_ok = sys.version_info >= (3, 10)
    checks.append(_check(
        "python", python_ok, "Python 3.10 or newer",
        required_for=("local_lyrics", "remote_mimo", "font_import"),
        detail=f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}",
    ))
    numpy_ok = find_spec("numpy") is not None
    checks.append(_check(
        "numpy", numpy_ok, "NumPy measured-analysis dependency",
        required_for=("local_lyrics", "remote_mimo"),
        detail="installed" if numpy_ok else "install NumPy for the active Python interpreter",
    ))

    shared_assets = (
        "tools/external_analysis.py", "tools/analysis_io.py",
        # `external_analysis` imports this at its own top level, so its absence
        # kills every job -- remote as well as local -- and killed the doctor
        # with them until the guarded imports above (audit B7).
        "tools/lyric_align.py",
        "tools/analyze_audio.py", "schemas/analysis-cache-v1.schema.json",
        "schemas/analysis-provenance-v1.schema.json",
        "schemas/measured-analysis-v1.schema.json",
        "schemas/scene-plan-v1.schema.json",
    )
    missing = _missing_files(root, shared_assets)
    checks.append(_check(
        "analysis_assets", not missing, "Shared analysis helpers and schemas",
        required_for=("local_lyrics", "remote_mimo"),
        detail="present" if not missing else "missing: " + ", ".join(missing),
    ))

    lyric_assets = (
        "tools/import_whisper.py", "tools/force_align_lyrics.py",
        "tools/lyric_anchor_block.py", "tools/anchor_block_align.py",
        "prompts/lyrics_cleanup_system.md",
        "schemas/codex-lyric-review-output-v1.schema.json",
        "schemas/lyric-review-v1.schema.json",
        "schemas/lyric-timing-v1.schema.json",
    )
    missing = _missing_files(root, lyric_assets)
    checks.append(_check(
        "lyric_assets", not missing, "Local lyric helpers, prompt, and schemas",
        required_for=("local_lyrics",),
        detail="present" if not missing else "missing: " + ", ".join(missing),
    ))

    # Reachability is deliberately not probed: a doctor run must not itself
    # open the network boundary it is reporting on. This says the helper and
    # its contract are installed, which is the part a distribution can get
    # wrong. Whether fonts.google.com answers is between the user and their
    # connection, and the import surfaces that failure where it happens.
    font_assets = (
        "tools/google_fonts.py", "tools/analysis_io.py",
        "schemas/font-import-v1.schema.json",
    )
    missing = _missing_files(root, font_assets)
    checks.append(_check(
        "font_assets", not missing, "Google Fonts import helper and schema",
        required_for=("font_import",),
        detail="present" if not missing else "missing: " + ", ".join(missing),
    ))

    mimo_assets = (
        "tools/mimo_openrouter.py", "schemas/semantic-notes-v1.schema.json",
        "schemas/semantic-score-v1.schema.json",
    )
    missing = _missing_files(root, mimo_assets)
    checks.append(_check(
        "mimo_assets", not missing, "MiMo helper and semantic schemas",
        required_for=("remote_mimo",),
        detail="present" if not missing else "missing: " + ", ".join(missing),
    ))

    codex, codex_detail = _codex_executable(codex_bin, which)
    checks.append(_check(
        "codex", codex is not None, "Codex executable for lyric review",
        required_for=("local_lyrics",),
        detail=codex_detail,
    ))

    # The configured path wins, exactly as it does in a job -- and it wins even
    # when it is wrong, which is the half of audit B1 a doctor calling the bare
    # discovery defaults could never see. A typo'd `whisper_bin` in `assist.json`
    # used to get a green doctor and a job that died at
    # `external_analysis.py`'s own file check.
    if external_analysis is not None:
        discovered_bin, discovered_model = external_analysis._default_whisper_paths()
        discovered_align = external_analysis._default_alignment_python()
        align_model = external_analysis._default_alignment_model()
    else:
        discovered_bin = discovered_model = discovered_align = align_model = None
    whisper_bin, whisper_bin_source = _resolved_path(whisper_bin, discovered_bin)
    whisper_model, whisper_model_source = _resolved_path(whisper_model, discovered_model)
    align_python, align_python_source = _resolved_path(align_python, discovered_align)

    whisper_bin_ok = bool(whisper_bin and whisper_bin.is_file() and
                          (os.name == "nt" or os.access(whisper_bin, os.X_OK)))
    whisper_model_ok = bool(whisper_model and whisper_model.is_file())
    checks.append(_check(
        "whisper_binary", whisper_bin_ok, "Whisper executable",
        required_for=("local_lyrics",),
        detail=_path_detail(
            whisper_bin, whisper_bin_source,
            "set MUSIALIZER_WHISPER_BIN or install the discovered whisper.cpp build"),
    ))
    checks.append(_check(
        "whisper_model", whisper_model_ok, "Whisper model",
        required_for=("local_lyrics",),
        detail=_path_detail(
            whisper_model, whisper_model_source,
            "set MUSIALIZER_WHISPER_MODEL or install ggml-medium.en.bin"),
    ))
    checks.append(_check(
        "alignment_python", align_python is not None,
        "MMS forced-alignment Python runtime",
        required_for=("local_lyrics",),
        detail=_path_detail(
            align_python, align_python_source,
            "set MUSIALIZER_ALIGN_PYTHON or install the lyrics-align runtime"),
    ))
    checks.append(_check(
        "alignment_model", bool(align_model and align_model.is_file()),
        "MMS forced-alignment acoustic model",
        required_for=("local_lyrics",),
        detail=(str(align_model) if align_model and align_model.is_file() else
                "run the forced-align helper once to install the MMS_FA model"),
    ))

    # This is the sole credential read. Membership is checked, never the value.
    #
    # `allow_dotenv` is the desktop's reality, not a convenience: the
    # application always spawns the helper with `--no-dotenv`, so a key living
    # only in the repository `.env` authorizes nothing there. A doctor that read
    # it anyway answered "configured" about a credential no job would use
    # (audit B1). The dotenv rung stays for the command line, which is the only
    # place it was ever meant to serve.
    openrouter_ok = False
    if external_analysis is not None:
        openrouter_environment = external_analysis._openrouter_env(
            root / ".env", allow_dotenv=allow_dotenv)
        openrouter_ok = "OPENROUTER_API_KEY" in openrouter_environment
    checks.append(_check(
        "openrouter", openrouter_ok, "OpenRouter credential for remote MiMo",
        required_for=("remote_mimo",),
        detail="configured in the environment" if openrouter_ok else (
            "no OPENROUTER_API_KEY in this environment; the repository .env was "
            "not consulted, because the application never does (--no-dotenv). "
            "The desktop supplies its own key from the credentials store, which "
            "this doctor deliberately does not read"
            if not allow_dotenv else
            "set OPENROUTER_API_KEY or add only that key to the repository .env"),
    ))

    # Audit B7. A helper that failed to import is named here, with the
    # exception that named it, instead of taking the whole report down.
    checks.append(_check(
        "support_imports", not _IMPORT_FAILURES,
        "Sibling helper modules import cleanly",
        required_for=("local_lyrics", "remote_mimo"),
        detail="present" if not _IMPORT_FAILURES else "; ".join(
            f"{name}: {reason}" for name, reason in sorted(_IMPORT_FAILURES.items())),
    ))

    analysis_ok, analysis_detail = _probe_directory(analysis_dir)
    checks.append(_check(
        "analysis_directory", analysis_ok, "Writable analysis cache directory",
        required_for=("local_lyrics", "remote_mimo"), detail=analysis_detail,
    ))
    output_ok, output_detail = _probe_directory(output_dir)
    checks.append(_check(
        "output_directory", output_ok, "Writable export output directory",
        required_for=("export",), detail=output_detail,
    ))

    capabilities: dict[str, dict[str, Any]] = {}
    for capability in CAPABILITIES:
        missing_ids = [item["id"] for item in checks
                       if capability in item["required_for"] and not item["ok"]]
        capabilities[capability] = {"ready": not missing_ids, "missing": missing_ids}

    # Per-runtime identity (AP2-a): richer than the flat checks above, and
    # never a reason the overall run fails -- a missing optional runtime is
    # this dict's own "unavailable" entry, not a raised exception. See
    # runtime_inventory.py for what each field means and how it is probed.
    if runtime_inventory is None or analysis_io is None:
        missing_module = "runtime_inventory" if runtime_inventory is None else "analysis_io"
        runtimes = _unmeasured_runtimes(
            f"tools/{missing_module}.py could not be imported "
            f"({_IMPORT_FAILURES.get(missing_module, 'unknown error')}), so no runtime "
            "could be probed; reinstall the Musializer support files")
    else:
        runtimes = runtime_inventory.collect(
            whisper_binary=whisper_bin, whisper_model=whisper_model,
            align_python=align_python, align_model=align_model,
            which=which, runner=runner, environ=environ,
            sha256_file=analysis_io.sha256_file,
        )

    if antigravity_audio is not None:
        runtimes["antigravity_acp"] = antigravity_audio.inventory(
            server=antigravity_server, harness=antigravity_harness,
            profile=antigravity_profile, environ=environ)

    return {
        "schema_version": SCHEMA_VERSION,
        "root": str(root),
        "analysis_directory": str(analysis_dir),
        "output_directory": str(output_dir),
        "checks": checks,
        "capabilities": capabilities,
        "runtimes": runtimes,
        # Which installation this verdict is about. Every path above is either
        # one the caller configured or one discovery found, and the two produce
        # the same green tick from opposite facts -- so the report says which,
        # and an audit of a wrong verdict has somewhere to start (B1).
        "paths": {
            "whisper_bin": _path_fact(whisper_bin, whisper_bin_source),
            "whisper_model": _path_fact(whisper_model, whisper_model_source),
            "align_python": _path_fact(align_python, align_python_source),
            "align_model": _path_fact(align_model, "discovered" if align_model else "none"),
            "codex": _path_fact(
                Path(codex) if codex else None,
                "configured" if codex_bin is not None else
                ("discovered" if codex else "none")),
            "openrouter": {
                "dotenv_consulted": bool(allow_dotenv),
                "dotenv_path": str(root / ".env"),
            },
        },
        "import_failures": dict(sorted(_IMPORT_FAILURES.items())),
        "models_directory": _models_directory(
            root=root, application=application, environ=environ),
        "gpu": _gpu_hint(which, runner, environ),
    }


def render_human(report: Mapping[str, Any]) -> str:
    lines = ["Musializer product doctor", f"Root: {report['root']}", "", "Checks:"]
    for item in report["checks"]:
        marker = "OK" if item["ok"] else ("WARN" if item["status"] == "warning" else "MISS")
        line = f"  [{marker:4}] {item['summary']}"
        if item.get("detail"):
            line += f" - {item['detail']}"
        lines.append(line)
    lines.extend(("", "Capabilities:"))
    labels = {
        "preview": "Preview/playback",
        "export": "MP4 export",
        "local_lyrics": "Local Whisper + Codex lyrics",
        "remote_mimo": "Remote MiMo analysis",
        "font_import": "Google Fonts caption face import",
    }
    for name in CAPABILITIES:
        capability = report["capabilities"][name]
        status = "READY" if capability["ready"] else "BLOCKED"
        suffix = "" if capability["ready"] else ": " + ", ".join(capability["missing"])
        lines.append(f"  {labels[name]}: {status}{suffix}")
    lines.extend(("", "Runtimes:"))
    runtime_labels = {
        "whisper": "Whisper", "mms_ctc_aligner": "MMS/CTC forced aligner",
        "stem_separator": "Stem separator",
    }
    for runtime_id, label in runtime_labels.items():
        runtime = report["runtimes"][runtime_id]
        if runtime["state"] != "ok":
            lines.append(f"  {label}: UNAVAILABLE - {runtime['remediation']}")
            continue
        detail_bits = []
        if runtime["path"]:
            detail_bits.append(runtime["path"])
        if runtime["version"]:
            detail_bits.append(f"version {runtime['version']}")
        if runtime["gpu_ready"] is not None:
            detail_bits.append("GPU-ready" if runtime["gpu_ready"] else "CPU-only")
        lines.append(f"  {label}: OK - " + ", ".join(detail_bits))
        if runtime["language_support"]:
            lines.append(f"    language support: {runtime['language_support']}")
        if runtime["model_path"]:
            hash_bits = f" ({runtime['model_sha256']})" if runtime["model_sha256"] else " (hash skipped)"
            lines.append(f"    model: {runtime['model_path']}{hash_bits}")
    paths = report.get("paths", {})
    if paths:
        lines.extend(("", "Probed paths:"))
        for key in ("whisper_bin", "whisper_model", "align_python", "align_model", "codex"):
            fact = paths.get(key, {})
            value = fact.get("value") or "none found"
            lines.append(f"  {key}: {value} ({fact.get('source', 'none')})")
        openrouter = paths.get("openrouter", {})
        lines.append(
            "  openrouter: environment only"
            if not openrouter.get("dotenv_consulted")
            else f"  openrouter: environment, then {openrouter.get('dotenv_path')}")
    models = report["models_directory"]
    lines.extend(("", "Models directory:"))
    if models["resolved"]:
        lines.append(f"  Resolved: {models['resolved']} ({models['source']}"
                     f"{'' if models['writable'] else ', NOT WRITABLE'})")
    else:
        lines.append(f"  Resolved: none - {models['detail']}")
    lines.append(f"  Install default: {models['install_default']}"
                 f" ({'writable' if models['install_default_writable'] else 'not writable'})")
    if models["home_fallback"]:
        lines.append(f"  Home fallback: {models['home_fallback']}")
    settings_path = models["settings_path"] or "no per-user config directory"
    if models["override"]:
        lines.append(f"  Override: {models['override']}"
                     f" (local_runtimes.models_dir in {settings_path}; it wins over the default above)")
    else:
        lines.append("  Override: none"
                     f" (set local_runtimes.models_dir in {settings_path} and it wins over the default above)")
    if models["settings_error"]:
        lines.append(f"  Settings problem: {models['settings_error']}")
    gpu = report["gpu"]
    hint = ", ".join(gpu["devices"]) if gpu["devices"] else "no optional GPU hint detected"
    lines.extend(("", f"GPU hint: {hint}"))
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    parser.add_argument("--root", type=Path, default=ROOT,
                        help="checkout or distribution root")
    parser.add_argument("--analysis-dir", type=Path,
                        help="analysis cache directory to probe")
    parser.add_argument("--output-dir", type=Path,
                        help="video output directory to probe (default: current directory)")
    parser.add_argument("--codex-bin", type=Path,
                        help="resolved Codex executable from the desktop discovery ladder")
    # The same four spellings `external_analysis.py assist` takes, so the
    # desktop can hand the doctor the identical installation a job would run
    # (audit B1). A flag beats discovery here exactly as it does there.
    parser.add_argument("--whisper-bin", type=Path,
                        help="configured whisper.cpp executable (as a job receives it)")
    parser.add_argument("--whisper-model", type=Path,
                        help="configured whisper.cpp model (as a job receives it)")
    parser.add_argument("--align-python", type=Path,
                        help="configured forced-alignment interpreter (as a job receives it)")
    parser.add_argument("--antigravity-server", type=Path)
    parser.add_argument("--antigravity-harness", type=Path)
    parser.add_argument("--antigravity-profile", type=Path)
    parser.add_argument("--no-dotenv", action="store_true",
                        help="do not consult the repository .env for OPENROUTER_API_KEY, "
                             "which is what the desktop always does")
    parser.add_argument("--require", action="append", choices=CAPABILITIES,
                        default=[], help="exit nonzero unless this capability is ready")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    report = audit(root=args.root, analysis_dir=args.analysis_dir,
                   output_dir=args.output_dir, codex_bin=args.codex_bin,
                   whisper_bin=args.whisper_bin, whisper_model=args.whisper_model,
                   align_python=args.align_python, allow_dotenv=not args.no_dotenv,
                   antigravity_server=args.antigravity_server,
                   antigravity_harness=args.antigravity_harness,
                   antigravity_profile=args.antigravity_profile)
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    else:
        print(render_human(report))
    required = args.require or ["preview"]
    return 0 if all(report["capabilities"][name]["ready"] for name in required) else 1


if __name__ == "__main__":
    raise SystemExit(main())
