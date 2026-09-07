"""Additional performed instances of authored words, independent of sheet order.

Two original crops propose an occurrence; two fresh short audio-only crops must
confirm it. These instances never consume or reorder the authored-line ledger.
"""
from collections import Counter
import copy
from difflib import SequenceMatcher
from pathlib import Path
import re
import statistics

from analysis_io import canonical_sha256, sha256_file
import antigravity_audio
import antigravity_lyrics as audio
import authored_audio_boundaries as boundaries
import lyric_align

VERSION = '2'
CONFIRMATION_VERSION = '1'


def rendered_rows(document):
    """Replace a compound cue only when its split still names that exact cue."""
    replacements = {}
    for split in document.get('phrase_splits', []):
        key = (split['source_kind'], split['source_index'])
        rows = document.get(key[0], [])
        if (key[0] not in ('lines', 'performed_occurrences') or key in replacements
                or type(key[1]) is not int or not 0 <= key[1] < len(rows)
                or canonical_sha256(rows[key[1]]) != split['source_sha256']
                or len(split['phrases']) != 2):
            raise ValueError('Phrase split does not identify one unchanged source cue')
        replacements[key] = split['phrases']
    return [phrase for kind in ('lines', 'performed_occurrences')
            for index, cue in enumerate(document.get(kind, []))
            for phrase in replacements.get((kind, index), [cue])]


def rendered_lines(document):
    return sorted(rendered_rows(document),
                  key=lambda row: (row['start_seconds'], row['end_seconds']))


def _explained(event, placements):
    words = audio._occurrence_tokens(event['text'])
    for cue in placements:
        overlap = max(0, min(event['end_seconds'], cue['end_seconds'])
                      - max(event['start_seconds'], cue['start_seconds']))
        shorter = min(event['end_seconds'] - event['start_seconds'],
                      cue['end_seconds'] - cue['start_seconds'])
        whole = audio._occurrence_tokens(cue['text'])
        included_lead = event.get('text_scope') == 'authored_inline_lead' and any(
            whole[i:i + len(words)] == words for i in range(len(whole) - len(words) + 1))
        if (shorter > 0 and overlap / shorter >= .5 and
                (included_lead or SequenceMatcher(None, words, whole, autojunk=False).ratio() >= .72)):
            return True
    return False


def proposals(document, reference_text, reports):
    templates = {}
    source_lines = lyric_align.classify_reference_lines(reference_text)
    for row in source_lines:
        if row['kind'] not in ('lyric', 'backing') or not row['tokens']:
            continue
        key = tuple(audio._occurrence_tokens(row['display']))
        template = templates.setdefault(key, dict(text=row['display'], reference_line_indices=[]))
        template['reference_line_indices'].append(row['index'])
    complete_keys = set(templates)
    unresolved = {row['reference_line_index'] for row in document.get('unresolved', [])}
    for row in source_lines:
        if row['index'] not in unresolved or row['kind'] not in ('lyric', 'backing'):
            continue
        # A trailing inline reply is optional performance structure. Do not
        # strip standalone backing lines or parentheses inside the main phrase.
        match = re.fullmatch(r'(\S.*?)\s+\([^()]+\)\s*(["”]?)', row['display'])
        if match is None:
            continue
        lead = match.group(1).rstrip() + match.group(2)
        key = tuple(audio._occurrence_tokens(lead))
        if len(key) < 3 or key in complete_keys:
            continue
        template = templates.setdefault(key, dict(text=lead, reference_line_indices=[],
                                                   text_scope='authored_inline_lead'))
        template['reference_line_indices'].append(row['index'])
    occupied = set()
    for cue in document['lines']:
        for field in ('audio_boundary_evidence', 'audio_occurrence_evidence'):
            occupied.update(boundaries._observation_keys(cue.get(field, {}).get('observations', [])))
    found = []
    for template in templates.values():
        for event in boundaries.supported_occurrences(template['text'], reports):
            event = dict(**template, **event)
            if (not boundaries._observation_keys(event['observations']) & occupied
                    and not _explained(event, document['lines'])):
                found.append(event)
    # A complete sung call-and-response owns its lead. Otherwise the optional
    # fragment would compete for the same rows and discard both performances.
    complete = [event for event in found if 'text_scope' not in event]
    found = [event for event in found if 'text_scope' not in event
             or not _explained(event, complete)]
    uses = Counter(key for event in found for key in boundaries._observation_keys(event['observations']))
    return sorted((event for event in found if not any(
        uses[key] > 1 for key in boundaries._observation_keys(event['observations']))),
        key=lambda row: (row['start_seconds'], row['end_seconds']))


def confirmation_spans(events, length):
    base = audio.confirmation_spans([dict(event, timing_observations=[]) for event in events], length)
    return sorted({(round(min(max(0, start + shift), length - seconds), 6), seconds)
                   for start, seconds in base for shift in (-.325, .325)})


def confirm(events, reports, placements):
    accepted = []
    for event in events:
        matches = [hit for hit in boundaries.supported_occurrences(event['text'], reports)
                   if all(abs(hit[edge] - event[edge]) <= .5 + 1e-9
                          for edge in ('start_seconds', 'end_seconds'))]
        if len(matches) != 1:
            continue
        match = matches[0]
        cue = dict(text=event['text'], reference_line_indices=event['reference_line_indices'],
                   confidence=None, uncertain=True,
                   reason='Additional performance confirmed in separate short audio crops',
                   occurrence_evidence=dict(discovery=event['observations'],
                                            confirmation=match['observations']))
        if event.get('text_scope') == 'authored_inline_lead':
            cue['text_scope'] = 'authored_inline_lead'
            cue['reason'] = 'Performed main phrase; written backing remains unresolved'
        for edge in ('start_seconds', 'end_seconds'):
            cue[edge] = statistics.median(view[edge] for view in
                event['observations'] + match['observations'])
        if not _explained(cue, placements):
            accepted.append(cue)
    # Two proposed occurrences cannot borrow the same fresh performance.
    uses = Counter(key for cue in accepted for key in boundaries._observation_keys(
        cue['occurrence_evidence']['confirmation']))
    return [cue for cue in accepted if not any(uses[key] > 1 for key in
        boundaries._observation_keys(cue['occurrence_evidence']['confirmation']))]


async def extend(audio_path: Path, source: dict, document: dict, output_dir: Path, *,
                 model: str, confirmed: bool, server=None, harness=None, profile=None):
    if confirmed is not True:
        raise ValueError('Antigravity audio transfer requires confirmation for this job')
    if model not in antigravity_audio.MODELS:
        raise ValueError('Unsupported Antigravity lyric model')
    audio_sha = sha256_file(audio_path)
    if any(value.get('audio', {}).get('sha256') != audio_sha for value in (source, document)):
        raise ValueError('Performed occurrence evidence belongs to different audio')
    if source.get('provenance', {}).get('source_kind') != 'antigravity_audio':
        raise ValueError('Performed occurrence confirmation requires original Antigravity observations')
    result = copy.deepcopy(document)
    result.pop('phrase_splits', None)
    result.pop('phrase_analysis', None)
    result.pop('observed_occurrence_analysis', None)
    data = source['source']
    events = proposals(document, data['reference']['text'], data['boundary_observations'])
    length = source['audio']['duration_seconds']
    reports, identity = [], None
    if events:
        paths = antigravity_audio.discover(server=server, harness=harness, profile=profile)
        identity = dict(version=CONFIRMATION_VERSION, phase='performed-occurrence-confirmation',
            model=model, audio_sha256=audio_sha, duration_seconds=length,
            prompt_sha256=canonical_sha256(audio.PROMPT),
            runtime_sha256=sha256_file(paths['server']), harness_sha256=sha256_file(paths['harness']),
            proposals_sha256=canonical_sha256(events), spans=confirmation_spans(events, length))
        cache = output_dir / 'antigravity-occurrences' / canonical_sha256(identity)
        print(f'Antigravity lyrics: checking {len(events)} additional performances in '
              f'{len(identity["spans"])} short audio clips', flush=True)
        reports = await audio.observe_spans(audio_path, identity, cache, paths)
    result['performed_occurrences'] = confirm(events, reports, document['lines'])
    result['occurrence_analysis'] = dict(version=VERSION, proposed=len(events),
        confirmed=len(result['performed_occurrences']), request_identity=identity,
        source_sha256=canonical_sha256(source),
        observations=[dict(clip_start_seconds=row['clip_start_seconds'],
                           clip_duration_seconds=row['clip_duration_seconds'],
                           clip_sha256=row['clip_sha256'], lines=row['lines']) for row in reports])
    result['performed_candidates'] = lyric_align.find_performed_candidates(source, rendered_lines(result),
        [(row['start_seconds'], row['end_seconds']) for row in result['unreliable_evidence']],
        audio_duration=length)
    result['statistics'].update(performed_occurrences=len(result['performed_occurrences']),
                                performed_candidates=len(result['performed_candidates']))
    if 'timing_refinement' in result:
        result['timing_refinement']['statistics'].update(
            performed_occurrences=len(result['performed_occurrences']),
            performed_candidates=len(result['performed_candidates']))
    return result
