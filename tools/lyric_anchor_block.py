#!/usr/bin/env python3
"""Anchor-to-block localization of authored lyrics: the pure half.

This module owns every decision the anchor/block lane makes that does not need
an acoustic model, so the policy can be tested, cached and versioned without a
CUDA device. ``tools/anchor_block_align.py`` supplies the CTC evidence and
calls back into ``assemble_document`` here.

Why the pipeline is shaped this way
-----------------------------------

The previous production path made Whisper the *authority* for localization: an
authored line that did not match Whisper evidence was omitted before the
acoustic aligner ever saw it, and a line that did match was refined only inside
a window derived from that same suspect evidence. Both failure classes are
documented in ``docs/LYRICS_TIMING_RESEARCH_PLAN.md`` and both were measured on
the coverage canary, which lost its two outro lines to a Whisper repetition
loop.

Here Whisper is *evidence* and not authority, following "Low Resource
Audio-to-Lyrics Alignment From Polyphonic Music Recordings" (arXiv:2102.09202)
and the winning benchmark lane ``tools/lyrics_research/anchor_block_mms.py``:

1. spot rare n-grams that occur exactly once in the authored text and exactly
   once in the Whisper words;
2. keep the heaviest monotonic subset of those matches as anchors;
3. cut the song into ordered blocks at the anchors, including a first-class
   initial block before the first anchor and terminal block after the last;
4. force each block's complete consecutive text through one CTC path, so a
   repeated phrase is disambiguated by its ordered neighbours;
5. compare the block path with the independent coarse occurrence proposal;
   localize high-confidence proposals in their own bounded windows and reject
   occurrence-scale or authored-order contradictions rather than publishing a
   known-bad cue;
6. emit either an accepted cue or an explicit ``unresolved`` record — never a
   silent omission — for every alignable authored line (Invariant 1).

Two things this module deliberately does **not** do:

* It never derives a confidence claim from the aligner's own score. The
  operator adjudication of 2026-08-04 measured median score 0.139 on
  confirmed-correct lines against 0.142 on confirmed-wrong ones, and flagged
  ``weak`` on 9 of 16 correct lines. The score orders nothing, so review flags
  come from cross-view disagreement and abstention instead (Invariant 4).
* It never picks an occurrence the evidence did not decide. Repeated-line
  competition, an occurrence-scale cross-view disagreement, or a backwards
  authored path all abstain. ``shut-up-cat`` line 26 and the 2026-08-17
  Groyper Idol chorus jump are the pinned examples.

Nothing here opens an audio device, reads audio, or touches the network.
"""

from __future__ import annotations

import math
from typing import Any, Sequence

from analysis_io import AnalysisValidationError

import lyric_align

# Bump when any constant or rule below changes the times a track resolves to.
# `external_analysis` records this in cache provenance, so an artifact written
# under an older policy is regenerated rather than silently reused.
LOCALIZATION_POLICY = "anchor-block-mms"
LOCALIZATION_POLICY_VERSION = "2"
# Acoustic request identity, shared with the runner and cache reader without
# importing torch into the orchestration process.
ALIGNMENT_VERSION = "3"

# Anchor spotting. An n-gram is only an anchor when it is unique on *both*
# sides: a phrase repeated in the lyrics cannot say which chorus it belongs to,
# and a phrase Whisper emitted twice cannot say which occurrence is real.
ANCHOR_LENGTHS = (5, 4, 3)
# Two words is common in almost any lyric sheet, so the shortest anchor is
# three.
ANCHOR_MINIMUM_LENGTH = 3

# Block windows. The lead reaches back before the anchor so the anchored line's
# own onset is inside the window; the tail reaches past the next anchor's onset
# so the block's last line has room to end.
BLOCK_LEAD_SECONDS = 1.5
BLOCK_TAIL_SECONDS = 3.0
# One forward pass over the block. 90 s of 16 kHz audio is comfortable on a
# 24 GiB device; longer blocks are split rather than refused, because refusing
# would reintroduce the coverage failure this lane exists to remove.
MAX_BLOCK_SECONDS = 90.0
BLOCK_SPLIT_OVERLAP_SECONDS = 4.0
# A reference carrying section headings gives us stronger boundaries than a
# token-proportional split of a sparse-anchor tail.  Coarse proposals are not
# accepted as timings here; their section centres only bound the independent
# CTC search, with enough overlap for long instrumental transitions.
SECTION_WINDOW_OVERLAP_SECONDS = 6.0
MINIMUM_TRUSTED_COARSE_CONFIDENCE = 0.80

# Boundary padding, matching tools/force_align_lyrics.py so cue edges from the
# two lanes stay directly comparable.
CUE_LEAD_SECONDS = 0.10
CUE_TAIL_SECONDS = 0.15

MINIMUM_ALIGNMENT_SCORE = 0.15
MINIMUM_LINE_SECONDS = 0.15

# Review policy. A flag is cross-view disagreement or an unresolved line; it is
# never the aligner's own score.
REVIEW_DISAGREEMENT_SECONDS = 3.0
# A disagreement this large is an occurrence dispute, not a boundary tweak.
# The 2026-08-04 adjudication caught every wrong checked placement through
# cross-view disagreement, and Groyper Idol demonstrated that merely painting
# a +22..45 s jump amber still lets a known-bad cue reach Apply.  Such a line is
# now unresolved unless a later independent lane supplies a deciding view.
MAXIMUM_ACCEPTED_DISAGREEMENT_SECONDS = 8.0
# Two identical authored lines whose placements start this close and overlap
# have collapsed onto one acoustic phrase: the block path claimed the same
# audio twice, which is exactly the ambiguity it exists to resolve.
REPEATED_COLLAPSE_SECONDS = 0.25
ORDER_SUPPORT_MARGIN_SECONDS = 0.25


# --------------------------------------------------------------------------
# Authored text
# --------------------------------------------------------------------------


def alignable_lines(reference_text: str) -> list[dict[str, Any]]:
    """Authored lines that carry alignment tokens, in authored order.

    Section headings, stage directions and delivery notes are not sung text,
    so they are not part of the coverage denominator. Everything this returns
    must end up either as a cue or as an ``unresolved`` record.
    """
    return [line for line in lyric_align.classify_reference_lines(reference_text)
            if line["kind"] in {"lyric", "backing"} and line["tokens"]]


def authored_tokens(lines: Sequence[dict[str, Any]]) -> list[dict[str, Any]]:
    tokens: list[dict[str, Any]] = []
    for position, line in enumerate(lines):
        for token in line["tokens"]:
            tokens.append({"token": token, "line_position": position})
    return tokens


# --------------------------------------------------------------------------
# Evidence
# --------------------------------------------------------------------------


def evidence_tokens(
    whisper: dict[str, Any], unreliable: Sequence[tuple[float, float]],
) -> list[dict[str, Any]]:
    """Timed normalized tokens from the Whisper lane, loops excluded.

    A repetition loop is high-confidence and completely wrong, so its words are
    the worst possible anchors: the canary's loop repeats a genuine authored
    line 20 times across the outro, and anchoring on it would pin that line to
    the wrong occurrence and bury the two lines that follow it.
    """
    words = whisper.get("words")
    if not isinstance(words, list) or not words:
        # Models without DTW word timing still give line intervals; every token
        # of a line then shares the line's window.
        words = whisper.get("lines") if isinstance(whisper.get("lines"), list) else []
    tokens: list[dict[str, Any]] = []
    for word in words or []:
        if not isinstance(word, dict):
            continue
        try:
            start = float(word["start_seconds"])
            end = float(word["end_seconds"])
        except (KeyError, TypeError, ValueError):
            continue
        if not math.isfinite(start) or not math.isfinite(end) or end < start:
            continue
        middle = (start + end) / 2.0
        if any(low <= middle <= high for low, high in unreliable):
            continue
        for token in lyric_align.normalize_tokens(str(word.get("text", ""))):
            tokens.append({"token": token, "start": start, "end": end})
    return tokens


def coarse_proposals(
    sync_document: Any, *, trusted_only: bool = False,
) -> dict[int, tuple[float, float]]:
    """The Whisper-derived proposal per authored line, as an independent view.

    This is ``musializer.lyric-sync/v1`` — the lane that used to *be* the
    answer and is now only one of two views. It is what a review flag compares
    against, and what decides a repeated phrase's occurrence when it disagrees.
    """
    result: dict[int, tuple[float, float]] = {}
    if not isinstance(sync_document, dict):
        return result
    for line in sync_document.get("lines") or []:
        if not isinstance(line, dict):
            continue
        index = line.get("reference_line_index")
        start = line.get("start_seconds")
        end = line.get("end_seconds")
        if not isinstance(index, int) or isinstance(index, bool):
            continue
        if not isinstance(start, (int, float)) or isinstance(start, bool):
            continue
        if not isinstance(end, (int, float)) or isinstance(end, bool):
            continue
        if trusted_only:
            confidence = line.get("confidence")
            if (line.get("estimated") is True
                    or not isinstance(confidence, (int, float))
                    or isinstance(confidence, bool)
                    or float(confidence)
                    < MINIMUM_TRUSTED_COARSE_CONFIDENCE):
                continue
        result[int(index)] = (float(start), float(end))
    return result


# --------------------------------------------------------------------------
# Anchors
# --------------------------------------------------------------------------


def _ngram_index(tokens: Sequence[str], length: int) -> dict[str, list[int]]:
    index: dict[str, list[int]] = {}
    for start in range(len(tokens) - length + 1):
        key = " ".join(tokens[start:start + length])
        index.setdefault(key, []).append(start)
    return index


def spot_anchors(
    authored: Sequence[dict[str, Any]], evidence: Sequence[dict[str, Any]],
    *, lengths: Sequence[int] = ANCHOR_LENGTHS,
) -> list[dict[str, Any]]:
    """Rare n-gram matches, longest first, non-overlapping and monotonic."""
    authored_words = [item["token"] for item in authored]
    evidence_words = [item["token"] for item in evidence]
    candidates: list[dict[str, Any]] = []
    taken_authored: list[tuple[int, int]] = []
    taken_evidence: list[tuple[int, int]] = []
    for length in sorted({int(value) for value in lengths}, reverse=True):
        if length < ANCHOR_MINIMUM_LENGTH:
            continue
        left = _ngram_index(authored_words, length)
        right = _ngram_index(evidence_words, length)
        for key, positions in left.items():
            if len(positions) != 1:
                continue
            hits = right.get(key)
            if hits is None or len(hits) != 1:
                continue
            authored_start = positions[0]
            evidence_start = hits[0]
            authored_span = (authored_start, authored_start + length)
            evidence_span = (evidence_start, evidence_start + length)
            if any(authored_span[0] < end and start < authored_span[1]
                   for start, end in taken_authored):
                continue
            if any(evidence_span[0] < end and start < evidence_span[1]
                   for start, end in taken_evidence):
                continue
            taken_authored.append(authored_span)
            taken_evidence.append(evidence_span)
            candidates.append({
                "text": key,
                "length": length,
                "authored_token_index": authored_start,
                "evidence_token_index": evidence_start,
                "line_position": authored[authored_start]["line_position"],
                "start_seconds": float(evidence[evidence_start]["start"]),
                "end_seconds": float(
                    evidence[evidence_start + length - 1]["end"]),
            })
    candidates.sort(key=lambda item: item["authored_token_index"])
    return monotonic_subset(candidates)


def monotonic_subset(
    candidates: Sequence[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Heaviest subset whose times increase with authored position.

    A unique-on-both-sides match can still be wrong, and a wrong anchor is
    worse than a missing one because it mislocates a whole block. Contradictory
    anchors are resolved globally rather than by a local score: keep the
    heaviest increasing subsequence (weighted by n-gram length) and drop the
    rest. O(n^2) is ample — a song yields tens of candidates, not thousands.
    """
    count = len(candidates)
    if count == 0:
        return []
    best = [float(candidates[index]["length"]) for index in range(count)]
    previous = [-1] * count
    for index in range(count):
        for earlier in range(index):
            if (candidates[earlier]["start_seconds"]
                    < candidates[index]["start_seconds"]
                    and candidates[earlier]["end_seconds"]
                    <= candidates[index]["start_seconds"]
                    and best[earlier] + candidates[index]["length"] > best[index]):
                best[index] = best[earlier] + candidates[index]["length"]
                previous[index] = earlier
    tail = max(range(count), key=lambda index: best[index])
    chain: list[dict[str, Any]] = []
    while tail >= 0:
        chain.append(candidates[tail])
        tail = previous[tail]
    chain.reverse()
    return chain


# --------------------------------------------------------------------------
# Blocks
# --------------------------------------------------------------------------


def token_time_map(
    anchors: Sequence[dict[str, Any]], *, token_count: int,
    audio_duration: float,
) -> list[tuple[int, float]]:
    """Monotone control points mapping authored token index to seconds.

    Anchors pin *tokens*, not lines, and a rare n-gram often begins in the
    middle of a line — the canary's ``spark of who we are`` starts on the last
    word of one line. Treating the anchor's time as that line's onset would
    start the search window several seconds after the line begins and clip its
    first words. Interpolating between anchors instead gives every line an
    onset estimate that degrades gracefully rather than lying.

    The endpoints are deliberately loose bounds (zero and the track duration):
    an estimate that reaches back too far only widens a search window, while
    one that reaches back too little loses audio that a line needs.
    """
    points: list[tuple[int, float]] = []
    for anchor in anchors:
        start = int(anchor["authored_token_index"])
        points.append((start, float(anchor["start_seconds"])))
        points.append((start + int(anchor["length"]),
                       float(anchor["end_seconds"])))
    points.sort()
    monotone: list[tuple[int, float]] = []
    for index, seconds in points:
        if monotone and index == monotone[-1][0]:
            monotone[-1] = (index, min(monotone[-1][1], seconds))
            continue
        if monotone and seconds < monotone[-1][1]:
            seconds = monotone[-1][1]
        monotone.append((index, seconds))
    if not monotone or monotone[0][0] > 0:
        monotone.insert(0, (0, 0.0))
    if monotone[-1][0] < token_count:
        monotone.append((token_count, audio_duration))
    return monotone


def interpolate(points: Sequence[tuple[int, float]], index: int) -> float:
    if index <= points[0][0]:
        return points[0][1]
    if index >= points[-1][0]:
        return points[-1][1]
    for (left_index, left_time), (right_index, right_time) in zip(points, points[1:]):
        if left_index <= index <= right_index:
            if right_index == left_index:
                return left_time
            fraction = (index - left_index) / (right_index - left_index)
            return left_time + fraction * (right_time - left_time)
    return points[-1][1]


def build_blocks(
    lines: Sequence[dict[str, Any]], anchors: Sequence[dict[str, Any]],
    *, audio_duration: float, max_block_seconds: float = MAX_BLOCK_SECONDS,
    coarse: dict[int, tuple[float, float]] | None = None,
) -> list[dict[str, Any]]:
    """Partition every authored line into an ordered, timed search block.

    Cuts fall at the lines where anchors begin, so consecutive unanchored lines
    stay together and are aligned as one ordered path. Each block searches from
    its first line's estimated onset to its last line's estimated offset, both
    padded. Lines before the first anchor form an initial block searching from
    zero; lines from the last anchor on form a terminal block searching to the
    end of the track. With no anchors at all the whole song is one block —
    still a search, never an omission.
    """
    line_count = len(lines)
    if line_count == 0:
        return []

    section_blocks = _section_evidence_blocks(
        lines, anchors, coarse or {}, audio_duration=audio_duration,
        max_block_seconds=max_block_seconds)
    if section_blocks is not None:
        return section_blocks

    first_token: list[int] = []
    running = 0
    for line in lines:
        first_token.append(running)
        running += len(line["tokens"])
    token_count = running
    points = token_time_map(
        anchors, token_count=token_count, audio_duration=audio_duration)

    def onset(position: int) -> float:
        return interpolate(points, first_token[position])

    def offset(position: int) -> float:
        end = (first_token[position + 1] if position + 1 < line_count
               else token_count)
        return interpolate(points, end)

    cuts: list[int] = []
    for anchor in anchors:
        position = int(anchor["line_position"])
        if not cuts or cuts[-1] != position:
            cuts.append(position)

    spans: list[dict[str, Any]] = []
    if not cuts:
        spans.append({
            "first_line": 0, "last_line": line_count - 1,
            "window_start": 0.0, "window_end": audio_duration,
            "kind": "unanchored",
        })
    else:
        if cuts[0] > 0:
            spans.append({
                "first_line": 0, "last_line": cuts[0] - 1,
                "kind": "initial",
            })
        for index, position in enumerate(cuts):
            following = cuts[index + 1] if index + 1 < len(cuts) else None
            last_line = (following - 1) if following is not None else line_count - 1
            if last_line < position:
                continue
            spans.append({
                "first_line": position, "last_line": last_line,
                "kind": "terminal" if following is None else "anchored",
            })
    for span in spans:
        span.setdefault(
            "window_start",
            0.0 if span["first_line"] == 0
            else max(0.0, onset(span["first_line"]) - BLOCK_LEAD_SECONDS))
        span.setdefault(
            "window_end",
            audio_duration if span["last_line"] == line_count - 1
            else min(audio_duration, offset(span["last_line"]) + BLOCK_TAIL_SECONDS))

    blocks: list[dict[str, Any]] = []
    for span in spans:
        blocks.extend(_split_span(
            span, lines, max_block_seconds=max_block_seconds))
    for index, block in enumerate(blocks):
        block["index"] = index
    return blocks


def _section_evidence_blocks(
    lines: Sequence[dict[str, Any]], anchors: Sequence[dict[str, Any]],
    coarse: dict[int, tuple[float, float]], *, audio_duration: float,
    max_block_seconds: float,
) -> list[dict[str, Any]] | None:
    """Bound section-sized CTC searches with independent coarse evidence.

    The v1 fallback after the final rare anchor interpolated all remaining
    tokens to the end of the song, then split that span by token count.  On
    Groyper Idol this made the first chorus search 35..112 s even though the
    coarse lane located it at 37..74 s; CTC dutifully forced the words onto a
    later passage.  Authored section order plus coarse *section centres* gives
    a much safer search partition while keeping the acoustic result
    independent of the coarse boundaries themselves.

    This path is used only when there are at least two authored sections and
    two of them carry coarse proposals.  Plain lyric sheets and evidence-poor
    runs retain the anchor partition below.
    """
    groups: list[tuple[int, int]] = []
    first = 0
    for position in range(1, len(lines)):
        if lines[position].get("section_position") != lines[first].get(
                "section_position"):
            groups.append((first, position - 1))
            first = position
    groups.append((first, len(lines) - 1))
    if len(groups) < 2:
        return None

    centres: list[float | None] = []
    for first_line, last_line in groups:
        moments: list[float] = []
        for position in range(first_line, last_line + 1):
            proposal = coarse.get(int(lines[position]["index"]))
            if proposal is not None:
                moments.append((proposal[0] + proposal[1]) / 2.0)
        moments.sort()
        centres.append(
            None if not moments else moments[len(moments) // 2])
    if sum(center is not None for center in centres) < 2:
        return None

    # Missing whole sections (the important authored-but-ASR-omitted case) get
    # a search centre interpolated between their nearest evidenced neighbours.
    # At the ends, distribute the remaining track duration rather than cloning
    # a neighbour and collapsing two sections onto one phrase.
    known = [index for index, center in enumerate(centres) if center is not None]
    for index, center in enumerate(centres):
        if center is not None:
            continue
        left = max((item for item in known if item < index), default=None)
        right = min((item for item in known if item > index), default=None)
        if left is not None and right is not None:
            fraction = (index - left) / (right - left)
            centres[index] = float(centres[left]) + fraction * (
                float(centres[right]) - float(centres[left]))
        elif right is not None:
            centres[index] = float(centres[right]) * ((index + 1) / (right + 1))
        elif left is not None:
            remaining_groups = len(groups) - 1 - left
            fraction = (index - left) / max(1, remaining_groups)
            centres[index] = float(centres[left]) + fraction * (
                audio_duration - float(centres[left]))

    resolved = [float(center) for center in centres]
    for index in range(1, len(resolved)):
        resolved[index] = max(resolved[index], resolved[index - 1] + 0.25)

    boundaries = [0.0]
    boundaries.extend(
        (resolved[index - 1] + resolved[index]) / 2.0
        for index in range(1, len(resolved)))
    boundaries.append(audio_duration)

    blocks: list[dict[str, Any]] = []
    for group_index, (first_line, last_line) in enumerate(groups):
        group_proposals = [
            coarse[int(lines[position]["index"])]
            for position in range(first_line, last_line + 1)
            if int(lines[position]["index"]) in coarse
        ]
        fully_evidenced = len(group_proposals) == last_line - first_line + 1
        window_start = max(
            0.0,
            boundaries[group_index]
            - (SECTION_WINDOW_OVERLAP_SECONDS if group_index else 0.0),
        )
        window_end = min(
            audio_duration,
            boundaries[group_index + 1]
            + (SECTION_WINDOW_OVERLAP_SECONDS
               if group_index + 1 < len(groups) else 0.0),
        )
        if fully_evidenced:
            # Every line has an independently matched occurrence, so the
            # section cannot legitimately live beyond the envelope of those
            # proposals. This is the part v1 lacked on Groyper: midpoint-only
            # boundaries still left chorus 1 open through 95.8 s and let CTC
            # jump from its 37..74 s evidence onto the 80 s verse.
            window_start = max(
                window_start,
                min(proposal[0] for proposal in group_proposals)
                - SECTION_WINDOW_OVERLAP_SECONDS,
            )
            window_end = min(
                window_end,
                max(proposal[1] for proposal in group_proposals)
                + SECTION_WINDOW_OVERLAP_SECONDS,
            )
        span = {
            "first_line": first_line,
            "last_line": last_line,
            "window_start": window_start,
            "window_end": window_end,
            "kind": "section-evidence",
            "section_position": lines[first_line].get("section_position"),
            "coarse_evidence_lines": sum(
                int(lines[position]["index"]) in coarse
                for position in range(first_line, last_line + 1)),
            "fully_coarse_evidenced": fully_evidenced,
            "anchor_evidence_count": sum(
                first_line <= int(anchor["line_position"]) <= last_line
                for anchor in anchors),
        }
        blocks.extend(_split_span(
            span, lines, max_block_seconds=max_block_seconds))
    for index, block in enumerate(blocks):
        block["index"] = index
    return blocks


def _split_span(
    span: dict[str, Any], lines: Sequence[dict[str, Any]],
    *, max_block_seconds: float,
) -> list[dict[str, Any]]:
    """Cut an over-long span into overlapping sub-blocks by token weight.

    Only reached when anchors are sparse. The split is recorded on every piece
    so a later reader can tell a proportional guess from a real anchor.
    """
    window_start = float(span["window_start"])
    length = float(span["window_end"]) - window_start
    if length <= max_block_seconds:
        return [{**span, "split": False}]
    positions = list(range(span["first_line"], span["last_line"] + 1))
    if len(positions) < 2:
        # One line cannot be cut further, and a CTC forward pass is quadratic
        # in window length: an unbounded window here is an out-of-memory abort,
        # not a slow answer. Keep the centre of the estimate and record that
        # the search space was clamped, so a miss is explainable.
        middle = (window_start + float(span["window_end"])) / 2.0
        half = max_block_seconds / 2.0
        return [{
            **span,
            "window_start": max(window_start, middle - half),
            "window_end": min(float(span["window_end"]), middle + half),
            "split": False,
            "oversized": True,
            "clamped": True,
        }]
    weights = [max(1, len(lines[position]["tokens"])) for position in positions]
    prefix = [0]
    for weight in weights:
        prefix.append(prefix[-1] + weight)
    total = float(prefix[-1])
    pieces = min(int(math.ceil(length / max_block_seconds)), len(positions))
    target = total / pieces

    groups: list[tuple[int, int]] = []
    group_start = 0
    for offset in range(len(positions)):
        last_line = offset == len(positions) - 1
        full = prefix[offset + 1] >= target * (len(groups) + 1)
        if last_line or (len(groups) < pieces - 1 and full):
            groups.append((group_start, offset))
            group_start = offset + 1
            if group_start >= len(positions):
                break

    result: list[dict[str, Any]] = []
    for first, last in groups:
        start = window_start + (prefix[first] / total) * length
        end = window_start + (prefix[last + 1] / total) * length
        result.append({
            **span,
            "first_line": positions[first],
            "last_line": positions[last],
            "window_start": max(
                window_start, start - BLOCK_SPLIT_OVERLAP_SECONDS),
            "window_end": min(
                float(span["window_end"]), end + BLOCK_SPLIT_OVERLAP_SECONDS),
            "split": True,
        })
    return result


def plan_localization(
    reference_text: str, whisper: dict[str, Any], *, audio_duration: float,
    max_block_seconds: float = MAX_BLOCK_SECONDS,
    coarse: dict[int, tuple[float, float]] | None = None,
) -> dict[str, Any]:
    """Everything the acoustic pass needs, decided without a model."""
    classified = lyric_align.classify_reference_lines(reference_text)
    lines: list[dict[str, Any]] = []
    section_position = -1
    for line in classified:
        if line["kind"] == "section":
            section_position += 1
        elif line["kind"] in {"lyric", "backing"} and line["tokens"]:
            lines.append({**line, "section_position": section_position})
    if not lines:
        raise AnalysisValidationError("authored lyrics carry no alignable lines")
    unreliable = lyric_align.flag_unreliable_intervals(
        whisper.get("lines") if isinstance(whisper.get("lines"), list) else [])
    evidence = evidence_tokens(whisper, unreliable)
    authored = authored_tokens(lines)
    anchors = spot_anchors(authored, evidence)
    blocks = build_blocks(
        lines, anchors, audio_duration=audio_duration,
        max_block_seconds=max_block_seconds, coarse=coarse)
    return {
        "lines": lines,
        # Headings, sound events and delivery notes are not sung and are not in
        # the coverage denominator, but the lane still reports them so the
        # review surface can show a line in its authored context.
        "structure": [{"reference_line_index": line["index"],
                       "kind": line["kind"], "text": line["display"]}
                      for line in classified
                      if line["kind"] in ("section", "event", "delivery")],
        "anchors": anchors,
        "blocks": blocks,
        "unreliable_evidence": [
            {"start_seconds": low, "end_seconds": high}
            for low, high in unreliable
        ],
        "evidence_token_count": len(evidence),
        "authored_token_count": len(authored),
    }


# --------------------------------------------------------------------------
# Repeated-phrase abstention (Invariant 2)
# --------------------------------------------------------------------------


def repeated_phrase_abstentions(
    lines: Sequence[dict[str, Any]],
    placements: dict[int, tuple[float, float]],
    coarse: dict[int, tuple[float, float]],
    *, disagreement_seconds: float = REVIEW_DISAGREEMENT_SECONDS,
    collapse_seconds: float = REPEATED_COLLAPSE_SECONDS,
) -> dict[int, str]:
    """Line positions that must abstain, mapped to the reason.

    Two triggers, both about *which occurrence* a repeated line owns rather
    than about how well it scored:

    ``repeated_occurrence_ambiguous``
        The coarse Whisper-derived view puts this line nearer to a *sibling*
        occurrence's block placement than to its own, by more than the review
        tolerance. Two independent views then disagree about the occurrence,
        not merely about a boundary, and the global ordering has not decided
        it. ``shut-up-cat`` line 26 is this case: the block path placed it at
        106.8 s, the coarse view at 122.3 s, and 122.3 s is closer to the
        *next* occurrence's 132.8 s. Both were adjudicated wrong, and
        abstention is the adjudicated-correct response.

    ``repeated_phrase_collapsed``
        Two identical authored lines were placed on the same acoustic phrase.
        The path claimed one span twice, so at most one of them can be right
        and nothing here says which.

    A line whose text is unique in the authored lyrics can never abstain from
    this rule: there is no competing occurrence to confuse it with.
    """
    groups: dict[str, list[int]] = {}
    for position, line in enumerate(lines):
        groups.setdefault(" ".join(line["tokens"]), []).append(position)

    abstentions: dict[int, str] = {}
    for positions in groups.values():
        if len(positions) < 2:
            continue
        placed = [position for position in positions if position in placements]
        for offset, left in enumerate(placed):
            for right in placed[offset + 1:]:
                left_span, right_span = placements[left], placements[right]
                overlaps = (min(left_span[1], right_span[1])
                            > max(left_span[0], right_span[0]))
                if overlaps and abs(left_span[0] - right_span[0]) <= collapse_seconds:
                    abstentions[left] = "repeated_phrase_collapsed"
                    abstentions[right] = "repeated_phrase_collapsed"
        for position in placed:
            proposal = coarse.get(int(lines[position]["index"]))
            if proposal is None:
                continue
            own = placements[position][0]
            nearest = min(
                placed,
                key=lambda other: (abs(placements[other][0] - proposal[0]), other))
            if nearest != position and abs(proposal[0] - own) > disagreement_seconds:
                abstentions.setdefault(position, "repeated_occurrence_ambiguous")
    return abstentions


def order_abstentions(
    lines: Sequence[dict[str, Any]],
    placements: dict[int, tuple[float, float]],
    coarse: dict[int, tuple[float, float]],
) -> dict[int, str]:
    """Remove the least-supported side of every backwards authored pair.

    Sorting the bridge hides a localization contradiction; it does not solve
    one.  Prefer the side whose fine start is closer to its independent coarse
    proposal.  When only one side has a proposal, keep it.  With no deciding
    evidence both sides abstain, because choosing by acoustic score would reuse
    a metric the adjudication proved uninformative.
    """
    active = dict(placements)
    abstentions: dict[int, str] = {}
    while True:
        ordered = sorted(active)
        reversed_pair = next(
            ((left, right) for left, right in zip(ordered, ordered[1:])
             if active[right][0] < active[left][0]),
            None,
        )
        if reversed_pair is None:
            return abstentions
        left, right = reversed_pair
        left_coarse = coarse.get(int(lines[left]["index"]))
        right_coarse = coarse.get(int(lines[right]["index"]))
        if left_coarse is None and right_coarse is None:
            losers = (left, right)
        elif left_coarse is None:
            losers = (left,)
        elif right_coarse is None:
            losers = (right,)
        else:
            left_delta = abs(active[left][0] - left_coarse[0])
            right_delta = abs(active[right][0] - right_coarse[0])
            if abs(left_delta - right_delta) <= ORDER_SUPPORT_MARGIN_SECONDS:
                losers = (left, right)
            else:
                losers = (left if left_delta > right_delta else right,)
        for loser in losers:
            abstentions[loser] = "authored_order_ambiguous"
            active.pop(loser, None)


def coarse_local_refinement_allowed(
    independent: dict[str, Any] | None, proposal_start: float,
) -> bool:
    """Whether a coarse-window CTC result has independent occurrence support.

    The local result itself is deliberately not an argument: its search window
    came from the coarse proposal, so comparing those two would be circular.
    Only the separately searched section/block placement can supply the second
    view needed to turn the local path into a boundary refinement.
    """
    if not isinstance(independent, dict):
        return False
    start = independent.get("acoustic_start_seconds")
    return (
        isinstance(start, (int, float))
        and not isinstance(start, bool)
        and abs(float(start) - proposal_start)
        <= MAXIMUM_ACCEPTED_DISAGREEMENT_SECONDS
    )


def anchor_supports_section_occurrence(
    anchor: dict[str, Any], anchored_line_candidate: dict[str, Any] | None,
    section_block: dict[str, Any] | None,
) -> bool:
    """Whether a rare anchor actually validates this section occurrence.

    Authored membership alone is insufficient: a rare phrase at 10 seconds
    contradicts, rather than validates, a coarse-conditioned section at 100.
    The anchor must lie inside the candidate window and the CTC placement of
    its own authored line must remain occurrence-close to it.
    """
    if not isinstance(anchored_line_candidate, dict) or not isinstance(
            section_block, dict):
        return False
    try:
        anchor_start = float(anchor["start_seconds"])
        anchor_end = float(anchor["end_seconds"])
        candidate_start = float(
            anchored_line_candidate["acoustic_start_seconds"])
        window_start = float(section_block["window_start"])
        window_end = float(section_block["window_end"])
    except (KeyError, TypeError, ValueError):
        return False
    values = (
        anchor_start, anchor_end, candidate_start, window_start, window_end)
    if not all(math.isfinite(value) for value in values):
        return False
    anchor_middle = (anchor_start + anchor_end) / 2.0
    return (
        window_start <= anchor_middle <= window_end
        and abs(candidate_start - anchor_start)
        <= MAXIMUM_ACCEPTED_DISAGREEMENT_SECONDS
    )


# --------------------------------------------------------------------------
# Document assembly
# --------------------------------------------------------------------------


def assemble_document(
    plan: dict[str, Any], decisions: dict[int, dict[str, Any]],
    owning_block: dict[int, int], coarse: dict[int, tuple[float, float]], *,
    audio_duration: float,
    trusted_coarse: dict[int, tuple[float, float]] | None = None,
) -> dict[str, Any]:
    """Turn per-line acoustic decisions into the lyric lane.

    ``decisions`` is keyed by line *position* (index into ``plan["lines"]``)
    and carries at least ``status`` and ``score``; an aligned or weak decision
    also carries ``acoustic_start_seconds`` / ``acoustic_end_seconds``.

    The one hard invariant: every alignable authored line leaves here either as
    a cue or as an ``unresolved`` record. ``validate_full_coverage`` re-checks
    it on the finished document, because a silent omission is exactly the bug
    this lane replaced.
    """
    lines = plan["lines"]
    deciding_coarse = coarse if trusted_coarse is None else trusted_coarse

    placements: dict[int, tuple[float, float]] = {}
    records: dict[int, dict[str, Any]] = {}
    for position, line in enumerate(lines):
        decision = decisions.get(position, {"status": "no_alignment", "score": 0.0})
        status = str(decision.get("status", "no_alignment"))
        record: dict[str, Any] = {
            "reference_line_index": int(line["index"]),
            "line_position": position,
            "kind": line["kind"],
            "text": line["display"],
            "start_seconds": None,
            "end_seconds": None,
            "score": float(decision.get("score", 0.0)),
            "status": status,
            "block_index": owning_block.get(position),
        }
        if status in {"aligned", "weak"}:
            start = max(
                0.0,
                float(decision["acoustic_start_seconds"]) - CUE_LEAD_SECONDS)
            end = min(
                audio_duration,
                float(decision["acoustic_end_seconds"]) + CUE_TAIL_SECONDS)
            if end > start:
                record["start_seconds"] = start
                record["end_seconds"] = end
                placements[position] = (start, end)
            else:
                record["status"] = "collapsed"
            record["word_alignments"] = decision.get("word_alignments", [])
            record["first_word_score"] = decision.get("first_word_score")
            record["last_word_score"] = decision.get("last_word_score")
        # Audit challengers survive even when the conditioned path found no
        # placement. They do not create a cue, but dropping them from the
        # unresolved record would hide the evidence the dual-path policy used.
        if decision.get("candidate_source") is not None:
            record["candidate_source"] = decision["candidate_source"]
        if decision.get("coarse_confidence") is not None:
            record["coarse_confidence"] = decision["coarse_confidence"]
        if decision.get("independent_block_candidate") is not None:
            record["independent_block_candidate"] = decision[
                "independent_block_candidate"]
        if decision.get("independent_global_candidate") is not None:
            record["independent_global_candidate"] = decision[
                "independent_global_candidate"]
        if decision.get("global_disagreement_anchor_resolved") is True:
            record["global_disagreement_anchor_resolved"] = True
        if len(str(line["display"]).encode("utf-8")) > lyric_align.MAX_CUE_TEXT_BYTES:
            # A line longer than one caption cannot become a cue whatever the
            # acoustics say. It is still reported by name rather than dropped.
            record["start_seconds"] = None
            record["end_seconds"] = None
            record["status"] = "line_too_long"
            placements.pop(position, None)
        records[position] = record

    abstentions = repeated_phrase_abstentions(
        lines, placements, deciding_coarse)
    for position, decision in decisions.items():
        if (position in placements
                and decision.get("occurrence_disputed") is True):
            abstentions.setdefault(
                position, "global_localization_disagreement")
    for position, placement in placements.items():
        proposal = deciding_coarse.get(int(lines[position]["index"]))
        if (proposal is not None
                and abs(placement[0] - proposal[0])
                > MAXIMUM_ACCEPTED_DISAGREEMENT_SECONDS):
            abstentions.setdefault(
                position, "cross_view_occurrence_ambiguous")
    # Resolve occurrence disputes before testing the residual path's order.
    # Otherwise a known-bad +40 s placement can make its sound neighbour
    # abstain as collateral damage even though removing the disputed line
    # already restores a monotonic sequence.
    ordered_candidates = {
        position: placement for position, placement in placements.items()
        if position not in abstentions
    }
    for position, reason in order_abstentions(
            lines, ordered_candidates, deciding_coarse).items():
        abstentions.setdefault(position, reason)
    for position, reason in abstentions.items():
        records[position]["acoustic_start_seconds"] = records[position][
            "start_seconds"]
        records[position]["acoustic_end_seconds"] = records[position][
            "end_seconds"]
        records[position]["start_seconds"] = None
        records[position]["end_seconds"] = None
        records[position]["status"] = reason
        records[position]["abstained"] = True
        placements.pop(position, None)

    cues: list[dict[str, Any]] = []
    unresolved: list[dict[str, Any]] = []
    review_flags: list[dict[str, Any]] = []
    order_violations = _order_violations(records)
    for position in range(len(lines)):
        record = records[position]
        reference_index = int(record["reference_line_index"])
        proposal = coarse.get(reference_index)
        record["coarse_start_seconds"] = None if proposal is None else proposal[0]
        record["coarse_end_seconds"] = None if proposal is None else proposal[1]
        if record["start_seconds"] is None:
            unresolved.append({
                "reference_line_index": reference_index,
                "line_position": position,
                "kind": record["kind"],
                "text": record["text"],
                "reason": record["status"],
                "abstained": bool(record.get("abstained", False)),
                "coarse_start_seconds": record["coarse_start_seconds"],
                "coarse_end_seconds": record["coarse_end_seconds"],
                "acoustic_start_seconds": record.get(
                    "acoustic_start_seconds"),
                "acoustic_end_seconds": record.get(
                    "acoustic_end_seconds"),
                "independent_global_candidate": record.get(
                    "independent_global_candidate"),
            })
            review_flags.append({
                "reference_line_index": reference_index,
                "text": record["text"],
                "flag": "unresolved",
                "reason": record["status"],
                "start_seconds": None,
                "end_seconds": None,
                "coarse_start_seconds": record["coarse_start_seconds"],
                "delta_seconds": None,
            })
            continue
        delta = (None if proposal is None
                 else float(record["start_seconds"]) - proposal[0])
        coarse_flagged = (
            delta is not None and abs(delta) > REVIEW_DISAGREEMENT_SECONDS)
        global_flagged = bool(
            record.get("global_disagreement_anchor_resolved", False))
        flagged = coarse_flagged or global_flagged
        if flagged:
            if global_flagged:
                flag = "global_disagreement_anchor_resolved"
                reason = (
                    "the unconditioned global path disagrees on occurrence, "
                    "but a rare exact anchor resolves this section")
            else:
                flag = "coarse_disagreement"
                reason = (f"the coarse Whisper proposal and the anchor/block "
                          f"placement differ by {abs(delta):.1f} s")
            review_flags.append({
                "reference_line_index": reference_index,
                "text": record["text"],
                "flag": flag,
                "reason": reason,
                "start_seconds": record["start_seconds"],
                "end_seconds": record["end_seconds"],
                "coarse_start_seconds": (
                    None if proposal is None else proposal[0]),
                "delta_seconds": delta,
            })
        cue = dict(record)
        # Confidence stays null on purpose. The adjudication of 2026-08-04
        # measured the aligner's score at median 0.139 on confirmed-correct
        # lines against 0.142 on confirmed-wrong ones, so publishing it as a
        # confidence would be a claim the evidence does not support.
        cue["confidence"] = None
        cue["estimated"] = False
        cue["uncertain"] = flagged
        cue["review_flagged"] = flagged
        cues.append(cue)

    cues.sort(key=lambda line: (line["start_seconds"], line["end_seconds"],
                                line["reference_line_index"]))
    document = {
        "schema_version": lyric_align.LYRIC_SYNC_VERSION,
        "lane": "lyric_sync",
        "aligner_version": lyric_align.ALIGNER_VERSION,
        "localization_policy": LOCALIZATION_POLICY,
        "localization_policy_version": LOCALIZATION_POLICY_VERSION,
        "lines": cues,
        "performed_candidates": list(plan.get("performed_candidates", [])),
        "unresolved": unresolved,
        # `unmatched` keeps its established meaning — authored lines with no
        # timing — so the manifest count and the CLI summary keep working.
        "unmatched": [
            {"reference_line_index": entry["reference_line_index"],
             "text": entry["text"], "reason": entry["reason"]}
            for entry in unresolved
        ],
        "review_flags": review_flags,
        "structure": plan.get("structure", []),
        "anchors": plan["anchors"],
        "blocks": plan["blocks"],
        "unreliable_evidence": plan["unreliable_evidence"],
        "order_violations": order_violations,
        "statistics": {
            "reference_lines": len(lines),
            "matched_lines": len(cues),
            "estimated_lines": 0,
            "unmatched_lines": len(unresolved),
            "performed_candidates": len(plan.get("performed_candidates", [])),
            "unresolved_lines": len(unresolved),
            "abstained_lines": len(abstentions),
            "review_flagged_lines": len(review_flags),
            "coarse_disagreement_lines": sum(
                1 for flag in review_flags if flag["flag"] == "coarse_disagreement"),
            "anchor_count": len(plan["anchors"]),
            "block_count": len(plan["blocks"]),
            "evidence_tokens": plan["evidence_token_count"],
            "reference_tokens": plan["authored_token_count"],
            "order_violations": len(order_violations),
        },
    }
    validate_full_coverage(document, lines)
    return document


def _order_violations(
    records: dict[int, dict[str, Any]],
) -> list[dict[str, Any]]:
    """Timed lines whose start goes backwards against authored order."""
    violations: list[dict[str, Any]] = []
    previous: dict[str, Any] | None = None
    for position in sorted(records):
        record = records[position]
        if record.get("start_seconds") is None:
            continue
        if previous is not None and record["start_seconds"] < previous["start_seconds"]:
            violations.append({
                "previous_reference_line_index": previous["reference_line_index"],
                "reference_line_index": record["reference_line_index"],
                "previous_start_seconds": previous["start_seconds"],
                "start_seconds": record["start_seconds"],
            })
        previous = record
    return violations


def validate_full_coverage(
    document: dict[str, Any], authored: Sequence[dict[str, Any]],
) -> None:
    """Refuse a lane that lost an authored line on the way through.

    This is the executable form of Invariant 1 and it is a guard, not a test
    helper: the previous pipeline dropped lines *quietly*, so the interesting
    failure mode is one nobody notices. Every alignable authored line must
    appear exactly once, as a cue or as an ``unresolved`` record.
    """
    expected = [int(line["index"]) for line in authored]
    seen: list[int] = [int(line["reference_line_index"])
                       for line in document.get("lines") or []]
    seen += [int(line["reference_line_index"])
             for line in document.get("unresolved") or []]
    missing = sorted(set(expected) - set(seen))
    duplicated = sorted({index for index in seen if seen.count(index) > 1})
    if missing:
        raise AnalysisValidationError(
            "lyric localization lost authored lines "
            f"{missing}; every alignable line must be a cue or unresolved")
    if duplicated:
        raise AnalysisValidationError(
            f"lyric localization emitted authored lines {duplicated} twice")


__all__ = [
    "ANCHOR_LENGTHS",
    "BLOCK_LEAD_SECONDS",
    "BLOCK_SPLIT_OVERLAP_SECONDS",
    "BLOCK_TAIL_SECONDS",
    "CUE_LEAD_SECONDS",
    "CUE_TAIL_SECONDS",
    "ALIGNMENT_VERSION",
    "LOCALIZATION_POLICY",
    "LOCALIZATION_POLICY_VERSION",
    "MAXIMUM_ACCEPTED_DISAGREEMENT_SECONDS",
    "MAX_BLOCK_SECONDS",
    "MINIMUM_ALIGNMENT_SCORE",
    "MINIMUM_LINE_SECONDS",
    "MINIMUM_TRUSTED_COARSE_CONFIDENCE",
    "ORDER_SUPPORT_MARGIN_SECONDS",
    "REPEATED_COLLAPSE_SECONDS",
    "REVIEW_DISAGREEMENT_SECONDS",
    "SECTION_WINDOW_OVERLAP_SECONDS",
    "alignable_lines",
    "anchor_supports_section_occurrence",
    "assemble_document",
    "authored_tokens",
    "build_blocks",
    "coarse_local_refinement_allowed",
    "coarse_proposals",
    "evidence_tokens",
    "interpolate",
    "monotonic_subset",
    "order_abstentions",
    "plan_localization",
    "repeated_phrase_abstentions",
    "spot_anchors",
    "token_time_map",
    "validate_full_coverage",
]
