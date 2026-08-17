#!/usr/bin/env python3
"""Deterministic alignment of known reference lyrics to Whisper word evidence.

This module is the timing core of the "sync known lyrics" assist mode. It is
intentionally dependency-free and fully deterministic: the display text always
comes verbatim from the authored reference lines, timing always comes from
acoustic evidence (Whisper word timestamps) through a monotonic global
alignment, and everything the aligner is unsure about is flagged for human
review instead of guessed silently.

Pipeline stages, all pure functions:

1. ``classify_reference_lines`` splits authored text into lyric, backing
   lyric, section heading, sound event, and delivery instruction lines.
2. ``normalize_tokens`` converts display text into pronunciation-oriented
   alignment tokens (case/punctuation folding, stutter splitting, number
   verbalization, a small technical-vocabulary pronunciation table).
3. ``flag_unreliable_intervals`` detects Whisper repetition-loop
   hallucinations so garbage evidence cannot anchor or bridge timing.
4. ``align_tokens`` runs a global monotonic alignment (Needleman-Wunsch over
   token similarity) between reference tokens and evidence words.
5. ``sync_lyrics`` assembles the ``musializer.lyric-sync/v1`` document:
   per-line windows from matched words, bounded interpolation only across
   short trusted gaps, and explicit unmatched reporting.

The evidence is untrusted data. Authored cues always keep authored display
text, while plausible words heard outside every authored placement are kept in
a separate ``performed_candidates`` review lane. They are never promoted to
normal cues automatically.
"""

from __future__ import annotations

import re
import unicodedata
from typing import Any, Sequence

from analysis_io import AnalysisValidationError, duration

LYRIC_SYNC_VERSION = "musializer.lyric-sync/v1"
ALIGNER_VERSION = "5"

# Hard input bounds. A full-length song is a few hundred lines and well under
# a thousand words; these caps keep the O(ref * hyp) alignment matrix small
# enough for pure Python while rejecting absurd inputs loudly.
MAX_REFERENCE_BYTES = 64 * 1024
MAX_REFERENCE_LINES = 1024
MAX_REFERENCE_TOKENS = 4000
MAX_EVIDENCE_TOKENS = 8000

# A gap between trusted neighbors wider than this is a coverage hole, not an
# interpolation opportunity: the line is reported unmatched for review.
MAX_INTERPOLATION_GAP_SECONDS = 10.0
# Real lyrics legitimately repeat an identical short line a few times in a
# row ("ctrl+z ctrl+z", "seconds seconds seconds"); Whisper hallucination
# loops repeat one segment dozens of times across a long stretch. Both
# thresholds must hold before evidence is discarded as a loop.
LOOP_MINIMUM_REPEATS = 4
LOOP_MINIMUM_SECONDS = 12.0
# A directly timed line needs at least this share of its tokens matched (or
# three absolute matches) before its window is trusted.
DIRECT_MATCH_MINIMUM_RATIO = 1.0 / 3.0
DIRECT_MATCH_STRONG_HITS = 3
# A line's matching words must form one acoustic phrase. Whisper can emit a
# correct word during an instrumental intro and then recognize the actual line
# twenty seconds later; taking the min/max of every match turned that one
# outlier into a twenty-second caption. A pause longer than this starts a new
# candidate cluster, and only the best-supported cluster times the line.
DIRECT_MATCH_MAX_WORD_GAP_SECONDS = 5.0
# When a global match is discarded as a remote outlier, a locally repeated
# occurrence of that same token can replace it only this close to the retained
# phrase. This recovers a real line-initial word without reopening the global
# repeated-chorus ambiguity.
DIRECT_MATCH_RECOVERY_MARGIN_SECONDS = 2.0
DIRECT_MATCH_NEARBY_WORD_MARGIN_SECONDS = 0.75
# Below this coverage a timed line keeps its window but is flagged uncertain.
CONFIDENT_MATCH_RATIO = 0.6
MINIMUM_CUE_SECONDS = 0.5
# The C editor caps cue text at 512 bytes including the terminator and the
# caption renderer shows at most three wrapped lines; authored lines beyond
# this are flagged for review instead of being truncated silently.
MAX_CUE_TEXT_BYTES = 480
# Whisper frequently hears real ad-libs, backing vocals and generated repeats
# that are absent from an authored Suno sheet. Preserve short unmatched spans
# for review, but refuse the long, low-information segments characteristic of
# Whisper's instrumental-tail hallucinations.
PERFORMED_CANDIDATE_MAX_SECONDS = 15.0
PERFORMED_CANDIDATE_MIN_SECONDS = 0.15
# Whisper segments do not share lyric-line boundaries: a real authored line can
# occupy only the latter third of one segment. A one-third overlap is therefore
# enough to call the segment explained; truly extra spans in the Groyper
# regression have no authored overlap at all.
PERFORMED_COVERED_OVERLAP_RATIO = 0.35

_SMALL_NUMBERS = (
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
    "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen",
    "sixteen", "seventeen", "eighteen", "nineteen",
)
_TENS = ("", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy",
         "eighty", "ninety")

# Written technical vocabulary versus how it is sung. Keys are already
# lowercase; values are space-separated replacement tokens.
_PRONUNCIATION = {
    "ctrl": "control",
    "cmd": "command",
    "esc": "escape",
    "alt": "alt",
    "&": "and",
    "%": "percent",
}

# Small equivalence groups for systematic mishearings that survive
# normalization. Scored below an exact match so real matches win.
_EQUIVALENT_TOKENS = (
    {"sudo", "pseudo"},
    {"z", "v"},
    {"ok", "okay"},
)
_EQUIVALENCE_INDEX: dict[str, set[str]] = {}
for _group in _EQUIVALENT_TOKENS:
    for _token in _group:
        _EQUIVALENCE_INDEX[_token] = _group

# Words that mark a parenthetical as a delivery/performance instruction
# rather than a sung backing line.
_DELIVERY_MARKERS = frozenset({
    "voice", "voices", "vocal", "vocals", "spoken", "whispered", "whisper",
    "shouted", "screamed", "sung", "singing", "counting", "storytelling",
    "layered", "pitched", "chipper", "bouncy", "relieved", "soft", "softly",
    "quiet", "quieter", "sweet", "cute", "beat", "pause", "silence",
    "instrumental", "glitching", "malfunctioning", "adorable", "peppy",
    "unrepentant", "excitement", "energy", "intro", "verse", "prechorus",
    "chorus", "refrain", "bridge", "breakdown", "interlude", "outro", "coda",
})


def _number_words(token: str) -> list[str]:
    if not token.isdigit():
        return [token]
    value = int(token)
    if value < 20:
        return [_SMALL_NUMBERS[value]]
    if value < 100:
        tens, unit = divmod(value, 10)
        return [_TENS[tens]] + ([_SMALL_NUMBERS[unit]] if unit else [])
    if value == 100:
        return ["one", "hundred"]
    # Long digit strings are sung digit by digit more often than not.
    return [word for digit in token for word in (_SMALL_NUMBERS[int(digit)],)]


def normalize_tokens(text: str) -> list[str]:
    """Pronunciation-oriented tokens for alignment; never shown to users."""
    folded = unicodedata.normalize("NFKC", text).lower()
    folded = folded.replace("ctrl+z", " control z ")
    folded = re.sub(r"(\d+)\.(\d+)", r"\1 point \2", folded)
    folded = re.sub(r"[-‐‑‒–—―_/+]", " ", folded)
    folded = re.sub(r"[^a-z0-9' ]+", " ", folded)
    tokens: list[str] = []
    for raw in folded.split():
        raw = raw.strip("'")
        if not raw:
            continue
        raw = _PRONUNCIATION.get(raw, raw)
        for piece in raw.split():
            stripped = re.sub(r"(?:'d|'s|'re|'ll|'ve|'m|n't)$", "", piece)
            piece = stripped or piece
            if piece.isdigit():
                tokens.extend(_number_words(piece))
            elif piece:
                tokens.append(piece)
    return tokens


def token_similarity(a: str, b: str) -> float:
    """Similarity in [0, 1]; 0 means the tokens must not be paired."""
    if a == b:
        return 1.0
    group = _EQUIVALENCE_INDEX.get(a)
    if group is not None and b in group:
        return 0.8
    if abs(len(a) - len(b)) > 3:
        return 0.0
    length_a, length_b = len(a), len(b)
    previous = list(range(length_b + 1))
    for i in range(1, length_a + 1):
        current = [i] + [0] * length_b
        for j in range(1, length_b + 1):
            current[j] = min(
                previous[j] + 1,
                current[j - 1] + 1,
                previous[j - 1] + (a[i - 1] != b[j - 1]),
            )
        previous = current
    similarity = 1.0 - previous[length_b] / max(length_a, length_b)
    return similarity if similarity >= 0.6 else 0.0


def classify_reference_lines(text: str) -> list[dict[str, Any]]:
    """Split authored lyrics into classified lines.

    Returns one entry per non-blank source line:
    ``{"index", "kind", "display", "tokens"}`` where kind is one of
    ``lyric``, ``backing``, ``section``, ``event``, ``delivery``. Only
    ``lyric`` and ``backing`` lines carry alignment tokens.
    """
    if len(text.encode("utf-8", errors="replace")) > MAX_REFERENCE_BYTES:
        raise AnalysisValidationError("reference lyrics exceed the size bound")
    lines: list[dict[str, Any]] = []
    source_lines = text.replace("﻿", "").splitlines()
    if len(source_lines) > MAX_REFERENCE_LINES:
        raise AnalysisValidationError("reference lyrics exceed the line bound")
    for index, raw in enumerate(source_lines):
        stripped = raw.strip()
        if not stripped:
            continue
        if re.fullmatch(r"\[.*\]", stripped):
            lines.append({"index": index, "kind": "section",
                          "display": stripped, "tokens": []})
            continue
        if re.fullmatch(r"\*.*\*", stripped):
            lines.append({"index": index, "kind": "event",
                          "display": stripped, "tokens": []})
            continue
        body = stripped
        kind = "lyric"
        enclosed = re.fullmatch(r"\((.*)\)", stripped)
        if enclosed:
            kind = "parenthetical"
            body = enclosed.group(1).strip()
        else:
            mixed = re.match(r"^\((.*?)\)\s+(\S.*)$", stripped)
            if mixed:
                body = mixed.group(2).strip()
        lines.append({"index": index, "kind": kind,
                      "display": stripped, "tokens": normalize_tokens(body)})
    _resolve_parentheticals(lines)
    return lines


def _resolve_parentheticals(lines: list[dict[str, Any]]) -> None:
    for position, line in enumerate(lines):
        if line["kind"] != "parenthetical":
            continue
        tokens = line["tokens"]
        token_set = set(tokens)
        if not token_set:
            line["kind"] = "delivery"
            line["tokens"] = []
            continue
        if token_set & _DELIVERY_MARKERS:
            line["kind"] = "delivery"
            line["tokens"] = []
            continue
        neighbors: set[str] = set()
        for other in lines[max(0, position - 2):position + 3]:
            if other is not line and other["kind"] in ("lyric", "parenthetical"):
                neighbors.update(other["tokens"])
        shared = len(token_set & neighbors)
        vocalization = len(token_set) <= 2 and len(tokens) >= 2
        if shared / len(token_set) >= 0.5 or vocalization:
            line["kind"] = "backing"
        else:
            line["kind"] = "delivery"
            line["tokens"] = []


def flag_unreliable_intervals(
    evidence_lines: Sequence[dict[str, Any]],
) -> list[tuple[float, float]]:
    """Time intervals covered by repetition-loop hallucinations."""
    runs: list[tuple[float, float]] = []

    def close_run(start_position: int, end_position: int, repeats: int) -> None:
        if repeats < LOOP_MINIMUM_REPEATS:
            return
        start = float(evidence_lines[start_position]["start_seconds"])
        end = float(evidence_lines[end_position]["end_seconds"])
        if end - start >= LOOP_MINIMUM_SECONDS:
            runs.append((start, end))

    run_start = 0
    previous_key: str | None = None
    count = 0
    for position, line in enumerate(evidence_lines):
        key = " ".join(normalize_tokens(str(line.get("text", ""))))
        if key and key == previous_key:
            count += 1
        else:
            if previous_key:
                close_run(run_start, position - 1, count)
            previous_key = key
            run_start = position
            count = 1
    if previous_key:
        close_run(run_start, len(evidence_lines) - 1, count)
    return runs


def _inside(moment: float, intervals: Sequence[tuple[float, float]]) -> bool:
    return any(start <= moment <= end for start, end in intervals)


def align_tokens(
    reference: Sequence[str], evidence: Sequence[str],
) -> list[tuple[int, int]]:
    """Monotonic (reference index, evidence index) pairs, best global score."""
    ref_count, hyp_count = len(reference), len(evidence)
    if ref_count > MAX_REFERENCE_TOKENS:
        raise AnalysisValidationError("reference lyrics exceed the token bound")
    if hyp_count > MAX_EVIDENCE_TOKENS:
        raise AnalysisValidationError("whisper evidence exceeds the token bound")
    gap = -0.4
    mismatch = -1.2
    score = [[0.0] * (hyp_count + 1) for _ in range(ref_count + 1)]
    back = [[0] * (hyp_count + 1) for _ in range(ref_count + 1)]
    for i in range(1, ref_count + 1):
        score[i][0] = score[i - 1][0] + gap
        back[i][0] = 1
    for j in range(1, hyp_count + 1):
        score[0][j] = score[0][j - 1] + gap
        back[0][j] = 2
    for i in range(1, ref_count + 1):
        token = reference[i - 1]
        row, above, back_row = score[i], score[i - 1], back[i]
        for j in range(1, hyp_count + 1):
            similarity = token_similarity(token, evidence[j - 1])
            best = above[j - 1] + (2.0 * similarity - 1.0
                                   if similarity > 0.0 else mismatch)
            direction = 0
            if above[j] + gap > best:
                best, direction = above[j] + gap, 1
            if row[j - 1] + gap > best:
                best, direction = row[j - 1] + gap, 2
            row[j] = best
            back_row[j] = direction
    pairs: list[tuple[int, int]] = []
    i, j = ref_count, hyp_count
    while i > 0 or j > 0:
        direction = back[i][j]
        if direction == 0 and i > 0 and j > 0:
            if token_similarity(reference[i - 1], evidence[j - 1]) > 0.0:
                pairs.append((i - 1, j - 1))
            i -= 1
            j -= 1
        elif direction == 1 or j == 0:
            i -= 1
        else:
            j -= 1
    pairs.reverse()
    return pairs


def _evidence_words(document: dict[str, Any]) -> list[dict[str, Any]]:
    words = document.get("words")
    if isinstance(words, list) and words:
        return words
    # Without word timing (no DTW support for the model), fall back to the
    # coarser line intervals; every token of a line shares its window.
    lines = document.get("lines")
    return lines if isinstance(lines, list) else []


def _coherent_match_cluster(
    windows: Sequence[tuple[float, float]],
) -> list[tuple[float, float]]:
    """Return the densest time-coherent cluster of matches for one line."""
    if not windows:
        return []
    ordered = sorted(windows)
    clusters: list[list[tuple[float, float]]] = [[ordered[0]]]
    for window in ordered[1:]:
        if window[0] - clusters[-1][-1][1] > DIRECT_MATCH_MAX_WORD_GAP_SECONDS:
            clusters.append([])
        clusters[-1].append(window)

    # More corroborating words wins. For equal support, prefer the tighter
    # phrase; stable input order then makes an exact tie choose the earlier one.
    return min(
        clusters,
        key=lambda cluster: (
            -len(cluster),
            cluster[-1][1] - cluster[0][0],
            cluster[0][0],
        ),
    )


def sync_lyrics(
    reference_text: str,
    whisper_document: dict[str, Any],
    *,
    audio_duration: float,
) -> dict[str, Any]:
    """Build a ``musializer.lyric-sync/v1`` document.

    ``reference_text`` is the authored lyrics (display truth). Timing comes
    exclusively from the Whisper evidence document. The caller supplies
    ``audio`` identity and provenance afterwards.
    """
    audio_duration = duration(audio_duration)
    if whisper_document.get("schema_version") != "musializer.lyric-timing/v1":
        raise AnalysisValidationError(
            "lyric sync requires musializer.lyric-timing/v1 evidence")
    classified = classify_reference_lines(reference_text)
    timeable = [line for line in classified
                if line["kind"] in ("lyric", "backing") and line["tokens"]]
    if not timeable:
        raise AnalysisValidationError(
            "reference lyrics contain no alignable lyric lines")

    unreliable = flag_unreliable_intervals(
        whisper_document.get("lines") or [])

    reference_tokens: list[str] = []
    token_owner: list[int] = []
    for position, line in enumerate(timeable):
        for token in line["tokens"]:
            reference_tokens.append(token)
            token_owner.append(position)

    evidence_tokens: list[str] = []
    evidence_windows: list[tuple[float, float]] = []
    for word in _evidence_words(whisper_document):
        start = float(word.get("start_seconds", 0.0))
        end = float(word.get("end_seconds", 0.0))
        midpoint = (start + end) / 2.0
        if _inside(midpoint, unreliable):
            continue
        for token in normalize_tokens(str(word.get("text", ""))):
            evidence_tokens.append(token)
            evidence_windows.append((start, end))

    pairs = align_tokens(reference_tokens, evidence_tokens)

    starts: list[float | None] = [None] * len(timeable)
    ends: list[float | None] = [None] * len(timeable)
    matches: list[list[tuple[int, int]]] = [[] for _ in timeable]
    for ref_index, hyp_index in pairs:
        owner = token_owner[ref_index]
        matches[owner].append((ref_index, hyp_index))

    hits = [0] * len(timeable)
    discarded_outlier_tokens = 0
    recovered_outlier_tokens = 0
    recovered_nearby_tokens = 0
    globally_used_evidence = {hyp_index for _, hyp_index in pairs}
    for position, match_indices in enumerate(matches):
        windows = [evidence_windows[hyp_index]
                   for _, hyp_index in match_indices]
        cluster = _coherent_match_cluster(windows)
        if not cluster:
            continue
        remaining_cluster = list(cluster)
        discarded: list[tuple[int, int]] = []
        for ref_index, hyp_index in match_indices:
            window = evidence_windows[hyp_index]
            if window in remaining_cluster:
                remaining_cluster.remove(window)
            else:
                discarded.append((ref_index, hyp_index))

        # Global alignment can spend a reference token on a remote false
        # occurrence. After that match is rejected, recover only an otherwise
        # unused occurrence of the same token immediately beside the retained
        # phrase. This is deliberately local; searching the whole song again
        # would recreate the repeated-chorus ambiguity the global pass solved.
        cluster_start = min(start for start, _ in cluster)
        cluster_end = max(end for _, end in cluster)
        recovered_for_line = 0
        for ref_index, _ in discarded:
            ref_token = reference_tokens[ref_index]
            candidates: list[tuple[float, float, int]] = []
            for hyp_index, (hyp_token, window) in enumerate(
                zip(evidence_tokens, evidence_windows)
            ):
                if hyp_index in globally_used_evidence:
                    continue
                start, end = window
                if (end < cluster_start - DIRECT_MATCH_RECOVERY_MARGIN_SECONDS or
                        start > cluster_end + DIRECT_MATCH_RECOVERY_MARGIN_SECONDS):
                    continue
                similarity = token_similarity(ref_token, hyp_token)
                if similarity <= 0.0:
                    continue
                distance = (cluster_start - end if end < cluster_start else
                            start - cluster_end if start > cluster_end else 0.0)
                candidates.append((-similarity, distance, hyp_index))
            if not candidates:
                continue
            _, _, recovered_index = min(candidates)
            globally_used_evidence.add(recovered_index)
            cluster.append(evidence_windows[recovered_index])
            recovered_outlier_tokens += 1
            recovered_for_line += 1

        discarded_outlier_tokens += len(discarded) - recovered_for_line

        # A substituted or badly recognized boundary word may have no lexical
        # similarity at all (authored "skys", heard "stars"). If an otherwise
        # unused evidence word directly touches an incomplete matched phrase,
        # include its acoustic window. Words already claimed by any aligned
        # line are excluded, so this cannot steal the last word of a neighbor.
        missing_slots = max(0, len(timeable[position]["tokens"]) - len(cluster))
        for _ in range(missing_slots):
            cluster_start = min(start for start, _ in cluster)
            cluster_end = max(end for _, end in cluster)
            nearby: list[tuple[float, int]] = []
            for hyp_index, (start, end) in enumerate(evidence_windows):
                if hyp_index in globally_used_evidence:
                    continue
                if end <= cluster_start:
                    gap = cluster_start - end
                elif start >= cluster_end:
                    gap = start - cluster_end
                else:
                    gap = 0.0
                if gap <= DIRECT_MATCH_NEARBY_WORD_MARGIN_SECONDS:
                    nearby.append((gap, hyp_index))
            if not nearby:
                break
            _, recovered_index = min(nearby)
            globally_used_evidence.add(recovered_index)
            cluster.append(evidence_windows[recovered_index])
            recovered_nearby_tokens += 1
        starts[position] = min(start for start, _ in cluster)
        ends[position] = max(end for _, end in cluster)
        hits[position] = len(cluster)

    # Demote weak direct matches to interpolation candidates.
    for position, line in enumerate(timeable):
        total = len(line["tokens"])
        if starts[position] is None:
            continue
        ratio = hits[position] / total
        if ratio < DIRECT_MATCH_MINIMUM_RATIO and hits[position] < DIRECT_MATCH_STRONG_HITS:
            starts[position] = None
            ends[position] = None
            hits[position] = 0

    estimated = _interpolate_gaps(timeable, starts, ends, unreliable)

    matched_lines = []
    unmatched_lines = []
    for position, line in enumerate(timeable):
        total = len(line["tokens"])
        if len(line["display"].encode("utf-8")) > MAX_CUE_TEXT_BYTES:
            unmatched_lines.append({
                "reference_line_index": line["index"],
                "text": line["display"][:120],
                "reason": "line too long for one caption",
            })
            continue
        if starts[position] is None:
            unmatched_lines.append({
                "reference_line_index": line["index"],
                "text": line["display"],
                "reason": ("no trusted evidence nearby"
                           if _nearest_gap_is_unbridgeable(position, starts)
                           else "not found in evidence"),
            })
            continue
        start = max(0.0, min(float(starts[position]), audio_duration))
        end = max(start, min(float(ends[position]), audio_duration))
        if end - start < MINIMUM_CUE_SECONDS:
            end = min(audio_duration, start + MINIMUM_CUE_SECONDS)
        if end <= start:
            unmatched_lines.append({
                "reference_line_index": line["index"],
                "text": line["display"],
                "reason": "window collapsed at the audio boundary",
            })
            continue
        ratio = hits[position] / total
        is_estimated = position in estimated
        matched_lines.append({
            "start_seconds": start,
            "end_seconds": end,
            "text": line["display"],
            "kind": line["kind"],
            "reference_line_index": line["index"],
            "confidence": None if is_estimated else round(ratio, 4),
            "estimated": is_estimated,
            "uncertain": is_estimated or ratio < CONFIDENT_MATCH_RATIO,
        })

    matched_lines.sort(key=lambda line: (line["start_seconds"],
                                         line["end_seconds"],
                                         line["reference_line_index"]))
    performed_candidates = find_performed_candidates(
        whisper_document, matched_lines, unreliable,
        audio_duration=audio_duration)
    structure = [{"reference_line_index": line["index"], "kind": line["kind"],
                  "text": line["display"]}
                 for line in classified
                 if line["kind"] in ("section", "event", "delivery")]
    return {
        "schema_version": LYRIC_SYNC_VERSION,
        "lane": "lyric_sync",
        "aligner_version": ALIGNER_VERSION,
        "lines": matched_lines,
        "performed_candidates": performed_candidates,
        "unmatched": unmatched_lines,
        "structure": structure,
        "unreliable_evidence": [
            {"start_seconds": start, "end_seconds": end}
            for start, end in unreliable
        ],
        "statistics": {
            "reference_lines": len(timeable),
            "matched_lines": len(matched_lines),
            "estimated_lines": len(estimated),
            "unmatched_lines": len(unmatched_lines),
            "performed_candidates": len(performed_candidates),
            "matched_tokens": sum(hits),
            "discarded_outlier_tokens": discarded_outlier_tokens,
            "recovered_outlier_tokens": recovered_outlier_tokens,
            "recovered_nearby_tokens": recovered_nearby_tokens,
            "reference_tokens": len(reference_tokens),
        },
    }


def find_performed_candidates(
    whisper_document: dict[str, Any],
    authored_placements: Sequence[dict[str, Any]],
    unreliable: Sequence[tuple[float, float]],
    *,
    audio_duration: float,
) -> list[dict[str, Any]]:
    """Return short ASR spans not already explained by an authored cue.

    These are deliberately *candidates*, not truth. Suno can add vocals the
    prompt never contained, while Whisper can hallucinate speech over music.
    Keeping the spans as Potential cues lets a human promote the former without
    letting either class render or export silently.
    """
    words = whisper_document.get("words") or []
    candidates: list[dict[str, Any]] = []
    for line in whisper_document.get("lines") or []:
        try:
            start = max(0.0, float(line.get("start_seconds")))
            end = min(audio_duration, float(line.get("end_seconds")))
        except (TypeError, ValueError):
            continue
        span = end - start
        text = str(line.get("text") or "").strip()
        if (span < PERFORMED_CANDIDATE_MIN_SECONDS or
                span > PERFORMED_CANDIDATE_MAX_SECONDS or
                not normalize_tokens(text) or
                len(text.encode("utf-8")) > MAX_CUE_TEXT_BYTES):
            continue
        if any(max(start, low) < min(end, high) for low, high in unreliable):
            continue
        covered = any(
            max(0.0, min(end, float(cue["end_seconds"])) -
                max(start, float(cue["start_seconds"]))) / span
            >= PERFORMED_COVERED_OVERLAP_RATIO
            for cue in authored_placements
        )
        if covered:
            continue
        confidences = []
        for word in words:
            try:
                word_start = float(word.get("start_seconds"))
                word_end = float(word.get("end_seconds"))
                confidence = word.get("confidence")
                if (max(start, word_start) < min(end, word_end) and
                        confidence is not None):
                    confidences.append(float(confidence))
            except (TypeError, ValueError):
                continue
        candidates.append({
            "start_seconds": start,
            "end_seconds": end,
            "text": text,
            "confidence": (None if not confidences else
                           round(sum(confidences) / len(confidences), 4)),
            "source": "whisper-unmatched",
            "uncertain": True,
            "reason": "heard outside every authored lyric placement",
        })
    return candidates


# Display bounds for transcription-review cues. Roughly two wrapped caption
# lines and a readable dwell time; anything larger is split deterministically.
REVIEW_MAX_CUE_CHARS = 90
REVIEW_MAX_CUE_SECONDS = 7.0
_SENTENCE_DELIMITERS = ".!?;:—"
_CLAUSE_DELIMITERS = ","


def split_long_cues(
    lines: Sequence[dict[str, Any]],
    words: Sequence[dict[str, Any]],
    *,
    max_chars: int = REVIEW_MAX_CUE_CHARS,
    max_seconds: float = REVIEW_MAX_CUE_SECONDS,
) -> list[dict[str, Any]]:
    """Split oversized cues into readable pieces, deterministically.

    Piece timing is proportional to character share, snapped to the nearest
    gap between evidence words when one is close. Pieces inherit their
    parent's citations, confidence, and uncertainty; a cue that cannot be
    split (one enormous token) is kept but flagged uncertain.
    """
    gaps = _word_gap_moments(words)
    result: list[dict[str, Any]] = []
    for line in lines:
        result.extend(_split_cue(dict(line), gaps, max_chars, max_seconds))
    return result


def _word_gap_moments(words: Sequence[dict[str, Any]]) -> list[float]:
    moments: list[float] = []
    ordered = sorted(
        (float(word.get("start_seconds", 0.0)),
         float(word.get("end_seconds", 0.0)))
        for word in words
    )
    for (_, previous_end), (next_start, _) in zip(ordered, ordered[1:]):
        if next_start >= previous_end:
            moments.append((previous_end + next_start) / 2.0)
    return moments


def _split_position(text: str) -> int | None:
    """Best split index (start of the right piece), balanced around center."""
    center = len(text) / 2.0
    window_low = int(len(text) * 0.15)
    window_high = int(len(text) * 0.85)
    for delimiters in (_SENTENCE_DELIMITERS, _CLAUSE_DELIMITERS, None):
        best: tuple[float, int] | None = None
        for match in re.finditer(r"\s+", text):
            position = match.start()
            if not window_low <= position <= window_high:
                continue
            if delimiters is not None:
                if position == 0 or text[position - 1] not in delimiters:
                    continue
            distance = abs(position - center)
            if best is None or distance < best[0]:
                best = (distance, match.end())
        if best is not None:
            return best[1]
    return None


def _split_cue(
    line: dict[str, Any], gaps: Sequence[float],
    max_chars: int, max_seconds: float,
) -> list[dict[str, Any]]:
    text = str(line["text"])
    start = float(line["start_seconds"])
    end = float(line["end_seconds"])
    if len(text) <= max_chars and end - start <= max_seconds:
        return [line]
    position = _split_position(text)
    if position is None:
        flagged = dict(line)
        flagged["uncertain"] = True
        return [flagged]
    left_text = text[:position].rstrip()
    right_text = text[position:].lstrip()
    if not left_text or not right_text:
        flagged = dict(line)
        flagged["uncertain"] = True
        return [flagged]
    boundary = start + (end - start) * (len(left_text) / len(text))
    snapped = _snap_to_gap(boundary, gaps, start, end)
    left = dict(line); left["text"] = left_text
    left["start_seconds"] = start; left["end_seconds"] = snapped
    right = dict(line); right["text"] = right_text
    right["start_seconds"] = snapped; right["end_seconds"] = end
    return (_split_cue(left, gaps, max_chars, max_seconds) +
            _split_cue(right, gaps, max_chars, max_seconds))


def _snap_to_gap(
    boundary: float, gaps: Sequence[float], start: float, end: float,
) -> float:
    margin = min(0.8, (end - start) * 0.25)
    best = boundary
    best_distance = margin
    for moment in gaps:
        if start + 0.2 < moment < end - 0.2:
            distance = abs(moment - boundary)
            if distance < best_distance:
                best = moment
                best_distance = distance
    return best


def _nearest_gap_is_unbridgeable(
    position: int, starts: Sequence[float | None],
) -> bool:
    return all(value is None
               for value in list(starts[max(0, position - 1):position + 2]))


def _interpolate_gaps(
    timeable: Sequence[dict[str, Any]],
    starts: list[float | None],
    ends: list[float | None],
    unreliable: Sequence[tuple[float, float]],
) -> set[int]:
    """Assign proportional windows to unmatched lines inside short gaps."""
    estimated: set[int] = set()
    count = len(timeable)
    position = 0
    while position < count:
        if starts[position] is not None:
            position += 1
            continue
        run = [position]
        cursor = position + 1
        while cursor < count and starts[cursor] is None:
            run.append(cursor)
            cursor += 1
        previous_end = None
        for back in range(position - 1, -1, -1):
            if ends[back] is not None:
                previous_end = ends[back]
                break
        next_start = starts[cursor] if cursor < count else None
        if previous_end is not None and next_start is not None:
            gap = next_start - previous_end
            crosses_garbage = any(
                start_flagged < next_start and end_flagged > previous_end
                for start_flagged, end_flagged in unreliable)
            # Every interpolated cue must fit before MINIMUM_CUE_SECONDS is
            # enforced below. Without this capacity check, four missing chorus
            # lines squeezed into a 0.5 s gap became four overlapping 0.5 s
            # captions and looked like confident timing rather than a hole.
            enough_room = gap >= len(run) * MINIMUM_CUE_SECONDS
            if (0.0 < gap <= MAX_INTERPOLATION_GAP_SECONDS and enough_room
                    and not crosses_garbage):
                weights = [max(1, len(timeable[index]["tokens"]))
                           for index in run]
                total_weight = sum(weights)
                offset = previous_end
                for index, weight in zip(run, weights):
                    width = gap * weight / total_weight
                    starts[index] = offset
                    ends[index] = offset + width
                    estimated.add(index)
                    offset += width
        position = cursor
    return estimated
