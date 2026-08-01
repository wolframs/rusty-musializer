#!/usr/bin/env python3
"""Fetch an open-source caption face from Google Fonts.

The helper is opt-in and external to the renderer. It sends no user content:
the only thing that leaves the machine is a family name, plus whatever the
network layer necessarily reveals. ``--dry-run`` never opens a connection and
prints exactly which hosts a real run would contact.

Two subcommands:

``catalogue``  Fetch the family list and reduce it to the fields the picker
               needs. Cached on disk, refreshed when the cache is older than
               ``--max-age`` seconds.
``fetch``      Resolve one family to a regular-weight TrueType file, download
               it, verify it is a font, retrieve the licence it is distributed
               under, and write both plus a manifest into an output directory.

Nothing here mutates a project. The application verifies the manifest digests
itself before the bytes reach a font rasteriser or a .musi file.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Callable

from analysis_io import AnalysisValidationError, atomic_write_json

CATALOGUE_URL = "https://fonts.google.com/metadata/fonts"
CSS_URL = "https://fonts.googleapis.com/css2"
LICENCE_URL = "https://raw.githubusercontent.com/google/fonts/main"

# Every host this helper is permitted to contact. A redirect to anywhere else
# is refused rather than followed: the point of naming the boundary is that it
# stays where it was named.
ALLOWED_HOSTS = frozenset(
    ("fonts.google.com", "fonts.googleapis.com", "fonts.gstatic.com",
     "raw.githubusercontent.com")
)

CATALOGUE_SCHEMA_VERSION = "musializer.font-catalogue/v1"
MANIFEST_SCHEMA_VERSION = "musializer.font-import/v1"
# Fixed name inside the job's own output directory. The application reads this
# rather than the JSON manifest, for the same reason it reads the catalogue as
# TSV: it has no JSON parser and this table does not justify one.
MANIFEST_INDEX_NAME = "import.tsv"

DEFAULT_TIMEOUT = 60.0
DEFAULT_MAX_AGE_SECONDS = 7 * 24 * 60 * 60

# A catalogue response is a couple of megabytes; a face is a few hundred
# kilobytes and the largest CJK families a few megabytes. These caps exist so a
# redirect to something enormous fails fast instead of filling a disk.
CATALOGUE_BYTE_LIMIT = 32 * 1024 * 1024
FONT_BYTE_LIMIT = 32 * 1024 * 1024
LICENCE_BYTE_LIMIT = 1 * 1024 * 1024

# The licence directories google/fonts sorts families into, in the order they
# are tried. The name is the terms; "UFL" is the Ubuntu Font Licence.
LICENCE_DIRECTORIES = (
    ("ofl", "OFL.txt", "OFL-1.1"),
    ("apache", "LICENSE.txt", "Apache-2.0"),
    ("ufl", "UFL.txt", "UFL-1.0"),
)

# sfnt magic numbers stb_truetype will accept. Checked before anything is
# written, so a captive-portal login page cannot land on disk named .ttf.
SFNT_MAGIC = (b"\x00\x01\x00\x00", b"true", b"ttcf", b"OTTO")

FAMILY_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 '+.-]{0,127}$")
TRUETYPE_PATTERN = re.compile(
    r"src:\s*url\((https://[^)\s]+\.ttf)\)\s*format\('truetype'\)"
)

Transport = Callable[..., Any]


class FontFetchError(RuntimeError):
    """A font could not be retrieved. The message is shown to the user."""


def check_host(url: str) -> str:
    """Reject a URL outside the declared boundary before it is requested."""
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "https":
        raise FontFetchError(f"refusing a non-HTTPS URL: {parsed.scheme or 'none'}")
    if parsed.hostname not in ALLOWED_HOSTS:
        raise FontFetchError(f"refusing a request to {parsed.hostname or 'an unnamed host'}")
    return url


def read_url(
    url: str,
    *,
    byte_limit: int,
    transport: Transport = urllib.request.urlopen,
    timeout: float = DEFAULT_TIMEOUT,
) -> bytes:
    """GET one resource, bounded, and confirm the response stayed in bounds.

    The final URL is re-checked because a redirect is a second request to a
    host nobody has approved yet.
    """
    check_host(url)
    request = urllib.request.Request(url, method="GET")
    try:
        with transport(request, timeout=timeout) as response:
            final = getattr(response, "url", url)
            if final:
                check_host(final)
            # One byte past the limit, so a resource of exactly the limit is
            # accepted and a larger one is detected rather than truncated.
            payload = response.read(byte_limit + 1)
    except urllib.error.HTTPError as error:
        code = error.code
        error.close()
        if code == 404:
            raise FontFetchError(f"not found: {url}") from None
        raise FontFetchError(f"request failed with HTTP {code}") from None
    except urllib.error.URLError as error:
        raise FontFetchError(f"could not reach the network: {error.reason}") from None
    if len(payload) > byte_limit:
        raise FontFetchError(f"response is larger than the {byte_limit} byte limit")
    return payload


def family_directory(family: str) -> str:
    """The google/fonts directory name for a family: lowercase, no separators."""
    return re.sub(r"[^a-z0-9]", "", family.lower())


def validate_family(family: str) -> str:
    family = family.strip()
    if not FAMILY_PATTERN.match(family):
        raise FontFetchError(
            "a family name may only contain letters, digits, spaces and - . ' +"
        )
    return family


def reduce_catalogue(payload: Any) -> dict[str, Any]:
    """Keep only what the picker shows, and drop families we cannot serve.

    A family with no Latin subset would download and then render a caption of
    empty boxes, which is a worse outcome than not offering it.
    """
    if not isinstance(payload, dict):
        raise AnalysisValidationError("font catalogue is not an object")
    entries = payload.get("familyMetadataList")
    if not isinstance(entries, list) or not entries:
        raise AnalysisValidationError("font catalogue has no family list")
    families: list[dict[str, Any]] = []
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        family = entry.get("family")
        if not isinstance(family, str) or not FAMILY_PATTERN.match(family):
            continue
        subsets = entry.get("subsets")
        subsets = [s for s in subsets if isinstance(s, str)] if isinstance(subsets, list) else []
        if "latin" not in subsets:
            continue
        category = entry.get("category")
        popularity = entry.get("popularity")
        families.append(
            {
                "family": family,
                "category": category if isinstance(category, str) else "",
                "subsets": sorted(s for s in subsets if s != "menu"),
                "popularity": popularity if isinstance(popularity, int) else 0,
            }
        )
    if not families:
        raise AnalysisValidationError("font catalogue contained no usable family")
    families.sort(key=lambda item: (item["popularity"] or 10**9, item["family"]))
    return {
        "schema_version": CATALOGUE_SCHEMA_VERSION,
        "source": CATALOGUE_URL,
        "family_count": len(families),
        "families": families,
    }


def catalogue_is_fresh(path: Path, max_age_seconds: float, now: Callable[[], float]) -> bool:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return False
    if not isinstance(payload, dict):
        return False
    if payload.get("schema_version") != CATALOGUE_SCHEMA_VERSION:
        return False
    try:
        age = now() - path.stat().st_mtime
    except OSError:
        return False
    return 0 <= age < max_age_seconds


def load_catalogue(
    cache: Path,
    *,
    max_age_seconds: float = DEFAULT_MAX_AGE_SECONDS,
    transport: Transport = urllib.request.urlopen,
    timeout: float = DEFAULT_TIMEOUT,
    now: Callable[[], float] = time.time,
    force: bool = False,
) -> dict[str, Any]:
    if not force and catalogue_is_fresh(cache, max_age_seconds, now):
        return json.loads(cache.read_text(encoding="utf-8"))
    raw = read_url(
        CATALOGUE_URL, byte_limit=CATALOGUE_BYTE_LIMIT, transport=transport, timeout=timeout
    )
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AnalysisValidationError("font catalogue is not valid JSON") from error
    reduced = reduce_catalogue(payload)
    atomic_write_json(cache, reduced)
    return reduced


CATALOGUE_INDEX_HEADER = "musializer.font-catalogue/v1"


def write_catalogue_index(catalogue: dict[str, Any], path: Path) -> int:
    """Emit the catalogue as bounded TSV for the application to read.

    The renderer has no JSON parser and does not want one for this: a
    tab-separated table with a version header is what the analysis bridge
    already uses, and it is trivially bounded on the reading side. Family
    names are validated on the way in, so no field can contain a tab or a
    newline and the table cannot be made ragged by a catalogue change.
    """
    lines = [f"{CATALOGUE_INDEX_HEADER}\t{catalogue['family_count']}"]
    for entry in catalogue["families"]:
        family = entry["family"]
        category = entry.get("category", "")
        subsets = ",".join(entry.get("subsets", ()))
        if any("\t" in field or "\n" in field for field in (family, category, subsets)):
            raise AnalysisValidationError(
                f"catalogue entry for {family!r} contains a field separator"
            )
        lines.append(f"{family}\t{category}\t{subsets}")
    payload = "\n".join(lines) + "\n"
    temporary = path.with_name(path.name + ".partial")
    temporary.parent.mkdir(parents=True, exist_ok=True)
    temporary.write_text(payload, encoding="utf-8")
    temporary.replace(path)
    return len(catalogue["families"])


def resolve_truetype_url(
    family: str,
    *,
    transport: Transport = urllib.request.urlopen,
    timeout: float = DEFAULT_TIMEOUT,
) -> str:
    """Ask the stylesheet endpoint where the regular weight actually lives.

    Requested without advertising woff2 support, which is what makes the
    response name a .ttf: raylib's rasteriser has no woff2 decompressor, so a
    woff2 URL would download successfully and then fail to load.
    """
    query = urllib.parse.urlencode({"family": family})
    css = read_url(
        f"{CSS_URL}?{query}", byte_limit=LICENCE_BYTE_LIMIT, transport=transport, timeout=timeout
    ).decode("utf-8", errors="replace")
    match = TRUETYPE_PATTERN.search(css)
    if match is None:
        raise FontFetchError(
            f"Google Fonts did not offer a TrueType file for {family!r}"
        )
    return check_host(match.group(1))


def fetch_licence(
    family: str,
    *,
    transport: Transport = urllib.request.urlopen,
    timeout: float = DEFAULT_TIMEOUT,
) -> tuple[bytes, str]:
    """Retrieve the terms the family is distributed under.

    Google Fonts sorts families into one directory per licence, so which
    directory holds it *is* the answer. Every family is in exactly one.
    """
    directory = family_directory(family)
    if not directory:
        raise FontFetchError(f"{family!r} has no licence directory name")
    for licence_directory, filename, licence_name in LICENCE_DIRECTORIES:
        url = f"{LICENCE_URL}/{licence_directory}/{directory}/{filename}"
        try:
            payload = read_url(
                url, byte_limit=LICENCE_BYTE_LIMIT, transport=transport, timeout=timeout
            )
        except FontFetchError as error:
            if "not found" in str(error):
                continue
            raise
        if payload.strip():
            return payload, licence_name
    raise FontFetchError(
        f"no licence could be retrieved for {family!r}; refusing to bundle a face "
        "whose terms cannot travel with it"
    )


def looks_like_a_font(payload: bytes) -> bool:
    return payload[:4] in SFNT_MAGIC


def fetch_family(
    family: str,
    destination: Path,
    *,
    transport: Transport = urllib.request.urlopen,
    timeout: float = DEFAULT_TIMEOUT,
) -> dict[str, Any]:
    """Download one family into destination and describe what landed there.

    Writes the face, the licence, and a manifest naming both with their
    digests. The application re-verifies those digests before it trusts either.
    """
    family = validate_family(family)
    url = resolve_truetype_url(family, transport=transport, timeout=timeout)
    payload = read_url(url, byte_limit=FONT_BYTE_LIMIT, transport=transport, timeout=timeout)
    if not looks_like_a_font(payload):
        raise FontFetchError(
            f"what {family!r} resolved to is not a font file; nothing was written"
        )
    licence_bytes, licence_name = fetch_licence(family, transport=transport, timeout=timeout)

    destination.mkdir(parents=True, exist_ok=True)
    stem = family_directory(family) or "font"
    face_path = destination / f"{stem}.ttf"
    licence_path = destination / f"{stem}.licence.txt"
    face_path.write_bytes(payload)
    licence_path.write_bytes(licence_bytes)
    manifest = {
        "schema_version": MANIFEST_SCHEMA_VERSION,
        "family": family,
        "source": url,
        "font_path": str(face_path),
        "font_sha256": hashlib.sha256(payload).hexdigest(),
        "font_bytes": len(payload),
        "licence_path": str(licence_path),
        "licence_sha256": hashlib.sha256(licence_bytes).hexdigest(),
        "licence_name": licence_name,
    }
    atomic_write_json(destination / f"{stem}.manifest.json", manifest)
    write_manifest_index(manifest, destination / MANIFEST_INDEX_NAME)
    return manifest


def write_manifest_index(manifest: dict[str, Any], path: Path) -> None:
    """Emit the manifest as one bounded TSV row for the application.

    Written last, and by rename, so its presence is what says the download
    finished: a job killed halfway leaves no index and the result is discarded
    rather than half-applied.
    """
    fields = [
        manifest["family"],
        manifest["font_path"],
        manifest["font_sha256"],
        manifest["licence_path"],
        manifest["licence_sha256"],
        manifest["licence_name"],
    ]
    if any("\t" in field or "\n" in field for field in fields):
        raise AnalysisValidationError("manifest field contains a separator")
    payload = f"{MANIFEST_SCHEMA_VERSION}\t1\n" + "\t".join(fields) + "\n"
    temporary = path.with_name(path.name + ".partial")
    temporary.write_text(payload, encoding="utf-8")
    temporary.replace(path)


def dry_run_description(command: str, family: str | None) -> dict[str, Any]:
    """What a real run would contact, without contacting it."""
    hosts = [CATALOGUE_URL] if command == "catalogue" else [
        f"{CSS_URL}?family=...",
        "https://fonts.gstatic.com/... (named by the stylesheet response)",
        f"{LICENCE_URL}/<licence>/<family>/...",
    ]
    return {
        "command": command,
        "family": family,
        "would_contact": hosts,
        "sends_user_content": False,
        "allowed_hosts": sorted(ALLOWED_HOSTS),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT)
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="describe the request without opening a connection",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    catalogue = subparsers.add_parser("catalogue", help="fetch the family list")
    catalogue.add_argument("cache", type=Path)
    catalogue.add_argument("--max-age", type=float, default=DEFAULT_MAX_AGE_SECONDS)
    catalogue.add_argument("--force", action="store_true")
    catalogue.add_argument(
        "--index", type=Path,
        help="also write the catalogue as bounded TSV for the application",
    )

    fetch = subparsers.add_parser("fetch", help="download one family")
    fetch.add_argument("family")
    fetch.add_argument("output", type=Path)

    args = parser.parse_args(argv)
    family = getattr(args, "family", None)
    if args.dry_run:
        print(json.dumps(dry_run_description(args.command, family), indent=2))
        return 0
    try:
        if args.command == "catalogue":
            result = load_catalogue(
                args.cache,
                max_age_seconds=args.max_age,
                timeout=args.timeout,
                force=args.force,
            )
            if args.index is not None:
                write_catalogue_index(result, args.index)
            print(json.dumps({
                "schema_version": result["schema_version"],
                "family_count": result["family_count"],
                "cache": str(args.cache),
                "index": str(args.index) if args.index else None,
            }))
        else:
            print(json.dumps(fetch_family(family, args.output, timeout=args.timeout)))
    except (OSError, ValueError, RuntimeError) as error:
        print(f"Google Fonts request failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
