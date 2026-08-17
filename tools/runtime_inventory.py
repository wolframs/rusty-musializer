#!/usr/bin/env python3
"""Per-runtime identity for the local lyric-assist executables (AP2-a).

`musializer_doctor.py` already reports coarse present/absent checks for the
Whisper binary, its model, and the MMS forced-alignment Python runtime. This
module adds the detail an operator needs to actually debug one of them:
resolved path, a best-effort version, the model's path and content hash
"where practical" (see `HASH_MAX_BYTES` below), language coverage, and
whether the build in hand is GPU-capable.

A runtime that is not installed, or installed but missing its model, is
reported as `state: "unavailable"` with an actionable `remediation` string.
This module never raises for that -- a missing *optional* runtime must stay
a per-runtime detail, never a reason for the whole doctor run to fail (that
is `musializer_doctor.audit`'s job to enforce via `--require`, which nothing
here is wired into).

Every probe here is read-only and bounded: `--help`-adjacent introspection
(`git describe`, `ldd`, a short `python -c` probe with a hard timeout), and a
hash of a file already on disk. Nothing is downloaded, nothing runs the
model, nothing mutates state.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import Any, Callable, Mapping, Optional

Which = Callable[[str], Optional[str]]
Runner = Callable[..., "subprocess.CompletedProcess[str]"]
Sha256File = Callable[[Path], str]

# Anything bigger than this is reported without a hash rather than making a
# doctor run slow on a spinning disk or a network mount. Both runtimes this
# module knows about ship comfortably under it (whisper large-v3-turbo is
# ~1.55 GiB, the MMS_FA checkpoint ~1.18 GiB).
HASH_MAX_BYTES = 2 * 1024 * 1024 * 1024  # 2 GiB

_SENSITIVE_MARKERS = ("KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH")


def _safe_environ(environ: Mapping[str, str]) -> dict[str, str]:
    # Same rule as musializer_doctor._gpu_hint and
    # external_analysis._safe_local_env: never hand a spawned probe process
    # anything credential-shaped, even though none of these probes should
    # need one.
    return {key: value for key, value in environ.items()
            if not any(marker in key.upper() for marker in _SENSITIVE_MARKERS)}


def _runtime(state: str, *, path: Optional[Path] = None, version: Optional[str] = None,
             model_path: Optional[Path] = None, model_sha256: Optional[str] = None,
             language_support: Optional[str] = None, gpu_ready: Optional[bool] = None,
             remediation: Optional[str] = None) -> dict[str, Any]:
    return {
        "state": state,
        "path": str(path) if path else None,
        "version": version,
        "model_path": str(model_path) if model_path else None,
        "model_sha256": model_sha256,
        "language_support": language_support,
        "gpu_ready": gpu_ready,
        "remediation": remediation,
    }


def _hash_if_practical(path: Optional[Path], sha256_file: Sha256File) -> Optional[str]:
    if path is None or not path.is_file():
        return None
    try:
        if path.stat().st_size > HASH_MAX_BYTES:
            return None
    except OSError:
        return None
    try:
        return sha256_file(path)
    except OSError:
        return None


def _whisper_version(binary: Path, runner: Runner) -> Optional[str]:
    # whisper.cpp's CLI has no --version flag (checked against the installed
    # 0.146.0-era build: `whisper-cli --help` lists none), so the only signal
    # is the install directory's own git identity, when it has one.
    parents = binary.parents
    install_root = parents[2] if len(parents) >= 3 else None
    if install_root is None or not (install_root / ".git").exists():
        return None
    try:
        result = runner(
            ["git", "-C", str(install_root), "describe", "--tags", "--always"],
            text=True, capture_output=True, timeout=3, check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    version = result.stdout.strip() if result.returncode == 0 else ""
    return version or None


def _binary_links_cuda(binary: Path, which: Which, runner: Runner) -> Optional[bool]:
    ldd = which("ldd")
    if not ldd:
        return None
    try:
        result = runner([ldd, str(binary)], text=True, capture_output=True,
                         timeout=3, check=False)
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return False
    linked = result.stdout.lower()
    return any(marker in linked for marker in ("libcuda", "libcublas", "libcudart"))


def whisper_identity(*, binary: Optional[Path], model: Optional[Path], which: Which,
                      runner: Runner, sha256_file: Sha256File) -> dict[str, Any]:
    binary_ok = bool(binary and binary.is_file() and
                      (os.name == "nt" or os.access(binary, os.X_OK)))
    if not binary_ok:
        return _runtime(
            "unavailable",
            remediation="set MUSIALIZER_WHISPER_BIN or install the discovered whisper.cpp build",
        )
    model_ok = bool(model and model.is_file())
    if not model_ok:
        return _runtime(
            "unavailable", path=binary,
            remediation="set MUSIALIZER_WHISPER_MODEL or install ggml-medium.en.bin",
        )
    assert model is not None
    language_support = ("en-only" if model.name.endswith(".en.bin") else
                         "multilingual (whisper.cpp auto-detect / --language)")
    return _runtime(
        "ok", path=binary, version=_whisper_version(binary, runner),
        model_path=model, model_sha256=_hash_if_practical(model, sha256_file),
        language_support=language_support,
        gpu_ready=_binary_links_cuda(binary, which, runner),
    )


def _python_probe(python_bin: Path, probe: str, runner: Runner,
                   environ: Mapping[str, str]) -> Optional[str]:
    try:
        result = runner([str(python_bin), "-c", probe], text=True, capture_output=True,
                         timeout=15, check=False, env=_safe_environ(environ))
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return None
    text = result.stdout.strip()
    return text or None


def _alignment_gpu_ready(python_bin: Path, runner: Runner,
                          environ: Mapping[str, str]) -> Optional[bool]:
    probe = "import torch; print('true' if torch.cuda.is_available() else 'false')"
    text = _python_probe(python_bin, probe, runner, environ)
    if text is None:
        return None
    return {"true": True, "false": False}.get(text.strip().splitlines()[-1].lower())


def _alignment_version(python_bin: Path, runner: Runner,
                        environ: Mapping[str, str]) -> Optional[str]:
    probe = ("import torch, torchaudio; "
             "print(f'torch {torch.__version__}, torchaudio {torchaudio.__version__}')")
    return _python_probe(python_bin, probe, runner, environ)


def alignment_identity(*, python_bin: Optional[Path], model: Optional[Path], runner: Runner,
                        environ: Mapping[str, str], sha256_file: Sha256File) -> dict[str, Any]:
    python_ok = bool(python_bin and python_bin.is_file() and
                      (os.name == "nt" or os.access(python_bin, os.X_OK)))
    if not python_ok:
        return _runtime(
            "unavailable",
            remediation="set MUSIALIZER_ALIGN_PYTHON or install the lyrics-align runtime",
        )
    model_ok = bool(model and model.is_file())
    if not model_ok:
        return _runtime(
            "unavailable", path=python_bin,
            remediation="run the forced-align helper once to install the MMS_FA model",
        )
    assert python_bin is not None and model is not None
    # MMS/CTC forced alignment runs against a single fixed romanized
    # alphabet rather than a chosen language model; the display text is
    # normalized into it before alignment (force_align_lyrics.alignment_words).
    language_support = "romanized text normalized to the MMS_FA alphabet (force_align_lyrics.alignment_words)"
    return _runtime(
        "ok", path=python_bin, version=_alignment_version(python_bin, runner, environ),
        model_path=model, model_sha256=_hash_if_practical(model, sha256_file),
        language_support=language_support,
        gpu_ready=_alignment_gpu_ready(python_bin, runner, environ),
    )


def stem_separator_identity(*, which: Which, runner: Runner,
                            environ: Mapping[str, str]) -> dict[str, Any]:
    # No stem separator is wired into production Assist yet. Report Demucs as
    # installed/detected only after its launcher can actually import and show
    # help; an executable bit alone accepted the stale build-investigation
    # shim whose Python environment had already been deleted.
    configured = environ.get("MUSIALIZER_STEM_SEPARATOR_BIN", "").strip()
    resolved = configured or which("demucs")
    if not resolved:
        return _runtime(
            "not installed (optional; unused)",
            remediation=("Current Assist always analyzes the full mix. Demucs is not "
                         "installed, and installing it alone would not enable a stem lane."),
        )
    binary = Path(resolved)
    if not (binary.is_file() and (os.name == "nt" or os.access(binary, os.X_OK))):
        return _runtime(
            "broken (unused)",
            remediation=f"MUSIALIZER_STEM_SEPARATOR_BIN={resolved!r} does not resolve to an executable",
        )
    try:
        probe = runner(
            [str(binary), "--help"], text=True, capture_output=True,
            timeout=15, check=False, env=_safe_environ(environ),
        )
    except (OSError, subprocess.SubprocessError) as error:
        return _runtime(
            "broken (unused)", path=binary,
            remediation=f"Demucs was found but could not start: {error}",
        )
    if probe.returncode != 0:
        detail = (probe.stderr or probe.stdout or "launcher exited nonzero").strip()
        detail = " ".join(detail.split())[:240]
        return _runtime(
            "broken (unused)", path=binary,
            remediation=f"Demucs was found but its import/help probe failed: {detail}",
        )
    return _runtime(
        "detected (unused)", path=binary,
        language_support="n/a (audio-domain source separation)",
        remediation=("Detected, but current Assist does not invoke stem separation; "
                     "TC-COARSE still analyzes the full mix."),
    )


def collect(*, whisper_binary: Optional[Path], whisper_model: Optional[Path],
            align_python: Optional[Path], align_model: Optional[Path], which: Which,
            runner: Runner, environ: Mapping[str, str],
            sha256_file: Sha256File) -> dict[str, dict[str, Any]]:
    """Build the `runtimes` section of a doctor report."""
    return {
        "whisper": whisper_identity(binary=whisper_binary, model=whisper_model,
                                     which=which, runner=runner, sha256_file=sha256_file),
        "mms_ctc_aligner": alignment_identity(python_bin=align_python, model=align_model,
                                               runner=runner, environ=environ,
                                               sha256_file=sha256_file),
        "stem_separator": stem_separator_identity(
            which=which, runner=runner, environ=environ),
    }
