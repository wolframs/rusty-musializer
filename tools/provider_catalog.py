#!/usr/bin/env python3
"""OpenRouter model catalog cache (AP2-d).

Fetches `GET https://openrouter.ai/api/v1/models` -- a public, unauthenticated
endpoint; no API key is sent or needed -- normalizes the response to a bounded
allowlist of fields, and writes it under `$XDG_CACHE_HOME/musializer/`
atomically (`tools/atomic_cache.py`). This is the Python-side half of AP2-d;
the Rust side (`ui/preferences.rs`'s `catalog.*` settings, not this module's
concern) decides *when* to call `refresh()` and *how* to filter a picker from
what lands in the cache file.

Refusal is whole-document: a byte-size cap, a model-count cap, a required
`id`/`name` on every entry, and a duplicate-id check all guard the *shape* of
the fetch, and any violation refuses the entire refresh rather than silently
dropping the offending rows. A refused refresh never touches the cache file,
so a prior valid catalog survives a bad fetch untouched -- this is the same
rule `codex_model_discovery.refresh_and_cache` applies to the Codex catalog.

Per docs/ASSIST_PROVIDER_CONTRACTS.md E6: "Catalog strings are untrusted
display data and never become paths or shell fragments." Every string field
kept here is stored as an opaque JSON string and this module never uses one
to build a path, a shell command, or an import -- callers must keep that
invariant too.
"""

from __future__ import annotations

import argparse
import json
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping, Optional, Sequence

import atomic_cache

SCHEMA_VERSION = "musializer.openrouter-catalog/v1"
DEFAULT_URL = "https://openrouter.ai/api/v1/models"
CACHE_FILENAME = "openrouter-models-v1.json"

# Bounds on the *fetched* response, checked before any field-level parsing.
# The live catalog is ~530 KiB / ~340 models at the time this was written;
# both caps leave generous headroom for organic catalog growth while still
# refusing an obviously wrong or adversarial response.
MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_MODEL_COUNT = 2000
# Defends against a pathologically long description bloating the cache; not
# a claim that every real description is this short.
MAX_STRING_FIELD_LENGTH = 4000

Fetcher = Callable[[str, float], bytes]


class CatalogValidationError(ValueError):
    """The fetched catalog does not have a shape this module trusts."""


@dataclass
class RefreshResult:
    ok: bool
    document: Optional[dict[str, Any]]
    error: Optional[str] = None


def _bounded_str(value: Any) -> Optional[str]:
    if not isinstance(value, str):
        return None
    return value[:MAX_STRING_FIELD_LENGTH]


def _string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item[:MAX_STRING_FIELD_LENGTH] for item in value if isinstance(item, str)]


def _price(value: Any) -> Optional[str]:
    # OpenRouter prices are decimal strings ("0.000002"), never numbers.
    # Keep the exact string (JSON has no fixed-point type and a float
    # round-trip would corrupt it) but only after confirming it parses as a
    # number, so a stray non-numeric value cannot ride along as if it were a
    # price.
    if not isinstance(value, str):
        return None
    try:
        float(value)
    except ValueError:
        return None
    return value[:64]


def _optional_number(value: Any) -> Optional[float]:
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def _optional_bool(value: Any) -> Optional[bool]:
    return value if isinstance(value, bool) else None


def normalize_model(entry: Any) -> dict[str, Any]:
    """Normalize one catalog entry to the bounded allowlist, or refuse it.

    Required-identity failures (not an object, missing/invalid `id` or
    `name`) raise, refusing the whole catalog -- see the module docstring.
    Every other field degrades gracefully to `None`/`[]` on a type mismatch,
    because a single unexpected optional field (say, a new pricing tier)
    should not itself invalidate an otherwise-good catalog.
    """
    if not isinstance(entry, dict):
        raise CatalogValidationError("model entry is not an object")
    model_id = entry.get("id")
    if not isinstance(model_id, str) or not model_id:
        raise CatalogValidationError("model entry missing a string id")
    name = entry.get("name")
    if not isinstance(name, str) or not name:
        raise CatalogValidationError(f"model {model_id!r} missing a string name")

    architecture = entry.get("architecture") if isinstance(entry.get("architecture"), dict) else {}
    pricing = entry.get("pricing") if isinstance(entry.get("pricing"), dict) else {}
    top_provider = entry.get("top_provider") if isinstance(entry.get("top_provider"), dict) else {}

    return {
        "id": model_id,
        "canonical_slug": _bounded_str(entry.get("canonical_slug")),
        "name": name[:MAX_STRING_FIELD_LENGTH],
        "description": _bounded_str(entry.get("description")),
        "context_length": _optional_number(entry.get("context_length")),
        "input_modalities": _string_list(architecture.get("input_modalities")),
        "output_modalities": _string_list(architecture.get("output_modalities")),
        "tokenizer": _bounded_str(architecture.get("tokenizer")),
        "pricing": {
            "prompt": _price(pricing.get("prompt")),
            "completion": _price(pricing.get("completion")),
            "request": _price(pricing.get("request")),
            "image": _price(pricing.get("image")),
            "audio": _price(pricing.get("audio", pricing.get("input_audio"))),
        },
        "top_provider_context_length": _optional_number(top_provider.get("context_length")),
        "max_completion_tokens": _optional_number(top_provider.get("max_completion_tokens")),
        "is_moderated": _optional_bool(top_provider.get("is_moderated")),
    }


def _matches_filters(model: Mapping[str, Any], filters: Mapping[str, str]) -> bool:
    for key, value in filters.items():
        if key == "input_modalities":
            if value not in model["input_modalities"]:
                return False
        elif key == "output_modalities":
            if value not in model["output_modalities"]:
                return False
        # Unknown filter keys are ignored rather than rejected, so a future
        # filter this module does not yet know about degrades to "no-op"
        # instead of refusing the whole refresh.
    return True


def normalize_catalog(raw_bytes: bytes, *, filters: Mapping[str, str],
                       source_url: str) -> dict[str, Any]:
    """Validate and normalize a raw `GET /api/v1/models` response body.

    Raises `CatalogValidationError` for anything this module does not trust
    enough to cache: oversized response, invalid JSON, wrong top-level shape,
    a malformed entry, a duplicate id, or too many models. The caller is
    expected to leave any existing cache file untouched when this raises.
    """
    if len(raw_bytes) > MAX_RESPONSE_BYTES:
        raise CatalogValidationError(
            f"response is {len(raw_bytes)} bytes, over the {MAX_RESPONSE_BYTES}-byte cap")
    try:
        payload = json.loads(raw_bytes)
    except json.JSONDecodeError as error:
        raise CatalogValidationError(f"response is not valid JSON: {error}") from error
    if not isinstance(payload, dict) or not isinstance(payload.get("data"), list):
        raise CatalogValidationError("response is not an object with a data array")

    models: list[dict[str, Any]] = []
    seen_ids: set[str] = set()
    for entry in payload["data"]:
        model = normalize_model(entry)
        if model["id"] in seen_ids:
            raise CatalogValidationError(f"duplicate model id: {model['id']}")
        seen_ids.add(model["id"])
        models.append(model)
    if len(models) > MAX_MODEL_COUNT:
        raise CatalogValidationError(
            f"catalog has {len(models)} models, over the {MAX_MODEL_COUNT}-model cap")

    filtered = [model for model in models if _matches_filters(model, filters)]
    now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    return {
        "schema_version": SCHEMA_VERSION,
        "source_url": source_url,
        "fetched_at_utc": now,
        "validated_at_utc": now,
        "filters": dict(filters),
        "model_count": len(filtered),
        "unfiltered_model_count": len(models),
        "models": filtered,
    }


def _http_get(url: str, timeout: float) -> bytes:
    request = urllib.request.Request(
        url, headers={"Accept": "application/json",
                       "User-Agent": "musializer-provider-catalog/1"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:  # noqa: S310 (fixed https URL)
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = response.read(65536)
            if not chunk:
                break
            total += len(chunk)
            if total > MAX_RESPONSE_BYTES:
                raise CatalogValidationError(
                    f"response exceeded the {MAX_RESPONSE_BYTES}-byte cap while streaming")
            chunks.append(chunk)
        return b"".join(chunks)


def cache_path(environ: Optional[Mapping[str, str]] = None) -> Path:
    return atomic_cache.cache_dir(environ) / CACHE_FILENAME


def refresh(*, cache_file: Optional[Path] = None, filters: Optional[Mapping[str, str]] = None,
            url: str = DEFAULT_URL, timeout: float = 10.0, fetch: Fetcher = _http_get,
            environ: Optional[Mapping[str, str]] = None) -> RefreshResult:
    """Fetch, validate, and atomically cache the OpenRouter catalog.

    On any failure (network error, refused shape) the prior cache at
    `cache_file` (or the resolved default) is left exactly as it was and the
    result carries the failure reason; nothing is written.
    """
    path = cache_file if cache_file is not None else cache_path(environ)
    filters = dict(filters or {})
    try:
        raw_bytes = fetch(url, timeout)
        document = normalize_catalog(raw_bytes, filters=filters, source_url=url)
    except (CatalogValidationError, urllib.error.URLError, OSError, TimeoutError) as error:
        return RefreshResult(ok=False, document=read_cache(path), error=str(error))
    atomic_cache.write_json_atomic(path, document)
    return RefreshResult(ok=True, document=document, error=None)


def read_cache(cache_file: Optional[Path] = None,
                environ: Optional[Mapping[str, str]] = None) -> Optional[dict[str, Any]]:
    path = cache_file if cache_file is not None else cache_path(environ)
    document = atomic_cache.read_json(path)
    if document is None or document.get("schema_version") != SCHEMA_VERSION:
        return None
    if not isinstance(document.get("models"), list):
        return None
    return document


def _parse_filters(pairs: Sequence[str]) -> dict[str, str]:
    filters: dict[str, str] = {}
    for pair in pairs:
        key, separator, value = pair.partition("=")
        if not separator:
            raise argparse.ArgumentTypeError(f"--filter expects KEY=VALUE, got {pair!r}")
        filters[key] = value
    return filters


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    refresh_cmd = sub.add_parser("refresh", help="fetch and cache the OpenRouter catalog")
    refresh_cmd.add_argument("--filter", action="append", default=[], metavar="KEY=VALUE",
                              help="e.g. input_modalities=audio (repeatable)")
    refresh_cmd.add_argument("--url", default=DEFAULT_URL)
    refresh_cmd.add_argument("--timeout", type=float, default=10.0)
    refresh_cmd.add_argument("--cache-dir", type=Path, help="override $XDG_CACHE_HOME/musializer")
    refresh_cmd.add_argument("--json", action="store_true")

    show_cmd = sub.add_parser("show", help="print the current cache, if any")
    show_cmd.add_argument("--cache-dir", type=Path)

    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    cache_file = (args.cache_dir / CACHE_FILENAME) if args.cache_dir else None

    if args.command == "refresh":
        filters = _parse_filters(args.filter)
        result = refresh(cache_file=cache_file, filters=filters, url=args.url, timeout=args.timeout)
        if args.json:
            print(json.dumps({"ok": result.ok, "error": result.error,
                              "document": result.document}, ensure_ascii=False, indent=2))
        elif result.ok:
            assert result.document is not None
            print(f"cached {result.document['model_count']} models "
                  f"(of {result.document['unfiltered_model_count']}) to "
                  f"{cache_file if cache_file else cache_path()}")
        else:
            print(f"refresh failed: {result.error}")
            if result.document is not None:
                print(f"prior cache preserved: {result.document['model_count']} models, "
                      f"fetched {result.document['fetched_at_utc']}")
        return 0 if result.ok else 1

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
