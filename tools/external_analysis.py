#!/usr/bin/env python3
"""Offline orchestration for Whisper, Codex lyric review, and scene planning.

External programs are invoked as argv arrays without a shell, with bounded
timeouts. Audio/lyrics are sent through files or stdin, never command-line
arguments. Only the explicit ``assist --mode mimo|all`` path may call the
existing OpenRouter helper.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import math
import os
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any, Callable, Sequence

from analysis_io import (
    ADAPTER_VERSION,
    AnalysisValidationError,
    atomic_write_json,
    canonical_sha256,
    duration,
    read_json,
    sha256_file,
)
from import_whisper import normalize_whisper
import force_align_lyrics
import lyric_align
import lyric_anchor_block
import mimo_openrouter as mimo_adapter


ROOT = Path(__file__).resolve().parents[1]
LYRIC_PROMPT = ROOT / "prompts" / "lyrics_cleanup_system.md"
CODEX_OUTPUT_SCHEMA = ROOT / "schemas" / "codex-lyric-review-output-v1.schema.json"
LYRIC_REVIEW_VERSION = "musializer.lyric-review/v1"
LYRIC_PROMPT_VERSION = "lyrics_cleanup_system/v2"
LYRIC_SYNC_VERSION = lyric_align.LYRIC_SYNC_VERSION
FORCED_ALIGNER = ROOT / "tools" / "force_align_lyrics.py"
ANCHOR_BLOCK_ALIGNER = ROOT / "tools" / "anchor_block_align.py"
# whisper-cli 1.8.6 has no `--no-context`; `--max-context 0` is the reachable
# equivalent and is recorded here so a lane decoded with conditioning on cannot
# be reused. See `whisper_vad_model` for the measurement behind both choices.
WHISPER_TEXT_CONDITIONING = "none_max_context_0"
# The model may emit up to this many characters per reviewed line; the
# deterministic splitter then reduces cues to display size. The C editor
# rejects cue text at 512 bytes, so both bounds stay far inside it.
REVIEW_TEXT_LIMIT = 200
REVIEW_DURATION_LIMIT_SECONDS = 15.0
# ffprobe JSON escaping can expand an embedded lyric tag several times over;
# this bound comfortably holds the 64 KiB reference limit after escaping.
_PROBE_STDOUT_LIMIT = 512 * 1024
REFERENCE_SIBLING_SUFFIX = ".lyrics.txt"
SCENE_PLAN_VERSION = "musializer.scene-plan/v1"
SEMANTIC_NOTES_VERSION = "musializer.semantic-notes/v1"
BRIDGE_VERSION = "MUSIALIZER_BRIDGE\t1"
# The scene vocabulary the model may recommend. `phosphor` postdates the frozen
# C (2026-08-08); it is listed here because a scene the planner cannot name is a
# scene an assist run can never choose, which is how a finished feature stays
# unreachable from the one surface that picks scenes automatically.
SCENES = (
    "spectrum", "pulse", "orbital", "ascii", "atlas", "terrarium",
    "constellation", "cadence", "loom", "pentagram", "phosphor", "clawd",
)

# Keep the orchestrator's measured-cache contract dependency-free. These are
# the explicit defaults passed to tools/analyze_audio.py; its adapter version is
# the invalidation boundary for algorithm/band-definition changes.
MEASURED_ANALYZER_VERSION = "1"
MEASURED_SAMPLE_RATE = 24000
MEASURED_CHANNELS = 1
MEASURED_WINDOW = 2048
MEASURED_HOP = 1024

# Tranche P4: the frozen route graph the application resolves at Start.
#
# `docs/ASSIST_PROVIDER_CONTRACTS.md` §6. The application writes it into the job
# folder before this process exists and passes the path; nothing here resolves a
# route, and nothing here rewrites that file. What this module does with it is
# three things: configure the model and provider constraints of every remote
# lane, key cache acceptance on the route identity (§5 rule 7), and embed it —
# with the model ids it OBSERVED rather than the ones it was asked for — in the
# manifest and in each produced artifact's provenance.
EXECUTION_SNAPSHOT_SCHEMA = "musializer.assist-execution/v1"
EXECUTION_SNAPSHOT_BYTE_LIMIT = 256 * 1024

# The fields of a contract row that make up its cache identity.
#
# Deliberately excludes everything observed — `provider_served`,
# `runtime_version`, `model_sha256` — because those describe the run rather than
# the route, and including them would regenerate every artifact whenever a
# provider rebalanced. There is exactly one implementation of this list, here,
# and the identity is compared as a dict rather than a digest so a mismatch is
# legible in the manifest a user can open.
EXECUTION_ROUTE_IDENTITY_FIELDS = (
    "contract", "route_type", "runtime_id", "model_id", "reasoning_effort",
    "boundary_applied", "audio_scope", "provider_constraints",
    "fallback_policy",
)

# The label the settings dialog uses for a Codex route with no chosen model
# (§5 rule 6). It is a documented fallback, not a model id, so it must never
# reach `codex exec --model`.
CODEX_DEFAULT_LABEL = "Codex default"

Runner = Callable[..., subprocess.CompletedProcess[str]]


DIAGNOSTIC_TAIL_LIMIT = 16384


class _BoundedTail:
    """Continuously drained child output with a strict in-memory byte bound."""

    def __init__(self, limit: int) -> None:
        self.limit = limit
        self.data = bytearray()

    def append(self, chunk: bytes) -> None:
        self.data.extend(chunk)
        overflow = len(self.data) - self.limit
        if overflow > 0:
            del self.data[:overflow]

    def text(self) -> str:
        return bytes(self.data).decode("utf-8", "replace")


def _drain_child_stream(stream: Any, tail: _BoundedTail) -> None:
    try:
        while True:
            chunk = stream.read(4096)
            if not chunk:
                break
            tail.append(chunk)
    finally:
        stream.close()


def _write_child_stdin(stream: Any, content: str) -> None:
    try:
        stream.write(content.encode("utf-8"))
        stream.flush()
    except (BrokenPipeError, OSError):
        pass
    finally:
        stream.close()


def _terminate_child_tree(process: subprocess.Popen[bytes]) -> None:
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGKILL)
            return
        except ProcessLookupError:
            return
        except OSError:
            pass
    try:
        process.kill()
    except OSError:
        pass


def _terminate_remaining_posix_group(process: subprocess.Popen[bytes]) -> None:
    if os.name != "posix":
        return
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, OSError):
        pass


def _join_workers_bounded(workers: Sequence[threading.Thread],
                          timeout: float = 2.0) -> None:
    deadline = time.monotonic() + timeout
    for worker in workers:
        worker.join(max(0.0, deadline - time.monotonic()))


def _run_bounded_process(
    argv: Sequence[str], *, timeout: float, stdin: str | None,
    cwd: Path | None, env: dict[str, str] | None,
    stdout_limit: int = DIAGNOSTIC_TAIL_LIMIT,
) -> subprocess.CompletedProcess[str]:
    """Run a real child while draining stdout/stderr into bounded tails."""
    options: dict[str, Any] = {
        "stdin": subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
        "stdout": subprocess.PIPE,
        "stderr": subprocess.PIPE,
        "cwd": str(cwd) if cwd else None,
        "env": env,
    }
    if os.name == "posix":
        options["start_new_session"] = True
    elif os.name == "nt":
        options["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    process = subprocess.Popen(list(argv), **options)
    assert process.stdout is not None and process.stderr is not None
    stdout_tail = _BoundedTail(stdout_limit)
    stderr_tail = _BoundedTail(DIAGNOSTIC_TAIL_LIMIT)
    readers = [
        threading.Thread(target=_drain_child_stream,
                         args=(process.stdout, stdout_tail), daemon=True),
        threading.Thread(target=_drain_child_stream,
                         args=(process.stderr, stderr_tail), daemon=True),
    ]
    for reader in readers:
        reader.start()
    writer: threading.Thread | None = None
    if stdin is not None:
        assert process.stdin is not None
        writer = threading.Thread(target=_write_child_stdin,
                                  args=(process.stdin, stdin), daemon=True)
        writer.start()
    try:
        returncode = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        _terminate_child_tree(process)
        try:
            process.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            pass
        _join_workers_bounded([*([] if writer is None else [writer]), *readers])
        raise subprocess.TimeoutExpired(
            list(argv), timeout, output=stdout_tail.text(),
            stderr=stderr_tail.text(),
        ) from error
    # A command is not allowed to report success while leaving descendants
    # behind. On POSIX each child owns a private process group, so any process
    # still holding the capture pipes after the direct child exits is stopped.
    # Windows descendants remain inside the desktop worker's Job Object; the
    # bounded joins let the root helper exit so the C host can verify/close it.
    _terminate_remaining_posix_group(process)
    _join_workers_bounded([*([] if writer is None else [writer]), *readers])
    return subprocess.CompletedProcess(
        list(argv), returncode, stdout_tail.text(), stderr_tail.text())


def _write_diagnostic(sink: Path, name: str, detail: str,
                      stdout: str | None, stderr: str | None) -> bool:
    """Persist a bounded child-output tail beside the other job artifacts.

    The sink must live in the per-job directory, which already holds the
    private evidence the child consumed, so this records no new content
    class; it only makes an opaque exit explainable after the fact.
    """
    sections = [f"{name}: {detail}"]
    for label, text in (("stderr", stderr), ("stdout", stdout)):
        if text:
            sections.append(f"--- {label} (last {DIAGNOSTIC_TAIL_LIMIT} chars) ---")
            sections.append(text[-DIAGNOSTIC_TAIL_LIMIT:])
    try:
        sink.write_text("\n".join(sections) + "\n", encoding="utf-8")
        return True
    except OSError:
        return False


def _run(
    argv: Sequence[str], *, timeout: float, stdin: str | None = None,
    cwd: Path | None = None, env: dict[str, str] | None = None,
    runner: Runner = subprocess.run, diagnostic_sink: Path | None = None,
    stdout_limit: int = DIAGNOSTIC_TAIL_LIMIT,
) -> subprocess.CompletedProcess[str]:
    if not argv or timeout <= 0 or not math.isfinite(timeout):
        raise AnalysisValidationError("external command and positive finite timeout are required")
    name = Path(argv[0]).name
    try:
        if runner is subprocess.run:
            result = _run_bounded_process(
                argv, timeout=timeout, stdin=stdin, cwd=cwd, env=env,
                stdout_limit=stdout_limit)
        else:
            # Injected runners are used only by deterministic offline tests.
            result = runner(
                list(argv), input=stdin, text=True, capture_output=True,
                timeout=timeout, check=False, cwd=str(cwd) if cwd else None,
                env=env,
            )
    except subprocess.TimeoutExpired as error:
        detail = f"exceeded its {timeout:g}s timeout"
        out = error.stdout.decode("utf-8", "replace") if isinstance(error.stdout, bytes) else error.stdout
        err = error.stderr.decode("utf-8", "replace") if isinstance(error.stderr, bytes) else error.stderr
        if diagnostic_sink is not None and _write_diagnostic(
                diagnostic_sink, name, detail, out, err):
            detail += f" (child output: {diagnostic_sink.name})"
        raise RuntimeError(f"{name} {detail}") from error
    except OSError as error:
        raise RuntimeError(f"could not start {name}: {error}") from error
    if result.returncode != 0:
        # Child output may contain private lyrics, paths, or provider
        # diagnostics; the summary log stays clean. When the caller names a
        # diagnostic sink inside the per-job artifact directory, a bounded
        # output tail is preserved there so the failure stays actionable.
        detail = f"exited with code {result.returncode}"
        if diagnostic_sink is not None and _write_diagnostic(
                diagnostic_sink, name, detail, result.stdout, result.stderr):
            detail += f" (child output: {diagnostic_sink.name})"
        raise RuntimeError(f"{name} {detail}")
    return result


def _safe_local_env() -> dict[str, str]:
    """Do not expose unrelated API credentials to local analysis children."""
    sensitive = ("KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH")
    return {key: value for key, value in os.environ.items()
            if not any(marker in key.upper() for marker in sensitive)}


def _local_runtime_env(executable: str | Path) -> dict[str, str]:
    """Safe child environment with user-local interpreter directories.

    Musializer can discover an npm Codex wrapper outside a desktop launcher's
    PATH. The wrapper may in turn use ``#!/usr/bin/env node``, so passing only
    its absolute path is insufficient. Add the documented user-local runtime
    directories for this child without changing the application environment.
    """
    environment = _safe_local_env()
    home = Path(environment.get("HOME", str(Path.home())))
    executable_path = Path(executable)
    candidates = []
    if executable_path.is_absolute():
        candidates.append(str(executable_path.parent))
    candidates.extend(str(home / suffix) for suffix in (
        ".local/bin", ".local/npm-global/bin", ".npm-global/bin",
        ".volta/bin", ".bun/bin", ".asdf/shims",
    ))
    candidates.extend(environment.get("PATH", "").split(os.pathsep))
    entries = []
    for entry in candidates:
        if entry and entry not in entries:
            entries.append(entry)
    environment["PATH"] = os.pathsep.join(entries)
    return environment


def _openrouter_env(dotenv_path: Path | None = None, *, allow_dotenv: bool = True) -> dict[str, str]:
    """Expose only the one credential authorized for the MiMo helper.

    Desktop launchers do not normally inherit interactive shell variables, so
    the repository's ignored .env is accepted as an explicit local credential
    store. It is parsed as data, never sourced as shell code.

    allow_dotenv=False is what the desktop passes. The application imports its
    key through a dialog and hands it over deliberately, so a .env the dialog
    never saw must not be able to authorize a job: the run would succeed and
    then record a provenance snapshot naming a credential nobody chose. The
    command line keeps the fallback, which is the only place it was ever meant
    to serve.
    """
    environment = _safe_local_env()
    key = os.environ.get("OPENROUTER_API_KEY", "").strip()
    path = dotenv_path or ROOT / ".env"
    if not key and allow_dotenv and path.is_file():
        for raw_line in path.read_text(encoding="utf-8").splitlines():
            line = raw_line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            name, value = line.split("=", 1)
            if name.strip() != "OPENROUTER_API_KEY":
                continue
            value = value.strip()
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
                value = value[1:-1]
            key = value.strip()
            break
    if key:
        environment["OPENROUTER_API_KEY"] = key
    return environment


def _enter_process_group() -> None:
    """Isolate a UI-owned worker so cancellation reaches all of its children."""
    if os.name == "posix":
        os.setsid()


def _command_description(argv: Sequence[str], private_positions: set[int] | None = None) -> list[str]:
    private_positions = private_positions or set()
    return ["<private-file>" if i in private_positions else value for i, value in enumerate(argv)]


# whisper.cpp only accepts these exact --dtw preset names; passing anything
# else makes whisper-cli fail outright, so unknown models run without DTW
# instead of not at all.
_DTW_PRESETS = frozenset({
    "tiny", "tiny.en", "base", "base.en", "small", "small.en",
    "medium", "medium.en", "large.v1", "large.v2", "large.v3",
    "large.v3.turbo",
})


def _dtw_model_name(model_file_name: str) -> str | None:
    if not (model_file_name.startswith("ggml-") and model_file_name.endswith(".bin")):
        return None
    stem = model_file_name[len("ggml-"):-len(".bin")]
    stem = re.sub(r"-q\d+_\d+$", "", stem)
    preset = stem.replace("large-v", "large.v").replace("-turbo", ".turbo")
    return preset if preset in _DTW_PRESETS else None


def _whisper_thread_count() -> int:
    return max(1, os.cpu_count() or 4)


def whisper_vad_model() -> Path | None:
    """An opt-in whisper.cpp VAD model, or None.

    whisper-cli 1.8.6 has no ``--no-context`` flag — the library's
    ``params.no_context`` is never wired to an argument — but it does accept
    ``--max-context 0``, and ``whisper.cpp:7097`` skips history conditioning
    entirely when ``n_max_text_ctx`` is zero. That is the CLI-reachable form of
    ``condition_on_prev_text=False`` and it is always on here.

    Its VAD (``--vad``/``--vad-model``) is a different matter. Measured on this
    project's four benchmark tracks on 2026-08-04, Silero v6.2.0 rejects sung
    vocals over accompaniment almost completely: the coverage canary kept 1
    segment of 0.4 s at the default 0.50 threshold and 3 segments at 0.10.
    Whisper's own long-form pass is far better evidence for this material, so
    VAD stays **off** unless the operator names a model, and an unreadable or
    absent model degrades to no VAD rather than failing the job.
    """
    configured = os.environ.get("MUSIALIZER_WHISPER_VAD_MODEL", "").strip()
    if not configured:
        return None
    candidate = Path(configured).expanduser()
    if candidate.is_file():
        return candidate
    print(f"Ignoring MUSIALIZER_WHISPER_VAD_MODEL: {candidate} is not a file; "
          "continuing without voice-activity segmentation.", file=sys.stderr)
    return None


def whisper_request(
    audio: Path, *, whisper_bin: Path, model: Path, language: str,
    dtw_model: str | None, ffmpeg: str, output_prefix: Path,
    threads: int | None = None, vad_model: Path | None = None,
) -> tuple[list[str], list[str]]:
    wav = output_prefix.with_suffix(".16k.wav")
    decode = [
        ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "error", "-y",
        "-i", str(audio), "-vn", "-ac", "1", "-ar", "16000",
        "-c:a", "pcm_s16le", str(wav),
    ]
    whisper = [
        str(whisper_bin), "-f", str(wav), "--output-file", str(output_prefix),
        "--output-json", "-ojf", "-m", str(model), "-l", language,
        "-t", str(threads if threads is not None else _whisper_thread_count()),
        # No cross-segment text conditioning. A decoder that can quote its own
        # previous segment will, and on singing it loops: the canary repeated
        # one genuine line from 90.0 s to the end of the track and buried the
        # two outro lines behind it. With `-mc 0` that loop is gone and the
        # outro is transcribed at 90.6 s.
        "-mc", "0",
    ]
    if vad_model is not None:
        whisper.extend(["--vad", "-vm", str(vad_model)])
    if dtw_model:
        # whisper.cpp's CLI defaults flash attention on, while the library
        # explicitly disables its experimental DTW token timestamps whenever
        # flash attention is active. Asking for both therefore succeeds but
        # silently emits the ordinary heuristic token times. Make the selected
        # timing backend real rather than merely recording `dtw_model` in
        # provenance.
        whisper.extend(["--no-flash-attn", "--dtw", dtw_model])
    return decode, whisper


def _whisper_request_settings(
    *, language: str, dtw_model: str | None, model_sha256: str,
    vad_model: Path | None,
) -> dict[str, Any]:
    """The identity a cached Whisper lane has to match to be reused."""
    return {
        "language": language, "dtw_model": dtw_model,
        "model_sha256": model_sha256, "gpu_requested": True,
        "timing_backend": ("dtw_no_flash_attention" if dtw_model
                           else "token_onsets_flash_attention"),
        "word_timing": ("dtw_centers_v1" if dtw_model
                        else "token_onsets_v1"),
        "text_conditioning": WHISPER_TEXT_CONDITIONING,
        "vad_model_sha256": (None if vad_model is None
                             else sha256_file(vad_model)),
    }


def run_whisper(
    audio: Path, output: Path, *, audio_duration: float, whisper_bin: Path,
    model: Path, language: str = "en", dtw_model: str | None = None,
    ffmpeg: str = "ffmpeg", timeout: float = 3600.0,
    decode_timeout: float = 600.0, raw_output: Path | None = None,
    dry_run: bool = False, runner: Runner = subprocess.run,
) -> dict[str, Any]:
    audio_duration = duration(audio_duration)
    if not audio.is_file() or not whisper_bin.is_file() or not model.is_file():
        raise AnalysisValidationError("audio, Whisper executable, and model must be files")
    audio_sha = sha256_file(audio)
    model_sha = sha256_file(model)
    vad_model = whisper_vad_model()
    with tempfile.TemporaryDirectory(prefix="musializer-whisper-") as temporary:
        prefix = Path(temporary) / "transcription"
        decode, whisper = whisper_request(
            audio, whisper_bin=whisper_bin, model=model, language=language,
            dtw_model=dtw_model, ffmpeg=ffmpeg, output_prefix=prefix,
            vad_model=vad_model,
        )
        request = {
            "dry_run": dry_run,
            "audio_sha256": audio_sha,
            "model_sha256": model_sha,
            "language": language,
            "dtw_model": dtw_model,
            "text_conditioning": WHISPER_TEXT_CONDITIONING,
            "vad_model_sha256": (None if vad_model is None
                                 else sha256_file(vad_model)),
            "timeouts_seconds": {"decode": decode_timeout, "whisper": timeout},
            "decode_argv": _command_description(decode, {7, len(decode) - 1}),
            "whisper_argv": _command_description(whisper, {2, 4, whisper.index("-m") + 1}),
        }
        if dry_run:
            return request
        local_env = _safe_local_env()
        _run(decode, timeout=decode_timeout, env=local_env, runner=runner)
        _run(whisper, timeout=timeout, env=local_env, runner=runner)
        produced = Path(f"{prefix}.json")
        if not produced.is_file():
            raise RuntimeError("Whisper completed without producing its requested JSON file")
        raw = read_json(produced)
        normalized = normalize_whisper(
            raw, audio_sha256=audio_sha, audio_duration=audio_duration,
            model=model.name, prefer_dtw=bool(dtw_model),
        )
        normalized["provenance"]["adapter"] = "tools/external_analysis.py"
        normalized["provenance"]["adapter_version"] = ADAPTER_VERSION
        normalized["provenance"]["request_settings"] = _whisper_request_settings(
            language=language, dtw_model=dtw_model, model_sha256=model_sha,
            vad_model=vad_model,
        )
        normalized["provenance"]["generation"] = {
            "raw_whisper_sha256": canonical_sha256(raw),
        }
        atomic_write_json(raw_output or output.with_suffix(".whisper.raw.json"), raw)
        atomic_write_json(output, normalized)
        return normalized


def _reference_from_text(text: str, source: str) -> dict[str, Any]:
    if not text.strip():
        raise AnalysisValidationError("reference lyrics are empty")
    encoded = text.encode("utf-8", errors="replace")
    if len(encoded) > lyric_align.MAX_REFERENCE_BYTES:
        raise AnalysisValidationError("reference lyrics exceed the size bound")
    return {"source": source, "text": text,
            "sha256": hashlib.sha256(encoded).hexdigest()}


def discover_reference_lyrics(
    audio: Path, *, override: Path | None = None, ffprobe: str = "ffprobe",
    timeout: float = 60.0, runner: Runner = subprocess.run,
) -> dict[str, Any] | None:
    """Locate authored lyrics for a track without any network access.

    Priority: an explicit user-supplied file, a sibling
    ``<stem>.lyrics.txt``, then unsynchronized lyric tags embedded in the
    audio container (ID3 USLT and friends, surfaced by ffprobe as tags whose
    key contains "lyric"). Returns None when nothing usable exists. An
    explicit override that is empty or oversized is an error; problems with
    merely discovered sources are reported to stderr (the job log) and
    skipped so transcription can still run.
    """
    if override is not None:
        return _reference_from_text(
            override.read_text(encoding="utf-8"), f"file:{override.name}")
    sibling = audio.with_name(audio.stem + REFERENCE_SIBLING_SUFFIX)
    if sibling.is_file():
        try:
            return _reference_from_text(
                sibling.read_text(encoding="utf-8"), f"file:{sibling.name}")
        except (AnalysisValidationError, UnicodeDecodeError, OSError) as error:
            print(f"Ignoring sibling lyrics file: {error}", file=sys.stderr)
    probe = [
        ffprobe, "-v", "error", "-show_entries", "format_tags:stream_tags",
        "-of", "json", str(audio),
    ]
    try:
        completed = _run(probe, timeout=timeout, env=_safe_local_env(),
                         runner=runner, stdout_limit=_PROBE_STDOUT_LIMIT)
        payload = json.loads(completed.stdout or "{}")
    except (RuntimeError, json.JSONDecodeError):
        return None
    tag_sets = [payload.get("format", {}).get("tags", {})]
    for stream in payload.get("streams") or []:
        if isinstance(stream, dict):
            tag_sets.append(stream.get("tags", {}))
    candidates: list[tuple[str, str]] = []
    for tags in tag_sets:
        if not isinstance(tags, dict):
            continue
        for key, value in tags.items():
            if ("lyric" in key.lower() and isinstance(value, str)
                    and value.strip()):
                candidates.append((key, value))
    # Prefer English, then plain "lyrics"-style keys; the ordering must stay
    # deterministic when a container carries several lyric tags.
    candidates.sort(key=lambda item: (
        0 if item[0].lower().endswith("eng") else 1,
        0 if item[0].lower().startswith("lyric") else 1,
        item[0].lower(),
    ))
    for key, value in candidates:
        try:
            return _reference_from_text(value, f"embedded:{key}")
        except AnalysisValidationError as error:
            print(f"Ignoring embedded lyric tag {key}: {error}",
                  file=sys.stderr)
    return None


# The sync lane is no longer the timing authority. Since LT1 it is the
# *coarse proposal*: one of two independent views, kept because the review
# flags and the repeated-phrase abstention are both defined as disagreement
# between it and the anchor/block placement. Recording the role in the request
# identity keeps a lane written under the old authority from being reused as
# if it were still the answer.
LYRIC_SYNC_ROLE = "coarse_proposal"


def _sync_request_settings() -> dict[str, Any]:
    return {"aligner_version": lyric_align.ALIGNER_VERSION,
            "role": LYRIC_SYNC_ROLE}


def run_lyric_sync(
    source: Path, reference: dict[str, Any], output: Path,
) -> dict[str, Any]:
    """Align discovered reference lyrics to Whisper evidence, atomically."""
    evidence = read_json(source)
    if evidence.get("schema_version") != "musializer.lyric-timing/v1":
        raise AnalysisValidationError(
            "lyric sync requires musializer.lyric-timing/v1 evidence")
    audio = evidence.get("audio", {})
    audio_duration = duration(audio.get("duration_seconds"))
    document = lyric_align.sync_lyrics(
        reference["text"], evidence, audio_duration=audio_duration)
    document["audio"] = {
        "sha256": audio.get("sha256"),
        "duration_seconds": audio_duration,
    }
    document["reference"] = {
        "source": reference["source"],
        "sha256": reference["sha256"],
    }
    document["provenance"] = {
        "adapter": "tools/external_analysis.py",
        "adapter_version": ADAPTER_VERSION,
        "source_kind": "lyric_sync",
        "audio_sha256": audio.get("sha256"),
        "schema_version": LYRIC_SYNC_VERSION,
        "request_settings": _sync_request_settings(),
        "generation": {
            "whisper_sha256": sha256_file(source),
            "reference_sha256": reference["sha256"],
        },
    }
    atomic_write_json(output, document)
    return document


def _sync_cache_accepts(
    document: dict[str, Any], *, whisper_sha256: str,
    reference_sha256: str | None,
) -> bool:
    if document.get("aligner_version") != lyric_align.ALIGNER_VERSION:
        return False
    if (reference_sha256 is not None and
            document.get("reference", {}).get("sha256") != reference_sha256):
        return False
    provenance = document.get("provenance", {})
    generation = (provenance.get("generation", {})
                  if isinstance(provenance, dict) else {})
    if generation.get("whisper_sha256") != whisper_sha256:
        return False
    return _provenance_matches(
        document,
        adapter="tools/external_analysis.py",
        adapter_version=ADAPTER_VERSION,
        source_kind="lyric_sync",
        request_settings=_sync_request_settings(),
    )


def _validate_codex_review(raw: Any, source: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
    if not isinstance(raw, dict) or not isinstance(raw.get("lines"), list) or not isinstance(raw.get("notes"), list):
        raise AnalysisValidationError("Codex lyric review must contain lines and notes arrays")
    source_lines = source.get("lines")
    if not isinstance(source_lines, list):
        raise AnalysisValidationError("source lyric lane has no lines array")
    audio_duration = duration(source.get("audio", {}).get("duration_seconds"))
    previous_start = -1.0
    previous_first_index = -1
    cleaned: list[dict[str, Any]] = []
    for index, line in enumerate(raw["lines"]):
        if not isinstance(line, dict):
            raise AnalysisValidationError(f"review line {index} is not an object")
        indices = line.get("source_line_indices")
        if not isinstance(indices, list) or not indices or any(type(i) is not int for i in indices):
            raise AnalysisValidationError(f"review line {index} lacks source evidence")
        if len(set(indices)) != len(indices) or any(i < 0 or i >= len(source_lines) for i in indices):
            raise AnalysisValidationError(f"review line {index} cites an invalid source line")
        # A long Whisper segment may legitimately be split across several
        # display cues, so citation reuse is allowed, but citations must stay
        # chronological so the review cannot shuffle evidence.
        if min(indices) < previous_first_index:
            raise AnalysisValidationError("review citations must remain chronological")
        previous_first_index = min(indices)
        try:
            start = float(line["start_seconds"])
            end = float(line["end_seconds"])
            confidence = float(line["confidence"])
        except (KeyError, TypeError, ValueError) as error:
            raise AnalysisValidationError(f"review line {index} has invalid numbers") from error
        text = line.get("text")
        if not all(math.isfinite(value) for value in (start, end, confidence)) or not isinstance(text, str):
            raise AnalysisValidationError(f"review line {index} has non-finite values or non-text content")
        text = text.strip()
        envelope_start = min(float(source_lines[i]["start_seconds"]) for i in indices)
        envelope_end = max(float(source_lines[i]["end_seconds"]) for i in indices)
        if (not text or len(text) > REVIEW_TEXT_LIMIT or
            start < max(0.0, envelope_start - 0.25) or
            end > min(audio_duration, envelope_end + 0.25) or end <= start or
            end - start > REVIEW_DURATION_LIMIT_SECONDS or
            start < previous_start or not 0.0 <= confidence <= 1.0):
            raise AnalysisValidationError(f"review line {index} violates evidence/timing bounds")
        if type(line.get("uncertain")) is not bool:
            raise AnalysisValidationError(f"review line {index} lacks an uncertainty flag")
        previous_start = start
        cleaned.append({
            "start_seconds": start, "end_seconds": end, "text": text,
            "source_line_indices": indices, "confidence": confidence,
            "uncertain": line["uncertain"],
        })
    notes = [str(note).strip() for note in raw["notes"] if str(note).strip()]
    return cleaned, notes


def _line_midpoint_in_intervals(
    line: dict[str, Any], intervals: Sequence[tuple[float, float]],
) -> bool:
    midpoint = (float(line.get("start_seconds", 0.0)) +
                float(line.get("end_seconds", 0.0))) / 2.0
    return any(start <= midpoint <= end for start, end in intervals)


def codex_review_request(source: dict[str, Any]) -> str:
    prompt = LYRIC_PROMPT.read_text(encoding="utf-8")
    evidence = {
        "audio": source.get("audio"),
        "lines": source.get("lines", []),
        "words": source.get("words", []),
        # Detected repetition-loop hallucinations; the prompt instructs the
        # model to omit evidence inside these windows and note the omission.
        "suspected_hallucination_intervals": [
            {"start_seconds": start, "end_seconds": end}
            for start, end in lyric_align.flag_unreliable_intervals(
                source.get("lines") or [])
        ],
    }
    return f"{prompt}\n\nWhisper evidence JSON follows:\n{json.dumps(evidence, ensure_ascii=False)}\n"


def run_codex_review(
    lyrics: Path, output: Path, *, codex_bin: str = "codex",
    model: str | None = None, reasoning_effort: str | None = None,
    timeout: float = 600.0, dry_run: bool = False,
    runner: Runner = subprocess.run,
) -> dict[str, Any]:
    source = read_json(lyrics)
    if source.get("schema_version") != "musializer.lyric-timing/v1":
        raise AnalysisValidationError("Codex review input must be a lyric-timing/v1 lane")
    source_sha = sha256_file(lyrics)
    prompt_sha = sha256_file(LYRIC_PROMPT)
    argv = [
        codex_bin, "exec", "--ephemeral", "--ignore-user-config",
        "--sandbox", "read-only", "--output-schema", str(CODEX_OUTPUT_SCHEMA),
        "--skip-git-repo-check", "-C", "<isolated-workdir>",
        "-o", "<temporary-output>", "-",
    ]
    if model:
        argv[2:2] = ["--model", model]
    # `codex exec -c` sets a config key; the reasoning effort is one, and it is
    # the only way to state it per invocation. Inserted after the model so the
    # argv order is stable enough to assert on.
    if reasoning_effort:
        argv[2:2] = ["-c", f"model_reasoning_effort={reasoning_effort}"]
    request = {
        "dry_run": dry_run, "timeout_seconds": timeout,
        "source_sha256": source_sha, "prompt_sha256": prompt_sha,
        "model": model, "reasoning_effort": reasoning_effort, "argv": argv,
        "stdin": "<repository prompt plus private lyric evidence omitted>",
    }
    if dry_run:
        return request
    with tempfile.TemporaryDirectory(prefix="musializer-codex-") as temporary:
        result_path = Path(temporary) / "review.json"
        actual = [str(result_path) if value == "<temporary-output>" else
                  temporary if value == "<isolated-workdir>" else value for value in argv]
        diagnostic_sink = output.with_name(output.stem + ".diagnostic.log")
        _run(actual, timeout=timeout, stdin=codex_review_request(source),
             cwd=Path(temporary), env=_local_runtime_env(codex_bin), runner=runner,
             diagnostic_sink=diagnostic_sink)
        # A stale diagnostic from an earlier failed attempt would misdescribe
        # this successful run; drop it once the child has exited cleanly.
        diagnostic_sink.unlink(missing_ok=True)
        raw = read_json(result_path)
    lines, notes = _validate_codex_review(raw, source)
    lines = lyric_align.split_long_cues(lines, source.get("words") or [])
    source_lines = source.get("lines") or []
    unreliable = lyric_align.flag_unreliable_intervals(source_lines)
    cited: set[int] = set()
    for line in lines:
        cited.update(line["source_line_indices"])
    uncited_reliable = sum(
        1 for index, line in enumerate(source_lines)
        if index not in cited
        and not _line_midpoint_in_intervals(line, unreliable))
    reviewed = {
        "schema_version": LYRIC_REVIEW_VERSION,
        "lane": "lyric_review",
        "audio": source["audio"],
        "source": {
            "schema_version": source["schema_version"],
            "sha256": source_sha,
            "adapter": source.get("provenance", {}).get("adapter"),
        },
        "coverage": {
            "source_lines": len(source_lines),
            "cited_source_lines": len(cited),
            "uncited_reliable_source_lines": uncited_reliable,
            "suspected_hallucination_intervals": [
                {"start_seconds": start, "end_seconds": end}
                for start, end in unreliable
            ],
        },
        "provenance": {
            "adapter": "tools/external_analysis.py",
            "adapter_version": ADAPTER_VERSION,
            "source_kind": "codex_lyric_review",
            "prompt_version": LYRIC_PROMPT_VERSION,
            "prompt_sha256": prompt_sha,
            # §6: what `codex exec --model` was actually invoked with. An
            # absent flag is the documented `codex-default`, never a guess.
            "model": model or "codex-default",
            "request_settings": _codex_request_settings(reasoning_effort),
        },
        "lines": lines,
        "notes": notes,
    }
    atomic_write_json(output, reviewed)
    return reviewed


def import_mimo_export(export_path: Path, audio_path: Path, audio_duration: float) -> dict[str, Any]:
    """Extract assistant final text without copying embedded audio or reasoning."""
    value = read_json(export_path)
    texts: list[str] = []
    if isinstance(value, dict):
        items = value.get("items", {})
        iterable = items.values() if isinstance(items, dict) else items if isinstance(items, list) else []
        for item in iterable:
            data = item.get("data", {}) if isinstance(item, dict) else {}
            content = data.get("content", [])
            for part in content if isinstance(content, list) else []:
                if (isinstance(part, dict) and part.get("type") == "output_text" and
                    isinstance(part.get("text"), str) and part["text"].strip()):
                    texts.append(part["text"].strip())
        if not texts and isinstance(value.get("choices"), list):
            for choice in value["choices"]:
                content = choice.get("message", {}).get("content") if isinstance(choice, dict) else None
                if isinstance(content, str) and content.strip(): texts.append(content.strip())
    if not texts:
        raise AnalysisValidationError("MiMo export contains no assistant output_text")
    return {
        "schema_version": SEMANTIC_NOTES_VERSION,
        "lane": "semantic_interpretation_notes",
        "audio": {"sha256": sha256_file(audio_path), "duration_seconds": duration(audio_duration)},
        "provenance": {
            "adapter": "tools/external_analysis.py", "adapter_version": ADAPTER_VERSION,
            "source_kind": "mimo_openrouter_export", "source_sha256": sha256_file(export_path),
            "quantitative_values_available": False,
        },
        "text": "\n\n".join(texts),
    }


def _semantic_document(value: Any) -> dict[str, Any] | None:
    if value is None:
        return None
    if isinstance(value, dict) and value.get("schema_version") == "musializer.analysis-cache/v1":
        value = value.get("normalized")
    if isinstance(value, dict) and value.get("schema_version") == SEMANTIC_NOTES_VERSION:
        if not isinstance(value.get("text"), str) or not value["text"].strip():
            raise AnalysisValidationError("semantic notes are empty")
        return value
    if not isinstance(value, dict) or value.get("schema_version") != "musializer.semantic-score/v1":
        raise AnalysisValidationError("semantic input must be a semantic-score/v1 document or cache")
    if not isinstance(value.get("segments"), list) or not value["segments"]:
        raise AnalysisValidationError("semantic score has no segments")
    return value


def _source_ref(path: Path, document: dict[str, Any]) -> dict[str, Any]:
    return {
        "path": str(path), "sha256": sha256_file(path),
        "schema_version": document.get("schema_version"), "lane": document.get("lane"),
    }


def _frame_average(frames: list[dict[str, Any]], start: float, end: float) -> dict[str, float]:
    selected = [f for f in frames if start <= float(f.get("time_seconds", -1)) < end]
    if not selected:
        return {"rms": 0.0, "spectral_flux": 0.0, "onset_strength": 0.0}
    return {
        key: sum(float(frame.get(key, 0.0)) for frame in selected)/len(selected)
        for key in ("rms", "spectral_flux", "onset_strength")
    }


def _semantic_at(semantic: dict[str, Any] | None, time_seconds: float) -> dict[str, Any] | None:
    if semantic is None:
        return None
    if semantic.get("schema_version") == SEMANTIC_NOTES_VERSION:
        return {"summary": semantic["text"], "moods": [], "motion": [], "imagery": []}
    for segment in semantic["segments"]:
        if float(segment["start_seconds"]) <= time_seconds < float(segment["end_seconds"]):
            return segment
    return semantic["segments"][-1]


def _scene_for(features: dict[str, float], semantic: dict[str, Any] | None,
               lyric_count: int, section_index: int) -> tuple[str, list[dict[str, Any]]]:
    reasons: list[dict[str, Any]] = [{
        "source_lane": "measured_audio",
        "detail": f"rms={features['rms']:.3f}, flux={features['spectral_flux']:.3f}, onset={features['onset_strength']:.3f}",
    }]
    words: set[str] = set()
    energy = features["rms"]
    if semantic:
        semantic_words = (semantic.get("moods", []) + semantic.get("motion", []) +
                          semantic.get("imagery", []) + [semantic.get("summary", "")])
        words = {token.lower() for phrase in semantic_words for token in str(phrase).replace("-", " ").split()}
        energy = max(energy, float(semantic.get("energy", 0.0)))
        reasons.append({
            "source_lane": "semantic_interpretation",
            "detail": str(semantic.get("summary", ""))[:512],
        })
    keyword_scenes = (
        ({"cosmic", "stars", "celestial", "dream", "space"}, "constellation"),
        ({"lyric", "voice", "word", "vocal", "spoken"}, "cadence"),
        ({"arc", "woven", "textile", "narrative", "tapestry"}, "loom"),
        ({"organic", "growth", "forest", "creature", "earth"}, "terrarium"),
        ({"journey", "landscape", "terrain", "vast", "horizon"}, "atlas"),
        ({"mechanical", "drive", "tunnel", "kinetic", "industrial"}, "orbital"),
        ({"geometric", "mathematical", "recursive", "hypnotic", "ritual"}, "pentagram"),
        ({"psychedelic", "trippy", "nostalgic", "retro", "kaleidoscopic"}, "phosphor"),
        ({"playful", "cute", "whimsical", "cheeky", "cartoon", "mascot"}, "clawd"),
    )
    for keywords, scene in keyword_scenes:
        if words.intersection(keywords):
            return scene, reasons
    if lyric_count > 0 and energy < 0.48:
        return "ascii", reasons
    if energy > 0.72 or features["onset_strength"] > 0.55:
        return "pulse", reasons
    if features["spectral_flux"] > 0.42:
        return "orbital", reasons
    return ("spectrum" if section_index % 2 == 0 else "constellation"), reasons


def build_scene_plan(
    measured_path: Path, *, lyrics_path: Path | None = None,
    semantic_path: Path | None = None, minimum_section: float = 4.0,
    maximum_section: float = 24.0,
) -> dict[str, Any]:
    measured = read_json(measured_path)
    if measured.get("schema_version") != "musializer.measured-analysis/v1":
        raise AnalysisValidationError("measured input must be measured-analysis/v1")
    audio = measured.get("audio", {})
    audio_duration = duration(audio.get("duration_seconds"))
    audio_sha = audio.get("sha256")
    lyrics = read_json(lyrics_path) if lyrics_path else None
    semantic = _semantic_document(read_json(semantic_path)) if semantic_path else None
    for name, document in (("lyrics", lyrics), ("semantic", semantic)):
        if document and document.get("audio", {}).get("sha256") != audio_sha:
            raise AnalysisValidationError(f"{name} lane belongs to different audio")

    candidates: list[tuple[float, float, str]] = [(0.0, 1.0, "start"), (audio_duration, 1.0, "end")]
    for section in measured.get("summary", {}).get("sections", [])[1:]:
        candidates.append((float(section["start_seconds"]), 0.68, "measured_section"))
    if semantic and semantic.get("schema_version") == "musializer.semantic-score/v1":
        previous = semantic["segments"][0]
        for segment in semantic["segments"][1:]:
            change = abs(float(segment.get("energy", 0)) - float(previous.get("energy", 0)))
            change += abs(float(segment.get("tension", 0)) - float(previous.get("tension", 0)))*0.5
            candidates.append((float(segment["start_seconds"]), min(0.95, 0.55 + change*0.35), "semantic_change"))
            previous = segment
    lyric_lines = lyrics.get("lines", []) if isinstance(lyrics, dict) else []
    for previous, following in zip(lyric_lines, lyric_lines[1:]):
        gap = float(following["start_seconds"]) - float(previous["end_seconds"])
        if gap >= 2.5:
            candidates.append((float(following["start_seconds"]), min(0.8, 0.45 + gap/20.0), "lyric_reentry"))

    candidates.sort()
    merged: list[tuple[float, float, str]] = []
    for candidate in candidates:
        if merged and candidate[0] - merged[-1][0] < minimum_section and candidate[0] != audio_duration:
            if candidate[1] > merged[-1][1] and merged[-1][0] != 0.0:
                merged[-1] = candidate
            continue
        merged.append(candidate)
    if merged[-1][0] != audio_duration:
        merged.append((audio_duration, 1.0, "end"))

    boundaries: list[tuple[float, float, str]] = [merged[0]]
    for right in merged[1:]:
        left_time = boundaries[-1][0]
        while right[0] - left_time > maximum_section:
            forced = min(right[0], left_time + maximum_section)
            boundaries.append((forced, 0.35, "maximum_duration"))
            left_time = forced
        boundaries.append(right)

    frames = measured.get("frames", [])
    sections: list[dict[str, Any]] = []
    for index, (left, right) in enumerate(zip(boundaries, boundaries[1:])):
        start, end = left[0], right[0]
        features = _frame_average(frames, start, end)
        midpoint = (start + end)*0.5
        semantic_segment = _semantic_at(semantic, midpoint)
        lyric_count = sum(start <= float(line["start_seconds"]) < end for line in lyric_lines)
        scene, reasons = _scene_for(features, semantic_segment, lyric_count, index)
        reasons.append({"source_lane": left[2], "detail": "boundary evidence"})
        sections.append({
            "id": f"section-{index:04d}", "start_seconds": start,
            "end_seconds": end, "recommended_scene": scene,
            "transition_strength": 0.0 if index == 0 else left[1],
            "reasons": reasons,
        })
    sources = [_source_ref(measured_path, measured)]
    if lyrics_path and lyrics: sources.append(_source_ref(lyrics_path, lyrics))
    if semantic_path and semantic: sources.append(_source_ref(semantic_path, semantic))
    plan = {
        "schema_version": SCENE_PLAN_VERSION, "lane": "scene_plan",
        "audio": {"sha256": audio_sha, "duration_seconds": audio_duration},
        "sources": sources,
        "provenance": {
            "adapter": "tools/external_analysis.py", "adapter_version": ADAPTER_VERSION,
            "source_kind": "deterministic_section_planner",
            "request_settings": {"minimum_section": minimum_section, "maximum_section": maximum_section},
        },
        "sections": sections,
    }
    validate_scene_plan(plan)
    return plan


def validate_scene_plan(plan: dict[str, Any]) -> None:
    total = duration(plan.get("audio", {}).get("duration_seconds"))
    sections = plan.get("sections")
    if not isinstance(sections, list) or not sections:
        raise AnalysisValidationError("scene plan needs sections")
    cursor = 0.0
    for index, section in enumerate(sections):
        start = float(section.get("start_seconds", -1))
        end = float(section.get("end_seconds", -1))
        if abs(start - cursor) > 1e-6 or end <= start or end > total + 1e-6:
            raise AnalysisValidationError(f"scene section {index} breaks full-duration coverage")
        if section.get("recommended_scene") not in SCENES:
            raise AnalysisValidationError(f"scene section {index} has an unknown scene")
        cursor = end
    if abs(cursor - total) > 1e-6:
        raise AnalysisValidationError("scene plan does not cover complete audio")


def _stable_id(kind: str, index: int, start_ms: int, text: str = "") -> int:
    digest = hashlib.sha256(f"{kind}\0{index}\0{start_ms}\0{text}".encode("utf-8")).digest()
    return int.from_bytes(digest[:8], "big") or 1


def _b64(value: str) -> str:
    return base64.b64encode(value.encode("utf-8")).decode("ascii")


def build_bridge(
    plan: dict[str, Any], *, lyrics: dict[str, Any] | None = None,
    semantic: dict[str, Any] | None = None,
) -> str:
    validate_scene_plan(plan)
    audio = plan["audio"]
    lines = [BRIDGE_VERSION, f"AUDIO\t{audio['sha256']}\t{round(float(audio['duration_seconds'])*1000)}"]
    for index, lyric in enumerate((lyrics or {}).get("lines", [])):
        start = round(float(lyric["start_seconds"])*1000)
        end = round(float(lyric["end_seconds"])*1000)
        confidence = lyric.get("confidence")
        confidence_milli = -1 if confidence is None else round(float(confidence)*1000)
        flags = "uncertain" if lyric.get("uncertain") else "none"
        text = str(lyric["text"])
        lines.append(f"LYRIC\t{_stable_id('lyric', index, start, text)}\t{start}\t{end}\t{confidence_milli}\t{flags}\t{_b64(text)}")
    for index, section in enumerate(plan["sections"]):
        start = round(float(section["start_seconds"])*1000)
        end = round(float(section["end_seconds"])*1000)
        reason = json.dumps(section["reasons"], ensure_ascii=False, separators=(",", ":"))
        lines.append(f"SECTION\t{_stable_id('section', index, start)}\t{start}\t{end}\t{section['recommended_scene']}\t{round(float(section['transition_strength'])*1000)}\t{_b64(reason)}")
    if semantic and semantic.get("schema_version") == "musializer.semantic-score/v1":
        for index, cue in enumerate(semantic["segments"]):
            start = round(float(cue["start_seconds"])*1000)
            end = round(float(cue["end_seconds"])*1000)
            summary = str(cue.get("summary", ""))
            values = [
                round(float(cue.get("energy", 0))*1000),
                round(float(cue.get("tension", 0))*1000),
                round(float(cue.get("valence", 0))*1000),
                round(float(cue.get("confidence", 0))*1000),
            ]
            lines.append(f"SEMANTIC\t{_stable_id('semantic', index, start, summary)}\t{start}\t{end}\t" +
                         "\t".join(map(str, values)) + f"\t{_b64(summary)}")
    elif semantic and semantic.get("schema_version") == SEMANTIC_NOTES_VERSION:
        text = semantic["text"]
        lines.append(f"SEMANTIC_NOTE\t{_stable_id('semantic-note', 0, 0, text)}\t{_b64(text)}")
    result = "\n".join(lines) + "\n"
    parse_bridge(result)
    return result


def parse_bridge(value: str) -> list[list[str]]:
    rows = [line.split("\t") for line in value.splitlines()]
    if not rows or rows[0] != ["MUSIALIZER_BRIDGE", "1"]:
        raise AnalysisValidationError("invalid bridge header")
    expected = {"AUDIO": 3, "LYRIC": 7, "SECTION": 7, "SEMANTIC": 9, "SEMANTIC_NOTE": 3}
    if len(rows) < 2 or rows[1][0] != "AUDIO":
        raise AnalysisValidationError("bridge lacks AUDIO record")
    if len(rows[1][1]) != 64 or any(char not in "0123456789abcdef" for char in rows[1][1]):
        raise AnalysisValidationError("bridge AUDIO hash is invalid")
    audio_duration_ms = int(rows[1][2])
    if audio_duration_ms <= 0: raise AnalysisValidationError("bridge AUDIO duration is invalid")
    ids: set[int] = set()
    previous_time: dict[str, int] = {}
    for row in rows[1:]:
        if row[0] not in expected or len(row) != expected[row[0]]:
            raise AnalysisValidationError("invalid bridge record shape")
        if row[0] in {"LYRIC", "SECTION", "SEMANTIC"}:
            stable_id, start, end = int(row[1]), int(row[2]), int(row[3])
            if stable_id == 0 or stable_id in ids or start < 0 or end <= start or end > audio_duration_ms:
                raise AnalysisValidationError("bridge record id/timing is invalid")
            if start < previous_time.get(row[0], -1):
                raise AnalysisValidationError("bridge records are not time ordered")
            ids.add(stable_id); previous_time[row[0]] = start
            decoded = base64.b64decode(row[-1], validate=True)
            if len(decoded) > 1024*1024: raise AnalysisValidationError("bridge decoded field is too large")
            if row[0] == "LYRIC":
                confidence = int(row[4])
                if confidence < -1 or confidence > 1000 or row[5] not in {"none", "uncertain"}:
                    raise AnalysisValidationError("bridge lyric metadata is invalid")
            elif row[0] == "SECTION":
                if row[4] not in SCENES or not 0 <= int(row[5]) <= 1000:
                    raise AnalysisValidationError("bridge section metadata is invalid")
            else:
                energy, tension, valence, confidence = map(int, row[4:8])
                if not (0 <= energy <= 1000 and 0 <= tension <= 1000 and
                        -1000 <= valence <= 1000 and 0 <= confidence <= 1000):
                    raise AnalysisValidationError("bridge semantic metadata is invalid")
        elif row[0] == "SEMANTIC_NOTE":
            stable_id = int(row[1])
            decoded = base64.b64decode(row[2], validate=True)
            if stable_id == 0 or stable_id in ids or len(decoded) > 1024*1024:
                raise AnalysisValidationError("bridge semantic note is invalid")
            ids.add(stable_id)
    return rows


def atomic_write_text(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent,
                                     prefix=f".{path.name}.", suffix=".tmp", delete=False) as output:
        temporary = Path(output.name)
        output.write(value)
        output.flush()
        os.fsync(output.fileno())
    try:
        os.replace(temporary, path)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


# A probe seam, and the narrowest one that can prove what it has to prove.
#
# `tools/secret_canary_check.sh` has to show that a **local-only** job's helper
# never sees the provider credential, and "the boolean says absent" only checks
# one variable name — a key smuggled under another name would pass. Dumping the
# whole environment and scanning it for the sentinel catches that, and it is the
# child's own view rather than the parent's belief about it.
#
# Gated on `--dry-run`, because a dry run opens no socket and produces no
# artifact a user keeps. Inert when unset.
ENVIRONMENT_DUMP_VARIABLE = "MUSIALIZER_ASSIST_ENV_DUMP"

# The companion seam, and the reason there is one at all: `AssistSpec` builds the
# **production** argv and has no `--dry-run`, on purpose — a flag the application
# can pass is a flag a job can accidentally run under. But the canary gate has to
# exercise the real spawn path, credential decision included, without contacting
# a model. So the dry run is asked for through the environment instead: setting
# this to a path implies `--dry-run` and writes the report there rather than to a
# stdout the supervisor discards. Inert when unset.
DRY_RUN_REPORT_VARIABLE = "MUSIALIZER_ASSIST_DRY_RUN_REPORT"


def _dump_environment_for_probe() -> str | None:
    destination = os.environ.get(ENVIRONMENT_DUMP_VARIABLE, "").strip()
    if not destination:
        return None
    path = Path(destination)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(f"{name}={value}" for name, value in sorted(os.environ.items())),
        encoding="utf-8",
    )
    return str(path)


def read_execution_snapshot(path: Path | None) -> dict[str, Any] | None:
    """Reads the frozen route graph, or refuses it.

    A missing path is `None` and means "this run was not routed" — the command
    line, and every pre-P4 caller. A path that exists but is not a
    `musializer.assist-execution/v1` record is an error rather than a silent
    fall back to defaults: a job that believes it is routed and is not would
    record provenance nobody chose.
    """
    if path is None:
        return None
    if not path.is_file():
        raise AnalysisValidationError(f"execution snapshot {path} does not exist")
    if path.stat().st_size > EXECUTION_SNAPSHOT_BYTE_LIMIT:
        raise AnalysisValidationError("execution snapshot is implausibly large")
    document = read_json(path)
    if not isinstance(document, dict):
        raise AnalysisValidationError("execution snapshot must be a JSON object")
    if document.get("snapshot_schema") != EXECUTION_SNAPSHOT_SCHEMA:
        raise AnalysisValidationError(
            f"execution snapshot declares {document.get('snapshot_schema')!r}, "
            f"not {EXECUTION_SNAPSHOT_SCHEMA}")
    contracts = document.get("contracts")
    if not isinstance(contracts, list):
        raise AnalysisValidationError("execution snapshot has no contracts array")
    for entry in contracts:
        if not isinstance(entry, dict):
            raise AnalysisValidationError("execution snapshot contract is not an object")
        contract = entry.get("contract")
        route_type = entry.get("route_type")
        runtime_id = entry.get("runtime_id")
        model_id = entry.get("model_id")
        implemented = {
            "TC-MEASURED": route_type == "builtin" and runtime_id == "builtin-analyzer",
            "TC-COARSE": (route_type == "local-proc" and runtime_id == "whisper.cpp"
                          and model_id in (None, "", "whisper.cpp")),
            "TC-WORDING": route_type == "codex" and runtime_id == "codex",
            "TC-ALIGN": (route_type == "local-proc" and runtime_id == "mms-ctc"
                         and model_id in (None, "", "mms-ctc")),
            "TC-SEMANTIC": route_type == "openrouter" and runtime_id == "openrouter",
            "TC-PLAN": route_type == "builtin" and runtime_id == "builtin-planner",
            "TC-VERIFY": False,
        }.get(contract, False)
        if not implemented:
            raise AnalysisValidationError(
                f"execution snapshot asks {contract!r} to use "
                f"{route_type!r}/{runtime_id!r}, which this helper does not implement")
    return document


def execution_route(snapshot: dict[str, Any] | None, contract: str) -> dict[str, Any] | None:
    """One contract's row, or `None` when the run composed no such stage."""
    if not snapshot:
        return None
    for entry in snapshot.get("contracts", []):
        if isinstance(entry, dict) and entry.get("contract") == contract:
            return entry
    return None


def route_identity(snapshot: dict[str, Any] | None, contract: str) -> dict[str, Any] | None:
    """The identity a cached artifact for this contract has to match (§5 rule 7)."""
    entry = execution_route(snapshot, contract)
    if entry is None:
        return None
    return {field: entry.get(field) for field in EXECUTION_ROUTE_IDENTITY_FIELDS}


def execution_provenance(snapshot: dict[str, Any] | None,
                         contract: str) -> dict[str, Any] | None:
    """The per-contract identity every produced artifact carries (§6)."""
    identity = route_identity(snapshot, contract)
    if identity is None:
        return None
    return {
        "snapshot_schema": snapshot.get("snapshot_schema"),
        "settings_schema": snapshot.get("settings_schema"),
        "profile_id": snapshot.get("profile_id"),
        "resolved_at_utc": snapshot.get("resolved_at_utc"),
        "contract": contract,
        "route_identity": identity,
    }


def _execution_route_accepts(document: dict[str, Any],
                             identity: dict[str, Any] | None) -> bool:
    """Whether a cached artifact was produced by this contract's route.

    Split out from `_provenance_matches` because the two alignment lanes have
    their own acceptance functions with their own digests; this is the one
    clause they share (§5 rule 7). An unrouted run compares nothing, which is
    what keeps the command line's behaviour exactly as it was.
    """
    if identity is None:
        return True
    recorded = document.get("provenance")
    if not isinstance(recorded, dict):
        return False
    execution = recorded.get("execution")
    return (isinstance(execution, dict)
            and execution.get("route_identity") == identity)


def _attach_execution(path: Path, document: dict[str, Any],
                      execution: dict[str, Any] | None) -> dict[str, Any]:
    """Stamps a produced artifact with the route that produced it, on disk.

    Done after the lane wrote its file rather than inside every lane runner:
    the runners are also the command line's, and threading a snapshot through
    five signatures to reach one dict key would be five chances to forget one.
    Rewriting is safe because the artifact is this job's own and the digest the
    next stage takes is computed after this returns.
    """
    if execution is None:
        return document
    provenance = document.setdefault("provenance", {})
    if not isinstance(provenance, dict):
        return document
    if provenance.get("execution") == execution:
        return document
    provenance["execution"] = execution
    atomic_write_json(path, document)
    return document


def _cache_matches(
    path: Path,
    schema_version: str,
    audio_sha: str,
    *,
    accept: Callable[[dict[str, Any]], bool] | None = None,
) -> dict[str, Any] | None:
    if not path.is_file(): return None
    try:
        value = read_json(path)
    except (OSError, ValueError):
        return None
    if value.get("schema_version") != schema_version: return None
    document = value.get("normalized", {}) if schema_version == "musializer.analysis-cache/v1" else value
    if document.get("audio", {}).get("sha256") != audio_sha: return None
    if accept is not None and not accept(value): return None
    return value


def _provenance_matches(
    document: dict[str, Any], *, adapter: str, adapter_version: str,
    source_kind: str, request_settings: dict[str, Any] | None = None,
    model: str | None = None, prompt_version: str | None = None,
    prompt_sha256: str | None = None,
    execution_route: dict[str, Any] | None = None,
) -> bool:
    provenance = document.get("provenance")
    if not isinstance(provenance, dict): return False
    # §5 rule 7: cache acceptance compares route identity per contract. An
    # artifact produced by a different model, runtime or provider policy is
    # regenerated, never reused — the same rule LT1 applied to the localization
    # policy, keyed by contract instead of by lane.
    #
    # A routed run refuses an artifact that carries no identity at all, which
    # invalidates every cache written before this tranche. That is the correct
    # direction: an unknown route is not a matching route.
    if execution_route is not None:
        recorded = provenance.get("execution")
        if not isinstance(recorded, dict):
            return False
        if recorded.get("route_identity") != execution_route:
            return False
    if (provenance.get("adapter") != adapter or
            provenance.get("adapter_version") != adapter_version or
            provenance.get("source_kind") != source_kind):
        return False
    if request_settings is not None and provenance.get("request_settings") != request_settings:
        return False
    if model is not None and provenance.get("model") != model: return False
    if prompt_version is not None and provenance.get("prompt_version") != prompt_version:
        return False
    if prompt_sha256 is not None and provenance.get("prompt_sha256") != prompt_sha256:
        return False
    return True


def _measured_cache_accepts(document: dict[str, Any]) -> bool:
    settings = {
        "sample_rate": MEASURED_SAMPLE_RATE,
        "channels": MEASURED_CHANNELS,
        "window_size": MEASURED_WINDOW,
        "hop_size": MEASURED_HOP,
    }
    analysis = document.get("analysis")
    if not isinstance(analysis, dict): return False
    if analysis.get("analyzer_version") != MEASURED_ANALYZER_VERSION:
        return False
    if any(analysis.get(key) != value for key, value in settings.items()):
        return False
    return _provenance_matches(
        document,
        adapter="tools/analyze_audio.py",
        adapter_version=MEASURED_ANALYZER_VERSION,
        source_kind="offline_measured_analysis",
        request_settings=settings,
    )


def _whisper_cache_accepts(
    document: dict[str, Any], *, measured_duration: float,
    model: Path | None, execution_route: dict[str, Any] | None = None,
) -> bool:
    audio = document.get("audio", {})
    if (not isinstance(audio, dict) or
            audio.get("duration_seconds") != measured_duration):
        return False
    provenance = document.get("provenance", {})
    settings = provenance.get("request_settings", {}) if isinstance(provenance, dict) else {}
    if (not isinstance(settings, dict) or settings.get("language") != "en" or
            settings.get("gpu_requested") is not True):
        return False
    if model is not None:
        if not model.is_file(): return False
        # Assist deliberately uses flash-attention decoding plus bounded token
        # onsets. DTW remains an explicit lower-level diagnostic option because
        # disabling flash attention caused severe repetition loops on singing.
        expected_settings = _whisper_request_settings(
            language="en", dtw_model=None, model_sha256=sha256_file(model),
            vad_model=whisper_vad_model(),
        )
        if settings != expected_settings or provenance.get("model") != model.name:
            return False
    elif not isinstance(settings.get("model_sha256"), str):
        return False
    return _provenance_matches(
        document,
        adapter="tools/external_analysis.py",
        adapter_version=ADAPTER_VERSION,
        source_kind="whisper_import",
        execution_route=execution_route,
    )


def _review_cache_accepts(
    document: dict[str, Any], *, source_sha256: str, model: str | None,
    reasoning_effort: str | None = None,
    execution_route: dict[str, Any] | None = None,
) -> bool:
    prompt_sha = sha256_file(LYRIC_PROMPT)
    return (document.get("source", {}).get("sha256") == source_sha256 and
            _provenance_matches(
                document,
                adapter="tools/external_analysis.py",
                adapter_version=ADAPTER_VERSION,
                source_kind="codex_lyric_review",
                model=model or "codex-default",
                prompt_version=LYRIC_PROMPT_VERSION,
                prompt_sha256=prompt_sha,
                request_settings=_codex_request_settings(reasoning_effort),
                execution_route=execution_route,
            ))


def _codex_request_settings(reasoning_effort: str | None) -> dict[str, Any]:
    """The identity of a Codex review request.

    `reasoning_effort` joined it in tranche P4: it is a routing choice a user
    makes in the AI settings dialog, and a review produced at `low` is not the
    review `high` would have produced. A run that names no effort keeps the
    pre-P4 two-key dict byte-for-byte, so an existing cache stays valid.
    """
    settings: dict[str, Any] = {"sandbox": "read-only", "ephemeral": True}
    if reasoning_effort:
        settings["reasoning_effort"] = reasoning_effort
    return settings


def _semantic_route(route: dict[str, Any] | None) -> dict[str, Any]:
    """The TC-SEMANTIC route's model and provider constraints, as sent.

    Everything comes from the snapshot the application froze at Start. An
    unrouted run — the command line — falls back to the helper's own default
    model with no constraints, which is exactly what it did before tranche P4.
    """
    constraints = (route or {}).get("provider_constraints") or {}
    if not isinstance(constraints, dict):
        constraints = {}
    max_price = {
        name: constraints[key]
        for name, key in (("prompt", "max_price_prompt"),
                          ("completion", "max_price_completion"),
                          ("audio", "max_price_audio"))
        if isinstance(constraints.get(key), (int, float))
    }
    return {
        "model": (route or {}).get("model_id") or mimo_adapter.MODEL,
        "order": list(constraints.get("order") or []),
        "only": list(constraints.get("only") or []),
        "ignore": list(constraints.get("ignore") or []),
        # The two defaults below are the *helper's* pre-P4 behaviour, kept so an
        # unrouted command line is unchanged. A routed run always states both.
        "allow_fallbacks": bool(constraints.get("allow_fallbacks", True))
                           if route else True,
        "zdr_required": bool(constraints.get("zdr_required", False)),
        "max_price": max_price,
    }


def _mimo_request_identity(
    audio: Path, *, audio_sha: str, measured_duration: float, zdr: bool,
    semantic: dict[str, Any] | None = None,
) -> dict[str, Any]:
    semantic = semantic or _semantic_route(None)
    settings = mimo_adapter.request_settings(
        model=semantic["model"],
        audio_duration=measured_duration,
        audio_format=audio.suffix.lstrip(".").lower(),
        provider_order=semantic["order"],
        provider_only=semantic["only"],
        provider_ignore=semantic["ignore"],
        allow_fallbacks=semantic["allow_fallbacks"],
        zero_data_retention=zdr,
        max_price=semantic["max_price"],
    )
    return {"audio_sha256": audio_sha, **settings}


def _mimo_cache_accepts(
    envelope: dict[str, Any], *, request_identity: dict[str, Any],
    execution_route: dict[str, Any] | None = None,
) -> bool:
    normalized = envelope.get("normalized")
    return (isinstance(normalized, dict) and
            envelope.get("request") == request_identity and
            envelope.get("cache_key") == canonical_sha256(request_identity) and
            _provenance_matches(
                normalized,
                adapter="tools/mimo_openrouter.py",
                adapter_version=ADAPTER_VERSION,
                source_kind="mimo_openrouter",
                request_settings={
                    key: value for key, value in request_identity.items()
                    if key != "audio_sha256"
                },
                # The *requested* model, not the served one: a response that
                # was routed to a variant is still an answer to this route, and
                # comparing the observed id would regenerate on every reroute.
                model=None,
                prompt_version=mimo_adapter.PROMPT_VERSION,
                execution_route=execution_route,
            ) and
            normalized.get("provenance", {}).get("requested_model",
                                                 mimo_adapter.MODEL)
            == request_identity["model"])


# Discovery roots, most preferred first: the durable per-user install (its
# build is CUDA-enabled on this workstation), then the original /tmp setup,
# which is tmpfs and vanishes on reboot. MUSIALIZER_WHISPER_BIN and
# MUSIALIZER_WHISPER_MODEL always win over discovery.
def _whisper_installs() -> tuple[Path, ...]:
    return (
        Path.home() / ".local/share/musializer/whisper.cpp",
        Path("/tmp/music-visualizations-whisper-1.8.6"),
    )


# Model preference. Full large-v3 now leads when it is installed: the 2026-08-17
# Groyper regression recovered 17/18 authored lines in the coarse pass and
# 16/18 after conservative occurrence gating, versus turbo's 14/18 and 10/18.
# Turbo remains the fallback for smaller installations. The full model can be
# substantially slower on CPU-only systems, while an explicit override remains
# authoritative.
_WHISPER_MODEL_PREFERENCE = (
    "ggml-large-v3.bin",
    "ggml-large-v3-turbo.bin",
    "ggml-large-v3-q5_0.bin",
    "ggml-medium.en.bin",
)


def _default_whisper_paths() -> tuple[Path | None, Path | None]:
    binary = os.environ.get("MUSIALIZER_WHISPER_BIN")
    model = os.environ.get("MUSIALIZER_WHISPER_MODEL")
    installs = _whisper_installs()
    discovered_binary = next(
        (candidate for install in installs
         if (candidate := install / "build/bin/whisper-cli").is_file()),
        None,
    )
    # The best model anywhere beats a lesser model in a preferred install.
    discovered_model = next(
        (candidate for name in _WHISPER_MODEL_PREFERENCE
         for install in installs
         if (candidate := install / name).is_file()),
        None,
    )
    return (
        Path(binary) if binary else discovered_binary,
        Path(model) if model else discovered_model,
    )


def _default_alignment_python() -> Path | None:
    configured = os.environ.get("MUSIALIZER_ALIGN_PYTHON")
    candidates = (
        Path(configured).expanduser() if configured else None,
        Path.home() / ".local/share/musializer/lyrics-align/.venv/bin/python",
    )
    return next(
        (candidate for candidate in candidates
         if candidate is not None and candidate.is_file()
         and (os.name == "nt" or os.access(candidate, os.X_OK))),
        None,
    )


def _default_alignment_model() -> Path:
    torch_home = Path(os.environ.get("TORCH_HOME", Path.home() / ".cache/torch"))
    return torch_home / "hub/checkpoints/model.pt"


def _alignment_cache_accepts(
    document: dict[str, Any], *, source_sha256: str,
) -> bool:
    refinement = document.get("timing_refinement")
    return (
        isinstance(refinement, dict)
        and refinement.get("adapter") == "tools/force_align_lyrics.py"
        and refinement.get("alignment_version") == force_align_lyrics.ALIGNMENT_VERSION
        and refinement.get("model") == force_align_lyrics.MODEL_ID
        and refinement.get("source_sha256") == source_sha256
    )


def _anchor_block_cache_accepts(
    document: dict[str, Any], *, whisper_sha256: str, coarse_sha256: str,
    reference_sha256: str | None,
) -> bool:
    """Accept a cached anchor/block lane only under the same policy.

    The localization policy version is part of the identity on purpose: the
    LT1 tranche replaced what "located" means, so an artifact written by the
    per-cue Whisper-window path has to be regenerated rather than reused, even
    though its audio digest and schema still match.

    ``reference_sha256=None`` skips only the authored-text check, for the
    Scene-changes lane: it never discovers lyrics and never writes this
    artifact, so it has no authored digest to compare and is reusing evidence
    another run already validated against one.
    """
    refinement = document.get("timing_refinement")
    if not isinstance(refinement, dict):
        return False
    generation = document.get("generation")
    if not isinstance(generation, dict):
        return False
    return (
        document.get("localization_policy") == lyric_anchor_block.LOCALIZATION_POLICY
        and document.get("localization_policy_version")
        == lyric_anchor_block.LOCALIZATION_POLICY_VERSION
        and refinement.get("adapter") == "tools/anchor_block_align.py"
        and refinement.get("alignment_version")
        == lyric_anchor_block.ALIGNMENT_VERSION
        and refinement.get("model") == "torchaudio.pipelines.MMS_FA"
        and refinement.get("localization_policy")
        == lyric_anchor_block.LOCALIZATION_POLICY
        and refinement.get("localization_policy_version")
        == lyric_anchor_block.LOCALIZATION_POLICY_VERSION
        and generation.get("whisper_sha256") == whisper_sha256
        and generation.get("coarse_sha256") == coarse_sha256
        and (reference_sha256 is None
             or generation.get("reference_sha256") == reference_sha256)
        and isinstance(generation.get("reference_sha256"), str)
    )


def run_anchor_block_alignment(
    audio: Path, whisper: Path, reference_file: Path, coarse: Path,
    output: Path, *, audio_duration: float, align_python: Path, timeout: float,
    reference_sha256: str, reference_source: str = "",
    diagnostic_sink: Path | None = None, runner: Runner = subprocess.run,
) -> dict[str, Any]:
    """Locate every authored line acoustically, anchors first, blocks second."""
    command = [
        str(align_python), str(ANCHOR_BLOCK_ALIGNER), str(audio), str(whisper),
        str(output), "--duration", f"{audio_duration:.9f}",
        "--reference-file", str(reference_file), "--coarse", str(coarse),
    ]
    _run(command, timeout=timeout, env=_safe_local_env(), runner=runner,
         diagnostic_sink=diagnostic_sink)
    result = read_json(output)
    result["reference"] = {
        "source": reference_source,
        "sha256": reference_sha256,
    }
    result["generation"] = {
        "whisper_sha256": sha256_file(whisper),
        "coarse_sha256": sha256_file(coarse),
        "reference_sha256": reference_sha256,
    }
    result["provenance"] = {
        "adapter": "tools/external_analysis.py",
        "adapter_version": ADAPTER_VERSION,
        "source_kind": "lyric_anchor_block",
        "audio_sha256": result.get("audio", {}).get("sha256"),
        "schema_version": LYRIC_SYNC_VERSION,
        "request_settings": {
            "localization_policy": lyric_anchor_block.LOCALIZATION_POLICY,
            "localization_policy_version": (
                lyric_anchor_block.LOCALIZATION_POLICY_VERSION),
        },
        "generation": result["generation"],
    }
    if not _anchor_block_cache_accepts(
            result, whisper_sha256=result["generation"]["whisper_sha256"],
            coarse_sha256=result["generation"]["coarse_sha256"],
            reference_sha256=reference_sha256):
        raise RuntimeError("anchor/block aligner produced an invalid lyric lane")
    lyric_anchor_block.validate_full_coverage(
        result, lyric_anchor_block.alignable_lines(
            reference_file.read_text(encoding="utf-8")))
    atomic_write_json(output, result)
    return result


def run_forced_alignment(
    audio: Path, source: Path, output: Path, *, audio_duration: float,
    align_python: Path, timeout: float, runner: Runner = subprocess.run,
) -> dict[str, Any]:
    command = [
        str(align_python), str(FORCED_ALIGNER), str(audio), str(source), str(output),
        "--duration", f"{audio_duration:.9f}",
    ]
    _run(command, timeout=timeout, env=_safe_local_env(), runner=runner)
    result = read_json(output)
    if not _alignment_cache_accepts(result, source_sha256=sha256_file(source)):
        raise RuntimeError("forced aligner produced an invalid lyric lane")
    return result


def run_assist(
    audio: Path, output_dir: Path, *, audio_duration: float, mode: str,
    bridge_path: Path | None = None, whisper_bin: Path | None = None,
    whisper_model: Path | None = None, codex_bin: str = "codex",
    codex_model: str | None = None, align_python: Path | None = None,
    semantic_cache: Path | None = None,
    lyrics_file: Path | None = None,
    zdr: bool = False, external_timeout: float = 2400.0,
    allow_dotenv: bool = True,
    execution_snapshot: Path | None = None,
    codex_reasoning_effort: str | None = None,
    dry_run: bool = False, runner: Runner = subprocess.run,
) -> dict[str, Any]:
    """Run one complete, cache-aware UI action and emit JSON plus TSV bridge."""
    if mode not in {"lyrics", "sections", "mimo", "all"}:
        raise AnalysisValidationError("assist mode must be lyrics, sections, mimo, or all")
    audio_duration = duration(audio_duration)
    if not audio.is_file(): raise AnalysisValidationError("assist audio file does not exist")
    if external_timeout < 600 or not math.isfinite(external_timeout):
        raise AnalysisValidationError("assist external timeout must be at least 600 seconds")
    output_dir.mkdir(parents=True, exist_ok=True)
    audio_sha = sha256_file(audio)
    paths = {
        "measured": output_dir / "measured.json",
        "lyrics": output_dir / "lyrics.whisper.json",
        "sync": output_dir / "lyrics.sync.json",
        "reference": output_dir / "lyrics.reference.txt",
        "review": output_dir / "lyrics.review.json",
        "aligned": output_dir / "lyrics.aligned.json",
        "semantic": semantic_cache or output_dir / "semantic.cache.json",
        "plan": output_dir / "scene-plan.json",
        "bridge": bridge_path or output_dir / "analysis.bridge.tsv",
        "manifest": output_dir / "assist-manifest.json",
    }
    detected_bin, detected_model = _default_whisper_paths()
    whisper_bin = whisper_bin or detected_bin
    whisper_model = whisper_model or detected_model
    align_python = align_python or _default_alignment_python()

    # The frozen route graph (§6). Read once; never rewritten, and never
    # re-resolved — the snapshot the application wrote at Start is the whole of
    # what this run is allowed to do.
    snapshot = read_execution_snapshot(execution_snapshot)
    routes = {
        contract: route_identity(snapshot, contract)
        for contract in ("TC-MEASURED", "TC-COARSE", "TC-WORDING",
                         "TC-ALIGN", "TC-SEMANTIC", "TC-PLAN")
    }
    stamps = {
        contract: execution_provenance(snapshot, contract)
        for contract in routes
    }
    semantic_route = _semantic_route(execution_route(snapshot, "TC-SEMANTIC"))
    wording_route = execution_route(snapshot, "TC-WORDING") or {}
    # §5 rule 6: `Codex default` is a documented label, not a model id, so it
    # never reaches `codex exec --model`.
    routed_codex_model = wording_route.get("model_id")
    if routed_codex_model in (None, "", CODEX_DEFAULT_LABEL):
        routed_codex_model = None
    codex_model = routed_codex_model or codex_model
    codex_reasoning_effort = (wording_route.get("reasoning_effort")
                              or codex_reasoning_effort)
    # ZDR is the route's, when the route states one. `--zdr` remains the command
    # line's way to ask, and the two agree by OR rather than by override: a
    # constraint is never weakened here (§5 invariant 4).
    zdr = bool(zdr or semantic_route["zdr_required"])

    actions = ["measured", "plan", "bridge"]
    if mode in {"lyrics", "all"}:
        actions[1:1] = [
            "whisper", "lyric_sync_or_codex_review",
            "anchor_block_localization_or_ctc_forced_alignment"]
    if mode in {"mimo", "all"}: actions[1:1] = ["mimo_openrouter"]
    if dry_run:
        return {
            "dry_run": True, "mode": mode, "audio_sha256": audio_sha,
            "actions": actions, "paths": {key: str(value) for key, value in paths.items()},
            "whisper_configured": bool(whisper_bin and whisper_model),
            "forced_alignment_configured": bool(align_python),
            "external_timeout_seconds": external_timeout,
            "credentials": "environment only; omitted",
            "dotenv_fallback": allow_dotenv,
            # E7: membership, never the value. This is how a gate proves the
            # application's deliberate hand-off worked without a plaintext sink
            # to scan — a boolean and a fingerprint, the same pair §6 records.
            "openrouter_credential_present":
                bool(os.environ.get("OPENROUTER_API_KEY", "").strip()),
            "openrouter_credential_fingerprint": (
                hashlib.sha256(
                    os.environ["OPENROUTER_API_KEY"].strip().encode("utf-8")
                ).hexdigest()[:8]
                if os.environ.get("OPENROUTER_API_KEY", "").strip() else None),
            "execution_snapshot": snapshot,
            "environment_dump": _dump_environment_for_probe(),
            "resolved_routes": {
                "semantic_model": semantic_route["model"],
                "semantic_zdr": zdr,
                "semantic_provider": {
                    key: semantic_route[key]
                    for key in ("order", "only", "ignore", "allow_fallbacks",
                                "max_price")
                },
                "codex_model": codex_model or CODEX_DEFAULT_LABEL,
                "codex_reasoning_effort": codex_reasoning_effort,
                "whisper_bin": str(whisper_bin) if whisper_bin else None,
                "whisper_model": str(whisper_model) if whisper_model else None,
                "align_python": str(align_python) if align_python else None,
            },
        }

    cache_status: dict[str, str] = {}
    measured = _cache_matches(
        paths["measured"], "musializer.measured-analysis/v1", audio_sha,
        accept=_measured_cache_accepts,
    )
    if measured is None:
        _run(
            [sys.executable, str(ROOT / "tools/analyze_audio.py"), str(audio), str(paths["measured"])],
            timeout=external_timeout, env=_safe_local_env(), runner=runner,
        )
        measured = _cache_matches(
            paths["measured"], "musializer.measured-analysis/v1", audio_sha,
            accept=_measured_cache_accepts,
        )
        if measured is None: raise RuntimeError("measured analyzer produced an invalid cache")
        cache_status["measured"] = "generated"
    else: cache_status["measured"] = "reused"
    measured_duration = duration(measured.get("audio", {}).get("duration_seconds"))

    lyrics: dict[str, Any] | None = None
    lyrics_lane_path: Path | None = None
    if mode in {"lyrics", "all"}:
        whisper_lane = _cache_matches(
            paths["lyrics"], "musializer.lyric-timing/v1", audio_sha,
            accept=lambda value: _whisper_cache_accepts(
                value, measured_duration=measured_duration, model=whisper_model,
                execution_route=routes["TC-COARSE"],
            ),
        )
        if whisper_lane is None:
            if whisper_bin is None or whisper_model is None:
                raise AnalysisValidationError("GPU Whisper is not configured or autodetectable")
            whisper_lane = run_whisper(
                audio, paths["lyrics"], audio_duration=measured_duration,
                whisper_bin=whisper_bin, model=whisper_model,
                timeout=external_timeout, runner=runner,
            )
            whisper_lane = _attach_execution(
                paths["lyrics"], whisper_lane, stamps["TC-COARSE"])
            cache_status["lyrics"] = "generated"
        else: cache_status["lyrics"] = "reused"
        source_sha = sha256_file(paths["lyrics"])
        if align_python is None or not FORCED_ALIGNER.is_file():
            raise AnalysisValidationError(
                "MMS forced alignment is not configured; set "
                "MUSIALIZER_ALIGN_PYTHON or install the local lyrics-align runtime")
        reference = discover_reference_lyrics(
            audio, override=lyrics_file, runner=runner)
        if reference is not None:
            # Authored lyrics exist: display text is already decided, so the
            # question is only *where* each line is. Whisper stays evidence and
            # stops being authority — the sync lane becomes a coarse proposal
            # and the anchor/block localizer decides the times, so a line
            # Whisper missed or looped over still reaches the acoustic stage.
            if not ANCHOR_BLOCK_ALIGNER.is_file():
                raise AnalysisValidationError(
                    "the anchor/block localizer is missing from the support "
                    "bundle (tools/anchor_block_align.py)")
            lyrics = _cache_matches(
                paths["sync"], LYRIC_SYNC_VERSION, audio_sha,
                accept=lambda value: _sync_cache_accepts(
                    value, whisper_sha256=source_sha,
                    reference_sha256=reference["sha256"],
                ),
            )
            if lyrics is None:
                lyrics = run_lyric_sync(paths["lyrics"], reference, paths["sync"])
                cache_status["sync"] = "generated"
            else: cache_status["sync"] = "reused"

            # The aligner runs under its own interpreter, so the authored text
            # goes through a file rather than three ffprobe passes. It lives
            # beside the evidence it was aligned against, which is already
            # private to this job folder.
            atomic_write_text(paths["reference"], reference["text"])
            coarse_sha = sha256_file(paths["sync"])
            aligned = _cache_matches(
                paths["aligned"], LYRIC_SYNC_VERSION, audio_sha,
                accept=lambda value: (
                    _anchor_block_cache_accepts(
                        value, whisper_sha256=source_sha, coarse_sha256=coarse_sha,
                        reference_sha256=reference["sha256"])
                    and _execution_route_accepts(value, routes["TC-ALIGN"])),
            )
            if aligned is None:
                aligned = run_anchor_block_alignment(
                    audio, paths["lyrics"], paths["reference"], paths["sync"],
                    paths["aligned"], audio_duration=measured_duration,
                    align_python=align_python, timeout=external_timeout,
                    reference_sha256=reference["sha256"],
                    reference_source=reference["source"],
                    diagnostic_sink=output_dir / "lyrics.aligned.diagnostic.txt",
                    runner=runner,
                )
                aligned = _attach_execution(
                    paths["aligned"], aligned, stamps["TC-ALIGN"])
                cache_status["alignment"] = "generated"
            else:
                cache_status["alignment"] = "reused"
            lyrics = aligned
            lyrics_lane_path = paths["aligned"]
        else:
            lyrics = _cache_matches(
                paths["review"], LYRIC_REVIEW_VERSION, audio_sha,
                accept=lambda value: _review_cache_accepts(
                    value, source_sha256=source_sha, model=codex_model,
                    reasoning_effort=codex_reasoning_effort,
                    execution_route=routes["TC-WORDING"],
                ),
            )
            if lyrics is None:
                lyrics = run_codex_review(
                    paths["lyrics"], paths["review"], codex_bin=codex_bin,
                    model=codex_model, reasoning_effort=codex_reasoning_effort,
                    timeout=external_timeout, runner=runner,
                )
                lyrics = _attach_execution(
                    paths["review"], lyrics, stamps["TC-WORDING"])
                cache_status["review"] = "generated"
            else: cache_status["review"] = "reused"

            # No authored text: there is nothing to localize globally, because
            # the transcription *is* the text. The per-cue MMS refinement is
            # still the right tool for that lane and is unchanged.
            timing_source_sha = sha256_file(paths["review"])
            aligned = _cache_matches(
                paths["aligned"], lyrics["schema_version"], audio_sha,
                accept=lambda value: (
                    _alignment_cache_accepts(value, source_sha256=timing_source_sha)
                    and _execution_route_accepts(value, routes["TC-ALIGN"])),
            )
            if aligned is None:
                aligned = run_forced_alignment(
                    audio, paths["review"], paths["aligned"],
                    audio_duration=measured_duration, align_python=align_python,
                    timeout=external_timeout, runner=runner,
                )
                aligned = _attach_execution(
                    paths["aligned"], aligned, stamps["TC-ALIGN"])
                cache_status["alignment"] = "generated"
            else:
                cache_status["alignment"] = "reused"
            lyrics = aligned
            lyrics_lane_path = paths["aligned"]
    elif mode == "sections":
        # Scene changes may use already-established local lyric evidence, but
        # never trigger lyric generation and never inherit a semantic lane.
        whisper_lane = _cache_matches(
            paths["lyrics"], "musializer.lyric-timing/v1", audio_sha,
            accept=lambda value: _whisper_cache_accepts(
                value, measured_duration=measured_duration, model=whisper_model,
            ),
        )
        if whisper_lane is not None:
            source_sha = sha256_file(paths["lyrics"])
            timing_source_path: Path | None = None
            lyrics = _cache_matches(
                paths["sync"], LYRIC_SYNC_VERSION, audio_sha,
                accept=lambda value: _sync_cache_accepts(
                    value, whisper_sha256=source_sha, reference_sha256=None,
                ),
            )
            if lyrics is not None:
                timing_source_path = paths["sync"]
            else:
                lyrics = _cache_matches(
                    paths["review"], LYRIC_REVIEW_VERSION, audio_sha,
                    accept=lambda value: _review_cache_accepts(
                        value, source_sha256=source_sha, model=codex_model,
                    ),
                )
                if lyrics is not None:
                    timing_source_path = paths["review"]
            if lyrics is not None and timing_source_path is not None:
                # Either acoustic lane may have produced the aligned artifact:
                # the authored path writes an anchor/block lane over the sync
                # proposal, the transcription path a per-cue refinement of the
                # review lane. Accept whichever this workspace holds, and fall
                # back to the coarse lane rather than to nothing.
                aligned = _cache_matches(
                    paths["aligned"], lyrics["schema_version"], audio_sha,
                    accept=lambda value: (
                        _alignment_cache_accepts(
                            value, source_sha256=sha256_file(timing_source_path))
                        or _anchor_block_cache_accepts(
                            value, whisper_sha256=source_sha,
                            coarse_sha256=sha256_file(timing_source_path),
                            reference_sha256=None)),
                )
                if aligned is not None:
                    lyrics = aligned
                    lyrics_lane_path = paths["aligned"]
                else:
                    lyrics_lane_path = timing_source_path

    semantic: dict[str, Any] | None = None
    if mode in {"mimo", "all"}:
        request_identity = _mimo_request_identity(
            audio, audio_sha=audio_sha, measured_duration=measured_duration, zdr=zdr,
            semantic=semantic_route,
        )
        accepts = lambda value: _mimo_cache_accepts(
            value, request_identity=request_identity,
            execution_route=routes["TC-SEMANTIC"],
        )
        envelope = _cache_matches(
            paths["semantic"], "musializer.analysis-cache/v1", audio_sha,
            accept=accepts,
        )
        if envelope is None:
            command = [
                sys.executable, str(ROOT / "tools/mimo_openrouter.py"), str(audio),
                str(paths["semantic"]), "--duration", f"{measured_duration:.9f}",
                "--model", semantic_route["model"],
            ]
            for slug in semantic_route["order"]: command.extend(["--provider", slug])
            for slug in semantic_route["only"]: command.extend(["--only", slug])
            for slug in semantic_route["ignore"]: command.extend(["--ignore", slug])
            for name, value in sorted(semantic_route["max_price"].items()):
                command.extend([f"--max-price-{name}", f"{value:.9g}"])
            if not semantic_route["allow_fallbacks"]: command.append("--no-fallbacks")
            if zdr: command.append("--zdr")
            _run(command, timeout=external_timeout,
                 env=_openrouter_env(allow_dotenv=allow_dotenv), runner=runner)
            envelope = _cache_matches(
                paths["semantic"], "musializer.analysis-cache/v1", audio_sha,
                accept=lambda value: (
                    _mimo_cache_accepts(value, request_identity=request_identity)),
            )
            if envelope is None: raise RuntimeError("MiMo helper produced an invalid cache")
            # The envelope wraps the artifact, so the stamp goes on the
            # normalized document inside it rather than on the wrapper.
            normalized = envelope.get("normalized")
            if isinstance(normalized, dict) and stamps["TC-SEMANTIC"] is not None:
                normalized.setdefault("provenance", {})["execution"] = \
                    stamps["TC-SEMANTIC"]
                atomic_write_json(paths["semantic"], envelope)
            cache_status["semantic"] = "generated"
        else: cache_status["semantic"] = "reused"
        semantic = _semantic_document(envelope)

    plan = build_scene_plan(
        paths["measured"], lyrics_path=lyrics_lane_path if lyrics else None,
        semantic_path=paths["semantic"] if semantic else None,
    )
    atomic_write_json(paths["plan"], plan)
    atomic_write_text(paths["bridge"], build_bridge(plan, lyrics=lyrics, semantic=semantic))
    manifest = build_assist_manifest(
        mode=mode, audio_sha=audio_sha, measured_duration=measured_duration,
        cache_status=cache_status, paths=paths, plan=plan, lyrics=lyrics, semantic=semantic,
        execution_snapshot=observe_execution(
            snapshot,
            whisper=whisper_lane if mode in {"lyrics", "all", "sections"} else None,
            review=lyrics if (lyrics or {}).get("lane") == "lyric_review" else None,
            aligned=lyrics,
            semantic=semantic,
        ),
    )
    atomic_write_json(paths["manifest"], manifest)
    return manifest


def observe_execution(
    snapshot: dict[str, Any] | None, *,
    whisper: dict[str, Any] | None = None,
    review: dict[str, Any] | None = None,
    aligned: dict[str, Any] | None = None,
    semantic: dict[str, Any] | None = None,
) -> dict[str, Any] | None:
    """The snapshot with every `model_id` replaced by what actually ran (§6).

    This is the whole point of the field: "`model_id` is observed, not
    inferred". For OpenRouter that is the `model` the response reported, which
    can differ from the requested slug when a variant is served; for Codex it is
    what `--model` was invoked with; for the local lanes it is the model file
    the runner used. A stage that produced nothing this run keeps the resolved
    identity, because there is nothing to have observed.

    The file the application wrote is **not** touched. Re-resolving is what §5
    invariant 3 forbids; recording what happened is what §6 requires, and those
    are different operations on different copies.
    """
    if not snapshot:
        return None
    observed = json.loads(json.dumps(snapshot))
    sources = {
        "TC-COARSE": whisper,
        "TC-WORDING": review,
        "TC-ALIGN": aligned,
        "TC-SEMANTIC": semantic,
    }
    for entry in observed.get("contracts", []):
        if not isinstance(entry, dict):
            continue
        document = sources.get(entry.get("contract"))
        if not isinstance(document, dict):
            continue
        provenance = document.get("provenance")
        provenance = provenance if isinstance(provenance, dict) else {}
        # The acoustic lanes record their model under `timing_refinement`
        # rather than under `provenance`, because that block is the aligner's
        # own identity. Both are consulted, provenance first, so a lane that
        # gains a `provenance.model` later does not silently keep reporting the
        # older one.
        refinement = document.get("timing_refinement")
        if isinstance(refinement, dict):
            provenance = {**refinement, **{k: v for k, v in provenance.items()
                                           if v is not None}}
        model = provenance.get("model")
        if isinstance(model, str) and model:
            entry["model_id"] = model
        served = provenance.get("provider")
        if isinstance(served, str) and served:
            entry["provider_served"] = served
        for field, key in (("prompt_version", "prompt_version"),
                           ("prompt_sha256", "prompt_sha256"),
                           ("schema_version", "schema_version")):
            value = provenance.get(key)
            if isinstance(value, str) and value:
                entry[field] = value
        if entry.get("schema_version") is None:
            declared = document.get("schema_version")
            if isinstance(declared, str) and declared:
                entry["schema_version"] = declared
    return observed


def build_assist_manifest(*, mode: str, audio_sha: str, measured_duration: float,
                          cache_status: dict[str, Any], paths: dict[str, Path],
                          plan: dict[str, Any], lyrics: dict[str, Any] | None,
                          semantic: dict[str, Any] | None,
                          execution_snapshot: dict[str, Any] | None = None,
                          ) -> dict[str, Any]:
    """`assist-manifest.json`, as the Assist panel reads it.

    Pure, so the lane rules below can be tested without a run: the panel decides
    what to show from these keys alone, and one of them being present when it
    should not be is a whole-surface defect (review LT1-R, R1).
    """
    lyric_lane = lyrics.get("lane") if lyrics else None
    manifest: dict[str, Any] = {
        "schema_version": "musializer.assist-manifest/v1", "mode": mode,
        "audio": {"sha256": audio_sha, "duration_seconds": measured_duration},
        "cache_status": cache_status,
        "artifacts": {key: str(value) for key, value in paths.items() if key != "manifest"},
        "provenance_streams": [
            "measured_audio", *( ["lyrics", lyric_lane] if lyrics else [] ),
            *( [semantic.get("lane")] if semantic else [] ), "scene_plan",
        ],
        "lyric_source": (lyrics.get("reference", {}).get("source")
                         if lyric_lane == "lyric_sync" else None),
        "result_counts": {
            "lyrics": len(lyrics.get("lines", [])) if lyrics else 0,
            "lyrics_unmatched": len(lyrics.get("unmatched", [])) if lyrics else 0,
            # Additive: unresolved lines and review flags are the LT1 review
            # surface. They are never cues, so they cannot reach the bridge.
            #
            # Written **only** when this run had a lyrics lane (review LT1-R,
            # R1). The job folder is keyed by audio rather than by mode, so a
            # `--mode sections` run lands in the same directory a previous full
            # run left `lyrics.aligned.json` in; a `"lyrics_unresolved": 0` in
            # that manifest is an LT1 marker, and the reader then found the old
            # document beside it and drew the *previous* run's flags. Zero is a
            # claim about a lane, and a run without the lane has no standing to
            # make it.
            **({"lyrics_unresolved": len(lyrics.get("unresolved", [])),
                "lyrics_review_flags": (len(lyrics.get("review_flags", [])) +
                                         len(lyrics.get("performed_candidates", []))),
                "lyrics_performed_candidates": len(
                    lyrics.get("performed_candidates", []))}
               if lyrics else {}),
            "sections": len(plan.get("sections", [])),
            "semantics": len(semantic.get("segments", [])) if semantic else 0,
        },
    }
    # Same rule for the localization block, and the same reason: it is the third
    # LT1 marker, so a run without a lyrics lane must not carry the key at all
    # rather than carry it as null (review LT1-R, R1).
    if lyrics and lyrics.get("localization_policy"):
        manifest["lyric_localization"] = {
            "policy": lyrics.get("localization_policy"),
            "policy_version": lyrics.get("localization_policy_version"),
        }
    elif lyrics:
        manifest["lyric_localization"] = None
    if lyrics and lyric_lane == "lyric_sync":
        reference = lyrics.get("reference", {})
        statistics = lyrics.get("statistics", {})
        manifest["lyric_reference"] = {
            "source": reference.get("source"),
            "sha256": reference.get("sha256"),
            "alignable_lines": statistics.get("reference_lines"),
        }
    # The frozen route graph, with the model ids this run observed (§6). Absent
    # rather than null for an unrouted run: "this job carried no route graph"
    # and "its routes were empty" are different answers.
    if execution_snapshot is not None:
        manifest["execution_snapshot"] = execution_snapshot
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    whisper = sub.add_parser("whisper", help="run configured GPU whisper.cpp and normalize timings")
    whisper.add_argument("audio", type=Path); whisper.add_argument("output", type=Path)
    whisper.add_argument("--duration", type=float, required=True)
    whisper.add_argument("--whisper-bin", type=Path, default=os.environ.get("MUSIALIZER_WHISPER_BIN"))
    whisper.add_argument("--model", type=Path, default=os.environ.get("MUSIALIZER_WHISPER_MODEL"))
    whisper.add_argument("--language", default="en"); whisper.add_argument("--dtw-model")
    whisper.add_argument("--ffmpeg", default="ffmpeg"); whisper.add_argument("--timeout", type=float, default=3600)
    whisper.add_argument("--decode-timeout", type=float, default=600); whisper.add_argument("--raw-output", type=Path)
    whisper.add_argument("--dry-run", action="store_true"); whisper.add_argument("--request-dump", type=Path)

    sync = sub.add_parser("sync-lyrics", help="deterministically align known lyrics to Whisper evidence")
    sync.add_argument("lyrics", type=Path); sync.add_argument("reference", type=Path)
    sync.add_argument("output", type=Path)

    clean = sub.add_parser("clean-lyrics", help="run evidence-preserving Codex lyric review")
    clean.add_argument("lyrics", type=Path); clean.add_argument("output", type=Path)
    clean.add_argument("--codex-bin", default="codex"); clean.add_argument("--model")
    clean.add_argument("--timeout", type=float, default=600); clean.add_argument("--dry-run", action="store_true")
    clean.add_argument("--request-dump", type=Path)

    mimo = sub.add_parser("import-mimo", help="extract final notes from an existing OpenRouter Chat export")
    mimo.add_argument("export", type=Path); mimo.add_argument("audio", type=Path); mimo.add_argument("output", type=Path)
    mimo.add_argument("--duration", type=float, required=True)

    plan_parser = sub.add_parser("plan", help="derive deterministic scene-switch sections")
    plan_parser.add_argument("measured", type=Path); plan_parser.add_argument("output", type=Path)
    plan_parser.add_argument("--lyrics", type=Path); plan_parser.add_argument("--semantic", type=Path)
    plan_parser.add_argument("--minimum-section", type=float, default=4); plan_parser.add_argument("--maximum-section", type=float, default=24)
    plan_parser.add_argument("--bridge", type=Path)

    assist = sub.add_parser("assist", help="cache-aware one-shot orchestration for a UI action")
    assist.add_argument("audio", type=Path); assist.add_argument("output_dir", type=Path)
    assist.add_argument("--duration", type=float, required=True)
    assist.add_argument("--mode", choices=("lyrics", "sections", "mimo", "all"), required=True)
    assist.add_argument("--bridge", type=Path); assist.add_argument("--whisper-bin", type=Path)
    assist.add_argument("--whisper-model", type=Path); assist.add_argument("--codex-bin", default="codex")
    assist.add_argument("--codex-model")
    assist.add_argument("--codex-reasoning-effort",
                        choices=("minimal", "low", "medium", "high"))
    assist.add_argument("--execution-snapshot", type=Path,
                        help="the musializer.assist-execution/v1 record the "
                             "application froze at Start")
    assist.add_argument("--align-python", type=Path)
    assist.add_argument("--semantic-cache", type=Path)
    assist.add_argument("--lyrics-file", type=Path)
    assist.add_argument("--zdr", action="store_true"); assist.add_argument("--timeout", type=float, default=2400)
    assist.add_argument("--no-dotenv", action="store_true",
                        help="refuse the repository .env credential fallback (the desktop path)")
    assist.add_argument("--new-process-group", action="store_true", help=argparse.SUPPRESS)
    assist.add_argument("--dry-run", action="store_true"); assist.add_argument("--request-dump", type=Path)

    args = parser.parse_args(argv)
    try:
        if args.command == "whisper":
            if args.whisper_bin is None or args.model is None:
                raise AnalysisValidationError("configure --whisper-bin and --model (or MUSIALIZER_WHISPER_BIN/MODEL)")
            result = run_whisper(
                args.audio, args.output, audio_duration=args.duration,
                whisper_bin=args.whisper_bin, model=args.model, language=args.language,
                dtw_model=args.dtw_model, ffmpeg=args.ffmpeg, timeout=args.timeout,
                decode_timeout=args.decode_timeout, raw_output=args.raw_output,
                dry_run=args.dry_run,
            )
            if args.dry_run:
                if args.request_dump: atomic_write_json(args.request_dump, result)
                else: print(json.dumps(result, indent=2))
        elif args.command == "clean-lyrics":
            result = run_codex_review(
                args.lyrics, args.output, codex_bin=args.codex_bin,
                model=args.model, timeout=args.timeout, dry_run=args.dry_run,
            )
            if args.dry_run:
                if args.request_dump: atomic_write_json(args.request_dump, result)
                else: print(json.dumps(result, indent=2))
        elif args.command == "sync-lyrics":
            reference = _reference_from_text(
                args.reference.read_text(encoding="utf-8"),
                f"file:{args.reference.name}")
            run_lyric_sync(args.lyrics, reference, args.output)
        elif args.command == "import-mimo":
            atomic_write_json(args.output, import_mimo_export(args.export, args.audio, args.duration))
        elif args.command == "assist":
            if args.new_process_group:
                _enter_process_group()
            probe_report = os.environ.get(DRY_RUN_REPORT_VARIABLE, "").strip()
            if probe_report:
                args.dry_run = True
                args.request_dump = Path(probe_report)
            result = run_assist(
                args.audio, args.output_dir, audio_duration=args.duration, mode=args.mode,
                bridge_path=args.bridge, whisper_bin=args.whisper_bin,
                whisper_model=args.whisper_model, codex_bin=args.codex_bin,
                codex_model=args.codex_model, align_python=args.align_python,
                semantic_cache=args.semantic_cache,
                lyrics_file=args.lyrics_file,
                zdr=args.zdr, external_timeout=args.timeout,
                allow_dotenv=not args.no_dotenv,
                execution_snapshot=args.execution_snapshot,
                codex_reasoning_effort=args.codex_reasoning_effort,
                dry_run=args.dry_run,
            )
            if args.dry_run:
                if args.request_dump: atomic_write_json(args.request_dump, result)
                else: print(json.dumps(result, indent=2))
            else:
                counts = result["result_counts"]
                sync_note = ""
                if result.get("lyric_source"):
                    sync_note = (
                        f" Lyric timing was synchronized to {result['lyric_source']}"
                        f" ({counts.get('lyrics_unmatched', 0)} reference lines"
                        " found no timing).")
                print(
                    "External analysis completed: "
                    f"{counts['lyrics']} lyric cues, "
                    f"{counts['sections']} scene sections, and "
                    f"{counts['semantics']} semantic cues."
                    f"{sync_note} "
                    "The manifest, bridge, and evidence remain in the job folder.",
                    file=sys.stderr,
                )
        else:
            plan = build_scene_plan(
                args.measured, lyrics_path=args.lyrics, semantic_path=args.semantic,
                minimum_section=args.minimum_section, maximum_section=args.maximum_section,
            )
            atomic_write_json(args.output, plan)
            if args.bridge:
                lyrics = read_json(args.lyrics) if args.lyrics else None
                semantic = _semantic_document(read_json(args.semantic)) if args.semantic else None
                atomic_write_text(args.bridge, build_bridge(plan, lyrics=lyrics, semantic=semantic))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"External analysis failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
