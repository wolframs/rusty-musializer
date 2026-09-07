#!/usr/bin/env python3
"""CTC alignment with a separate allowed time window for every target token.

An emission score alone cannot distinguish repeated deliveries. Position-specific
windows preserve independently observed phrase locations even when two phrases
contain identical characters. This module is pure NumPy; it opens no audio device.
"""
from __future__ import annotations

import numpy as np


def align(log_probabilities, targets, windows, *, blank: int = 0):
    """Return one ``(first_frame, end_frame, mean_probability)`` per token.

    Windows are half-open frame ranges, or None for unconstrained wildcard
    tokens. A blank frame is required between identical consecutive labels.
    An impossible constrained path raises ValueError; it never becomes guessed
    timings. All frames must be explained, including leading/trailing blanks.
    """
    emissions = np.asarray(log_probabilities, dtype=np.float64)
    raw_labels = np.asarray(targets)
    if raw_labels.ndim != 1 or raw_labels.dtype.kind not in "iu":
        raise ValueError("CTC targets must be a one-dimensional integer sequence")
    labels = raw_labels.astype(np.int64)
    if emissions.ndim != 2 or not emissions.shape[0] or not len(labels):
        raise ValueError('CTC needs nonempty frame probabilities and targets')
    if np.isnan(emissions).any() or np.isposinf(emissions).any():
        raise ValueError('CTC emissions contain NaN or positive infinity')
    if len(windows) != len(labels) or blank < 0 or blank >= emissions.shape[1]:
        raise ValueError('CTC windows/blank do not match the token graph')
    if np.any(labels < 0) or np.any(labels >= emissions.shape[1]) or np.any(labels == blank):
        raise ValueError('CTC target contains a blank or invalid label')
    frames = emissions.shape[0]
    states = 2 * len(labels) + 1
    if frames * states > 25_000_000:
        raise ValueError("CTC trellis exceeds the short-clip memory limit")
    state_labels = np.full(states, blank, dtype=np.int64)
    state_labels[1::2] = labels
    low, high = np.zeros(states, dtype=np.int64), np.full(states, frames, dtype=np.int64)
    for i, window in enumerate(windows):
        if window is not None:
            first, end = window
            if not 0 <= first < end <= frames:
                raise ValueError('CTC target has an empty or out-of-range window')
            low[2 * i + 1], high[2 * i + 1] = first, end
    skip = np.zeros(states, dtype=bool)
    skip[2:] = (state_labels[2:] != blank) & (state_labels[2:] != state_labels[:-2])
    previous = np.full(states, -np.inf)
    previous[0] = 0
    back = np.zeros((frames, states), dtype=np.uint8)
    for frame in range(frames):
        one = np.r_[-np.inf, previous[:-1]]
        two = np.r_[-np.inf, -np.inf, previous[:-2]]
        two[~skip] = -np.inf
        candidates = np.stack((previous, one, two))
        choice = candidates.argmax(axis=0)
        current = candidates[choice, np.arange(states)] + emissions[frame, state_labels]
        current[(frame < low) | (frame >= high)] = -np.inf
        back[frame] = choice
        previous = current
    state = states - 1 if previous[-1] >= previous[-2] else states - 2
    if not np.isfinite(previous[state]):
        raise ValueError('No CTC path fits the observed phrase windows')
    assigned = [[] for _ in labels]
    for frame in range(frames - 1, -1, -1):
        if state % 2:
            assigned[state // 2].append(frame)
        state -= int(back[frame, state])
    spans = []
    for label, indices in zip(labels, assigned):
        if not indices:
            raise ValueError('CTC path did not visit every target token')
        spans.append((min(indices), max(indices) + 1,
                      float(np.exp(emissions[indices, label]).mean())))
    return spans
