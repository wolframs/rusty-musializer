#!/usr/bin/env python3
"""Opt-in audio phrase discovery and local, occurrence-constrained alignment.

The remote stage requires an explicitly confirmed route. Audio is decoded into
short overlapping WAV clips; each request has an independent context. Authored
lyrics stay local and retain caption authority when supplied. Discovery then
provides coarse evidence to the authored-line localizer, not replacement text.
No audio output device is opened by either stage.
"""
from __future__ import annotations

import argparse
import asyncio
from difflib import SequenceMatcher
import json
import math
from pathlib import Path
import re
import subprocess
import statistics
from types import SimpleNamespace

from analysis_io import atomic_write_json, canonical_sha256, sha256_file
import antigravity_audio
import lyric_align

VERSION = "11"
DISCOVERY_VERSION = "1"
ALIGNMENT_VERSION = "9"
CLIP_SECONDS = 30.0
STRIDE_SECONDS = 15.0
WINDOW_PADDING = 0.75
SCHEMA = "musializer.lyric-review/v1"
PROMPT = '''Listen directly to this audio clip. No tools. Transcribe every audible
sung or spoken lyric line, including repeats, backing vocals, ad-libs and chops.
Use only the audio, never an imagined lyric sheet or expected song structure.
Split at short natural performed phrases. Never output production directions
as lyrics. Instrumental sounds and vocal-like synthesizers are not words.
Return JSON only, no fences:
{"lines":[{"text":"words actually heard", "start_seconds":0.0,
"end_seconds":1.0,"uncertain":false,"partial_start":false,"partial_end":false}],"notes":[]}
Times are relative to the first audio sample of this clip. Start at the first
vocal phoneme and end at the last, including held vowels but excluding reverb.
Do not estimate timing from reading speed. Mark clipped phrases with partial
flags. Keep each line below 200 characters. An instrumental clip has no lines.
Do not omit audible speech between sung lines. Mark uncertain words honestly.
'''


def parse_response(raw: str, length: float) -> list[dict]:
    if not isinstance(raw, str) or len(raw.encode()) > 128 * 1024:
        raise ValueError("Antigravity transcript exceeds response limit")
    raw = raw.strip()
    if raw.startswith('```') and raw.endswith('```'):
        raw = raw.split('\n', 1)[1].rsplit('```', 1)[0]
    value = json.loads(raw)
    if not isinstance(value, dict) or not isinstance(value.get('lines'), list):
        raise ValueError("Antigravity response needs a lines array")
    if len(value['lines']) > 128:
        raise ValueError("Too many phrases in a short audio clip")
    lines = []
    for row in value['lines']:
        if not isinstance(row, dict):
            raise ValueError("Antigravity phrase is not an object")
        text = row.get('text')
        if (not isinstance(text, str) or not text.strip() or len(text.encode()) >= 512
                or any(ord(c) < 32 for c in text)):
            raise ValueError("Antigravity phrase has invalid caption text")
        start, end = row.get('start_seconds'), row.get('end_seconds')
        if (any(isinstance(x, bool) or not isinstance(x, (float, int))
                or not math.isfinite(x) for x in (start, end))
                or not 0 <= start < min(end, length) <= length
                or end > length + 0.02):
            raise ValueError("Antigravity phrase has invalid clip-relative timing")
        for flag in ('uncertain', 'partial_start', 'partial_end'):
            if not isinstance(row.get(flag), bool):
                raise ValueError(f"Antigravity phrase needs boolean {flag}")
        if not lyric_align.normalize_tokens(text):
            raise ValueError("Antigravity phrase contains no lyric words")
        lines.append(dict(text=text.strip(), start_seconds=float(start),
                          end_seconds=min(float(end), length),
                          **{flag: row[flag] for flag in
                             ('uncertain', 'partial_start', 'partial_end')}))
    return sorted(lines, key=lambda row: (row['start_seconds'], row['end_seconds']))


def clip_spans(length: float) -> list[tuple[float, float]]:
    if not math.isfinite(length) or length <= 0:
        raise ValueError("Audio duration must be finite and positive")
    spans = []
    for start in range(0, math.ceil(length), int(STRIDE_SECONDS)):
        spans.append((float(start), min(CLIP_SECONDS, length - start)))
        if start + CLIP_SECONDS >= length:
            break
    return spans


def _occurrence_tokens(text: str) -> list[str]:
    words = ['whoa' if word == 'woah' else word for word in lyric_align.normalize_tokens(text)]
    result = []
    for i, word in enumerate(words):
        following = i + 1
        while following < len(words) and words[following] == word:
            following += 1
        if len(word) <= 3 and following < len(words) and words[following].startswith(word) and words[following] != word:
            continue
        result.append(word)
    return result


def _same_occurrence(left: dict, right: dict) -> bool:
    # Never merge repetitions from the same observation. Cross-clip duplicates
    # need both textual and temporal agreement; mere text equality is not enough.
    left_views = set(left.get('evidence_indices', left.get('source_line_indices', [left['clip_index']])))
    right_views = set(right.get('evidence_indices', right.get('source_line_indices', [right['clip_index']])))
    if left_views & right_views:
        return False
    overlap = min(left['end_seconds'], right['end_seconds']) - max(left['start_seconds'], right['start_seconds'])
    shorter = min(left['end_seconds'] - left['start_seconds'], right['end_seconds'] - right['start_seconds'])
    if overlap <= 0 or overlap / shorter < 0.65:
        return False
    a, b = _occurrence_tokens(left['text']), _occurrence_tokens(right['text'])
    if SequenceMatcher(None, a, b, autojunk=False).ratio() >= 0.72:
        return True
    for partial, words, complete in ((left, a, b), (right, b, a)):
        if partial.get('partial_start') or partial.get('partial_end'):
            if any(complete[i:i + len(words)] == words for i in range(len(complete) - len(words) + 1)):
                return True
    return False


def _observation_rank(row: dict) -> tuple:
    return (row['partial_start'] or row['partial_end'], -row['edge_margin'], row['uncertain'])


def _sequence_matches(left: list[dict], right: list[dict]) -> list[tuple[int, int]]:
    """Pair separately cropped observations in performance order.

    Approximate model clocks may drift enough that identical phrases no longer
    overlap. A temporal-overlap deduper then emits the whole passage twice.
    Sequence matching tolerates that drift while assigning each delivery only
    once, including adjacent identical choruses. It does not adjust any clock.
    """
    n, m = len(left), len(right)
    scores = [[0.0] * (m + 1) for _ in range(n + 1)]
    actions = [[0] * (m + 1) for _ in range(n + 1)]
    left_words = [_occurrence_tokens(row['text']) for row in left]
    right_words = [_occurrence_tokens(row['text']) for row in right]
    # Explicit written elision ("cookin'") can describe the same performance as
    # "cooking". Use it only with shared phrase context and nearby edges; do not
    # generally fuzzy-correct short phrases or change their displayed wording.
    def elision_words(text):
        return _occurrence_tokens(re.sub(r"\b([a-z]+)in['’](?=\W|$)", r"\1ing", text, flags=re.I))
    left_elisions = [elision_words(row['text']) for row in left]
    right_elisions = [elision_words(row['text']) for row in right]
    for i, old in enumerate(left, 1):
        for j, new in enumerate(right, 1):
            best, action = (scores[i - 1][j], 1) if scores[i - 1][j] >= scores[i][j - 1] else (scores[i][j - 1], 2)
            distance = abs(old['start_seconds'] - new['start_seconds'])
            a, b = left_words[i - 1], right_words[j - 1]
            similarity = SequenceMatcher(None, a, b, autojunk=False).ratio()
            if (similarity < 0.72 and len(a) >= 2 and distance <= 1.0
                    and abs(old['end_seconds'] - new['end_seconds']) <= 1.0
                    and left_elisions[i - 1] == right_elisions[j - 1]):
                similarity = 0.9
            if similarity < 0.72:
                for partial, words, whole in ((old, a, b), (new, b, a)):
                    if (partial['partial_start'] or partial['partial_end']) and words and any(
                            whole[k:k + len(words)] == words for k in range(len(whole) - len(words) + 1)):
                        similarity = 0.8
            if distance <= 4.0 and similarity >= 0.72:
                paired = scores[i - 1][j - 1] + 3 * similarity - 0.15 * distance
                if paired > best:
                    best, action = paired, 3
            scores[i][j], actions[i][j] = best, action
    pairs = []
    i, j = n, m
    while i and j:
        action = actions[i][j]
        if action == 3:
            pairs.append((i - 1, j - 1)); i -= 1; j -= 1
        elif action == 1:
            i -= 1
        else:
            j -= 1
    return list(reversed(pairs))


def merge_observations(reports: list[dict], length: float) -> tuple[list[dict], list[dict]]:
    chosen = []
    for index, report in enumerate(reports):
        offset, seconds = report['clip_start_seconds'], report['clip_duration_seconds']
        observations = []
        for row in report['lines']:
            observations.append({**row, 'clip_index': index,
                'start_seconds': offset + row['start_seconds'],
                'end_seconds': offset + row['end_seconds'],
                'edge_margin': min(row['start_seconds'], seconds - row['end_seconds']),
                'evidence_indices': [index],
                'timing_observations': [dict(start_seconds=offset + row['start_seconds'],
                    end_seconds=offset + row['end_seconds'], clip_index=index,
                    complete=not (row['partial_start'] or row['partial_end']))]})
        # Earlier non-overlapping windows cannot observe this performance.
        eligible = [i for i, row in enumerate(chosen)
                    if row['end_seconds'] >= offset - 0.5
                    and row['start_seconds'] <= offset + seconds + 0.5]
        pairs = _sequence_matches([chosen[i] for i in eligible], observations)
        used = set()
        for old_index, new_index in pairs:
            old_index = eligible[old_index]
            old, new = chosen[old_index], observations[new_index]
            used.add(new_index)
            winner = dict(min((old, new), key=_observation_rank))
            winner['timing_observations'] = old['timing_observations'] + new['timing_observations']
            winner['evidence_indices'] = sorted(set(old['evidence_indices'] + new['evidence_indices']))
            complete = [view for view in winner['timing_observations'] if view['complete']]
            if complete:
                for field in ('start_seconds', 'end_seconds'):
                    winner[field] = statistics.median(view[field] for view in complete)
            # A better-centered crop can replace the displayed wording, but
            # cannot erase a conflict already recorded between earlier views.
            disagreement = old.get('observation_disagreement', False) or new.get('observation_disagreement', False)
            if (disagreement or (not (old['partial_start'] or old['partial_end'] or new['partial_start'] or new['partial_end'])
                    and (abs(old['start_seconds'] - new['start_seconds']) > 0.5
                         or abs(old['end_seconds'] - new['end_seconds']) > 0.5
                         or lyric_align.normalize_tokens(old['text']) != lyric_align.normalize_tokens(new['text'])))):
                winner['uncertain'] = True
                winner['observation_disagreement'] = True
            chosen[old_index] = winner
        chosen.extend(row for i, row in enumerate(observations) if i not in used)
        chosen.sort(key=lambda row: row['start_seconds'])
    result, unresolved = [], []
    for row in chosen:
        if ((row['partial_start'] and row['start_seconds'] > 0.15)
                or (row['partial_end'] and row['end_seconds'] < length - 0.15)):
            unresolved.append(row)
        else:
            result.append(row)
    return result, unresolved


def corroborated(row: dict) -> bool:
    return len({view['clip_index'] for view in row['timing_observations'] if view['complete']}) >= 2


def confirmation_spans(rows: list[dict], length: float) -> list[tuple[float, float]]:
    """Short fresh contexts for single-view phrases, without their words/times.

    The windows come from proposals; the model receives only their audio. Nearby
    targets share a request, bounded to 18 seconds except for a longer phrase.
    """
    targets = sorted((row for row in rows if not corroborated(row)), key=lambda row: row['start_seconds'])
    groups = []
    for row in targets:
        left, right = max(0, row['start_seconds'] - 3), min(length, row['end_seconds'] + 3)
        if groups and right - groups[-1][0] <= 18:
            groups[-1][1] = max(groups[-1][1], right)
        else:
            groups.append([left, right])
    spans = []
    for left, right in groups:
        seconds = min(length, 30, max(8, right - left))
        start = min(max(0, (left + right - seconds) / 2), length - seconds)
        spans.append((round(start, 6), round(seconds, 6)))
    return sorted(set(spans))


async def observe_spans(audio: Path, identity: dict, cache: Path, paths: dict,
                        *, timeout: float = 180, prompt: str = PROMPT) -> list[dict]:
    """Read or request isolated audio observations under one complete cache identity.

    Callers must validate the job's audio-transfer authorization first.
    """
    cache.mkdir(parents=True, exist_ok=True)
    model = identity["model"]
    semaphore = asyncio.Semaphore(2)

    async def observe(index, start, seconds):
        receipt = cache / f'{index:04}.json'
        if receipt.is_file() and receipt.stat().st_size <= 512 * 1024:
            try:
                report = json.loads(receipt.read_text())
                if report['identity'] == json.loads(json.dumps(identity)) and report['clip_start_seconds'] == start and report['clip_duration_seconds'] == seconds:
                    report['lines'] = parse_response(report['response'], seconds)
                    return report
            except (KeyError, ValueError, TypeError):
                pass
        async with semaphore:
            clip = await asyncio.to_thread(subprocess.check_output, [
                'ffmpeg', '-v', 'error', '-i', str(audio), '-ss', str(start),
                '-t', str(seconds), '-vn', '-ac', '1', '-ar', '16000',
                '-map_metadata', '-1', '-f', 'wav', 'pipe:1'], timeout=60)
            args = SimpleNamespace(**paths, model=model, timeout=timeout)
            validation_error = None
            for attempt in range(2):
                request_prompt = prompt if attempt == 0 else prompt + (
                    f"\nYour previous response was rejected: {validation_error}. "
                    f"This clip is exactly {seconds:.6f} seconds long. "
                    "Listen again; every timestamp must lie inside this clip. "
                    "Mark words cut by the clip boundary as partial. Return one JSON "
                    "object only, with all required boolean flags.")
                response = await antigravity_audio.ask(args, clip, request_prompt)
                try:
                    rows = parse_response(response['response'], seconds)
                    break
                except ValueError as error:
                    validation_error = str(error)
                    atomic_write_json(cache / f'{index:04}.attempt-{attempt}.failed.json',
                        dict(response=response['response'], error=str(error),
                             model=response['model'], agent=response['agent'],
                             clip_start_seconds=start, clip_duration_seconds=seconds))
                    if attempt:
                        raise ValueError(f"Antigravity clip {index + 1} returned invalid lyrics: {error}") from error
            report = dict(identity=identity, clip_start_seconds=start,
                          clip_duration_seconds=seconds, lines=rows,
                          response=response['response'], agent=response['agent'],
                          model=response['model'], session_id=response['session_id'],
                          clip_sha256=__import__('hashlib').sha256(clip).hexdigest(),
                          request_prompt_sha256=canonical_sha256(request_prompt), attempt=attempt)
            atomic_write_json(receipt, report)
            print(f'Antigravity lyrics: clip {index + 1}/{len(identity["spans"])} complete', flush=True)
            return report

    async def gather_spans():
        tasks = [asyncio.create_task(observe(i, start, seconds))
                 for i, (start, seconds) in enumerate(identity['spans'])]
        try:
            return await asyncio.gather(*tasks)
        finally:
            for task in tasks:
                if not task.done():
                    task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)

    return await gather_spans()


async def transcribe(audio: Path, output: Path, *, length: float, model: str,
                     confirmed: bool, reference: dict | None = None,
                     server=None, harness=None, profile=None, timeout=180.0) -> dict:
    # Check consent before discovery, cache access, decoding or subprocess spawn.
    if confirmed is not True:
        raise ValueError("Antigravity audio transfer requires confirmation for this job")
    if model not in antigravity_audio.MODELS:
        raise ValueError("Unsupported Antigravity lyric model")
    paths = antigravity_audio.discover(server=server, harness=harness, profile=profile)
    audio_sha = sha256_file(audio)
    # Complete lyric sheets caused entire expected verses to be hallucinated
    # over instrumental passages. Keep authored text local for comparison;
    # performance discovery receives audio alone.
    prompt = PROMPT
    identity = dict(version=DISCOVERY_VERSION, audio_sha256=audio_sha, duration_seconds=length,
                    model=model, prompt_sha256=canonical_sha256(prompt),
                    runtime_sha256=sha256_file(paths['server']),
                    harness_sha256=sha256_file(paths['harness']),
                    spans=clip_spans(length))
    cache = output.parent / 'antigravity-clips' / canonical_sha256(identity)
    cache.mkdir(parents=True, exist_ok=True)
    reports = await observe_spans(audio, identity, cache, paths, timeout=timeout, prompt=prompt)
    discovery_identity = identity
    lines, unresolved = merge_observations(reports, length)
    # Authored lyrics already decide the display text. Keep single-view phrases
    # as coarse evidence; filtering them here can make real authored lines
    # disappear before the acoustic localizer has examined them.
    spans = ([] if reference is not None else
             [span for span in confirmation_spans(lines + unresolved, length)
              if span not in discovery_identity['spans']])
    confirmation_identity = None
    if spans:
        identity = {**discovery_identity, 'phase': 'confirmation', 'policy_version': '1',
                    'discovery_sha256': canonical_sha256(reports), 'spans': spans}
        confirmation_identity = identity
        cache = output.parent / 'antigravity-confirmations' / canonical_sha256(identity)
        cache.mkdir(parents=True, exist_ok=True)
        print(f'Antigravity lyrics: confirming single-view phrases in {len(spans)} fresh audio clips', flush=True)
        reports.extend(await observe_spans(audio, identity, cache, paths, timeout=timeout, prompt=prompt))
        reports.sort(key=lambda report: (report['clip_start_seconds'], report['clip_duration_seconds']))
        lines, unresolved = merge_observations(reports, length)
    unconfirmed = []
    if reference is None:
        unconfirmed = [row for row in lines if not corroborated(row)]
        lines = [row for row in lines if corroborated(row)]
    for row in lines:
        row['source_line_indices'] = row.pop('evidence_indices')
        row['confidence'] = 0.5 if row['uncertain'] else 0.8
    result = dict(schema_version=('musializer.lyric-timing/v1' if reference is not None else SCHEMA),
        lane='lyrics' if reference is not None else 'lyric_review',
        audio=dict(sha256=audio_sha, duration_seconds=length),
        source=dict(schema_version='musializer.antigravity-clips/v1',
                    sha256=canonical_sha256(reports), reference=reference,
                    reference_scope='local-only',
                    confirmation_identity=confirmation_identity,
                    unconfirmed_phrases=unconfirmed,
                    observation_clips=[dict(start_seconds=report['clip_start_seconds'],
                        duration_seconds=report['clip_duration_seconds'],
                        clip_sha256=report['clip_sha256'],
                        phase=report['identity'].get('phase', 'discovery')) for report in reports],
                    boundary_observations=[dict(
                        clip_start_seconds=report['clip_start_seconds'],
                        clip_duration_seconds=report['clip_duration_seconds'],
                        lines=report['lines']) for report in reports] if reference is not None else [],
                    text_authority=('coarse-audio-evidence' if reference is not None else 'audio-model-proposal'),
                    unresolved_clipped_phrases=unresolved),
        provenance=dict(adapter='tools/antigravity_lyrics.py', adapter_version=VERSION,
                        source_kind='antigravity_audio', model=model,
                        prompt_version='antigravity-lyrics/2', request_settings=discovery_identity,
                        agent=reports[0]['agent'], audio_sha256=audio_sha),
        lines=lines, notes=[('Audio-derived text is timing evidence; supplied lyrics retain caption authority.'
                            if reference is not None else
                            'Audio-derived wording and timing are proposals for review.')] +
            ([f'{len(unresolved)} clipped phrases need review.'] if unresolved else []))
    atomic_write_json(output, result)
    return result


def fused_boundaries(row: dict) -> tuple[float, float]:
    """Use corroborating audio observations to resist one wrong acoustic path.

    This is a robust point estimate, not a calibrated error probability. A lone
    model observation keeps its proposed interval; it cannot become independent
    evidence just because its text was forced through CTC.
    """
    observed = [view for view in row.get('timing_observations', []) if view['complete']]
    boundaries = []
    for field in ('start_seconds', 'end_seconds'):
        points = [view[field] for view in observed]
        if len(points) > 1:
            points.append(row[field])
        boundaries.append(statistics.median(points) if points else row[field])
    return tuple(boundaries)


def competing_observations(rows: list[dict]) -> tuple[list[dict], list[dict]]:
    """Keep competing cross-clip transcriptions available without captioning both.

    Shared observation evidence proves the model heard simultaneous deliveries
    in one context; preserve those. Otherwise overlapping alternatives need
    review, and the observation with more corroboration/context is the proposal.
    This is not a claim that the parked version was absent from the performance.
    """
    retained, alternatives = [], []
    fragments = set()
    for i, row in enumerate(rows):
        words = _occurrence_tokens(row['text'])
        if len(words) < 2:
            continue
        for j, parent in enumerate(rows):
            if i == j or len(parent['source_line_indices']) < 2:
                continue
            whole = _occurrence_tokens(parent['text'])
            if (len(whole) >= len(words) + 2
                    and parent['start_seconds'] <= row['start_seconds']
                    and parent['end_seconds'] >= row['end_seconds']
                    and any(whole[k:k + len(words)] == words for k in range(len(whole) - len(words) + 1))):
                # A full phrase and its separately emitted substring are two
                # representations of one caption, even if a single crop emitted
                # both. Equal-text repeated deliveries cannot enter this branch.
                fragments.add(i)
                parent['uncertain'] = True
                parent['confidence'] = 0.5
                parent['alternative_observation'] = True
                break
    alternatives.extend(row for i, row in enumerate(rows) if i in fragments)
    rows = [row for i, row in enumerate(rows) if i not in fragments]
    for row in sorted(rows, key=lambda r: (-len(r['source_line_indices']),
                                          -r.get('edge_margin', 0))):
        conflict = None
        for old in retained:
            if set(old['source_line_indices']) & set(row['source_line_indices']):
                continue
            overlap = min(old['end_seconds'], row['end_seconds']) - max(old['start_seconds'], row['start_seconds'])
            shortest = min(old['end_seconds'] - old['start_seconds'], row['end_seconds'] - row['start_seconds'])
            if overlap > 0.5 * shortest:
                conflict = old
                break
        if conflict is None:
            retained.append(row)
        else:
            conflict['uncertain'] = True
            conflict['confidence'] = 0.5
            conflict['alternative_observation'] = True
            alternatives.append(row)
    return retained, alternatives


def observation_window(row: dict, length: float) -> tuple[float, float]:
    # Every complete crop is evidence for the search interval. Constraining CTC
    # to just the preferred crop can exclude the correct occurrence when that
    # crop's approximate clock drifts away from the corroborating observation.
    views = [row] + [view for view in row.get('timing_observations', []) if view['complete']]
    return (max(0.0, min(view['start_seconds'] for view in views) - WINDOW_PADDING),
            min(length, max(view['end_seconds'] for view in views) + WINDOW_PADDING))


def align(audio: Path, source_path: Path, output: Path) -> dict:
    import torch
    import torchaudio
    import anchor_block_align
    import ctc_window_align
    import force_align_lyrics
    source = json.loads(source_path.read_text())
    length = source['audio']['duration_seconds']
    force_align_lyrics._validate_lane(source, length)
    if source['audio']['sha256'] != sha256_file(audio):
        raise ValueError('Antigravity transcript belongs to different audio')
    waveform, rate = anchor_block_align._load_audio(audio)
    bundle = torchaudio.pipelines.MMS_FA
    device = 'cuda' if torch.cuda.is_available() else 'cpu'
    model = bundle.get_model(with_star=True).to(device).eval()
    tokenizer = bundle.get_tokenizer()
    result = json.loads(json.dumps(source))
    # Each observed phrase has its own time constraint. Group only consecutive
    # non-overlapping phrases; simultaneous backing vocals use separate paths.
    groups = []
    for i, row in enumerate(source['lines']):
        if (not groups or row['end_seconds'] - source['lines'][groups[-1][0]]['start_seconds'] > 30
                or row['start_seconds'] < source['lines'][groups[-1][-1]]['end_seconds']):
            groups.append([])
        groups[-1].append(i)
    for group in groups:
        first, last = group[0], group[-1]
        intervals = [observation_window(row, length) for row in source['lines'][first:last + 1]]
        left = min(a for a, _ in intervals)
        right = max(b for _, b in intervals)
        lines = [dict(display=row['text']) for row in source['lines'][first:last + 1]]
        windows = [None]
        for i, row in enumerate(source['lines'][first:last + 1]):
            if i:
                windows.append(None)
            words = anchor_block_align.alignment_words(row['text'])
            windows.extend([(intervals[i][0] - left, intervals[i][1] - left)] * len(words))
        windows.append(None)

        def constrained(emission, tokens):
            frames = len(emission)
            bounds = [None if window is None else (
                max(0, math.floor(window[0] / (right - left) * frames)),
                min(frames, math.ceil(window[1] / (right - left) * frames))) for window in windows]
            if len(tokens) != len(bounds):
                raise ValueError('CTC token/window graph mismatch')
            spans = iter(ctc_window_align.align(emission.numpy(),
                [token for word in tokens for token in word],
                [window for word, window in zip(tokens, bounds) for _ in word]))
            return [[SimpleNamespace(start=a, end=b, score=s)
                     for a, b, s in [next(spans) for _ in word]] for word in tokens]
        try:
            decisions = anchor_block_align.align_block(waveform, rate, lines,
                dict(first_line=0, last_line=len(lines) - 1, window_start=left, window_end=right),
                model, tokenizer, constrained, interior_stars=True)
        except ValueError as error:
            decisions = {i: dict(status='unresolved', reason=str(error)) for i in range(len(lines))}
        for local_index, index in enumerate(group):
            row = result['lines'][index]
            decision = decisions.get(local_index, dict(status='unresolved'))
            row['forced_alignment'] = {**decision,
                'input_start_seconds': row['start_seconds'], 'input_end_seconds': row['end_seconds']}
            if decision['status'] in ('aligned', 'weak'):
                start, end = decision['acoustic_start_seconds'], decision['acoustic_end_seconds']
                delta = max(abs(start - row['start_seconds']), abs(end - row['end_seconds']))
                row['start_seconds'], row['end_seconds'] = start, end
                row['uncertain'] |= delta > 0.5
            else:
                row['uncertain'] = True
            row['confidence'] = 0.5 if row['uncertain'] else 0.8
    # The acoustic pass may expose a cross-clip duplicate that approximate
    # model timestamps did not overlap enough to merge. Never deduplicate two
    # separately performed repetitions from the same audio observation.
    retained, duplicates = [], []
    for row in sorted(result['lines'], key=lambda r: (r['uncertain'], -r.get('edge_margin', 0))):
        existing = next((old for old in retained if _same_occurrence(old, row)), None)
        if existing is None:
            retained.append(row)
        else:
            existing.setdefault('timing_observations', []).extend(row.get('timing_observations', []))
            existing['source_line_indices'] = sorted(set(existing['source_line_indices'] + row['source_line_indices']))
            duplicates.append(row)
    result['source']['duplicate_observations'] = duplicates
    for row in retained:
        row['start_seconds'], row['end_seconds'] = fused_boundaries(row)
    retained, alternatives = competing_observations(retained)
    result['source']['competing_observations'] = alternatives
    result['lines'] = retained
    result['lines'].sort(key=lambda row: (row['start_seconds'], row['end_seconds']))
    result['review_flags'] = [dict(cue_index=i, flag='audio_uncertainty', text=row['text'],
        start_seconds=row['start_seconds'], end_seconds=row['end_seconds'],
        reason=('Competing audio transcription is parked as a Potential cue'
                if row.get('alternative_observation') else
                'Audio observations or acoustic boundaries need review'))
        for i, row in enumerate(result['lines']) if row['uncertain']]
    result['unresolved'] = []
    result['performed_candidates'] = [dict(text=row['text'],
        start_seconds=row['start_seconds'], end_seconds=row['end_seconds'],
        confidence=0.5, uncertain=True, source='antigravity-clipped',
        reason='Phrase clipped by an observation window')
        for row in source['source'].get('unresolved_clipped_phrases', [])]
    result['performed_candidates'].extend(dict(text=row['text'],
        start_seconds=row['start_seconds'], end_seconds=row['end_seconds'],
        confidence=0.5, uncertain=True, source='antigravity-alternative',
        reason='Competing transcription from another audio observation')
        for row in alternatives)
    result['performed_candidates'].extend(dict(text=row['text'],
        start_seconds=row['start_seconds'], end_seconds=row['end_seconds'],
        confidence=0.5, uncertain=True, source='antigravity-unconfirmed',
        reason='Not corroborated by a second independent audio observation')
        for row in source['source'].get('unconfirmed_phrases', []))
    result['timing_refinement'] = dict(adapter='tools/antigravity_lyrics.py',
        alignment_version=ALIGNMENT_VERSION, model=force_align_lyrics.MODEL_ID,
        source_sha256=sha256_file(source_path), window_padding_seconds=WINDOW_PADDING,
        boundary_policy='median-audio-observations-and-ctc')
    # TC-ALIGN identifies the local acoustic model, while source retains the
    # remote discovery provenance and its receipts.
    result['source']['discovery_provenance'] = result['provenance']
    result['provenance'] = dict(adapter='tools/antigravity_lyrics.py',
        adapter_version=ALIGNMENT_VERSION, source_kind='antigravity_window_ctc',
        model=force_align_lyrics.MODEL_ID, audio_sha256=source['audio']['sha256'])
    atomic_write_json(output, result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('audio', type=Path)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    align(args.audio, args.source, args.output)


if __name__ == '__main__':
    main()
