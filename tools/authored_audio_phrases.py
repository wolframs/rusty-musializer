"""Separate audible lead/reply phrases without rewriting the authored ledger."""
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
import authored_audio_occurrences as occurrences
import lyric_align

VERSION = '2'
CONFIRMATION_VERSION = '1'


def partitions(cue, reports):
    """Propose written-text seams only where a complete audio row ends.

    Punctuation alone does not establish a performed phrase boundary. For an
    ordinary sentence, a crop must first identify the whole existing cue and
    then locate the supplied prefix at one of its separately reported rows.
    Every resulting component still needs the original/fresh crop consensus.
    """
    match = re.fullmatch(r'(\S.*?)\s+\(([^()]+)\)\s*(["”]?)', cue['text'])
    if match is not None:
        return [(match.group(1).rstrip() + match.group(3), match.group(2).strip())]
    if '(' in cue['text'] or ')' in cue['text']:
        return []
    text, quote = cue['text'].strip(), ''
    if len(text) > 1 and text[0] == text[-1] and text[0] in ('"', "'"):
        quote, text = text[0], text[1:-1]
    seams = set()
    for hit in boundaries.candidates(text, reports):
        if (hit['first_row'] == hit['last_row'] or any(
                abs(hit[edge] - cue[edge]) > .5 + 1e-9
                for edge in ('start_seconds', 'end_seconds'))):
            continue
        rows = reports[hit['clip_index']]['lines']
        prefix = []
        for row in rows[hit['first_row']:hit['last_row']]:
            prefix.extend(audio._occurrence_tokens(row['text']))
            if not prefix:
                continue
            for seam in re.finditer(r'\s+', text):
                words = audio._occurrence_tokens(text[:seam.start()])
                if (len(words) >= 3 and words[0] == prefix[0] and words[-1] == prefix[-1]
                        and SequenceMatcher(None, words, prefix, autojunk=False).ratio() >= .9):
                    seams.add((seam.start(), seam.end()))
    return [(quote + text[:left] + quote, quote + text[right:] + quote)
            for left, right in sorted(seams)]


def supported_pair(cue, words, reports):
    options = [[event for event in boundaries.supported_occurrences(text, reports)
                if event['start_seconds'] >= cue['start_seconds'] - .5
                and event['end_seconds'] <= cue['end_seconds'] + .5] for text in words]
    pairs = [(lead, reply) for lead in options[0] for reply in options[1]
             if lead['start_seconds'] < reply['start_seconds']
             and abs(lead['start_seconds'] - cue['start_seconds']) <= .5
             and abs(max(lead['end_seconds'], reply['end_seconds']) - cue['end_seconds']) <= .5
             and not boundaries._observation_keys(lead['observations'])
                 & boundaries._observation_keys(reply['observations'])]
    return pairs[0] if len(pairs) == 1 else None


def proposals(document, reports):
    found = []
    for kind in ('lines', 'performed_occurrences'):
        for index, cue in enumerate(document.get(kind, [])):
            if cue.get('text_scope') == 'audio_observed':
                continue
            choices = [(words, pair) for words in partitions(cue, reports)
                       if (pair := supported_pair(cue, words, reports)) is not None]
            # Multiple valid seams are an unresolved grouping decision. Do not
            # arbitrarily prefer the first comma or the longest component.
            if len(choices) != 1:
                continue
            words, pair = choices[0]
            indices = cue.get('reference_line_indices', [cue.get('reference_line_index')])
            found.append(dict(source_kind=kind, source_index=index,
                source_sha256=canonical_sha256(cue), original=cue,
                phrases=[dict(text=text, reference_line_indices=indices, **event)
                         for text, event in zip(words, pair)]))
    uses = Counter(key for proposal in found for phrase in proposal['phrases']
                   for key in boundaries._observation_keys(phrase['observations']))
    return [proposal for proposal in found if not any(uses[key] > 1
            for phrase in proposal['phrases']
            for key in boundaries._observation_keys(phrase['observations']))]


def confirm(proposed, reports):
    accepted = []
    for proposal in proposed:
        parts = []
        for phrase in proposal['phrases']:
            matches = [hit for hit in boundaries.supported_occurrences(phrase['text'], reports)
                       if all(abs(hit[edge] - phrase[edge]) <= .5 + 1e-9
                              for edge in ('start_seconds', 'end_seconds'))]
            if len(matches) != 1:
                break
            fresh = matches[0]['observations']
            parts.append(dict(text=phrase['text'], reference_line_indices=phrase['reference_line_indices'],
                confidence=None, uncertain=True,
                reason='Separate performed phrase confirmed in short audio crops',
                occurrence_evidence=dict(discovery=phrase['observations'], confirmation=fresh),
                **{edge: statistics.median(view[edge] for view in phrase['observations'] + fresh)
                   for edge in ('start_seconds', 'end_seconds')}))
        if len(parts) == 2 and parts[0]['start_seconds'] < parts[1]['start_seconds']:
            accepted.append({**{key: proposal[key] for key in
                                ('source_kind', 'source_index', 'source_sha256')}, 'phrases': parts})
    # A repeated phrase cannot be its own echo, or another parent's reply.
    uses = Counter(key for split in accepted for phrase in split['phrases']
                   for key in boundaries._observation_keys(phrase['occurrence_evidence']['confirmation']))
    return [split for split in accepted if not any(uses[key] > 1 for phrase in split['phrases']
            for key in boundaries._observation_keys(phrase['occurrence_evidence']['confirmation']))]


async def split(audio_path: Path, source: dict, document: dict, output_dir: Path, *,
                model: str, confirmed: bool, server=None, harness=None, profile=None):
    if confirmed is not True:
        raise ValueError('Antigravity audio transfer requires confirmation for this job')
    if model not in antigravity_audio.MODELS:
        raise ValueError('Unsupported Antigravity lyric model')
    audio_sha = sha256_file(audio_path)
    if any(value.get('audio', {}).get('sha256') != audio_sha for value in (source, document)):
        raise ValueError('Phrase split evidence belongs to different audio')
    if source.get('provenance', {}).get('source_kind') != 'antigravity_audio':
        raise ValueError('Phrase splitting requires original Antigravity observations')
    proposed = proposals(document, source['source']['boundary_observations'])
    reports, identity = [], None
    length = source['audio']['duration_seconds']
    if proposed:
        paths = antigravity_audio.discover(server=server, harness=harness, profile=profile)
        identity = dict(version=CONFIRMATION_VERSION, phase='performed-phrase-split-confirmation', model=model,
            audio_sha256=audio_sha, duration_seconds=length,
            prompt_sha256=canonical_sha256(audio.PROMPT),
            runtime_sha256=sha256_file(paths['server']), harness_sha256=sha256_file(paths['harness']),
            spans=occurrences.confirmation_spans([row['original'] for row in proposed], length))
        print(f'Antigravity lyrics: checking {len(proposed)} phrase splits in '
              f'{len(identity["spans"])} short audio clips', flush=True)
        reports = await audio.observe_spans(audio_path, identity,
            output_dir / 'antigravity-phrase-splits' / canonical_sha256(identity), paths)
    result = copy.deepcopy(document)
    result['phrase_splits'] = confirm(proposed, reports)
    result['phrase_analysis'] = dict(version=VERSION, proposed=len(proposed),
        confirmed=len(result['phrase_splits']), request_identity=identity,
        source_sha256=canonical_sha256(source),
        observations=[{key: row[key] for key in
            ('clip_start_seconds', 'clip_duration_seconds', 'clip_sha256', 'lines')} for row in reports])
    result['performed_candidates'] = lyric_align.find_performed_candidates(source,
        occurrences.rendered_lines(result),
        [(row['start_seconds'], row['end_seconds']) for row in result['unreliable_evidence']],
        audio_duration=length)
    result['statistics'].update(phrase_splits=len(result['phrase_splits']),
                                performed_candidates=len(result['performed_candidates']))
    if 'timing_refinement' in result:
        result['timing_refinement']['statistics'].update(
            phrase_splits=len(result['phrase_splits']),
            performed_candidates=len(result['performed_candidates']))
    return result
