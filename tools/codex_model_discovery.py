#!/usr/bin/env python3
"""Codex `model/list` catalog discovery (AP2-c).

Investigation, recorded because it is not obvious from `codex --help`: the
installed Codex CLI (`codex-cli 0.146.0`) has no scriptable "list models"
subcommand anywhere in its top-level or `exec` help. The `model/list` method
docs/ASSIST_PROVIDER_CONTRACTS.md and
https://developers.openai.com/codex/app-server/ describe only exists on the
`codex app-server` JSON-RPC surface: newline-delimited JSON-RPC 2.0 over
stdio (confirmed against `codex app-server generate-json-schema`, whose `v2`
schema bundle emits `ModelListParams`/`ModelListResponse`, and against a live
`initialize` + `model/list` round trip during development of this module).
There is no lighter-weight, connectionless way to ask an installed Codex what
models it offers -- discovery here necessarily means spawning a short-lived
app-server process.

What this module does NOT do, by design:

- It never calls `codex login`, `codex logout`, or anything that mutates
  Codex's own auth state.
- It never opens `$CODEX_HOME/auth.json` or any file under `$CODEX_HOME`
  itself. Whatever the spawned `codex app-server` process reads to answer
  `model/list` is that process's own business, exactly as it would be for
  any other authenticated Codex invocation this project already shells out
  to (`external_analysis.run_codex_review`); this module adds no new file
  access of its own. `tests/test_provider_discovery.py` proves this
  dynamically: it points `$CODEX_HOME` at a directory holding a sentinel
  `auth.json`, patches `builtins.open`, runs a full discovery round trip
  against a stub `codex` process, and asserts nothing in this process ever
  opened that path.
- It always terminates the process it spawns, on every exit path (success,
  protocol error, timeout, or exception), via try/finally.

Per docs/ASSIST_PROVIDER_CONTRACTS.md section 5, rule 6 ("Codex discovery
failure preserves `Codex default`. Never a guessed model id."): whenever
discovery is unsupported (old Codex, no `model/list` method) or fails for any
other reason (binary missing, no response in time, malformed JSON-RPC), the
result is exactly `DEFAULT_LABEL` with an empty model list -- never a guess.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Mapping, Optional, Sequence

import atomic_cache

SCHEMA_VERSION = "musializer.codex-model-catalog/v1"
DEFAULT_LABEL = "Codex default"
CACHE_FILENAME = "codex-models-v1.json"

# A well-behaved catalog is a couple dozen entries; this is a sanity bound,
# not a product limit, and exists so a malformed/adversarial response cannot
# make the cache unbounded.
MAX_MODEL_COUNT = 500

_SENSITIVE_MARKERS = ("KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH")

Popen = Callable[..., "subprocess.Popen[bytes]"]


class DiscoveryError(RuntimeError):
    """A `model/list` response did not have the shape this module trusts."""


@dataclass
class DiscoveryResult:
    supported: bool
    models: list[dict[str, Any]] = field(default_factory=list)
    default_label: str = DEFAULT_LABEL
    error: Optional[str] = None


def _safe_environ(environ: Mapping[str, str]) -> dict[str, str]:
    return {key: value for key, value in environ.items()
            if not any(marker in key.upper() for marker in _SENSITIVE_MARKERS)}


class _AppServerSession:
    """A minimal newline-delimited JSON-RPC 2.0 client for one short call."""

    def __init__(self, argv: Sequence[str], *, popen: Popen, environ: Mapping[str, str]):
        self._proc = popen(
            list(argv), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, env=dict(environ),
        )
        self._queue: "queue.Queue[str]" = queue.Queue()
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()

    def _read_loop(self) -> None:
        stdout = self._proc.stdout
        if stdout is None:
            return
        try:
            for line in iter(stdout.readline, b""):
                self._queue.put(line.decode("utf-8", "replace"))
        except (OSError, ValueError):
            pass

    def send(self, obj: dict[str, Any]) -> None:
        stdin = self._proc.stdin
        if stdin is None:
            raise DiscoveryError("app-server process has no stdin pipe")
        stdin.write((json.dumps(obj) + "\n").encode("utf-8"))
        stdin.flush()

    def wait_for_id(self, wanted_id: int, deadline: float) -> Optional[dict[str, Any]]:
        # Notifications (e.g. remoteControl/status/changed) can arrive
        # between our request and its reply; skip anything not carrying the
        # id we are waiting for rather than treating it as the answer.
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return None
            try:
                line = self._queue.get(timeout=remaining)
            except queue.Empty:
                return None
            line = line.strip()
            if not line:
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(message, dict) and message.get("id") == wanted_id:
                return message

    def close(self) -> None:
        proc = self._proc
        try:
            if proc.poll() is None:
                try:
                    if proc.stdin:
                        proc.stdin.close()
                except OSError:
                    pass
                proc.terminate()
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=2)
            else:
                proc.wait()
        finally:
            # Popen leaves stdout/stderr pipes open after terminate()/kill();
            # close them explicitly so a caller doing many short-lived
            # discovery calls (a picker refresh, a retry loop) does not leak
            # file descriptors.
            for stream in (proc.stdout, proc.stderr):
                if stream is not None:
                    try:
                        stream.close()
                    except OSError:
                        pass


def _reasoning_efforts(value: Any) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        return []
    out = []
    for item in value:
        if isinstance(item, dict) and isinstance(item.get("reasoningEffort"), str):
            out.append({
                "reasoning_effort": item["reasoningEffort"],
                "description": item.get("description") if isinstance(item.get("description"), str) else None,
            })
    return out


def _string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item for item in value if isinstance(item, str)]


def _normalize_models(result: Any) -> list[dict[str, Any]]:
    if not isinstance(result, dict) or not isinstance(result.get("data"), list):
        raise DiscoveryError("model/list result missing a data array")
    data = result["data"]
    if len(data) > MAX_MODEL_COUNT:
        raise DiscoveryError(f"model/list reported more than {MAX_MODEL_COUNT} models")
    models: list[dict[str, Any]] = []
    seen_ids: set[str] = set()
    for entry in data:
        if not isinstance(entry, dict):
            raise DiscoveryError("model/list entry is not an object")
        model_id = entry.get("id")
        if not isinstance(model_id, str) or not model_id:
            raise DiscoveryError("model/list entry missing a string id")
        if model_id in seen_ids:
            raise DiscoveryError(f"model/list reported duplicate id {model_id!r}")
        seen_ids.add(model_id)
        models.append({
            "id": model_id,
            "model": entry.get("model") if isinstance(entry.get("model"), str) else model_id,
            "display_name": entry.get("displayName") if isinstance(entry.get("displayName"), str) else model_id,
            "description": entry.get("description") if isinstance(entry.get("description"), str) else None,
            "is_default": entry.get("isDefault") if isinstance(entry.get("isDefault"), bool) else False,
            "hidden": entry.get("hidden") if isinstance(entry.get("hidden"), bool) else False,
            "default_reasoning_effort": (entry.get("defaultReasoningEffort")
                                          if isinstance(entry.get("defaultReasoningEffort"), str) else None),
            "supported_reasoning_efforts": _reasoning_efforts(entry.get("supportedReasoningEfforts")),
            "input_modalities": _string_list(entry.get("inputModalities")),
        })
    return models


def discover_models(*, codex_bin: str | Sequence[str] = "codex", timeout: float = 8.0,
                     popen: Popen = subprocess.Popen, environ: Optional[Mapping[str, str]] = None,
                     client_name: str = "musializer-doctor",
                     client_version: str = "0") -> DiscoveryResult:
    """Ask an installed Codex for its model catalog via `app-server`.

    Always returns a result; never raises. `supported=False` covers every
    failure mode (binary missing, old Codex with no `model/list`, timeout,
    malformed response) uniformly, because the caller's only correct
    reaction to any of them is the same: fall back to `Codex default`.
    """
    argv_prefix = [codex_bin] if isinstance(codex_bin, str) else list(codex_bin)
    argv = [*argv_prefix, "app-server"]
    safe_environ = _safe_environ(environ if environ is not None else os.environ)
    deadline = time.monotonic() + timeout

    try:
        session = _AppServerSession(argv, popen=popen, environ=safe_environ)
    except (OSError, subprocess.SubprocessError) as error:
        return DiscoveryResult(False, error=f"could not start codex app-server: {error}")

    try:
        try:
            session.send({
                "id": 1, "method": "initialize",
                "params": {"clientInfo": {"name": client_name, "version": client_version}},
            })
        except (OSError, DiscoveryError) as error:
            return DiscoveryResult(False, error=f"could not write to codex app-server: {error}")

        init_response = session.wait_for_id(1, deadline)
        if init_response is None:
            return DiscoveryResult(False, error="codex app-server did not respond to initialize in time")
        if "error" in init_response:
            return DiscoveryResult(False, error=f"initialize failed: {init_response['error']}")

        try:
            session.send({"id": 2, "method": "model/list", "params": {}})
        except OSError as error:
            return DiscoveryResult(False, error=f"could not write model/list request: {error}")

        list_response = session.wait_for_id(2, deadline)
        if list_response is None:
            return DiscoveryResult(False, error="codex app-server did not respond to model/list in time")
        if "error" in list_response:
            # -32601 Method not found is exactly the "old Codex" case
            # docs/ASSIST_PROVIDER_CONTRACTS.md section 5 rule 6 names.
            # Every other JSON-RPC error gets the same fallback: never guess.
            error = list_response["error"]
            message = error.get("message") if isinstance(error, dict) else str(error)
            return DiscoveryResult(False, error=f"model/list unsupported or refused: {message}")

        try:
            models = _normalize_models(list_response.get("result"))
        except DiscoveryError as error:
            return DiscoveryResult(False, error=f"malformed model/list response: {error}")

        return DiscoveryResult(True, models=models)
    finally:
        session.close()


def cache_path(environ: Optional[Mapping[str, str]] = None) -> Path:
    return atomic_cache.cache_dir(environ) / CACHE_FILENAME


def refresh_and_cache(*, codex_bin: str | Sequence[str] = "codex", timeout: float = 8.0,
                       popen: Popen = subprocess.Popen, environ: Optional[Mapping[str, str]] = None,
                       cache_file: Optional[Path] = None) -> DiscoveryResult:
    """Run discovery and, only on full success, replace the cached catalog.

    A failed or unsupported discovery never touches the cache file: a stale
    but valid catalog from a previous, working Codex install must survive a
    later downgrade or a transient app-server failure (the same "prior valid
    cache stays intact" rule AP2-d applies to the OpenRouter catalog).
    """
    result = discover_models(codex_bin=codex_bin, timeout=timeout, popen=popen, environ=environ)
    if result.supported:
        path = cache_file if cache_file is not None else cache_path(environ)
        document = {
            "schema_version": SCHEMA_VERSION,
            "source": "codex app-server model/list",
            "fetched_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "codex_bin": codex_bin if isinstance(codex_bin, str) else " ".join(codex_bin),
            "model_count": len(result.models),
            "models": result.models,
        }
        atomic_cache.write_json_atomic(path, document)
    return result


def read_cache(cache_file: Optional[Path] = None,
                environ: Optional[Mapping[str, str]] = None) -> Optional[dict[str, Any]]:
    path = cache_file if cache_file is not None else cache_path(environ)
    document = atomic_cache.read_json(path)
    if document is None or document.get("schema_version") != SCHEMA_VERSION:
        return None
    if not isinstance(document.get("models"), list):
        return None
    return document


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    refresh_cmd = sub.add_parser("refresh", help="ask codex app-server for its model list")
    # `--codex-bin` exists because `PATH` is not a reliable answer to "where is
    # codex" in a GUI process: one started from a desktop entry inherits the
    # session manager's minimal PATH, not the login shell's. The dialog resolves
    # the binary itself (`musializer_runtime::assist::discover`) and hands the
    # answer down, rather than making this tool repeat a search that already
    # failed in the parent.
    refresh_cmd.add_argument("--codex-bin", default="codex",
                              help="path to the codex executable (default: found on PATH)")
    refresh_cmd.add_argument("--timeout", type=float, default=8.0)
    refresh_cmd.add_argument("--cache-dir", type=Path, help="override $XDG_CACHE_HOME/musializer")
    refresh_cmd.add_argument("--json", action="store_true")

    show_cmd = sub.add_parser("show", help="print the current cache, if any")
    show_cmd.add_argument("--cache-dir", type=Path)

    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    cache_file = (args.cache_dir / CACHE_FILENAME) if args.cache_dir else None

    if args.command == "refresh":
        result = refresh_and_cache(codex_bin=args.codex_bin, timeout=args.timeout,
                                    cache_file=cache_file)
        if args.json:
            print(json.dumps({"supported": result.supported, "error": result.error,
                              "model_count": len(result.models)},
                              ensure_ascii=False, indent=2))
        elif result.supported:
            print(f"cached {len(result.models)} models to "
                  f"{cache_file if cache_file else cache_path()}")
        else:
            # A failed discovery leaves the prior valid cache alone, which is
            # why this is a message rather than a deletion.
            print(f"discovery failed: {result.error}")
        return 0 if result.supported else 1

    if args.command == "show":
        document = read_cache(cache_file)
        if document is None:
            print("no valid cache")
            return 1
        print(json.dumps(document, ensure_ascii=False, indent=2))
        return 0

    return 2  # pragma: no cover - argparse enforces a valid subcommand


if __name__ == "__main__":
    raise SystemExit(main())
