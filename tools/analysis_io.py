"""Shared, dependency-free I/O helpers for optional analysis adapters."""

from __future__ import annotations

import hashlib
import json
import os
import tempfile
from pathlib import Path
from typing import Any


ADAPTER_VERSION = "1"


class AnalysisValidationError(ValueError):
    """An imported or model-produced analysis document is not usable."""


def audio_hash(value: Any) -> str:
    if not isinstance(value, str) or len(value) != 64:
        raise AnalysisValidationError("audio SHA-256 must be 64 lowercase hexadecimal characters")
    try:
        int(value, 16)
    except ValueError as error:
        raise AnalysisValidationError("audio SHA-256 must be 64 lowercase hexadecimal characters") from error
    if value != value.lower():
        raise AnalysisValidationError("audio SHA-256 must be 64 lowercase hexadecimal characters")
    return value


def sha256_file(path: str | os.PathLike[str], chunk_size: int = 1024 * 1024) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        while chunk := source.read(chunk_size):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sha256(value: Any) -> str:
    encoded = json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def read_json(path: str | os.PathLike[str]) -> Any:
    with open(path, "r", encoding="utf-8") as source:
        return json.load(source)


def atomic_write_json(path: str | os.PathLike[str], value: Any) -> None:
    """Durably replace *path* after the complete JSON document is serialized.

    The temporary file is created beside the destination, so os.replace is an
    atomic same-filesystem operation. Any existing valid cache remains intact
    if serialization or writing fails.
    """

    destination = Path(path)
    destination.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2)
    fd, temporary = tempfile.mkstemp(
        prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            output.write(payload)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, destination)
        try:
            directory_fd = os.open(destination.parent, os.O_RDONLY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
        except OSError:
            # Some platforms/filesystems do not support syncing directories.
            pass
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def number(value: Any, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise AnalysisValidationError(f"{field} must be a finite number")
    result = float(value)
    if not (float("-inf") < result < float("inf")):
        raise AnalysisValidationError(f"{field} must be a finite number")
    return result


def duration(value: Any) -> float:
    result = number(value, "audio duration")
    if result <= 0:
        raise AnalysisValidationError("audio duration must be greater than zero")
    return result


def clamp(value: Any, low: float, high: float, field: str) -> float:
    return max(low, min(high, number(value, field)))


def text(value: Any, field: str) -> str:
    if not isinstance(value, str):
        raise AnalysisValidationError(f"{field} must be a string")
    return value.strip()


def string_list(value: Any, field: str) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise AnalysisValidationError(f"{field} must be an array of strings")
    return [text(item, field) for item in value if text(item, field)]


# A child step's own reason, kept where its parent reads it (docs/TRAPS.md, "A
# helper's own reason dies at `_run`"): `external_analysis._run` reports a
# failed child as "python3 exited with code 1", and what the child knew about
# why is lost unless it writes it down. One small document per step, beside
# the step's output; the parent deletes it before the step runs and reads it,
# for its schema, its step and its audio, only when the step failed.
STEP_FAILURE_SCHEMA = "musializer.step-failed/v1"
STEP_FAILURE_MAX_BYTES = 64 * 1024


def step_failure_path(output: str | os.PathLike[str]) -> Path:
    """Where the step that writes ``output`` leaves its reason."""
    output = Path(output)
    return output.with_name(f"{output.stem}.failed.json")


def write_step_failure(path: str | os.PathLike[str], *, step: str, message: str,
                       audio: str | os.PathLike[str] | None = None) -> None:
    """Best effort: a failure to write the reason must not hide the failure."""
    import time
    digest = None
    if audio is not None:
        try:
            digest = sha256_file(audio)
        except OSError:
            digest = None
    try:
        atomic_write_json(path, {
            "schema_version": STEP_FAILURE_SCHEMA, "step": step,
            "failed_ns": time.time_ns(), "audio_sha256": digest,
            "message": " ".join(message.split()),
        })
    except OSError:
        pass


def read_step_failure(path: str | os.PathLike[str], *, step: str,
                      audio_sha256: str | None) -> str | None:
    """The step's own sentence, or None when there is none for this audio."""
    try:
        if Path(path).stat().st_size > STEP_FAILURE_MAX_BYTES:
            return None
        value = read_json(path)
    except (OSError, ValueError):
        return None
    if (not isinstance(value, dict) or value.get("schema_version") != STEP_FAILURE_SCHEMA
            or value.get("step") != step or not isinstance(value.get("message"), str)):
        return None
    recorded = value.get("audio_sha256")
    if audio_sha256 is not None and recorded is not None and recorded != audio_sha256:
        return None
    return " ".join(value["message"].split())[:300] or None
