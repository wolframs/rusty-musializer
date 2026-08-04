#!/usr/bin/env python3
"""Shared cache-directory resolution and atomic JSON writes.

`tools/provider_catalog.py` (AP2-d, OpenRouter's model catalog) and
`tools/codex_model_discovery.py` (AP2-c, Codex's `model/list`) both cache
small, non-secret catalog documents under the user's cache directory, and
both need the guarantee `docs/ASSIST_PROVIDER_CONTRACTS.md` section 3 spells
out for `credentials.json`: write to a sibling temp file, `fsync`, then
`rename` -- never truncate a file in place, so a crash mid-write can never
leave a half-written document at the real path. Kept in one place so the two
catalogs (and any future one) share the exact same write path rather than
two hand-rolled copies that could drift.

This module has no network code and no knowledge of either catalog's schema;
it only resolves the directory and performs the atomic write/read of JSON.
"""

from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path
from typing import Any, Mapping, Optional


def cache_dir(environ: Optional[Mapping[str, str]] = None) -> Path:
    """Resolve `$XDG_CACHE_HOME/musializer`, falling back to `~/.cache/musializer`."""
    environ = environ if environ is not None else os.environ
    base = environ.get("XDG_CACHE_HOME", "").strip()
    root = Path(base).expanduser() if base else Path.home() / ".cache"
    return root / "musializer"


def write_json_atomic(path: Path, document: Mapping[str, Any]) -> None:
    """Write `document` to `path` atomically. Never leaves a partial file at `path`."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(path.parent))
    tmp_path = Path(tmp_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(document, handle, ensure_ascii=False, sort_keys=True, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp_path, path)
    except BaseException:
        try:
            tmp_path.unlink()
        except OSError:
            pass
        raise


def read_json(path: Path) -> Optional[dict[str, Any]]:
    """Read a JSON object from `path`. Returns None for anything short of a
    well-formed JSON object -- missing file, truncated/corrupt bytes, or a
    top-level value that isn't an object -- rather than raising, so a caller
    can treat "no usable cache" uniformly whether the file is absent or
    damaged."""
    try:
        raw = path.read_bytes()
    except OSError:
        return None
    try:
        document = json.loads(raw)
    except json.JSONDecodeError:
        return None
    return document if isinstance(document, dict) else None
