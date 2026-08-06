"""Ground truth: what each claim is checked against, and where it comes from.

Four sources, each with a different authority, and the difference is recorded
rather than smoothed over:

``measured``
    ``build/lyrics-research-v2/workspace/<track>/measured.json``, produced by
    ``tools/analyze_audio.py``. Tempo and section boundaries. These are
    *estimates* — the repository's own — so a mismatch is scored as
    disagreement with the measurement, never as the model being wrong about
    the world. The tempo estimate is additionally recomputed over the excerpt
    window here, because the stored one is whole-track.
``aligned``
    ``lyrics.aligned.json`` from the LT1 lane: second-accurate line onsets for
    the authored lyrics. The strongest truth this benchmark has.
``adjudicated``
    ``build/lyrics-research-v2/ground_truth_adjudication.json``: the operator
    listened and named a true second for specific lines. Overrides ``aligned``
    where present, and marks a line as unusable where the operator could not
    name one.
``authored``
    ``tools/mimo_bench/ground_truth/tracks.json``: key, meter and the
    instrument list. Nothing in this repository measures these. Until the
    operator fills them in, every scorer that needs them abstains and says so.
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass, field
from typing import Any

from .bench_io import (
    ADJUDICATION_PATH,
    EXCERPT_SECONDS,
    GROUND_TRUTH_PATH,
    Track,
    read_json,
)

GROUND_TRUTH_VERSION = "musializer.mimo-bench-ground-truth/v1"

# The same bounds and the same confidence gate as `tools/analyze_audio.py`
# (`_estimate_pulse`, lines 137-175). Mirrored rather than improved on: an
# excerpt estimate produced by a different algorithm would not be comparable
# with the whole-track one the repository already stores.
MIN_BPM = 55.0
MAX_BPM = 190.0
MINIMUM_CONFIDENCE = 0.18
ATTACK_THRESHOLD = 0.55
ATTACKS_FOR_FULL_CONFIDENCE = 8.0
#: How many ranked candidate pulses to keep. Tempo autocorrelation on a real
#: onset envelope is genuinely ambiguous between a pulse and its multiples;
#: keeping the ranked set makes that ambiguity visible in the score instead of
#: resolving it with a constant nobody can check.
CANDIDATE_COUNT = 3
#: And how strong a local maximum has to be, relative to the peak, to count as
#: a candidate at all. Without a floor the ranked set reaches down into
#: *negatively* correlated lags, and accepting a claim against any of them
#: covers 47 % of the 60-200 BPM range — a coin flip dressed as a measurement.
#: With the floor it is 27 %.
CANDIDATE_SCORE_FLOOR = 0.5


@dataclass
class LyricTruth:
    reference_line_index: int
    text: str
    start_seconds: float
    end_seconds: float
    source: str            # "aligned" | "adjudicated"
    review_flagged: bool = False


@dataclass
class TrackTruth:
    slug: str
    excerpt_start: float
    excerpt_end: float
    measured_bpm: float | None
    measured_bpm_confidence: float | None
    excerpt_bpm: float | None
    excerpt_bpm_confidence: float | None = None
    #: Ranked local maxima of the same autocorrelation, best first. See
    #: `excerpt_pulse` for why the ambiguity is kept rather than resolved.
    excerpt_bpm_candidates: list[float] = field(default_factory=list)
    sections: list[tuple[float, float]] = field(default_factory=list)
    lyrics: list[LyricTruth] = field(default_factory=list)
    key_tonic: str | None = None
    key_mode: str | None = None
    key_status: str = "unadjudicated"
    meter: str | None = None
    meter_status: str = "unadjudicated"
    instruments_present: list[str] = field(default_factory=list)
    instruments_allowed_extra: list[str] = field(default_factory=list)
    instruments_absent: list[str] = field(default_factory=list)
    instruments_status: str = "unadjudicated"
    missing_sources: list[str] = field(default_factory=list)

    def abstaining_dimensions(self) -> list[str]:
        abstaining: list[str] = []
        if self.key_status == "unadjudicated":
            abstaining.append("key")
        if self.meter_status == "unadjudicated":
            abstaining.append("meter")
        if self.instruments_status == "unadjudicated":
            abstaining.append("instruments")
        if not self.lyrics:
            abstaining.append("lyric_position")
        if (self.measured_bpm is None and self.excerpt_bpm is None
                and not self.excerpt_bpm_candidates):
            abstaining.append("tempo")
        if not self.sections:
            abstaining.append("form")
        return abstaining


# ---------------------------------------------------------------------------
# measured.json
# ---------------------------------------------------------------------------


def excerpt_pulse(
    measured: dict[str, Any], start_seconds: float, end_seconds: float,
) -> dict[str, Any]:
    """The repository's own pulse estimator, run over the excerpt window.

    A faithful reimplementation of ``tools/analyze_audio.py`` ``_estimate_pulse``
    (lines 137-175): the same 55-190 BPM lag range, the same
    ``correlation / (energy * (1 - lag/N))`` normalization, the same
    attack-count confidence gate, and the same refusal to name a tempo below
    it. Reimplemented rather than imported because that helper takes a NumPy
    array built inside a decode, and because a *different* algorithm would
    produce an excerpt number that is not comparable with the whole-track one
    already stored in ``measured.json``.

    It returns the whole ranked candidate set rather than one number. Tempo
    autocorrelation on a real onset envelope is genuinely ambiguous between a
    pulse and its multiples — the argmax for this repository's own analysis of
    both benchmark tracks is a sub-multiple of the felt tempo — and a scorer
    that compares against a single ambiguous number is measuring the estimator
    as much as the model. The tempo scorer therefore checks a claim against
    every candidate and records *which* one and at what factor, so a
    systematic 2:1 or 3:1 appears in the data instead of being averaged away.
    """
    empty: dict[str, Any] = {"bpm": None, "confidence": 0.0,
                             "phase_seconds": None, "candidates": []}
    analysis = measured.get("analysis") or {}
    frames = measured.get("frames")
    hop = analysis.get("hop_size")
    rate = analysis.get("sample_rate")
    if not isinstance(frames, list) or not hop or not rate:
        return empty
    seconds_per_frame = float(hop) / float(rate)
    frame_rate = 1.0 / seconds_per_frame
    first = int(math.floor(start_seconds / seconds_per_frame))
    last = int(math.ceil(end_seconds / seconds_per_frame))
    onset = [
        float(frame.get("onset_strength") or 0.0)
        for frame in frames[max(0, first):max(0, last)]
        if isinstance(frame, dict)
    ]
    if len(onset) < max(8, int(frame_rate * 4)) or max(onset, default=0.0) < 0.1:
        return empty
    mean = sum(onset) / len(onset)
    centred = [value - mean for value in onset]
    energy = sum(value * value for value in centred)
    if energy <= 1e-9:
        return empty
    min_lag = max(1, int(math.ceil(frame_rate * 60.0 / MAX_BPM)))
    max_lag = min(len(onset) - 2, int(math.floor(frame_rate * 60.0 / MIN_BPM)))
    if max_lag <= min_lag:
        return empty

    normalized: dict[int, float] = {}
    for lag in range(min_lag, max_lag + 1):
        total = 0.0
        for index in range(len(centred) - lag):
            total += centred[index] * centred[index + lag]
        divisor = max(1e-12, energy * (1.0 - lag / max(1.0, len(onset))))
        normalized[lag] = total / divisor

    best_lag = max(normalized, key=normalized.__getitem__)
    peak = normalized[best_lag]
    attacks = sum(1 for value in onset if value >= ATTACK_THRESHOLD)
    confidence = peak * min(1.0, attacks / ATTACKS_FOR_FULL_CONFIDENCE)
    if confidence < MINIMUM_CONFIDENCE:
        return empty

    peaks = [
        (score, lag) for lag, score in normalized.items()
        if all(score >= normalized[lag + offset]
               for offset in (-1, 1) if lag + offset in normalized)
    ]
    peaks.sort(reverse=True)
    floor = peak * CANDIDATE_SCORE_FLOOR
    candidates = [
        {"bpm": round(60.0 * frame_rate / lag, 6), "score": score, "rank": rank}
        for rank, (score, lag) in enumerate(peaks[:CANDIDATE_COUNT], 1)
        if score >= floor and score > 0.0
    ]
    ordered = sorted(
        (index for index, value in enumerate(onset)
         if value >= max(0.45, _percentile(onset, 80))),
    )
    phase_frame = ordered[0] if ordered else onset.index(max(onset))
    return {
        "bpm": round(60.0 * frame_rate / best_lag, 6),
        "confidence": confidence,
        "phase_seconds": start_seconds + phase_frame / frame_rate,
        "candidates": candidates,
    }


def _percentile(values: list[float], percent: float) -> float:
    """NumPy's linear-interpolation percentile, so the gate matches the oracle."""
    if not values:
        return 0.0
    ordered = sorted(values)
    position = (len(ordered) - 1) * percent / 100.0
    lower = int(math.floor(position))
    upper = min(lower + 1, len(ordered) - 1)
    weight = position - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def excerpt_tempo_bpm(
    measured: dict[str, Any], start_seconds: float, end_seconds: float,
) -> float | None:
    """The argmax BPM alone, which is what the repository's own analysis reports."""
    return excerpt_pulse(measured, start_seconds, end_seconds)["bpm"]


def measured_sections(
    measured: dict[str, Any], start_seconds: float, end_seconds: float,
) -> list[tuple[float, float]]:
    sections = ((measured.get("summary") or {}).get("sections") or [])
    spans: list[tuple[float, float]] = []
    previous_end = 0.0
    for section in sections:
        if not isinstance(section, dict):
            continue
        section_start = float(section.get("start_seconds", previous_end))
        section_end = float(section.get("end_seconds", section_start))
        previous_end = section_end
        if section_end <= start_seconds or section_start >= end_seconds:
            continue
        spans.append((max(section_start, start_seconds), min(section_end, end_seconds)))
    return spans


# ---------------------------------------------------------------------------
# lyric lanes
# ---------------------------------------------------------------------------


def _adjudication_for(slug: str) -> dict[int, dict[str, Any]]:
    if not ADJUDICATION_PATH.is_file():
        return {}
    try:
        document = read_json(ADJUDICATION_PATH)
    except (OSError, json.JSONDecodeError):
        return {}
    entries = ((document.get("tracks") or {}).get(slug) or [])
    indexed: dict[int, dict[str, Any]] = {}
    for entry in entries:
        if isinstance(entry, dict) and isinstance(entry.get("line"), int):
            indexed[entry["line"]] = entry
    return indexed


def lyric_truth(
    aligned: dict[str, Any],
    adjudication: dict[int, dict[str, Any]],
    start_seconds: float,
    end_seconds: float,
) -> list[LyricTruth]:
    """Aligned lines inside the excerpt, with adjudicated onsets taking priority.

    A line the operator adjudicated as unlocatable (``true_start_seconds`` is
    null) is *dropped* rather than kept at its aligned time. Scoring a model
    against a placement the operator refused to certify would import exactly
    the error the adjudication exists to exclude.
    """
    truths: list[LyricTruth] = []
    for line in (aligned.get("lines") or []):
        if not isinstance(line, dict):
            continue
        index = line.get("reference_line_index")
        start = line.get("start_seconds")
        text = (line.get("text") or "").strip()
        if start is None or not text:
            continue
        source = "aligned"
        if isinstance(index, int) and index in adjudication:
            entry = adjudication[index]
            true_start = entry.get("true_start_seconds")
            if true_start is None:
                continue
            start = float(true_start)
            source = "adjudicated"
        start = float(start)
        if start < start_seconds or start >= end_seconds:
            continue
        end = line.get("end_seconds")
        truths.append(LyricTruth(
            reference_line_index=int(index) if isinstance(index, int) else -1,
            text=text,
            start_seconds=start,
            end_seconds=float(end) if isinstance(end, (int, float)) else start,
            source=source,
            review_flagged=bool(line.get("review_flagged")),
        ))
    truths.sort(key=lambda truth: truth.start_seconds)
    return truths


# ---------------------------------------------------------------------------
# operator-authored file
# ---------------------------------------------------------------------------


def authored(path=GROUND_TRUTH_PATH) -> dict[str, Any]:
    if not path.is_file():
        return {"tracks": {}}
    try:
        document = read_json(path)
    except (OSError, json.JSONDecodeError):
        return {"tracks": {}}
    return document if isinstance(document, dict) else {"tracks": {}}


def load(track: Track, *, authored_document: dict[str, Any] | None = None) -> TrackTruth:
    start = track.excerpt_start_seconds
    end = start + EXCERPT_SECONDS
    missing: list[str] = []

    measured_path = track.workspace / "measured.json"
    measured: dict[str, Any] = {}
    if measured_path.is_file():
        try:
            candidate = read_json(measured_path)
            measured = candidate if isinstance(candidate, dict) else {}
        except (OSError, json.JSONDecodeError):
            missing.append(f"unreadable {measured_path}")
    else:
        missing.append(f"missing {measured_path}")

    pulse = measured.get("pulse_estimate") or {}
    excerpt = (excerpt_pulse(measured, start, end) if measured
               else {"bpm": None, "confidence": None, "candidates": []})
    truth = TrackTruth(
        slug=track.slug,
        excerpt_start=start,
        excerpt_end=end,
        measured_bpm=pulse.get("bpm"),
        measured_bpm_confidence=pulse.get("confidence"),
        excerpt_bpm=excerpt["bpm"],
        excerpt_bpm_confidence=excerpt["confidence"],
        excerpt_bpm_candidates=[float(candidate["bpm"])
                                for candidate in excerpt["candidates"]],
        sections=measured_sections(measured, start, end) if measured else [],
        missing_sources=missing,
    )

    aligned_path = track.workspace / "lyrics.aligned.json"
    if aligned_path.is_file():
        try:
            aligned = read_json(aligned_path)
        except (OSError, json.JSONDecodeError):
            aligned = {}
            missing.append(f"unreadable {aligned_path}")
        if isinstance(aligned, dict):
            truth.lyrics = lyric_truth(
                aligned, _adjudication_for(track.slug), start, end)
    else:
        missing.append(f"missing {aligned_path}")

    document = authored_document if authored_document is not None else authored()
    entry = ((document.get("tracks") or {}).get(track.slug) or {})
    key = entry.get("key") or {}
    truth.key_status = str(key.get("status") or "unadjudicated")
    truth.key_tonic = key.get("tonic")
    truth.key_mode = key.get("mode")
    meter = entry.get("meter") or {}
    truth.meter_status = str(meter.get("status") or "unadjudicated")
    truth.meter = meter.get("value")
    instruments = entry.get("instruments") or {}
    truth.instruments_status = str(instruments.get("status") or "unadjudicated")
    truth.instruments_present = list(instruments.get("present") or [])
    truth.instruments_allowed_extra = list(instruments.get("allowed_extra") or [])
    truth.instruments_absent = list(instruments.get("absent") or [])
    truth.missing_sources = missing
    return truth


__all__ = [
    "GROUND_TRUTH_VERSION",
    "LyricTruth",
    "MAX_BPM",
    "MIN_BPM",
    "TrackTruth",
    "authored",
    "excerpt_pulse",
    "excerpt_tempo_bpm",
    "load",
    "lyric_truth",
    "measured_sections",
]
