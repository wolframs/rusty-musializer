"""Recover performed phrases from separately cropped local Whisper/MMS passes.

The sheet supplies candidate wording, never presence evidence. Each accepted
phrase must be transcribed in two different crops and have agreeing acoustic
word edges. All recovered captions remain marked for review. This module has
no remote-provider calls.
"""
from __future__ import annotations

import copy
from difflib import SequenceMatcher
import math
import json
from pathlib import Path
import re
import statistics
import tempfile
import time

from analysis_io import atomic_write_json, canonical_sha256, sha256_file
import lyric_align

VERSION = "3"
CROP_VERSION = "1"
EDGE_TOLERANCE = 0.35


def tokens(text):
    # A written stutter still names one lexical word. Preserve its display text.
    text = re.sub(r"\b([a-z]{1,3})(?:-\1)*-([a-z]+)\b",
                  lambda m: m[2] if m[2].lower().startswith(m[1].lower()) else m[0],
                  text, flags=re.I)
    return tuple(lyric_align.normalize_tokens(text))


def templates(reference, reports):
    choices = {}
    # Audio templates come first; an exact written match can supply spelling.
    for report in reports:
        for row in report['lines']:
            text = row['text'].replace('♪', '').strip()
            key = tokens(text)
            if 2 <= len(key) <= 16 and len(set(key)) >= 2:
                choices.setdefault(key, (text, False))
    written = {}
    for row in lyric_align.classify_reference_lines(reference):
        if row['kind'] not in ('lyric', 'backing'):
            continue
        for text in [row['display'], *re.split(r'[()]', row['display'])]:
            text = text.strip(' ."“”')
            key = tokens(text)
            if 2 <= len(key) <= 16 and len(set(key)) >= 2:
                written.setdefault(key, (text, True))
    choices.update(written)
    # An explicitly repeated whole phrase is two occurrences, not a larger
    # template that can hide their internal boundary.
    return {key: text for key, text in choices.items() if not any(
        len(key) % size == 0 and key == key[:size] * (len(key) // size)
        for size in range(2, len(key) // 2 + 1))}


def acoustic_words(row):
    words = row.get('word_alignments', [])
    stutters = {(m[1].lower(), m[2].lower()) for m in re.finditer(
        r"\b([a-z]{1,3})(?:-\1)*-([a-z]+)\b", row['text'], re.I)
        if m[2].lower().startswith(m[1].lower())}
    result = []
    index = 0
    while index < len(words):
        word = words[index]
        key = tokens(word['text'])
        following = index + 1
        while following < len(words) and words[following]['text'] == word['text']:
            following += 1
        if following < len(words) and (word['text'], words[following]['text']) in stutters:
            result.append((words[following]['text'], word['start_seconds'],
                           words[following]['end_seconds']))
            index = following + 1
        else:
            result.append((key[0], word['start_seconds'], word['end_seconds'])
                          if len(key) == 1 else (None, 0.0, 0.0))
            index += 1
    return result


def observations(reports, reference):
    choices = templates(reference, reports)
    found = []
    seen = set()
    for report in reports:
        crop = (report['start'], report['seconds'])
        if crop in seen:
            continue
        seen.add(crop)
        words = []
        for row in report['lines']:
            if row.get('status') not in ('aligned', 'weak'):
                continue
            # Whole-crop CTC can squeeze later lyrics into an earlier sound.
            # Keep ASR's independently located interval as an occurrence guard.
            coarse_start, coarse_end = row.get('asr_start_seconds'), row.get('asr_end_seconds')
            if coarse_start is None or coarse_end is None:
                continue
            words.extend((word, start, end) if start >= coarse_start - 1.0 and end <= coarse_end + 1.0
                         else (None, 0.0, 0.0) for word, start, end in acoustic_words(row))
        sequence = [word[0] for word in words]
        for key, (text, authored) in choices.items():
            for index in range(len(words) - len(key) + 1):
                if tuple(sequence[index:index + len(key)]) != key:
                    continue
                selected = words[index:index + len(key)]
                start, end = selected[0][1], selected[-1][2]
                if (not all(math.isfinite(a) and math.isfinite(b) and a < b
                            for _, a, b in selected)
                        or start < crop[0] + 0.25 or end > sum(crop) - 0.25
                        or any(b[1] < a[2] or b[1] - a[2] > 1.0
                               for a, b in zip(selected, selected[1:]))
                        or any(b - a > 2.5 for _, a, b in selected)):
                    continue
                found.append(dict(text=text, key=key, authored=authored, crop=crop,
                                  start_seconds=start, end_seconds=end))
    return found


def corroborate(reports, reference):
    hits = observations(reports, reference)
    groups = []
    for seed in hits:
        views = [seed]
        for hit in hits:
            if hit['key'] != seed['key'] or hit['crop'] in [v['crop'] for v in views]:
                continue
            if any(abs(hit['crop'][0] - v['crop'][0]) < 3.0 for v in views):
                continue
            if all(abs(hit[edge] - v[edge]) <= EDGE_TOLERANCE
                   for v in views for edge in ('start_seconds', 'end_seconds')):
                views.append(hit)
        if len(views) < 2:
            continue
        groups.append(dict(text=seed['text'], key=seed['key'], authored=seed['authored'], observations=views,
            **{edge: statistics.median(v[edge] for v in views)
               for edge in ('start_seconds', 'end_seconds')}))
    # Prefer a complete phrase over an interior template supported by the same
    # word spans. Time participates: equal-text echoes remain separate events.
    accepted = []
    for row in sorted(groups, key=lambda x: (not x['authored'], -len(x['key']), -len(x['observations']))):
        if any(row['start_seconds'] >= old['start_seconds'] - EDGE_TOLERANCE
               and row['end_seconds'] <= old['end_seconds'] + EDGE_TOLERANCE
               and any(old['key'][i:i + len(row['key'])] == row['key']
                       for i in range(len(old['key']) - len(row['key']) + 1))
               for old in accepted):
            continue
        if any(old['start_seconds'] >= row['start_seconds'] - EDGE_TOLERANCE
               and old['end_seconds'] <= row['end_seconds'] + EDGE_TOLERANCE
               and any(row['key'][i:i + len(old['key'])] == old['key']
                       for i in range(len(row['key']) - len(old['key']) + 1))
               for old in accepted):
            continue
        accepted.append(row)
    return sorted(accepted, key=lambda x: x['start_seconds'])


def overlap(a, b):
    shared = min(a['end_seconds'], b['end_seconds']) - max(a['start_seconds'], b['start_seconds'])
    shorter = min(a['end_seconds'] - a['start_seconds'], b['end_seconds'] - b['start_seconds'])
    return shared / shorter if shorter > 0 else 0.0


def recover(document, reports, reference):
    result = copy.deepcopy(document)
    heard = corroborate(reports, reference)
    additions, challenged, retimed = [], [], []
    consumed = set()
    kept = []
    for cue in result['lines']:
        key = tokens(cue['text'])
        matches = [(i, event) for i, event in enumerate(heard) if overlap(cue, event) >= 0.5]
        exact = [(i, e) for i, e in matches if e['key'] == key]
        if len(exact) == 1:
            i, event = exact[0]
            consumed.add(i)
            before = {edge: cue[edge] for edge in ('start_seconds', 'end_seconds')}
            for edge in before:
                cue[edge] = event[edge]
            cue['uncertain'] = cue['review_flagged'] = True
            cue['local_observations'] = event['observations']
            retimed.append(dict(reference_line_index=cue['reference_line_index'], before=before,
                                after={edge: cue[edge] for edge in before}))
            kept.append(cue)
            continue
        # A recognizer's nearby spelling disagreement must not erase authored
        # wording. Challenge only unrelated wording, or a compound written cue
        # whose performed subphrase has separately corroborated boundaries.
        compound = any(len(e['key']) < len(key) and any(
            key[j:j + len(e['key'])] == e['key'] for j in range(len(key) - len(e['key']) + 1))
            for _, e in matches)
        unrelated = bool(matches) and all(
            SequenceMatcher(None, key, e['key'], autojunk=False).ratio() < 0.5
            for _, e in matches)
        if compound or unrelated:
            reason = 'local_performance_differs_from_written_line'
            record = dict(reference_line_index=cue['reference_line_index'],
                line_position=cue['line_position'], kind=cue['kind'], text=cue['text'],
                reason=reason, abstained=True,
                coarse_start_seconds=cue.get('coarse_start_seconds', cue['start_seconds']),
                coarse_end_seconds=cue.get('coarse_end_seconds', cue['end_seconds']),
                acoustic_start_seconds=cue['start_seconds'], acoustic_end_seconds=cue['end_seconds'])
            result['unresolved'].append(record)
            result['unmatched'].append(dict(reference_line_index=cue['reference_line_index'],
                                             text=cue['text'], reason=reason))
            challenged.append(record)
        else:
            kept.append(cue)
    for i, event in enumerate(heard):
        if i in consumed:
            continue
        # Similar authored wording already represents this occurrence.
        if any(overlap(cue, event) >= 0.5 and SequenceMatcher(
                None, tokens(cue['text']), event['key'], autojunk=False).ratio() >= 0.5
               for cue in kept):
            continue
        additions.append(dict(text=event['text'], text_scope='audio_observed',
            reference_line_indices=[], confidence=None, uncertain=True,
            reason='Local audio wording; independently cropped Whisper and MMS agree',
            occurrence_evidence=dict(local_observations=event['observations']),
            start_seconds=event['start_seconds'], end_seconds=event['end_seconds']))
    result['lines'] = sorted(kept, key=lambda x: x['start_seconds'])
    result['performed_occurrences'] = additions
    # Preserve earlier review reasons, updating coordinates after retiming.
    kept_by_index = {row['reference_line_index']: row for row in kept}
    challenged_indices = {row['reference_line_index'] for row in challenged}
    flags = []
    for flag in result.get('review_flags', []):
        index = flag['reference_line_index']
        if index in challenged_indices:
            continue
        if index in kept_by_index:
            cue = kept_by_index[index]
            flag.update(start_seconds=cue['start_seconds'], end_seconds=cue['end_seconds'])
        flags.append(flag)
    flags.extend(dict(reference_line_index=row['reference_line_index'],
        text=row['text'], flag='unresolved', reason=row['reason'], start_seconds=None,
        end_seconds=None, coarse_start_seconds=row.get('coarse_start_seconds'), delta_seconds=None)
        for row in challenged)
    retimed_indices = {row['reference_line_index'] for row in retimed}
    flags.extend(dict(reference_line_index=row['reference_line_index'],
        text=row['text'], flag='local_audio_recovery', reason='Check locally corroborated timing',
        start_seconds=row['start_seconds'], end_seconds=row['end_seconds'],
        coarse_start_seconds=row.get('coarse_start_seconds'), delta_seconds=None)
        for row in kept if row['reference_line_index'] in retimed_indices)
    result['review_flags'] = flags
    result['local_recovery'] = dict(version=VERSION, corroborated=len(heard),
        added=len(additions), challenged=challenged, retimed=retimed,
        reports_sha256=canonical_sha256(reports), boundary_tolerance_seconds=EDGE_TOLERANCE)
    stats = result['statistics']
    stats.update(matched_lines=len(kept), unresolved_lines=len(result['unresolved']),
        unmatched_lines=len(result['unresolved']), abstained_lines=sum(bool(row.get('abstained')) for row in result['unresolved']),
        review_flagged_lines=len(result['review_flags']), performed_occurrences=len(additions))
    if 'timing_refinement' in result:
        result['timing_refinement']['statistics'].update(stats)
    return result


def crop_spans(length):
    starts = [float(start + offset) for start in range(0, math.ceil(length), 15)
              for offset in (0, 5)]
    return [(start, min(30.0, length - start)) for start in starts if length - start >= 3.0]


def cache_accepts(document, binary, model):
    if binary is None or model is None:
        return True
    identity = document.get('local_recovery', {}).get('request_identity', {})
    return (document.get('local_recovery', {}).get('version') == VERSION
            and identity.get('version') == CROP_VERSION
            and identity.get('whisper_binary_sha256') == sha256_file(binary)
            and identity.get('whisper_model_sha256') == sha256_file(model))


def run(audio: Path, document, reference, cache: Path, whisper_bin: Path, whisper_model: Path):
    # Imports stay here: pure policy tests need neither torch nor an ASR runtime.
    import anchor_block_align
    import external_analysis
    import soundfile
    import torch
    import torchaudio
    started = time.monotonic()
    length = document['audio']['duration_seconds']
    audio_sha = sha256_file(audio)
    if document['audio']['sha256'] != audio_sha:
        raise ValueError('Local recovery evidence belongs to different audio')
    identity = dict(version=CROP_VERSION, audio_sha256=audio_sha, duration_seconds=length,
        whisper_binary_sha256=sha256_file(whisper_bin), whisper_model_sha256=sha256_file(whisper_model),
        whisper_request=external_analysis._whisper_request_settings(language='en', dtw_model=None,
            model_sha256=sha256_file(whisper_model), vad_model=external_analysis.whisper_vad_model()))
    directory = cache / canonical_sha256(identity)
    directory.mkdir(parents=True, exist_ok=True)
    cached_reports = {}
    for saved in directory.glob('*.json'):
        try:
            cached = json.loads(saved.read_text())
            if isinstance(cached.get('request'), dict) and isinstance(cached.get('lines'), list):
                source_path = saved.with_suffix('.whisper.json')
                if source_path.exists():
                    source = json.loads(source_path.read_text())
                    if canonical_sha256(source) == cached.get('whisper_sha256'):
                        source_lines = [row for row in source['lines'] if tokens(row['text'])]
                        for row, coarse in zip(cached['lines'], source_lines):
                            if row['text'] != coarse['text']:
                                raise ValueError('Crop transcript identity mismatch')
                            row.update(asr_start_seconds=cached['start'] + coarse['start_seconds'],
                                       asr_end_seconds=cached['start'] + coarse['end_seconds'])
                if all('asr_start_seconds' in row for row in cached['lines']):
                    cached_reports[canonical_sha256(cached['request'])] = cached
        except (OSError, ValueError):
            pass
    waveform, rate = anchor_block_align._load_audio(audio)
    bundle = torchaudio.pipelines.MMS_FA
    model = bundle.get_model(with_star=True).to(torch.device('cuda')).eval()
    tokenizer, aligner = bundle.get_tokenizer(), bundle.get_aligner()
    reports = []
    spans = crop_spans(length)
    for index, (start, seconds) in enumerate(spans):
        path = directory / f'{start:010.3f}-{seconds:06.3f}.json'
        request = dict(**identity, start=start, seconds=seconds)
        report = cached_reports.get(canonical_sha256(request))
        print(f'Local lyric recovery: crop {index + 1}/{len(spans)} '
              f'({start:g}–{start + seconds:g}s)' + (' cached' if report else ''), flush=True)
        if report is None:
            with tempfile.TemporaryDirectory(prefix='musializer-local-lyrics-') as temporary:
                wav = Path(temporary) / 'crop.wav'
                # The same real mono PCM feeds ASR and MMS. No playback device.
                samples = waveform[0, int(start * rate):math.ceil((start + seconds) * rate)]
                soundfile.write(wav, samples.numpy(), rate, subtype='PCM_16')
                source = external_analysis.run_whisper(wav, path.with_suffix('.whisper.json'),
                    audio_duration=seconds, whisper_bin=whisper_bin, model=whisper_model, timeout=180)
            source_lines = [row for row in source['lines'] if tokens(row['text'])]
            lines = [dict(display=row['text']) for row in source_lines]
            decisions = anchor_block_align.align_block(waveform, rate, lines,
                dict(first_line=0, last_line=len(lines) - 1, window_start=start, window_end=start + seconds),
                model, tokenizer, aligner, interior_stars=True) if lines else {}
            report = dict(request=request, start=start, seconds=seconds,
                lines=[dict(text=lines[i]['display'],
                    asr_start_seconds=start + source_lines[i]['start_seconds'],
                    asr_end_seconds=start + source_lines[i]['end_seconds'], **decision) for i, decision in decisions.items()],
                whisper_sha256=canonical_sha256(source))
            atomic_write_json(path, report)
        reports.append(report)
    result = recover(document, reports, reference)
    result['local_recovery']['request_identity'] = identity
    result['local_recovery']['reports_directory'] = str(directory)
    # Remove coarse proposals now represented by a corroborated caption.
    rendered = result['lines'] + result['performed_occurrences']
    result['performed_candidates'] = [row for row in result.get('performed_candidates', []) if not any(
        tokens(row['text']) == tokens(cue['text']) and overlap(row, cue) >= 0.5 for cue in rendered)]
    result['statistics']['runtime_seconds'] += time.monotonic() - started
    result['statistics']['performed_candidates'] = len(result['performed_candidates'])
    result['timing_refinement']['statistics'].update(result['statistics'])
    return result
