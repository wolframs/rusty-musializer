#!/usr/bin/env python3
"""Locate authored lyrics with independently cropped audio observations.

Match the supplied words locally against each original crop, before cross-crop
merging. This tolerates one crop splitting a sentence that another keeps whole.
Boundary refinement preserves the chosen occurrence; missing-line recovery
requires a unique corroborated occurrence and keeps it marked for review.
Neither stage invents word timestamps inside a phrase.
"""
from collections import Counter
import copy
from difflib import SequenceMatcher
import statistics
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import antigravity_lyrics


def candidates(text, reports):
    target = antigravity_lyrics._occurrence_tokens(text)
    if len(target) < 2:
        return []
    found = []
    seen_clips = set()
    for clip_index, report in enumerate(reports):
        clip_key = (report['clip_start_seconds'], report['clip_duration_seconds'])
        if clip_key in seen_clips:
            continue
        seen_clips.add(clip_key)
        rows = report['lines']
        offset = report['clip_start_seconds']
        for i, first in enumerate(rows):
            if first['partial_start']:
                continue
            words = []
            for j in range(i, min(i + 4, len(rows))):
                last = rows[j]
                if j > i and last['start_seconds'] - rows[j - 1]['end_seconds'] > 2:
                    break
                if last['end_seconds'] - first['start_seconds'] > 18:
                    break
                words.extend(antigravity_lyrics._occurrence_tokens(last['text']))
                if not words or last['partial_end'] or any(row['uncertain'] for row in rows[i:j + 1]):
                    continue
                # A phrase edge is usable only when it actually bounds the
                # supplied first/last word. Interior word times are unknown.
                if words[0] != target[0] or words[-1] != target[-1]:
                    continue
                similarity = SequenceMatcher(None, target, words, autojunk=False).ratio()
                if similarity >= .85:
                    found.append(dict(clip_index=clip_index, first_row=i, last_row=j,
                        start_seconds=offset + first['start_seconds'],
                        end_seconds=offset + max(row['end_seconds'] for row in rows[i:j + 1]),
                        similarity=similarity))
    return found


def refine(document, reports):
    result = copy.deepcopy(document)
    proposals = {}
    for index, cue in enumerate(result['lines']):
        by_clip = {}
        for hit in candidates(cue['text'], reports):
            distance = abs(hit['start_seconds'] - cue['start_seconds'])
            if distance > 3:
                continue
            rank = (-hit['similarity'], distance)
            if hit['clip_index'] not in by_clip or rank < by_clip[hit['clip_index']][0]:
                by_clip[hit['clip_index']] = (rank, hit)
        views = [hit for _, hit in by_clip.values()]
        if len(views) < 2:
            continue
        if any(max(v[edge] for v in views) - min(v[edge] for v in views) > .5 + 1e-9
               for edge in ('start_seconds', 'end_seconds')):
            continue
        proposals[index] = views
    # Two authored cues must not silently consume the same performance. Keep
    # their original timings when the independently chosen occurrences collide.
    uses = Counter((v['clip_index'], row)
        for views in proposals.values() for v in views
        for row in range(v['first_row'], v['last_row'] + 1))
    applied, collisions = [], []
    for index, views in proposals.items():
        if any(uses[v['clip_index'], row] > 1 for v in views
               for row in range(v['first_row'], v['last_row'] + 1)):
            collisions.append(index)
            continue
        cue = result['lines'][index]
        original = {edge: cue[edge] for edge in ('start_seconds', 'end_seconds')}
        for edge in original:
            cue[edge] = statistics.median([original[edge]] + [v[edge] for v in views])
        cue['audio_boundary_evidence'] = dict(original=original, observations=views)
        applied.append(index)
    # Refinement may sharpen a decided occurrence, not reverse the authored
    # path. Revert changes implicated in a new inversion until order is stable.
    ordered = sorted(range(len(result['lines'])),
                     key=lambda i: result['lines'][i]['reference_line_index'])
    declined_order = set()
    while True:
        revert = set()
        for left, right in zip(ordered, ordered[1:]):
            if (document['lines'][left]['start_seconds'] <= document['lines'][right]['start_seconds']
                    and result['lines'][left]['start_seconds'] > result['lines'][right]['start_seconds']):
                revert.update(i for i in (left, right) if i in applied)
        if not revert:
            break
        for i in revert:
            result['lines'][i] = copy.deepcopy(document['lines'][i])
            applied.remove(i)
        declined_order.update(revert)
    result['audio_boundary_refinement'] = dict(version='1', refined=len(applied), collisions=collisions,
        declined_order=sorted(declined_order),
        policy='two complete original crops within 0.5s; median with existing cue')
    refined = {result['lines'][i]['reference_line_index']: result['lines'][i] for i in applied}
    for flag in result.get('review_flags', []):
        cue = refined.get(flag.get('reference_line_index'))
        if cue is not None and flag.get('start_seconds') is not None:
            flag['start_seconds'], flag['end_seconds'] = cue['start_seconds'], cue['end_seconds']
            # Keep the original warning as historical evidence, while its UI
            # link follows the refined cue rather than the superseded time.
            if flag.get('reason'):
                flag['reason'] += ' (before audio boundary refinement)'
    result['lines'].sort(key=lambda row: (row['start_seconds'], row['end_seconds']))
    return result


def _observation_keys(views):
    return frozenset((view['clip_index'], row) for view in views
                     for row in range(view['first_row'], view['last_row'] + 1))


def supported_occurrences(text, reports):
    """Distinct full-phrase occurrences with two agreeing original crops."""
    if len(antigravity_lyrics._occurrence_tokens(text)) < 3:
        return []
    hits = [hit for hit in candidates(text, reports) if hit['similarity'] >= .9]
    groups = []
    for seed in hits:
        chosen = {seed['clip_index']: seed}
        for hit in sorted(hits, key=lambda h: (-h['similarity'],
                abs(h['start_seconds'] - seed['start_seconds']))):
            if hit['clip_index'] in chosen:
                continue
            if all(abs(hit[edge] - view[edge]) <= .5 + 1e-9
                   for view in chosen.values() for edge in ('start_seconds', 'end_seconds')):
                chosen[hit['clip_index']] = hit
        if len(chosen) < 2:
            continue
        views = list(chosen.values())
        groups.append(dict(observations=views,
            start_seconds=statistics.median(v['start_seconds'] for v in views),
            end_seconds=statistics.median(v['end_seconds'] for v in views)))
    result, used = [], set()
    for event in sorted(groups, key=lambda e: (-len(e['observations']),
            -min(v['similarity'] for v in e['observations']), e['start_seconds'])):
        keys = _observation_keys(event['observations'])
        if keys & used:
            continue
        result.append(event)
        used.update(keys)
    return sorted(result, key=lambda e: e['start_seconds'])


def recover(document, reports, *, audio_duration):
    """Recover only uniquely supported missing occurrences between placed lines.

    A plausible competing single-view occurrence still prevents a decision.
    More overlapping crops must not win a vote over a real earlier repetition.
    Recovered cues remain marked for review and retain the rejected CTC record.
    """
    result = copy.deepcopy(document)
    occupied = set()
    for cue in document['lines']:
        for field in ('audio_boundary_evidence', 'audio_occurrence_evidence'):
            occupied.update(_observation_keys(cue.get(field, {}).get('observations', [])))
    proposals, ambiguous, collisions = {}, [], []
    for missing in document.get('unresolved', []):
        index = missing['reference_line_index']
        low = max((cue['start_seconds'] for cue in document['lines']
                   if cue['reference_line_index'] < index), default=0)
        high = min((cue['start_seconds'] for cue in document['lines']
                    if cue['reference_line_index'] > index), default=audio_duration)
        options = [event for event in supported_occurrences(missing['text'], reports)
                   if low <= event['start_seconds'] <= high
                   and not _observation_keys(event['observations']) & occupied]
        if len(options) != 1:
            if options:
                ambiguous.append(index)
            continue
        event = options[0]
        if any(low <= hit['start_seconds'] <= high
               and any(abs(hit[edge] - event[edge]) > .5 + 1e-9
                       for edge in ('start_seconds', 'end_seconds'))
               for hit in candidates(missing['text'], reports) if hit['similarity'] >= .9):
            ambiguous.append(index)
            continue
        # A placed cue without native boundary evidence may still own this
        # performance. Do not create a second nearby copy of the same words.
        target = antigravity_lyrics._occurrence_tokens(missing['text'])
        if any(abs(cue['start_seconds'] - event['start_seconds']) <= .75
               and SequenceMatcher(None, target,
                   antigravity_lyrics._occurrence_tokens(cue['text']), autojunk=False).ratio() >= .9
               for cue in document['lines']):
            collisions.append(index)
            continue
        proposals[index] = (missing, event)
    uses = Counter(key for _, event in proposals.values()
                   for key in _observation_keys(event['observations']))
    for index, (_, event) in list(proposals.items()):
        if any(uses[key] > 1 for key in _observation_keys(event['observations'])):
            collisions.append(index)
            del proposals[index]
    # Decide jointly: two missing lines must not reverse one another even if
    # each individually fits between the same pair of existing neighbours.
    timeline = {cue['reference_line_index']: cue['start_seconds'] for cue in document['lines']}
    timeline.update({index: event['start_seconds'] for index, (_, event) in proposals.items()})
    ordered = sorted(timeline)
    declined_order = set()
    for left, right in zip(ordered, ordered[1:]):
        if timeline[left] > timeline[right]:
            declined_order.update(index for index in (left, right) if index in proposals)
    for index in declined_order:
        del proposals[index]
    for index, (missing, event) in proposals.items():
        cue = {key: missing[key] for key in ('reference_line_index', 'line_position', 'kind', 'text')}
        cue.update(start_seconds=event['start_seconds'], end_seconds=event['end_seconds'],
            confidence=None, estimated=False, uncertain=True, review_flagged=True,
            status='audio_occurrence_recovered', candidate_source='audio-crop-occurrence-consensus',
            audio_occurrence_evidence=dict(previous_alignment=copy.deepcopy(missing),
                                           observations=event['observations']))
        result['lines'].append(cue)
    result['unresolved'] = [row for row in result.get('unresolved', [])
                            if row['reference_line_index'] not in proposals]
    result['unmatched'] = [{key: row[key] for key in ('reference_line_index', 'text', 'reason')}
                           for row in result['unresolved']]
    result['review_flags'] = [flag for flag in result.get('review_flags', [])
                              if flag['reference_line_index'] not in proposals]
    for index, (missing, event) in proposals.items():
        result['review_flags'].append(dict(reference_line_index=index, text=missing['text'],
            flag='audio_occurrence_recovered',
            reason='Separate audio crops locate this line; the earlier acoustic disagreement remains available in its evidence.',
            start_seconds=event['start_seconds'], end_seconds=event['end_seconds']))
    result['lines'].sort(key=lambda row: (row['start_seconds'], row['end_seconds']))
    result['review_flags'].sort(key=lambda row: row['reference_line_index'])
    result.setdefault('statistics', {}).update(matched_lines=len(result['lines']),
        unmatched_lines=len(result['unresolved']), unresolved_lines=len(result['unresolved']),
        abstained_lines=sum(row.get('abstained', False) for row in result['unresolved']),
        review_flagged_lines=len(result['review_flags']))
    result['audio_occurrence_recovery'] = dict(version='1', recovered=len(proposals),
        declined_ambiguous=ambiguous, declined_order=sorted(declined_order),
        declined_competing=sorted(collisions))
    return result
