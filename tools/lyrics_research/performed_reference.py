#!/usr/bin/env python3
"""Full-coverage, audio-only performed-phrase reference proposals.

No candidate words or timestamps enter the requests. These are independent
model observations, not adjudicated truth. Uncertain wording, clipped phrases
and ownership near a clip's scoring boundary remain explicitly unscored.
"""
import argparse
import asyncio
import json
import math
import copy
import statistics
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from analysis_io import atomic_write_json, canonical_sha256, sha256_file
import antigravity_audio as acp
import antigravity_lyrics as audio

VERSION = '1'
PROMPT = audio.PROMPT + '''\nA line means a distinct performed phrase, not a written lyric-sheet line.
Give an audible lead phrase and its separately sung reply/echo as separate
entries, even when their words are identical. Backing voices may overlap;
do not force them after the lead. Keep syllable stutters with their completed
word, including the first stutter phoneme in its start time. A separate full
repetition is another phrase. Do not invent a backing reply that is not audible.
'''
CHOP_GROUPING_VERSION = '2'
CHOP_GROUPING_PROMPT = audio.PROMPT + '''\nCount a continuous rhythmic vocal chop run as ONE performed phrase, even when
it retriggers the same complete word many times. Keep its audible words in one
entry spanning the first attack through the last vocal phoneme of that run.
Do not split a run into one entry per triggered word. Separately performed lead
lines and sung replies/echoes still require separate entries, including when
their words are identical. A pause or a new distinct performance can end a run;
do not merge separate calls and replies merely because their words match.
Keep syllable stutters with their completed word. Backing voices can overlap.
Do not invent unheard repetitions or replies. Mark uncertain grouping honestly.
'''


def intervals(length):
    if not math.isfinite(length) or length <= 0:
        raise ValueError('Reference needs a positive finite audio duration')
    return [(float(start), min(length, start + 18.), max(0., start - 3.),
             min(length, start + 21.)) for start in range(0, math.ceil(length), 18)]


def resolve_ownership(reference, reports):
    """Resolve scoring-window ownership with a unique adjacent audio view.

    This creates a new reference proposal, never edits the frozen input. Words
    and both boundaries must agree; wording/grouping uncertainty is retained.
    No candidate lyrics or candidate timestamps participate.
    """
    rebuilt = freeze(reference['audio'], reference['request_identity'], reports,
        regrouping_identity=reference.get('regrouping_identity'),
        regrouping_indices=reference.get('regrouping_indices', ()))
    if canonical_sha256(rebuilt) != canonical_sha256(reference):
        raise ValueError('Reference does not match its original audio receipts')
    result = copy.deepcopy(reference)
    resolved, retained = [], []
    def agrees(left, right):
        return (audio._occurrence_tokens(left['text']) == audio._occurrence_tokens(right['text'])
                and all(abs(left[e] - right[e]) <= .5 + 1e-9
                        for e in ('start_seconds', 'end_seconds')))
    for index, row in enumerate(reference['uncertain']):
        peers = []
        if row['reasons'] == ['interval_ownership']:
            owner = row['evidence_clip']
            for clip in (owner - 1, owner + 1):
                if not 0 <= clip < len(reports):
                    continue
                report = reports[clip]
                for position, phrase in enumerate(report['lines']):
                    if any(phrase[flag] for flag in ('uncertain', 'partial_start', 'partial_end')):
                        continue
                    view = dict(text=phrase['text'], **{e: report['clip_start_seconds'] + phrase[e]
                        for e in ('start_seconds', 'end_seconds')})
                    if agrees(row, view):
                        peers.append(dict(**view, evidence_clip=clip, evidence_row=position))
        # Competing instances or an already-owned row need a grouping review;
        # do not arbitrarily merge two repeated performances into one.
        collision = any(agrees(row, other) for other in reference['lines']) or any(
            i != index and agrees(row, other) for i, other in enumerate(reference['uncertain']))
        if len(peers) != 1 or collision:
            retained.append(copy.deepcopy(row))
            continue
        peer = peers[0]
        result['lines'].append(dict(text=row['text'], evidence_clip=row['evidence_clip'],
            **{e: statistics.median((row[e], peer[e])) for e in ('start_seconds', 'end_seconds')}))
        resolved.append(dict(uncertain_index=index, original=copy.deepcopy(row), adjacent=peer))
    result['lines'].sort(key=lambda row: row['start_seconds'])
    result['uncertain'] = retained
    result['ownership_resolution'] = dict(version='1', source_reference_sha256=canonical_sha256(reference),
                                         resolved=resolved)
    return result


def freeze(track, identity, reports, *, regrouping_identity=None, regrouping_indices=()):
    identity = json.loads(json.dumps(identity))
    layout = intervals(track['duration_seconds'])
    if (identity.get('audio_sha256') != track['sha256']
            or identity.get('duration_seconds') != track['duration_seconds']):
        raise ValueError('Reference identity belongs to another track')
    if len(layout) != len(reports) or identity['spans'] != [[a, b-a] for _, _, a, b in layout]:
        raise ValueError('Reference observations do not cover the full track')
    selected = set(regrouping_indices)
    if regrouping_identity is not None:
        regrouping_identity = json.loads(json.dumps(regrouping_identity))
        if (len(selected) != len(regrouping_indices)
                or any(type(index) is not int or not 0 <= index < len(layout) for index in selected)
                or regrouping_identity.get('audio_sha256') != track['sha256']
                or regrouping_identity.get('duration_seconds') != track['duration_seconds']
                or regrouping_identity.get('spans') != [identity['spans'][i] for i in sorted(selected)]):
            raise ValueError('Chop grouping identity does not match the selected audio intervals')
    elif selected:
        raise ValueError('Chop grouping intervals need their own request identity')
    lines, uncertain, coverage = [], [], []
    for index, ((begin, end, offset, stop), report) in enumerate(zip(layout, reports)):
        expected_identity = regrouping_identity if index in selected else identity
        if (json.loads(json.dumps(report['identity'])) != expected_identity or report['clip_start_seconds'] != offset
                or report['clip_duration_seconds'] != stop - offset):
            raise ValueError('Reference receipt belongs to another audio request')
        # Reparse the raw response rather than trusting an edited cached array.
        parsed = audio.parse_response(report['response'], stop - offset)
        if parsed != report['lines']:
            raise ValueError('Reference receipt changed after audio parsing')
        coverage.append(dict(start_seconds=begin, end_seconds=end,
                             clip_index=index, clip_sha256=report['clip_sha256']))
        if regrouping_identity is not None:
            coverage[-1]['request_identity_sha256'] = canonical_sha256(expected_identity)
        for phrase in parsed:
            start, finish = offset + phrase['start_seconds'], offset + phrase['end_seconds']
            if not begin <= start < end:
                continue
            row = dict(text=phrase['text'], start_seconds=start, end_seconds=finish,
                       evidence_clip=index)
            reasons = [key for key in ('uncertain', 'partial_start', 'partial_end') if phrase[key]]
            if (begin > 0 and start < begin + .5) or (end < track['duration_seconds'] and start > end - .5):
                reasons.append('interval_ownership')
            if reasons:
                uncertain.append(dict(**row, reasons=reasons))
            else:
                lines.append(row)
    result = dict(schema='musializer.research-reference/v1', audio=track,
        authority='independent_audio_only_model_proposal_not_adjudicated',
        scoring_unit='distinct_performed_phrase', request_identity=identity,
        coverage=coverage, lines=sorted(lines,key=lambda row:row['start_seconds']),
        uncertain=uncertain, acceptance_status='not_adjudicated')
    if regrouping_identity is not None:
        result.update(scoring_unit='distinct_performed_phrase_with_continuous_chop_run_as_one',
                      regrouping_identity=regrouping_identity, regrouping_indices=sorted(selected))
    return result


def chop_grouping_intervals(reports):
    """Find short repeated rows to re-hear, not boundaries to merge by formula.

    This deliberately includes ambiguous short calls/replies: the revised
    audio-only prompt decides whether they form a run or stay separate. Full
    lead/echo phrases are not collapsed by their repeated spelling.
    """
    selected = []
    for index, report in enumerate(reports):
        previous = {}
        for row in report['lines']:
            words = audio._occurrence_tokens(row['text'])
            unit = next((tuple(words[:size]) for size in (1, 2)
                         if len(words) >= size and len(words) % size == 0
                         and words == words[:size] * (len(words) // size)), None)
            if unit is None:
                continue
            prior = previous.get(unit)
            if prior is not None and -.1 <= row['start_seconds'] - prior['end_seconds'] <= 1:
                selected.append(index)
                break
            previous[unit] = row
    return selected


async def run(args):
    if args.confirm_audio_transfer is not True:
        raise ValueError('This reference run requires confirmed audio transfer')
    root = args.manifest.resolve().parent
    paths = acp.discover()
    for track in json.loads(args.manifest.read_text()):
        if args.track and track['id'] not in args.track:
            continue
        audio_path = Path(track['audio'])
        if sha256_file(audio_path) != track['sha256']:
            raise ValueError('Reference audio changed since the manifest was created')
        identity = dict(version=VERSION, phase='independent-performed-reference',
            model='gemini-3.8-flash-high', audio_sha256=track['sha256'],
            duration_seconds=track['duration_seconds'], prompt_sha256=canonical_sha256(PROMPT),
            runtime_sha256=sha256_file(paths['server']), harness_sha256=sha256_file(paths['harness']),
            spans=[(start,end-start) for _,_,start,end in intervals(track['duration_seconds'])])
        output = root / track['id'] / 'performed-reference-audio-v1'
        reports = await audio.observe_spans(audio_path,identity,output/canonical_sha256(identity),paths,prompt=PROMPT)
        reference = freeze(track,identity,reports)
        frozen = output/'reference.json'
        if frozen.exists() and json.loads(frozen.read_text()) != json.loads(json.dumps(reference)):
            raise ValueError('Reference is frozen; choose a new protocol version')
        atomic_write_json(frozen,reference)
        print('REFERENCE',track['id'],'phrases',len(reference['lines']),
              'uncertain',len(reference['uncertain']),flush=True)
        selected = chop_grouping_intervals(reports)
        grouping_identity = {**identity, 'version': CHOP_GROUPING_VERSION,
            'phase': 'independent-performed-reference-chop-grouping',
            'prompt_sha256': canonical_sha256(CHOP_GROUPING_PROMPT),
            'base_reference_sha256': sha256_file(frozen),
            'spans': [identity['spans'][i] for i in selected]}
        grouped_output = root / track['id'] / 'performed-reference-audio-v2'
        replacements = await audio.observe_spans(audio_path, grouping_identity,
            grouped_output / canonical_sha256(grouping_identity), paths,
            prompt=CHOP_GROUPING_PROMPT) if selected else []
        combined = list(reports)
        for index, report in zip(selected, replacements):
            combined[index] = report
        grouped = freeze(track, identity, combined, regrouping_identity=grouping_identity,
                         regrouping_indices=selected)
        grouped_path = grouped_output / 'reference.json'
        if grouped_path.exists() and json.loads(grouped_path.read_text()) != json.loads(json.dumps(grouped)):
            raise ValueError('Grouped reference is frozen; choose a new protocol version')
        atomic_write_json(grouped_path, grouped)
        print('GROUPED REFERENCE',track['id'],'phrases',len(grouped['lines']),
              'uncertain',len(grouped['uncertain']),'reheard clips',len(selected),flush=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest',type=Path)
    parser.add_argument('--track',action='append')
    parser.add_argument('--confirm-audio-transfer',action='store_true')
    asyncio.run(run(parser.parse_args()))

if __name__=='__main__':main()
