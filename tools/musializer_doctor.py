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

from analysis_io import sha256_file
import runtime_inventory

external_analysis = None
if sys.version_info >= (3, 10):
    # The adapters themselves require Python 3.10 syntax. Keep this import lazy
    # enough that an older interpreter can still run the doctor and explain the
    # version failure instead of dying while parsing an adapter.
    import external_analysis as _external_analysis
    external_analysis = _external_analysis


SCHEMA_VERSION = "musializer.doctor/v1"
ROOT = Path(__file__).resolve().parents[1]
CAPABILITIES = ("preview", "export", "local_lyrics", "remote_mimo", "font_import")

Which = Callable[[str], Optional[str]]
FindSpec = Callable[[str], Any]
Runner = Callable[..., subprocess.CompletedProcess[str]]


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


def audit(*, root: Path = ROOT, analysis_dir: Optional[Path] = None,
          output_dir: Optional[Path] = None,
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

    codex = which("codex")
    checks.append(_check(
        "codex", codex is not None, "Codex executable for lyric review",
        required_for=("local_lyrics",),
        detail=codex or "install Codex and add it to PATH",
    ))

    if external_analysis is not None:
        whisper_bin, whisper_model = external_analysis._default_whisper_paths()
    else:
        whisper_bin, whisper_model = None, None
    whisper_bin_ok = bool(whisper_bin and whisper_bin.is_file() and
                          (os.name == "nt" or os.access(whisper_bin, os.X_OK)))
    whisper_model_ok = bool(whisper_model and whisper_model.is_file())
    checks.append(_check(
        "whisper_binary", whisper_bin_ok, "Whisper executable",
        required_for=("local_lyrics",),
        detail=str(whisper_bin) if whisper_bin_ok else
               "set MUSIALIZER_WHISPER_BIN or install the discovered whisper.cpp build",
    ))
    checks.append(_check(
        "whisper_model", whisper_model_ok, "Whisper model",
        required_for=("local_lyrics",),
        detail=str(whisper_model) if whisper_model_ok else
               "set MUSIALIZER_WHISPER_MODEL or install ggml-medium.en.bin",
    ))
    align_python = (external_analysis._default_alignment_python()
                    if external_analysis is not None else None)
    align_model = (external_analysis._default_alignment_model()
                   if external_analysis is not None else None)
    checks.append(_check(
        "alignment_python", align_python is not None,
        "MMS forced-alignment Python runtime",
        required_for=("local_lyrics",),
        detail=(str(align_python) if align_python else
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
    openrouter_ok = False
    if external_analysis is not None:
        openrouter_environment = external_analysis._openrouter_env(root / ".env")
        openrouter_ok = "OPENROUTER_API_KEY" in openrouter_environment
    checks.append(_check(
        "openrouter", openrouter_ok, "OpenRouter credential for remote MiMo",
        required_for=("remote_mimo",),
        detail="configured" if openrouter_ok else
               "set OPENROUTER_API_KEY or add only that key to the repository .env",
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
    runtimes = runtime_inventory.collect(
        whisper_binary=whisper_bin, whisper_model=whisper_model,
        align_python=align_python, align_model=align_model,
        which=which, runner=runner, environ=environ, sha256_file=sha256_file,
    )

    return {
        "schema_version": SCHEMA_VERSION,
        "root": str(root),
        "analysis_directory": str(analysis_dir),
        "output_directory": str(output_dir),
        "checks": checks,
        "capabilities": capabilities,
        "runtimes": runtimes,
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
    parser.add_argument("--require", action="append", choices=CAPABILITIES,
                        default=[], help="exit nonzero unless this capability is ready")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    report = audit(root=args.root, analysis_dir=args.analysis_dir,
                   output_dir=args.output_dir)
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    else:
        print(render_human(report))
    required = args.require or ["preview"]
    return 0 if all(report["capabilities"][name]["ready"] for name in required) else 1


if __name__ == "__main__":
    raise SystemExit(main())
