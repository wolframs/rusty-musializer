#!/usr/bin/env python3
"""Compare candidate cues to independent Gemini audio observations.

This is a disagreement audit, not adjudicated accuracy. Unmatched words,
partial clip phrases and unobserved regions remain unscored. No method can
meet the acceptance gate by dropping its hard cases from the denominator.
"""
from __future__ import annotations

import argparse
from collections import defaultdict
import json
import math
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import lyric_align


def observations(directory: Path) -> tuple[list[dict], list[str]]:
    rows: list[dict] = []
    errors: list[str] = []
    for path in sorted(directory.glob('*.json')):
        try:
            report = json.loads(path.read_text())
            raw = report['response'].strip()
            if raw.startswith('```'):
                raw = raw.split('\n', 1)[1].rsplit('```', 1)[0]
            response = json.loads(raw)
            offset = float(report['clip_start_seconds'])
            length = float(report['clip_duration_seconds'])
            for line in response['lines']:
                start, end = float(line['start_seconds']), float(line['end_seconds'])
                if not all(math.isfinite(x) for x in (start, end)) or not 0 <= start < end <= length + 0.2:
                    raise ValueError('Out-of-range audio observation')
                # The next clip owns starts at or beyond the 15-second stride.
                # Cropped starts/ends cannot serve as actual lyric boundaries.
                if start >= 15 or (offset > 0 and start <= 0.15) or end >= length - 0.15:
                    continue
                if line.get('uncertain'):
                    continue
                rows.append(dict(text=line['text'], start_seconds=offset + start,
                                 end_seconds=offset + end, evidence=str(path),
                                 tokens=lyric_align.normalize_tokens(line['text'])))
        except (ValueError, KeyError, TypeError) as error:
            errors.append(f'{path.name}: {error}')
    rows.sort(key=lambda row: (row['start_seconds'], row['end_seconds']))
    return rows, errors


def reference_matches(authored: list[dict], heard: list[dict]) -> dict[int, dict]:
    ref_tokens, ref_owners = [], []
    for position, row in enumerate(authored):
        for index, token in enumerate(row['tokens']):
            ref_tokens.append(token)
            ref_owners.append((position, index))
    hyp_tokens, hyp_owners = [], []
    for position, row in enumerate(heard):
        for index, token in enumerate(row['tokens']):
            hyp_tokens.append(token)
            hyp_owners.append((position, index))
    grouped = defaultdict(list)
    for ref, hyp in lyric_align.align_tokens(ref_tokens, hyp_tokens):
        position, index = ref_owners[ref]
        grouped[position].append((index, *hyp_owners[hyp]))
    matched = {}
    for position, pairs in grouped.items():
        row = authored[position]
        first, last = pairs[0], pairs[-1]
        # Require word coverage and both phrase edges. We never synthesize
        # word timestamps by distributing a phrase uniformly through time.
        if (len(pairs) / len(row['tokens']) < 0.8 or first[0] != 0
                or last[0] != len(row['tokens']) - 1 or first[2] != 0
                or last[2] != len(heard[last[1]]['tokens']) - 1):
            continue
        start = heard[first[1]]['start_seconds']
        end = heard[last[1]]['end_seconds']
        if end - start > 18:
            continue
        matched[row['index']] = {
            'text': row['display'], 'start_seconds': start, 'end_seconds': end,
            'reference_line_index': row['index'],
            'evidence': sorted({heard[p[1]]['evidence'] for p in pairs}),
            'status': 'model_observation_not_adjudicated',
        }
    return matched


def compare(reference: dict[int, dict], prediction: dict, total: int) -> dict:
    cues = {row['reference_line_index']: row for row in prediction.get('lines', [])}
    results = []
    for index, expected in reference.items():
        cue = cues.get(index)
        if cue is None:
            results.append(dict(reference_line_index=index, issue='missing', expected=expected))
            continue
        start = abs(cue['start_seconds'] - expected['start_seconds'])
        end = abs(cue['end_seconds'] - expected['end_seconds'])
        results.append(dict(reference_line_index=index,
                            issue='disagreement' if max(start, end) > 0.5 else 'agreement',
                            start_delta_seconds=start, end_delta_seconds=end,
                            predicted_start_seconds=cue['start_seconds'],
                            predicted_end_seconds=cue['end_seconds'], expected=expected))
    return {'authored_lines': total, 'audio_observed_lines': len(reference),
            'unscored_lines': total - len(reference),
            'agree_with_model': sum(row['issue'] == 'agreement' for row in results),
            'disagree_with_model': sum(row['issue'] == 'disagreement' for row in results),
            'missing_observed_lines': sum(row['issue'] == 'missing' for row in results),
            'acceptance_status': 'not_adjudicated', 'lines': results}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    args = parser.parse_args()
    root = args.manifest.resolve().parent
    output = {}
    for track in json.loads(args.manifest.read_text()):
        directory = root / track['id']
        authored = [r for r in lyric_align.classify_reference_lines(
            (directory / 'reference.txt').read_text()) if r['tokens']]
        heard, errors = observations(directory / 'gemini-blind')
        reference = reference_matches(authored, heard)
        row = {'observation_errors': errors, 'methods': {}}
        for method, path in [
            ('baseline', directory / 'baseline/lyrics.aligned.json'),
            ('parser', directory / 'candidate-parser/lyrics.aligned.json'),
            ('joint', directory / 'candidate-joint.json'),
            ('joint-no-stars', directory / 'candidate-joint-no-stars.json'),
            ('bounded-local', directory / 'candidate-bounded-local.json'),
            ('ordered', directory / 'candidate-ordered/lyrics.aligned.json'),
            ('repeated', directory / 'candidate-repeated/lyrics.aligned.json'),
            ('interval-coarse', directory / 'candidate-interval-coarse.json')]:
            if path.exists():
                result = compare(reference, json.loads(path.read_text()), len(authored))
                row['methods'][method] = result
                print(track['id'], method, {k: v for k, v in result.items() if k != 'lines'})
        output[track['id']] = row
    (root / 'audio-audit.json').write_text(json.dumps(output, indent=2) + '\n')


if __name__ == '__main__':
    main()
