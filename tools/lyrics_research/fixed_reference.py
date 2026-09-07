#!/usr/bin/env python3
"""Freeze a timestamp-blind model audit and score subsequent cue candidates.

The reference is a model proposal, never human-adjudicated truth. It fixes the
phrase grouping and denominator across iterations. Split/merged caption lines
can remain unmatched; those cases must be reviewed, not removed from the score.
"""
import argparse
from difflib import SequenceMatcher
import json
import math
import re
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from analysis_io import atomic_write_json, canonical_sha256, sha256_file
import lyric_align
from authored_audio_occurrences import rendered_lines


def validate(rows):
    for row in rows:
        if not isinstance(row.get('text'), str) or not row['text'].strip():
            raise ValueError('Every reference and candidate needs caption text')
        start, end = row.get('start_seconds'), row.get('end_seconds')
        if (any(type(x) not in (float, int) or not math.isfinite(x) for x in (start, end))
                or not 0 <= start < end):
            raise ValueError('Invalid caption timing')


def freeze(directory):
    summary = json.loads((directory / 'summary.json').read_text())
    if summary.get('timestamps_blinded') is not True:
        raise ValueError('Timestamp-visible audits cannot become timing references')
    candidate = json.loads((directory / 'candidate.json').read_text())
    candidate_rows = rendered_lines(candidate)
    source_sha = canonical_sha256(candidate)
    if summary['candidate_sha256'] != source_sha:
        raise ValueError('Audit candidate changed')
    rows, uncertain, rejected, seen, evidence = [], [], [], set(), []
    length = summary['track']['duration_seconds']
    receipts = sorted(directory.glob('[0-9][0-9][0-9][0-9].json'))
    expected = [f'{i:04}.json' for i, start in enumerate(range(0, math.ceil(length), 18))]
    if [p.name for p in receipts] != expected:
        raise ValueError('Audit does not cover every audio interval')
    for interval_index, path in enumerate(receipts):
        receipt = json.loads(path.read_text())
        identity = receipt['identity']
        if (identity['candidate_sha256'] != source_sha
                or identity['audio_sha256'] != summary['track']['sha256']):
            raise ValueError('Audit receipt belongs to another candidate or audio')
        if (identity['start'] != max(0, interval_index * 18 - 3)
                or identity['end'] != min(length, interval_index * 18 + 27)):
            raise ValueError('Audit interval does not match full-coverage protocol')
        evidence.append(dict(path=str(path.resolve()), sha256=sha256_file(path),
                             prompt_sha256=identity['prompt_sha256']))
        for decision in receipt['audit']['cues']:
            index = decision['id']
            if index in seen or not 0 <= index < len(candidate_rows):
                raise ValueError('Invalid or duplicate audit cue')
            seen.add(index)
            original = candidate_rows[index]
            if decision['status'] == 'present':
                rows.append(dict(text=original['text'],
                    start_seconds=identity['start'] + decision['start_seconds'],
                    end_seconds=identity['start'] + decision['end_seconds'],
                    evidence=path.name, source_cue=index))
            elif decision['status'] == 'absent':
                rejected.append(dict(text=original['text'], evidence=path.name, source_cue=index))
            elif decision['status'] == 'uncertain':
                uncertain.append(dict(text=original['text'], evidence=path.name, source_cue=index))
            else:
                raise ValueError('Unknown audit judgement')
        for missing in receipt['audit']['missing']:
            row = dict(text=missing['text'],
                       start_seconds=identity['start'] + missing['start_seconds'],
                       end_seconds=identity['start'] + missing['end_seconds'], evidence=path.name)
            (uncertain if missing['uncertain'] else rows).append(row)
    if seen != set(range(len(candidate_rows))):
        raise ValueError('Audit omitted candidate cues')
    validate(rows)
    if any(row['end_seconds'] > length + .02 for row in rows):
        raise ValueError('Reference extends beyond the original audio')
    rows.sort(key=lambda row: (row['start_seconds'], row['end_seconds']))
    return dict(schema='musializer.research-reference/v1', audio=summary['track'],
                authority='timestamp_blind_model_audit_not_adjudicated',
                audited_candidate_sha256=source_sha, evidence=evidence,
                lines=rows, uncertain=uncertain, rejected_proposals=rejected)


def _assignment(costs):
    """Minimum-cost one-to-one rectangular assignment (rows <= columns)."""
    if not costs:
        return []
    n, m = len(costs), len(costs[0])
    u, v, owner = [0.] * (n + 1), [0.] * (m + 1), [0] * (m + 1)
    for row in range(1, n + 1):
        owner[0] = row
        current, previous = 0, [0] * (m + 1)
        best, used = [float('inf')] * (m + 1), [False] * (m + 1)
        while True:
            used[current] = True
            active = owner[current]
            delta, following = float('inf'), 0
            for column in range(1, m + 1):
                if used[column]:
                    continue
                reduced = costs[active - 1][column - 1] - u[active] - v[column]
                if reduced < best[column]:
                    best[column], previous[column] = reduced, current
                if best[column] < delta:
                    delta, following = best[column], column
            for column in range(m + 1):
                if used[column]:
                    u[owner[column]] += delta
                    v[column] -= delta
                else:
                    best[column] -= delta
            current = following
            if owner[current] == 0:
                break
        while current:
            predecessor = previous[current]
            owner[current] = owner[predecessor]
            current = predecessor
    return sorted((owner[column] - 1, column - 1)
                  for column in range(1, m + 1) if owner[column])


def _matching_tokens(text):
    # A written syllable stutter belongs to its word, not a separate phrase.
    # Keep whole-word repeats and separate words intact. Timing still includes
    # the initial stutter phoneme; this normalization never moves boundaries.
    def unstutter(match):
        syllable, word = match.group(1), match.group(2)
        return word if (len(word) > len(syllable)
                        and word.casefold().startswith(syllable.casefold())) else match.group(0)
    text = re.sub(r'\b([a-z]{1,3})(?:\s*-\s*\1)*\s*-\s*([a-z]+)\b', unstutter, text, flags=re.I)
    words = lyric_align.normalize_tokens(text)
    # A whole single-word chop run is one phrase under the operator's metric.
    # Repetition counts in its transcription do not identify another event.
    # Only normalize matching text: distinct rows and both edges stay intact.
    if words and len(set(words)) == 1:
        return words[:1]
    return words


def compare(reference, candidate):
    """Match performed instances independently of a wrong cue's sequence slot.

    Text determines eligibility. Timing distinguishes repeated occurrences but
    never prevents matching a badly shifted line; that line counts once. Once
    eligible, nearby instances take precedence over more exact wording in a
    distant verse. Otherwise ASR spelling variants can swap two correct cues.
    """
    validate(reference); validate(candidate)
    n, m = len(reference), len(candidate)
    a = [_matching_tokens(row['text']) for row in reference]
    b = [_matching_tokens(row['text']) for row in candidate]
    # A cardinality reward larger than every possible tie-break sum makes an
    # eligible pair preferable to dropping it. Dummy columns represent missing
    # lines. No timestamp tolerance is used as an eligibility condition.
    reward = (n + m + 1) * 1000
    costs = []
    for i in range(n):
        row = []
        for j in range(m):
            similarity = SequenceMatcher(None, a[i], b[j], autojunk=False).ratio()
            distance = min(100., sum(abs(reference[i][edge] - candidate[j][edge])
                                   for edge in ('start_seconds', 'end_seconds')))
            row.append(-reward + distance + .001 * (1 - similarity)
                       if similarity >= .72 else reward)
        costs.append(row + [0.] * n)
    pairs = [(i, j) for i, j in _assignment(costs) if j < m and costs[i][j] < 0]
    matched, used, decisions = set(), set(), []
    for i, j in pairs:
        matched.add(i); used.add(j)
        deltas = {edge: candidate[j][edge + '_seconds'] - reference[i][edge + '_seconds']
                  for edge in ('start', 'end')}
        decisions.append(dict(reference_index=i, candidate_index=j, deltas=deltas,
                              timing_error=max(abs(x) for x in deltas.values()) > .5 + 1e-9))
    missing = sorted(set(range(n)) - matched)
    extra = sorted(set(range(m)) - used)
    timing = sum(row['timing_error'] for row in decisions)
    errors = len(missing) + len(extra) + timing
    return dict(reference_lines=n, candidate_lines=m, matched=len(pairs),
                missing=missing, extra=extra, timing_errors=timing,
                error_count=errors, error_rate=errors/n if n else None,
                decisions=decisions, matching_policy='bipartite-performed-phrase-0.72-v6')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    frozen = sub.add_parser('freeze'); frozen.add_argument('audit', type=Path); frozen.add_argument('output', type=Path)
    score = sub.add_parser('score'); score.add_argument('reference', type=Path); score.add_argument('candidate', type=Path); score.add_argument('output', type=Path)
    args = parser.parse_args()
    if args.command == 'freeze':
        if args.output.exists():
            raise ValueError('Reference is frozen; use a new explicit version path')
        result = freeze(args.audit)
    else:
        reference = json.loads(args.reference.read_text()); candidate = json.loads(args.candidate.read_text())
        original_sha = candidate.get('source', {}).get('original_audio_sha256', candidate['audio']['sha256'])
        if original_sha != reference['audio']['sha256']:
            raise ValueError('Candidate and reference belong to different original audio')
        result = compare(reference['lines'], rendered_lines(candidate))
        result.update(reference_sha256=sha256_file(args.reference),
                      candidate_sha256=sha256_file(args.candidate),
                      authority=reference['authority'], uncertain_reference=reference['uncertain'],
                      acceptance_status='model_reference_not_adjudicated')
    atomic_write_json(args.output, result)
    print(args.output)


if __name__ == '__main__':
    main()
