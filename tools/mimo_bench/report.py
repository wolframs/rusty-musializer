"""Turn stored responses into scores. Offline, and safe to re-run at any time.

Scoring is deliberately separate from running: a scoring rule can be fixed,
a lexicon extended, or a threshold changed, and every number is recomputed
from the stored raw responses without spending anything. That is also why the
store keeps the whole response body rather than the extracted text alone.
"""

from __future__ import annotations

import json
import statistics
from typing import Any, Sequence

from . import ground_truth, matrix, schema, scorers
from .bench_io import (
    SCORES_ROOT,
    atomic_write_json,
    cell_state,
    stored_calls,
    track_by_slug,
)

REPORT_VERSION = "musializer.mimo-bench-report/v1"


def parsed_document(record: dict[str, Any]) -> tuple[dict[str, Any] | None, str | None]:
    """Parse and validate one structured completion, or say why it failed."""
    text = record.get("text") or ""
    if not text.strip():
        return None, "empty completion"
    try:
        document = json.loads(text)
    except json.JSONDecodeError as error:
        return None, f"not JSON: {error}"
    try:
        return schema.validate(document), None
    except schema.SchemaViolation as error:
        return document if isinstance(document, dict) else None, str(error)


def claims_for_record(record: dict[str, Any]) -> tuple[scorers.Claims, dict[str, Any]]:
    """Claims plus the machine-usability verdict for one stored call."""
    if record.get("structured"):
        document, error = parsed_document(record)
        conformance = {
            "parse_ok": error is None or not str(error).startswith("not JSON"),
            "schema_ok": error is None,
            "error": error,
        }
        if document is None:
            return scorers.Claims(source="structured"), conformance
        return scorers.claims_from_structured(document), conformance
    return (scorers.claims_from_text(record.get("text") or ""),
            {"parse_ok": True, "schema_ok": None, "error": None})


def merge_claims(
    parts: Sequence[scorers.Claims], offsets: Sequence[float] | None = None,
) -> scorers.Claims:
    """One cell's chunks combined into a single set of claims about the excerpt.

    ``offsets`` shifts each chunk's timestamps, which is how the chunk-local
    reading of a chunked answer is produced. Instrument and descriptor lists
    are unioned in first-mention order; timestamps are concatenated and then
    sorted, because a merged answer is about the whole excerpt.
    """
    if offsets is None:
        offsets = [0.0] * len(parts)
    merged = scorers.Claims(source=parts[0].source if parts else "structured")
    texts: list[str] = []
    for part, offset in zip(parts, offsets):
        texts.append(part.text)
        merged.tempo_bpm.extend(part.tempo_bpm)
        merged.meters.extend(part.meters)
        merged.keys.extend(part.keys)
        for instrument in part.instruments:
            if instrument not in merged.instruments:
                merged.instruments.append(instrument)
        merged.unknown_instrument_terms.extend(part.unknown_instrument_terms)
        merged.lyric_moments.extend(
            (seconds + offset, phrase) for seconds, phrase in part.lyric_moments)
        merged.sections.extend(
            (start + offset, label) for start, label in part.sections)
        for descriptor in part.descriptors:
            if descriptor not in merged.descriptors:
                merged.descriptors.append(descriptor)
        merged.uncertain.extend(part.uncertain)
        for name, value in part.numeric.items():
            merged.numeric.setdefault(name, value)
    merged.text = "\n\n".join(text for text in texts if text)
    merged.lyric_moments.sort(key=lambda item: (item[0] != item[0], item[0]))
    merged.sections.sort(key=lambda item: item[0])
    if len(parts) > 1:
        # Averaging the per-chunk numeric fields keeps a chunked cell
        # comparable with a whole-excerpt one on the same axis.
        for name in ("energy", "tension", "valence"):
            values = [part.numeric[name] for part in parts if name in part.numeric]
            if values:
                merged.numeric[name] = statistics.fmean(values)
    return merged


def score_unit(cell: matrix.Cell, repeat: int, truth: ground_truth.TrackTruth,
               records: Sequence[dict[str, Any]] | None = None) -> dict[str, Any]:
    """Score one (cell, repeat): every dimension, plus its own provenance."""
    stored = list(records) if records is not None else stored_calls(cell.id, repeat)
    stored.sort(key=lambda record: record.get("call_index", 0))
    if not stored:
        return {"cell": cell.id, "repeat": repeat, "status": "missing"}

    per_call = [claims_for_record(record) for record in stored]
    parts = [claims for claims, _ in per_call]
    conformance = [verdict for _, verdict in per_call]

    if cell.shaping == "S3":
        # Every call is an independent reformatting of the same description;
        # they are not chunks and must not be merged.
        documents = [parsed_document(record)[0] for record in stored]
        usable = [document for document in documents if isinstance(document, dict)]
        merged = parts[0] if parts else scorers.Claims(source="structured")
        determinism = scorers.field_disagreement(usable, schema.DETERMINISM_FIELDS)
    else:
        offsets_absolute = [0.0] * len(parts)
        merged = merge_claims(parts, offsets_absolute)
        determinism = {"status": "not-applicable"}

    lyric_absolute = scorers.score_lyric_moments(merged, truth)
    frame = {"status": "not-applicable"}
    if len(stored) > 1 and cell.shaping != "S3":
        chunk_offsets = [float(record.get("chunk_start_seconds") or 0.0)
                         for record in stored]
        chunk_local = merge_claims(parts, chunk_offsets)
        lyric_local = scorers.score_lyric_moments(chunk_local, truth)
        absolute_error = lyric_absolute.get("median_error_seconds")
        local_error = lyric_local.get("median_error_seconds")
        if absolute_error is not None or local_error is not None:
            better_local = (
                absolute_error is None
                or (local_error is not None and local_error < absolute_error))
            frame = {
                "status": "scored",
                "frame_used": "chunk-local" if better_local else "absolute",
                "obeyed": not better_local,
                "median_error_absolute": absolute_error,
                "median_error_chunk_local": local_error,
            }

    served = sorted({str(record.get("model_served"))
                     for record in stored if record.get("model_served")})
    providers = sorted({str(record.get("provider_served"))
                        for record in stored if record.get("provider_served")})
    usage_prompt = sum(int(((record.get("usage") or {}).get("prompt_tokens")) or 0)
                       for record in stored)
    usage_completion = sum(
        int(((record.get("usage") or {}).get("completion_tokens")) or 0)
        for record in stored)

    return {
        "cell": cell.id,
        "repeat": repeat,
        "status": "scored",
        "blocks": list(cell.blocks),
        "shaping": cell.shaping,
        "chunking": cell.chunking_id,
        "prompt_id": cell.prompt_id,
        "track": cell.track_slug,
        "calls": len(stored),
        "provenance": {
            "models_served": served,
            "providers_served": providers,
            "prompt_sha256": sorted({
                str((record.get("identity") or {}).get("prompt_sha256"))
                for record in stored}),
            "schema_version": sorted({
                str((record.get("identity") or {}).get("schema_version"))
                for record in stored}),
            "scorer_version": scorers.SCORER_VERSION,
            "lexicon_sha256": scorers.LEXICON_SHA256,
            "ground_truth_version": ground_truth.GROUND_TRUTH_VERSION,
        },
        "usage": {"prompt_tokens": usage_prompt, "completion_tokens": usage_completion},
        "conformance": {
            "calls": len(conformance),
            "parse_ok": sum(1 for item in conformance if item["parse_ok"]),
            "schema_ok": sum(1 for item in conformance if item["schema_ok"]),
            "errors": [item["error"] for item in conformance if item["error"]],
        },
        "tempo": scorers.score_tempo(merged, truth),
        "key": scorers.score_key(merged, truth),
        "meter": scorers.score_meter(merged, truth),
        "instruments": scorers.score_instruments(merged, truth),
        "lyric_position": lyric_absolute,
        "time_frame": frame,
        "form": scorers.score_sections(merged, truth),
        "concreteness": scorers.concreteness(merged.text),
        "determinism": determinism,
        "_claims": merged,
    }


def score_cell(cell: matrix.Cell, repeats: Sequence[int]) -> dict[str, Any]:
    truth = ground_truth.load(track_by_slug(cell.track_slug))
    units: list[dict[str, Any]] = []
    claims: list[scorers.Claims] = []
    for repeat in repeats:
        scored = score_unit(cell, repeat, truth)
        if scored.get("status") != "scored":
            continue
        claims.append(scored.pop("_claims"))
        units.append(scored)
    if not units:
        return {"cell": cell.id, "status": "missing", "repeats": list(repeats)}

    def _mean(path: Sequence[str]) -> float | None:
        values: list[float] = []
        for unit in units:
            node: Any = unit
            for key in path:
                node = (node or {}).get(key) if isinstance(node, dict) else None
            if isinstance(node, (int, float)) and not isinstance(node, bool):
                values.append(float(node))
        return statistics.fmean(values) if values else None

    return {
        "cell": cell.id,
        "status": "scored",
        "blocks": list(units[0]["blocks"]),
        "repeats_scored": len(units),
        "summary": {
            "tempo_accept_rate": _mean(("tempo", "accept_rate")),
            "instrument_f1": _mean(("instruments", "f1")),
            "instrument_recall": _mean(("instruments", "recall")),
            "lyric_median_error_seconds": _mean(
                ("lyric_position", "median_error_seconds")),
            "lyric_within_2s": _mean(("lyric_position", "within_2s")),
            "lyric_coverage": _mean(("lyric_position", "coverage")),
            "lyric_fabrication_rate": _mean(("lyric_position", "fabrication_rate")),
            "form_agreement_f1": _mean(("form", "agreement_f1")),
            "concrete_per_100_words": _mean(("concreteness", "concrete_per_100_words")),
            "concrete_share": _mean(("concreteness", "concrete_share")),
            "schema_ok_rate": _mean(("conformance", "schema_ok")),
            "determinism_field_disagreement": _mean(
                ("determinism", "field_disagreement_rate")),
            "determinism_identical_rate": _mean(
                ("determinism", "identical_output_rate")),
            "prompt_tokens": _mean(("usage", "prompt_tokens")),
            "completion_tokens": _mean(("usage", "completion_tokens")),
        },
        "consistency": scorers.inter_run_agreement(claims),
        "units": units,
    }


def score_all(repeats: int = matrix.DEFAULT_REPEATS) -> dict[str, Any]:
    cells: list[dict[str, Any]] = []
    for cell in matrix.cells():
        present = [repeat for repeat in range(1, repeats + 1)
                   if cell_state(cell.id, repeat) is not None
                   or stored_calls(cell.id, repeat)]
        cells.append(score_cell(cell, present or list(range(1, repeats + 1))))
    blocks: dict[str, list[str]] = {}
    for cell in matrix.cells():
        for block in cell.blocks:
            blocks.setdefault(block, []).append(cell.id)
    return {
        "report_version": REPORT_VERSION,
        "matrix_version": matrix.MATRIX_VERSION,
        "scorer_version": scorers.SCORER_VERSION,
        "lexicon_sha256": scorers.LEXICON_SHA256,
        "blocks": blocks,
        "cells": cells,
    }


def write_report(document: dict[str, Any]) -> Any:
    path = SCORES_ROOT / "report.json"
    atomic_write_json(path, document)
    return path


def format_report(document: dict[str, Any]) -> str:
    columns = (
        ("cell", 46), ("reps", 5), ("temp", 6), ("instF1", 7), ("lyr_med", 8),
        ("lyr<2s", 7), ("form", 6), ("conc/100", 9), ("schema", 7), ("agree", 6),
    )
    lines = ["  ".join(name.ljust(width) for name, width in columns)]
    lines.append("  ".join("-" * width for _, width in columns))
    for cell in document["cells"]:
        if cell.get("status") != "scored":
            lines.append(f"{cell['cell'][:46].ljust(46)}  (no stored responses)")
            continue
        summary = cell["summary"]
        consistency = cell.get("consistency") or {}

        def _cell(value: Any, width: int, digits: int = 2) -> str:
            if value is None:
                return "-".ljust(width)
            return f"{float(value):.{digits}f}".ljust(width)

        lines.append("  ".join((
            cell["cell"][:46].ljust(46),
            str(cell["repeats_scored"]).ljust(5),
            _cell(summary["tempo_accept_rate"], 6),
            _cell(summary["instrument_f1"], 7),
            _cell(summary["lyric_median_error_seconds"], 8),
            _cell(summary["lyric_within_2s"], 7),
            _cell(summary["form_agreement_f1"], 6),
            _cell(summary["concrete_per_100_words"], 9),
            _cell(summary["schema_ok_rate"], 7),
            _cell(consistency.get("descriptor_jaccard_mean"), 6),
        )))
    return "\n".join(lines)


__all__ = [
    "REPORT_VERSION",
    "claims_for_record",
    "format_report",
    "merge_claims",
    "parsed_document",
    "score_all",
    "score_cell",
    "score_unit",
    "write_report",
]
