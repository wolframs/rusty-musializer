#!/usr/bin/env python3
"""Create a deterministic, offline measured-audio analysis sidecar.

Runtime dependencies are Python 3, NumPy, and an ``ffmpeg`` executable.  The
input is decoded to explicitly selected float32 PCM before analysis, so codec
container details never leak into the feature implementation.
"""

from __future__ import annotations

import argparse
import json
import math
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

try:
    import numpy as np
except ImportError as error:  # pragma: no cover - exercised only on incomplete hosts
    raise SystemExit("tools/analyze_audio.py requires NumPy") from error

from analysis_io import (
    AnalysisValidationError,
    atomic_write_json,
    canonical_sha256,
    sha256_file,
)


SCHEMA_VERSION = "musializer.measured-analysis/v1"
ANALYZER_VERSION = "1"
DEFAULT_SAMPLE_RATE = 24000
DEFAULT_CHANNELS = 1
DEFAULT_WINDOW = 2048
DEFAULT_HOP = 1024

# Edges are intentionally fixed and become part of the cache key.  Frequencies
# beyond the selected PCM Nyquist are represented as zero-energy bands.
BANDS: tuple[tuple[str, float, float], ...] = (
    ("sub_bass", 20.0, 60.0),
    ("bass", 60.0, 250.0),
    ("low_mid", 250.0, 500.0),
    ("mid", 500.0, 2000.0),
    ("upper_mid", 2000.0, 4000.0),
    ("presence", 4000.0, 6000.0),
    ("brilliance", 6000.0, 12000.0),
)


def _finite(value: float, low: float = 0.0, high: float = 1.0) -> float:
    """Return a JSON-safe, bounded Python float."""

    value = float(value)
    if not math.isfinite(value):
        return low
    return max(low, min(high, value))


def decode_audio(
    path: str | Path,
    *,
    sample_rate: int = DEFAULT_SAMPLE_RATE,
    channels: int = DEFAULT_CHANNELS,
    ffmpeg: str = "ffmpeg",
) -> np.ndarray:
    """Decode *path* to interleaved little-endian float32 PCM via FFmpeg.

    The returned array has shape ``(sample_count, channels)``.  FFmpeg's
    default deterministic resampler is used with every output parameter made
    explicit.  No shell is involved.
    """

    if sample_rate < 8000 or sample_rate > 192000:
        raise AnalysisValidationError("sample rate must be between 8000 and 192000 Hz")
    if channels not in (1, 2):
        raise AnalysisValidationError("channels must be 1 or 2")
    executable = shutil.which(ffmpeg)
    if executable is None:
        raise AnalysisValidationError(f"FFmpeg executable not found: {ffmpeg}")
    command = [
        executable,
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(path),
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-ac",
        str(channels),
        "-ar",
        str(sample_rate),
        "-f",
        "f32le",
        "-acodec",
        "pcm_f32le",
        "pipe:1",
    ]
    try:
        completed = subprocess.run(command, capture_output=True, check=False)
    except OSError as error:
        raise AnalysisValidationError(f"could not start FFmpeg: {error}") from error
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise AnalysisValidationError(f"FFmpeg decode failed: {detail or 'unknown error'}")
    pcm = np.frombuffer(completed.stdout, dtype="<f4")
    if not pcm.size or pcm.size % channels:
        raise AnalysisValidationError("FFmpeg produced empty or truncated PCM")
    pcm = pcm.reshape((-1, channels)).astype(np.float64)
    if not np.all(np.isfinite(pcm)):
        raise AnalysisValidationError("decoded PCM contains non-finite samples")
    return pcm


def _frame_starts(sample_count: int, hop: int) -> np.ndarray:
    # Always represent the tail, including tracks shorter than one FFT window.
    return np.arange(0, sample_count, hop, dtype=np.int64)


def _robust_unit(values: np.ndarray, percentile: float = 95.0) -> np.ndarray:
    scale = float(np.percentile(values, percentile)) if values.size else 0.0
    if (not math.isfinite(scale) or scale <= 1e-12) and values.size:
        # Sparse onset envelopes commonly have a zero 90th percentile.
        scale = float(np.max(values))
    if not math.isfinite(scale) or scale <= 1e-12:
        return np.zeros_like(values, dtype=np.float64)
    return np.clip(values / scale, 0.0, 1.0)


def _estimate_pulse(onset: np.ndarray, frame_rate: float) -> dict[str, Any]:
    """Conservatively estimate one global pulse from the onset envelope."""

    empty = {"bpm": None, "confidence": 0.0, "phase_seconds": None}
    if onset.size < max(8, int(frame_rate * 4)) or float(np.max(onset)) < 0.1:
        return empty
    centered = onset - float(np.mean(onset))
    energy = float(np.dot(centered, centered))
    if energy <= 1e-9:
        return empty
    min_lag = max(1, int(math.ceil(frame_rate * 60.0 / 190.0)))
    max_lag = min(onset.size - 2, int(math.floor(frame_rate * 60.0 / 55.0)))
    if max_lag <= min_lag:
        return empty
    lags = np.arange(min_lag, max_lag + 1)
    # Only the musically plausible lag range is needed.  Computing these dots
    # directly keeps this O(frames * candidate_lags), not O(frames**2).
    correlations = np.array([
        float(np.dot(centered[:-lag], centered[lag:])) for lag in lags
    ])
    normalized = correlations / np.maximum(
        1e-12, energy * (1.0 - lags / max(1.0, onset.size))
    )
    best_index = int(np.argmax(normalized))
    lag = int(lags[best_index])
    peak = _finite(normalized[best_index])
    # Require both periodicity and enough discrete attacks.  This deliberately
    # returns null for ambiguous ambience rather than hallucinating a tempo.
    attack_count = int(np.count_nonzero(onset >= 0.55))
    confidence = _finite(peak * min(1.0, attack_count / 8.0))
    if confidence < 0.18:
        return empty
    candidates = np.flatnonzero(onset >= max(0.45, float(np.percentile(onset, 80))))
    phase_frame = int(candidates[0]) if candidates.size else int(np.argmax(onset))
    return {
        "bpm": round(60.0 * frame_rate / lag, 6),
        "confidence": confidence,
        "phase_seconds": phase_frame / frame_rate,
    }


def _feature_summary(frames: list[dict[str, Any]]) -> dict[str, Any]:
    if not frames:
        raise AnalysisValidationError("analysis produced no frames")
    return {
        "rms_mean": float(np.mean([frame["rms"] for frame in frames])),
        "rms_max": max(frame["rms"] for frame in frames),
        "peak_max": max(frame["peak"] for frame in frames),
        "centroid_mean": float(np.mean([frame["spectral_centroid"] for frame in frames])),
        "flux_mean": float(np.mean([frame["spectral_flux"] for frame in frames])),
        "onset_max": max(frame["onset_strength"] for frame in frames),
        "bands_mean": {
            name: float(np.mean([frame["bands"][name] for frame in frames]))
            for name, _low, _high in BANDS
        },
    }


def _aggregate_bins(
    frames: list[dict[str, Any]], duration: float, bin_seconds: float
) -> list[dict[str, Any]]:
    bins: list[dict[str, Any]] = []
    cursor = 0.0
    while cursor < duration:
        end = min(duration, cursor + bin_seconds)
        selected = [
            frame
            for frame in frames
            if cursor <= frame["time_seconds"] < end
            or (end == duration and frame["time_seconds"] == duration)
        ]
        if not selected:
            # Sparse/short inputs still receive a complete atlas interval.
            selected = [min(frames, key=lambda item: abs(item["time_seconds"] - cursor))]
        bins.append({
            "start_seconds": cursor,
            "end_seconds": end,
            "features": _feature_summary(selected),
        })
        cursor = end
    return bins


def _sections(frames: list[dict[str, Any]], duration: float) -> list[dict[str, Any]]:
    """Create stable coarse sections, splitting early on strong feature change."""

    target = 12.0
    minimum = 6.0
    boundaries = [0.0]
    coarse = _aggregate_bins(frames, duration, 2.0)
    previous: np.ndarray | None = None
    last = 0.0
    for item in coarse:
        features = item["features"]
        vector = np.array([
            features["rms_mean"],
            features["centroid_mean"],
            features["flux_mean"],
            *features["bands_mean"].values(),
        ])
        change = float(np.linalg.norm(vector - previous) / math.sqrt(vector.size)) if previous is not None else 0.0
        start = float(item["start_seconds"])
        if start - last >= minimum and (start - last >= target or change >= 0.24):
            boundaries.append(start)
            last = start
        previous = vector
    if duration - boundaries[-1] < minimum and len(boundaries) > 1:
        boundaries.pop()
    boundaries.append(duration)
    result = []
    for index, (start, end) in enumerate(zip(boundaries, boundaries[1:])):
        selected = [frame for frame in frames if start <= frame["time_seconds"] < end]
        if not selected:
            selected = [min(frames, key=lambda item: abs(item["time_seconds"] - start))]
        result.append({
            "index": index,
            "start_seconds": start,
            "end_seconds": end,
            "features": _feature_summary(selected),
        })
    return result


def analyze_pcm(
    pcm: np.ndarray,
    *,
    audio_sha256: str,
    sample_rate: int,
    window_size: int = DEFAULT_WINDOW,
    hop_size: int = DEFAULT_HOP,
) -> dict[str, Any]:
    """Analyze decoded PCM and return a validated measured-analysis document."""

    pcm = np.asarray(pcm, dtype=np.float64)
    if pcm.ndim == 1:
        pcm = pcm[:, None]
    if pcm.ndim != 2 or pcm.shape[1] not in (1, 2) or pcm.shape[0] == 0:
        raise AnalysisValidationError("PCM must be a non-empty mono or stereo array")
    if not np.all(np.isfinite(pcm)):
        raise AnalysisValidationError("PCM contains non-finite samples")
    if sample_rate < 8000 or sample_rate > 192000:
        raise AnalysisValidationError("sample rate must be between 8000 and 192000 Hz")
    if window_size < 64 or window_size & (window_size - 1):
        raise AnalysisValidationError("window size must be a power of two of at least 64")
    if hop_size < 1 or hop_size > window_size:
        raise AnalysisValidationError("hop size must be between 1 and window size")
    if len(audio_sha256) != 64 or any(char not in "0123456789abcdef" for char in audio_sha256):
        raise AnalysisValidationError("audio SHA-256 must be 64 lowercase hexadecimal characters")

    channels = int(pcm.shape[1])
    mono = np.mean(pcm, axis=1)
    starts = _frame_starts(mono.size, hop_size)
    taper = np.hanning(window_size)
    frequencies = np.fft.rfftfreq(window_size, 1.0 / sample_rate)
    spectra: list[np.ndarray] = []
    rms_values: list[float] = []
    peak_values: list[float] = []
    centroid_values: list[float] = []
    band_values: list[dict[str, float]] = []
    for start in starts:
        signal = np.zeros(window_size, dtype=np.float64)
        available = min(window_size, mono.size - int(start))
        signal[:available] = mono[int(start) : int(start) + available]
        rms_values.append(_finite(np.sqrt(np.mean(np.square(signal[:available])))))
        peak_values.append(_finite(np.max(np.abs(signal[:available]))))
        magnitude = np.abs(np.fft.rfft(signal * taper))
        total_magnitude = float(np.sum(magnitude))
        normalized_spectrum = magnitude / total_magnitude if total_magnitude > 1e-12 else np.zeros_like(magnitude)
        spectra.append(normalized_spectrum)
        centroid = float(np.dot(frequencies, magnitude) / total_magnitude) if total_magnitude > 1e-12 else 0.0
        centroid_values.append(_finite(centroid / (sample_rate * 0.5)))
        power = np.square(magnitude)
        total_power = float(np.sum(power))
        bands: dict[str, float] = {}
        for name, low, high in BANDS:
            mask = (frequencies >= low) & (frequencies < min(high, sample_rate * 0.5 + 1e-9))
            ratio = float(np.sum(power[mask]) / total_power) if total_power > 1e-18 and np.any(mask) else 0.0
            bands[name] = _finite(math.sqrt(max(0.0, ratio)))
        band_values.append(bands)

    raw_flux = np.zeros(len(spectra), dtype=np.float64)
    for index in range(1, len(spectra)):
        raw_flux[index] = float(np.sum(np.maximum(0.0, spectra[index] - spectra[index - 1])))
    flux = _robust_unit(raw_flux)
    # A local adaptive floor suppresses steady textures.  Keep strength as a
    # continuous scene signal and the boolean as conservative evidence.
    onset = np.zeros_like(flux)
    radius = 4
    for index, value in enumerate(flux):
        local = flux[max(0, index - radius) : index + 1]
        baseline = float(np.median(local))
        onset[index] = max(0.0, float(value) - baseline)
    onset = _robust_unit(onset, 90.0)
    onset_flags = np.zeros(onset.size, dtype=bool)
    for index in range(1, max(1, onset.size - 1)):
        onset_flags[index] = onset[index] >= 0.55 and onset[index] >= onset[index - 1] and onset[index] > onset[index + 1]

    frame_rate = sample_rate / hop_size
    pulse = _estimate_pulse(onset, frame_rate)
    frames: list[dict[str, Any]] = []
    for index, start in enumerate(starts):
        time_seconds = min(float(start) / sample_rate, mono.size / sample_rate)
        pulse_value = 0.0
        if pulse["bpm"] is not None and pulse["phase_seconds"] is not None:
            period = 60.0 / pulse["bpm"]
            distance = (time_seconds - pulse["phase_seconds"]) % period
            distance = min(distance, period - distance)
            pulse_value = math.exp(-0.5 * (distance / max(0.025, period * 0.08)) ** 2) * pulse["confidence"]
        frames.append({
            "time_seconds": time_seconds,
            "rms": rms_values[index],
            "peak": peak_values[index],
            "bands": band_values[index],
            "spectral_centroid": centroid_values[index],
            "spectral_flux": _finite(flux[index]),
            "onset_strength": _finite(onset[index]),
            "onset": bool(onset_flags[index]),
            "pulse": _finite(pulse_value),
        })

    duration = mono.size / sample_rate
    configuration = {
        "analyzer_version": ANALYZER_VERSION,
        "sample_rate": sample_rate,
        "channels": channels,
        "window_size": window_size,
        "hop_size": hop_size,
        "window_function": "hann",
        "band_edges_hz": {name: [low, high] for name, low, high in BANDS},
    }
    cache_material = {
        "schema_version": SCHEMA_VERSION,
        "audio_sha256": audio_sha256,
        "duration_seconds": duration,
        **configuration,
    }
    document = {
        "schema_version": SCHEMA_VERSION,
        "lane": "measured_audio",
        "cache_key": canonical_sha256(cache_material),
        "audio": {
            "sha256": audio_sha256,
            "duration_seconds": duration,
            "analysis_sample_rate": sample_rate,
            "channels": channels,
        },
        "analysis": configuration,
        "provenance": {
            "adapter": "tools/analyze_audio.py",
            "adapter_version": ANALYZER_VERSION,
            "source_kind": "offline_measured_analysis",
            "audio_sha256": audio_sha256,
            "schema_version": SCHEMA_VERSION,
            "model": None,
            "provider": None,
            "prompt_version": None,
            "request_settings": {
                "sample_rate": sample_rate,
                "channels": channels,
                "window_size": window_size,
                "hop_size": hop_size,
            },
            "generation": {},
        },
        "pulse_estimate": pulse,
        "frames": frames,
        "summary": {
            "global": _feature_summary(frames),
            "resolutions": [
                {"bin_seconds": seconds, "bins": _aggregate_bins(frames, duration, seconds)}
                for seconds in (1.0, 4.0, 16.0)
            ],
            "sections": _sections(frames, duration),
        },
    }
    validate_document(document)
    return document


def validate_document(document: dict[str, Any]) -> None:
    """Enforce cross-field invariants not expressible in portable JSON Schema."""

    try:
        duration = float(document["audio"]["duration_seconds"])
        frames = document["frames"]
        sections = document["summary"]["sections"]
        serialized = json.dumps(document, allow_nan=False)
    except (KeyError, TypeError, ValueError) as error:
        raise AnalysisValidationError(f"invalid measured analysis: {error}") from error
    if not serialized or not frames or duration <= 0:
        raise AnalysisValidationError("measured analysis must contain finite frames and positive duration")
    times = [frame["time_seconds"] for frame in frames]
    if times != sorted(times) or times[0] < 0 or times[-1] > duration:
        raise AnalysisValidationError("frame times must be ordered within the audio duration")
    bounded = ("rms", "peak", "spectral_centroid", "spectral_flux", "onset_strength", "pulse")
    for frame in frames:
        values = [frame[name] for name in bounded] + list(frame["bands"].values())
        if any(not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0 or value > 1 for value in values):
            raise AnalysisValidationError("frame features must be finite normalized numbers")
    if not sections or sections[0]["start_seconds"] != 0 or sections[-1]["end_seconds"] != duration:
        raise AnalysisValidationError("sections must cover the complete audio duration")
    for left, right in zip(sections, sections[1:]):
        if left["end_seconds"] != right["start_seconds"]:
            raise AnalysisValidationError("sections must be contiguous")


def analyze_file(
    audio_path: str | Path,
    *,
    sample_rate: int = DEFAULT_SAMPLE_RATE,
    channels: int = DEFAULT_CHANNELS,
    window_size: int = DEFAULT_WINDOW,
    hop_size: int = DEFAULT_HOP,
    ffmpeg: str = "ffmpeg",
) -> dict[str, Any]:
    before = Path(audio_path).stat()
    source_hash = sha256_file(audio_path)
    pcm = decode_audio(audio_path, sample_rate=sample_rate, channels=channels, ffmpeg=ffmpeg)
    after = Path(audio_path).stat()
    signature = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns)
    if signature(before) != signature(after):
        raise AnalysisValidationError("audio file changed during decoding")
    return analyze_pcm(
        pcm,
        audio_sha256=source_hash,
        sample_rate=sample_rate,
        window_size=window_size,
        hop_size=hop_size,
    )


def analyze_to_cache(audio_path: str | Path, output_path: str | Path, **settings: Any) -> dict[str, Any]:
    """Analyze fully, validate, then atomically replace *output_path*."""

    document = analyze_file(audio_path, **settings)
    validate_document(document)
    atomic_write_json(output_path, document)
    return document


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audio", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--sample-rate", type=int, default=DEFAULT_SAMPLE_RATE)
    parser.add_argument("--channels", type=int, choices=(1, 2), default=DEFAULT_CHANNELS)
    parser.add_argument("--window", type=int, default=DEFAULT_WINDOW)
    parser.add_argument("--hop", type=int, default=DEFAULT_HOP)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    args = parser.parse_args(argv)
    try:
        document = analyze_to_cache(
            args.audio,
            args.output,
            sample_rate=args.sample_rate,
            channels=args.channels,
            window_size=args.window,
            hop_size=args.hop,
            ffmpeg=args.ffmpeg,
        )
    except (OSError, AnalysisValidationError) as error:
        print(f"Measured analysis failed: {error}", file=sys.stderr)
        return 1
    print(
        f"analyzed {document['audio']['duration_seconds']:.3f}s into "
        f"{len(document['frames'])} frames ({document['cache_key']})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
