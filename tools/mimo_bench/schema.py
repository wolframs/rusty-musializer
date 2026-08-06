"""The benchmark's machine-usable output schema.

Deliberately *not* the application's ``SemanticScore`` schema. The application
asks MiMo for a creative interpretation and forbids it from claiming signal
facts; this benchmark's whole question is how exactly it can state those facts,
so the schema has the fields the operator named — instruments, tempo, meter,
key, form, lyric positions, texture — and the scorers check them against
measured or adjudicated truth.

Every field is `required` and `additionalProperties` is false, because
OpenRouter's strict json_schema mode demands both. Optionality is expressed as
a nullable type, so a refusal ("unknown") has somewhere to go that is not a
fabricated value.
"""

from __future__ import annotations

from typing import Any

from .bench_io import canonical_sha256

SCHEMA_VERSION = "musializer.mimo-bench-description/v1"
SCHEMA_NAME = "musializer_mimo_bench_description_v1"


def _object(properties: dict[str, Any]) -> dict[str, Any]:
    return {
        "type": "object",
        "additionalProperties": False,
        "required": list(properties),
        "properties": properties,
    }


DESCRIPTION_SCHEMA: dict[str, Any] = _object({
    "summary": {"type": "string"},
    "instruments": {
        "type": "array",
        "items": _object({
            "name": {"type": "string"},
            "timbre": {"type": "string"},
        }),
    },
    "tempo_bpm": {"type": ["number", "null"]},
    "meter": {"type": ["string", "null"]},
    "key": {"type": ["string", "null"]},
    "mode": {"type": ["string", "null"]},
    "sections": {
        "type": "array",
        "items": _object({
            "start_seconds": {"type": "number"},
            "end_seconds": {"type": "number"},
            "label": {"type": "string"},
        }),
    },
    "lyric_moments": {
        "type": "array",
        "description": "Short quoted phrases with the source-track second each is sung.",
        "items": _object({
            "seconds": {"type": "number"},
            "phrase": {"type": "string"},
        }),
    },
    "harmony_notes": {"type": "string"},
    "production_notes": {"type": "string"},
    "texture": {"type": "array", "items": {"type": "string"}},
    "feel": {"type": "array", "items": {"type": "string"}},
    "energy": {"type": "number"},
    "tension": {"type": "number"},
    "valence": {"type": "number"},
    "uncertain": {
        "type": "array",
        "description": "Names of fields the audio did not support a value for.",
        "items": {"type": "string"},
    },
})

SCHEMA_SHA256 = canonical_sha256(DESCRIPTION_SCHEMA)

# The fields the determinism probe compares. Free-text fields are compared
# after normalization; numeric fields to a tolerance. `summary` is excluded
# because two wordings of the same sentence are not a disagreement, and
# including it would make every reformatter run "disagree" trivially.
DETERMINISM_FIELDS: tuple[str, ...] = (
    "tempo_bpm", "meter", "key", "mode", "energy", "tension", "valence",
    "instruments", "sections", "lyric_moments", "texture", "feel", "uncertain",
)

NUMERIC_TOLERANCE = 1e-6


def response_format() -> dict[str, Any]:
    return {
        "type": "json_schema",
        "json_schema": {
            "name": SCHEMA_NAME,
            "strict": True,
            "schema": DESCRIPTION_SCHEMA,
        },
    }


class SchemaViolation(ValueError):
    """A stored completion does not satisfy the benchmark schema."""


def validate(document: Any) -> dict[str, Any]:
    """A small hand-written validator; the harness has no jsonschema dependency.

    It checks exactly what the scorers rely on: the shape of every field they
    read. It is deliberately strict about types and permissive about extra
    keys being *absent*, because a provider that drops a field is the failure
    this is meant to surface.
    """
    if not isinstance(document, dict):
        raise SchemaViolation("completion is not a JSON object")
    missing = [name for name in DESCRIPTION_SCHEMA["required"] if name not in document]
    if missing:
        raise SchemaViolation(f"missing fields: {', '.join(sorted(missing))}")
    extra = [name for name in document if name not in DESCRIPTION_SCHEMA["properties"]]
    if extra:
        raise SchemaViolation(f"unexpected fields: {', '.join(sorted(extra))}")

    def _string(name: str) -> None:
        if not isinstance(document[name], str):
            raise SchemaViolation(f"{name} must be a string")

    def _number(name: str, *, nullable: bool = False) -> None:
        value = document[name]
        if value is None and nullable:
            return
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise SchemaViolation(f"{name} must be a number")

    def _string_list(name: str) -> None:
        value = document[name]
        if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
            raise SchemaViolation(f"{name} must be an array of strings")

    def _objects(name: str, fields: dict[str, str]) -> None:
        value = document[name]
        if not isinstance(value, list):
            raise SchemaViolation(f"{name} must be an array")
        for index, item in enumerate(value):
            if not isinstance(item, dict):
                raise SchemaViolation(f"{name}[{index}] must be an object")
            for field, kind in fields.items():
                if field not in item:
                    raise SchemaViolation(f"{name}[{index}] is missing {field}")
                candidate = item[field]
                if kind == "string" and not isinstance(candidate, str):
                    raise SchemaViolation(f"{name}[{index}].{field} must be a string")
                if kind == "number" and (
                    isinstance(candidate, bool) or not isinstance(candidate, (int, float))
                ):
                    raise SchemaViolation(f"{name}[{index}].{field} must be a number")

    _string("summary")
    _string("harmony_notes")
    _string("production_notes")
    for name in ("tempo_bpm",):
        _number(name, nullable=True)
    for name in ("energy", "tension", "valence"):
        _number(name)
    for name in ("meter", "key", "mode"):
        if document[name] is not None and not isinstance(document[name], str):
            raise SchemaViolation(f"{name} must be a string or null")
    for name in ("texture", "feel", "uncertain"):
        _string_list(name)
    _objects("instruments", {"name": "string", "timbre": "string"})
    _objects("sections", {"start_seconds": "number", "end_seconds": "number", "label": "string"})
    _objects("lyric_moments", {"seconds": "number", "phrase": "string"})
    return document


__all__ = [
    "DESCRIPTION_SCHEMA",
    "DETERMINISM_FIELDS",
    "NUMERIC_TOLERANCE",
    "SCHEMA_NAME",
    "SCHEMA_SHA256",
    "SCHEMA_VERSION",
    "SchemaViolation",
    "response_format",
    "validate",
]
