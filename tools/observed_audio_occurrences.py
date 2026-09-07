"""Corroborated performed words outside the authored lyric lane.

These additions retain audio wording and its provenance. Original and fresh
crop pairs must independently agree; an isolated ASR guess stays Potential.
"""
from collections import Counter
import copy
from pathlib import Path
import re
import statistics

from analysis_io import canonical_sha256, sha256_file
import antigravity_audio
import antigravity_lyrics as audio
import authored_audio_boundaries as boundaries
import authored_audio_occurrences as authored
import lyric_align

VERSION = '1'


def _words(text):
    def unstutter(match):
        syllable, word = match.group(1), match.group(2)
        return word if len(word) > len(syllable) and word.lower().startswith(syllable.lower()) else match.group(0)
    return lyric_align.normalize_tokens(re.sub(r'\b([a-z]{1,3})(?:-\1)*-([a-z]+)\b',
                                               unstutter, text, flags=re.I))


def occurrences(reports):
    templates, seen = {}, set()
    for clip_index, report in enumerate(reports):
        clip = (report['clip_start_seconds'], report['clip_duration_seconds'])
        if clip in seen:
            continue
        seen.add(clip)
        for index, row in enumerate(report['lines']):
            words = tuple(lyric_align.normalize_tokens(row['text']))
            if not words or any(row[flag] for flag in ('uncertain', 'partial_start', 'partial_end')):
                continue
            template = templates.setdefault(words, dict(text=row['text'], hits=[]))
            template['hits'].append(dict(clip_index=clip_index, first_row=index, last_row=index,
                start_seconds=clip[0] + row['start_seconds'], end_seconds=clip[0] + row['end_seconds'],
                similarity=1.))
    found = []
    for template in templates.values():
        hits, groups = template['hits'], []
        for seed in hits:
            chosen = {seed['clip_index']: seed}
            for hit in sorted(hits, key=lambda h: sum(abs(h[e] - seed[e])
                                                     for e in ('start_seconds', 'end_seconds'))):
                if hit['clip_index'] in chosen:
                    continue
                if all(abs(hit[e] - view[e]) <= .5 + 1e-9 for view in chosen.values()
                       for e in ('start_seconds', 'end_seconds')):
                    chosen[hit['clip_index']] = hit
            views = list(chosen.values())
            if len(views) >= 2:
                groups.append(dict(text=template['text'], observations=views,
                    **{edge: statistics.median(view[edge] for view in views)
                       for edge in ('start_seconds', 'end_seconds')}))
        used = set()
        for event in sorted(groups, key=lambda e: (-len(e['observations']),
                sum(max(v[k] for v in e['observations']) - min(v[k] for v in e['observations'])
                    for k in ('start_seconds', 'end_seconds')), e['start_seconds'])):
            keys = boundaries._observation_keys(event['observations'])
            if keys & used:
                continue
            used.update(keys)
            found.append(event)
    return sorted(found, key=lambda event: event['start_seconds'])


def explained(event, placements):
    if authored._explained(event, placements):
        return True
    words = lyric_align.normalize_tokens(event['text'])
    for cue in placements:
        whole = lyric_align.normalize_tokens(cue['text'])
        overlap = min(event['end_seconds'], cue['end_seconds']) - max(event['start_seconds'], cue['start_seconds'])
        shorter = min(event['end_seconds'] - event['start_seconds'], cue['end_seconds'] - cue['start_seconds'])
        if (shorter > 0 and overlap / shorter >= .5 and any(
                whole[i:i + len(words)] == words for i in range(len(whole) - len(words) + 1))):
            return True
    return False


def confirm(proposed, reports, placements):
    heard, accepted = occurrences(reports), []
    for event in proposed:
        matches = [hit for hit in heard if _words(hit['text']) == _words(event['text'])
                   and all(abs(hit[edge] - event[edge]) <= .5 + 1e-9
                           for edge in ('start_seconds', 'end_seconds'))]
        if len(matches) != 1:
            continue
        fresh = matches[0]['observations']
        cue = dict(text=event['text'], text_scope='audio_observed', reference_line_indices=[],
            confidence=None, uncertain=True, reason='Audio wording; confirmed in separate short clips',
            occurrence_evidence=dict(discovery=event['observations'], confirmation=fresh),
            **{edge: statistics.median(view[edge] for view in event['observations'] + fresh)
               for edge in ('start_seconds', 'end_seconds')})
        if not explained(cue, placements):
            accepted.append(cue)
    uses = Counter(key for cue in accepted for key in
                   boundaries._observation_keys(cue['occurrence_evidence']['confirmation']))
    return [cue for cue in accepted if not any(uses[key] > 1 for key in
            boundaries._observation_keys(cue['occurrence_evidence']['confirmation']))]


def proposals(reports, placements):
    occupied = set()
    for cue in placements:
        for field in ('audio_boundary_evidence', 'audio_occurrence_evidence'):
            occupied.update(boundaries._observation_keys(cue.get(field, {}).get('observations', [])))
        occupied.update(boundaries._observation_keys(cue.get('occurrence_evidence', {}).get('discovery', [])))
    return [event for event in occurrences(reports)
            if not boundaries._observation_keys(event['observations']) & occupied
            and not explained(event, placements)]


async def extend(audio_path: Path, source: dict, document: dict, output_dir: Path, *,
                 model: str, confirmed: bool, server=None, harness=None, profile=None):
    if confirmed is not True:
        raise ValueError('Antigravity audio transfer requires confirmation for this job')
    if model not in antigravity_audio.MODELS:
        raise ValueError('Unsupported Antigravity lyric model')
    audio_sha = sha256_file(audio_path)
    if any(value.get('audio', {}).get('sha256') != audio_sha for value in (source, document)):
        raise ValueError('Observed occurrence evidence belongs to different audio')
    if source.get('provenance', {}).get('source_kind') != 'antigravity_audio':
        raise ValueError('Observed occurrence confirmation requires original Antigravity observations')
    result = copy.deepcopy(document)
    result['performed_occurrences'] = [row for row in result.get('performed_occurrences', [])
                                       if row.get('text_scope') != 'audio_observed']
    placements = authored.rendered_lines(result)
    proposed = proposals(source['source']['boundary_observations'], placements)
    reports, identity = [], None
    length = source['audio']['duration_seconds']
    if proposed:
        paths = antigravity_audio.discover(server=server, harness=harness, profile=profile)
        identity = dict(version=VERSION, phase='observed-adlib-confirmation', model=model,
            audio_sha256=audio_sha, duration_seconds=length,
            prompt_sha256=canonical_sha256(audio.PROMPT),
            runtime_sha256=sha256_file(paths['server']), harness_sha256=sha256_file(paths['harness']),
            spans=authored.confirmation_spans(proposed, length))
        print(f'Antigravity lyrics: checking {len(proposed)} unplaced audio phrases in '
              f'{len(identity["spans"])} short audio clips', flush=True)
        reports = await audio.observe_spans(audio_path, identity,
            output_dir / 'antigravity-observed-occurrences' / canonical_sha256(identity), paths)
    added = confirm(proposed, reports, placements)
    result['performed_occurrences'].extend(added)
    result['observed_occurrence_analysis'] = dict(version=VERSION, proposed=len(proposed),
        confirmed=len(added), request_identity=identity, source_sha256=canonical_sha256(source),
        observations=[{key: row[key] for key in
            ('clip_start_seconds', 'clip_duration_seconds', 'clip_sha256', 'lines')} for row in reports])
    result['performed_candidates'] = lyric_align.find_performed_candidates(source,
        authored.rendered_lines(result),
        [(row['start_seconds'], row['end_seconds']) for row in result['unreliable_evidence']],
        audio_duration=length)
    result['statistics'].update(performed_occurrences=len(result['performed_occurrences']),
                                performed_candidates=len(result['performed_candidates']))
    if 'timing_refinement' in result:
        result['timing_refinement']['statistics'].update(
            performed_occurrences=len(result['performed_occurrences']),
            performed_candidates=len(result['performed_candidates']))
    return result
