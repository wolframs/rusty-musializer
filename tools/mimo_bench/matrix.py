"""The experiment matrix: chunkings, shaping arms, cells and their calls.

The design is **one factor at a time around a named centre**, not a full
cross. A full 4 x 2 x 2 x 4 cross is 64 conditions and, because the 20-chunk
conditions cost twenty calls each, it multiplies out to something nobody
resumes after a suspend. Four blocks sweep one axis each around the centre
``(chunking = 1x100 s, register = strict, specificity = checklist,
shaping = single-turn structured)``:

===========  ==============================================================
block        what it varies, holding everything else at the centre
===========  ==============================================================
chunking     20x5 s, 10x10 s, 5x20 s, 1x100 s of the *same* 100 s excerpt
prompt       {casual, strict} x {open, checklist}, as free text
shaping      S1 single turn, S2a two-turn with the audio resent, S2b
             two-turn with the audio elided, S3 free text then a separate
             cheap text-only reformatter, run N times on identical input
replication  the centre and one neighbour, on a second track
===========  ==============================================================

Cells are keyed by their parameters, so a cell that two blocks both want is
run once and belongs to both. S2a/S2b/S3 do not re-listen: they consume the
recorded turn-1 text of the ``S0`` free-text cell, which is itself a cell of
the prompt block. That sharing is why the whole matrix is under 60 calls per
repeat rather than over a hundred.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Iterator, Sequence

from .bench_io import EXCERPT_SECONDS, Track, primary_track, replication_track
from . import prompts, schema

MATRIX_VERSION = "musializer.mimo-bench-matrix/v1"

MIMO_MODEL = "xiaomi/mimo-v2.5"
# Text-only, cheap, and *not* the model under test — the point of arm S3 is
# whether a second model can be trusted to reshape MiMo's prose. The operator
# confirms its slug and price on OpenRouter before the live run; the dry run
# prints both.
REFORMATTER_MODEL = "openai/gpt-4o-mini"

DEFAULT_REPEATS = 3
# Arm S3's determinism probe: the same description, reformatted this many
# times, so field-level disagreement is measured instead of assumed.
REFORMATTER_PROBE_RUNS = 5

TEMPERATURE = 0.2
AUDIO_FORMAT = "mp3"


@dataclass(frozen=True)
class Chunking:
    id: str
    count: int
    seconds: float

    @property
    def total_seconds(self) -> float:
        return self.count * self.seconds

    @property
    def label(self) -> str:
        return f"{self.count}x{self.seconds:g}s"


CHUNKINGS: tuple[Chunking, ...] = (
    Chunking("c20x05", 20, 5.0),
    Chunking("c10x10", 10, 10.0),
    Chunking("c05x20", 5, 20.0),
    Chunking("c01x100", 1, 100.0),
)

CHUNKINGS_BY_ID = {chunking.id: chunking for chunking in CHUNKINGS}
CENTRE_CHUNKING = CHUNKINGS_BY_ID["c01x100"]
CENTRE_PROMPT = "strict-checklist"


@dataclass(frozen=True)
class ChunkSpan:
    index: int
    count: int
    start_seconds: float
    end_seconds: float

    @property
    def seconds(self) -> float:
        return self.end_seconds - self.start_seconds


def chunk_spans(track: Track, chunking: Chunking) -> list[ChunkSpan]:
    """Absolute source-track spans for one chunking of one track's excerpt.

    The excerpt is always ``EXCERPT_SECONDS`` long and always the same audio;
    only the cut points move. The final chunk is clipped to the excerpt end so
    a chunking that does not divide evenly still covers exactly 100 s and no
    more — the axis is granularity at *fixed total duration*.
    """
    if chunking.seconds <= 0:
        raise ValueError("chunk seconds must be positive")
    start = track.excerpt_start_seconds
    end = start + EXCERPT_SECONDS
    spans: list[ChunkSpan] = []
    count = min(chunking.count, math.ceil(EXCERPT_SECONDS / chunking.seconds))
    for index in range(count):
        chunk_start = start + index * chunking.seconds
        if chunk_start >= end:
            break
        chunk_end = min(chunk_start + chunking.seconds, end)
        spans.append(ChunkSpan(index, count, chunk_start, chunk_end))
    if spans:
        spans[-1] = ChunkSpan(spans[-1].index, len(spans), spans[-1].start_seconds, end)
        spans = [ChunkSpan(span.index, len(spans), span.start_seconds, span.end_seconds)
                 for span in spans]
    return spans


# ---------------------------------------------------------------------------
# Shaping arms
# ---------------------------------------------------------------------------

# id      turns  audio in turn 2   model of turn 2   what it answers
SHAPING_ARMS: dict[str, dict[str, object]] = {
    "S0": {
        "label": "free text, no schema",
        "structured": False,
        "depends": False,
        "note": "the rich description the other arms reshape; also the prompt block's output",
    },
    "S1": {
        "label": "single turn, schema demanded with the audio",
        "structured": True,
        "depends": False,
        "note": "what the application does today",
    },
    "S2a": {
        "label": "two turns, same model, audio resent",
        "structured": True,
        "depends": True,
        "note": "chat completions are stateless, so a true second turn pays for the audio twice",
    },
    "S2b": {
        "label": "two turns, same model, audio elided",
        "structured": True,
        "depends": True,
        "note": "the same model reshapes its own words with no audio in context",
    },
    "S3": {
        "label": "separate cheap text-only model reformats",
        "structured": True,
        "depends": True,
        "note": f"run {REFORMATTER_PROBE_RUNS}x on identical input to measure determinism",
    },
}

STRUCTURED_ARMS: tuple[str, ...] = ("S1", "S2a", "S2b", "S3")


@dataclass(frozen=True)
class Cell:
    track_slug: str
    chunking_id: str
    prompt_id: str
    shaping: str
    blocks: tuple[str, ...]
    depends_on: str | None = None

    @property
    def id(self) -> str:
        return f"{self.track_slug}/{self.chunking_id}/{self.prompt_id}/{self.shaping}"

    @property
    def chunking(self) -> Chunking:
        return CHUNKINGS_BY_ID[self.chunking_id]

    @property
    def structured(self) -> bool:
        return bool(SHAPING_ARMS[self.shaping]["structured"])


@dataclass(frozen=True)
class Call:
    """One HTTP request the driver would make, fully determined before it runs."""

    index: int
    kind: str            # "audio" | "text"
    model: str
    chunk: ChunkSpan | None
    turn: int
    structured: bool
    probe_run: int = 0   # >0 only for arm S3's repeated reformatting
    audio_seconds: float = 0.0
    prompt_text_source: str = ""
    extra: dict[str, object] = field(default_factory=dict)


def cells() -> list[Cell]:
    """Every distinct cell, deduplicated across blocks, in execution order."""
    primary = primary_track().slug
    replication = replication_track().slug
    ordered: dict[str, Cell] = {}

    def add(cell: Cell) -> None:
        existing = ordered.get(cell.id)
        if existing is None:
            ordered[cell.id] = cell
            return
        merged = tuple(dict.fromkeys(existing.blocks + cell.blocks))
        ordered[cell.id] = Cell(
            existing.track_slug, existing.chunking_id, existing.prompt_id,
            existing.shaping, merged, existing.depends_on or cell.depends_on,
        )

    # Block 1 — chunk granularity at fixed total duration.
    for chunking in CHUNKINGS:
        add(Cell(primary, chunking.id, CENTRE_PROMPT, "S1", ("chunking",)))

    # Block 2 — register x specificity, as free text so no schema pressure
    # confounds the register effect the operator observed.
    for prompt_id in prompts.PROMPT_IDS:
        add(Cell(primary, CENTRE_CHUNKING.id, prompt_id, "S0", ("prompt",)))

    # Block 3 — output shaping. Every arm asks for the same content with the
    # same prompt; only where and by whom the schema is imposed changes.
    free_turn_one = f"{primary}/{CENTRE_CHUNKING.id}/{CENTRE_PROMPT}/S0"
    add(Cell(primary, CENTRE_CHUNKING.id, CENTRE_PROMPT, "S0", ("shaping",)))
    add(Cell(primary, CENTRE_CHUNKING.id, CENTRE_PROMPT, "S1", ("shaping",)))
    for arm in ("S2a", "S2b", "S3"):
        add(Cell(primary, CENTRE_CHUNKING.id, CENTRE_PROMPT, arm, ("shaping",),
                 depends_on=free_turn_one))

    # Block 4 — does the chunking answer replicate on a second track?
    for chunking_id in (CENTRE_CHUNKING.id, "c05x20"):
        add(Cell(replication, chunking_id, CENTRE_PROMPT, "S1", ("replication",)))

    return list(ordered.values())


def cell_by_id(cell_id: str) -> Cell:
    for cell in cells():
        if cell.id == cell_id:
            return cell
    raise KeyError(f"unknown cell id: {cell_id}")


def calls_for(cell: Cell, track: Track) -> list[Call]:
    """Every call one (cell, repeat) makes, in order, with its index pinned.

    The index is what the resume logic keys on, so it must be a pure function
    of the cell — never of what has already run.
    """
    arm = cell.shaping
    if arm in ("S0", "S1"):
        spans = chunk_spans(track, cell.chunking)
        return [
            Call(
                index=span.index,
                kind="audio",
                model=MIMO_MODEL,
                chunk=span,
                turn=1,
                structured=(arm == "S1"),
                audio_seconds=span.seconds,
                prompt_text_source=cell.prompt_id,
            )
            for span in spans
        ]

    # The dependent arms consume the S0 cell's recorded turn-1 text.
    span = chunk_spans(track, cell.chunking)[0]
    if arm == "S2a":
        return [Call(0, "audio", MIMO_MODEL, span, 2, True,
                     audio_seconds=span.seconds, prompt_text_source="reformat-turn2",
                     extra={"audio_resent": True})]
    if arm == "S2b":
        return [Call(0, "text", MIMO_MODEL, span, 2, True,
                     prompt_text_source="reformat-turn2",
                     extra={"audio_resent": False})]
    if arm == "S3":
        return [
            Call(run, "text", REFORMATTER_MODEL, span, 2, True, probe_run=run + 1,
                 prompt_text_source="reformat-standalone",
                 extra={"determinism_probe": True})
            for run in range(REFORMATTER_PROBE_RUNS)
        ]
    raise KeyError(f"unknown shaping arm: {arm}")


def units(repeats: int = DEFAULT_REPEATS) -> Iterator[tuple[Cell, int]]:
    """Every resumable unit — one (cell, repeat) — in dependency-safe order.

    Repeat-major would be nicer for early partial results, but a dependent arm
    must not run before the S0 cell of the same repeat, and cell-major with
    the S0 cells emitted first is the simplest order that guarantees it.
    """
    ordered = sorted(cells(), key=lambda cell: (cell.depends_on is not None, cell.id))
    for repeat in range(1, repeats + 1):
        for cell in ordered:
            yield cell, repeat


def matrix_summary(repeats: int = DEFAULT_REPEATS) -> dict[str, object]:
    from . import bench_io

    rows: list[dict[str, object]] = []
    audio_calls = text_calls = 0
    audio_seconds = 0.0
    for cell in cells():
        track = bench_io.track_by_slug(cell.track_slug)
        cell_calls = calls_for(cell, track)
        cell_audio = sum(1 for call in cell_calls if call.kind == "audio")
        cell_text = sum(1 for call in cell_calls if call.kind == "text")
        cell_seconds = sum(call.audio_seconds for call in cell_calls)
        audio_calls += cell_audio * repeats
        text_calls += cell_text * repeats
        audio_seconds += cell_seconds * repeats
        rows.append({
            "cell": cell.id,
            "blocks": ",".join(cell.blocks),
            "chunks": len(chunk_spans(track, cell.chunking)),
            "calls_per_repeat": len(cell_calls),
            "audio_calls_per_repeat": cell_audio,
            "text_calls_per_repeat": cell_text,
            "audio_seconds_per_repeat": cell_seconds,
            "depends_on": cell.depends_on,
        })
    return {
        "matrix_version": MATRIX_VERSION,
        "prompt_registry_version": prompts.PROMPT_REGISTRY_VERSION,
        "schema_version": schema.SCHEMA_VERSION,
        "repeats": repeats,
        "cells": len(rows),
        "units": len(rows) * repeats,
        "audio_calls": audio_calls,
        "text_calls": text_calls,
        "total_calls": audio_calls + text_calls,
        "audio_seconds": audio_seconds,
        "rows": rows,
    }


def blocks_of(cell_ids: Sequence[str]) -> dict[str, list[str]]:
    grouped: dict[str, list[str]] = {}
    for cell in cells():
        if cell.id not in cell_ids:
            continue
        for block in cell.blocks:
            grouped.setdefault(block, []).append(cell.id)
    return grouped


__all__ = [
    "AUDIO_FORMAT",
    "CENTRE_CHUNKING",
    "CENTRE_PROMPT",
    "CHUNKINGS",
    "CHUNKINGS_BY_ID",
    "Call",
    "Cell",
    "ChunkSpan",
    "Chunking",
    "DEFAULT_REPEATS",
    "MATRIX_VERSION",
    "MIMO_MODEL",
    "REFORMATTER_MODEL",
    "REFORMATTER_PROBE_RUNS",
    "SHAPING_ARMS",
    "STRUCTURED_ARMS",
    "TEMPERATURE",
    "blocks_of",
    "call_index_span",
    "calls_for",
    "cell_by_id",
    "cells",
    "chunk_spans",
    "matrix_summary",
    "units",
]


def call_index_span(cell: Cell, track: Track) -> tuple[int, int]:
    indices = [call.index for call in calls_for(cell, track)]
    return (min(indices), max(indices)) if indices else (0, -1)
