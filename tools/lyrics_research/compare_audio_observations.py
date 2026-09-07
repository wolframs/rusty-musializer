#!/usr/bin/env python3
"""Compare complete blind audio phrases with proposed captions.

This measures agreement on independently sampled audio, not ground truth. It
reports unmatched observations and predictions separately and never calls an
unobserved prediction an extra performed line. The full-track 3% acceptance
requires adjudicating those unknowns too.
"""
import argparse
from difflib import SequenceMatcher
import json
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import lyric_align
from audio_audit import observations


def compare(heard, predicted):
    possibilities = []
    for h, expected in enumerate(heard):
        target = lyric_align.normalize_tokens(expected['text'])
        for i, first in enumerate(predicted):
            if abs(first['start_seconds'] - expected['start_seconds']) > 5:
                continue
            for j in range(i, min(i + 4, len(predicted))):
                last = predicted[j]
                if last['end_seconds'] - first['start_seconds'] > 20:
                    break
                words = [word for row in predicted[i:j + 1] for word in lyric_align.normalize_tokens(row['text'])]
                score = SequenceMatcher(None, target, words, autojunk=False).ratio()
                # Both edges must be the same words; spelling differences inside
                # the phrase are allowed, fabricated per-word timing is not.
                if not target or not words or score < 0.8 or target[0] != words[0] or target[-1] != words[-1]:
                    continue
                proximity = abs(first['start_seconds'] - expected['start_seconds']) + abs(last['end_seconds'] - expected['end_seconds'])
                possibilities.append((-score, proximity, h, i, j))
    assigned, used = {}, set()
    for _, _, h, i, j in sorted(possibilities):
        if h in assigned or any(k in used for k in range(i, j + 1)):
            continue
        assigned[h] = (i, j); used.update(range(i, j + 1))
    rows = []
    for h, expected in enumerate(heard):
        span = assigned.get(h)
        if span is None:
            rows.append(dict(status='unmatched_observation', expected=expected))
            continue
        i, j = span
        a, b = predicted[i]['start_seconds'], predicted[j]['end_seconds']
        start, end = a - expected['start_seconds'], b - expected['end_seconds']
        rows.append(dict(status='agreement' if max(abs(start), abs(end)) <= 0.5 else 'disagreement',
                         expected=expected, predicted_start_seconds=a, predicted_end_seconds=b,
                         start_delta_seconds=start, end_delta_seconds=end, predicted_indices=list(range(i, j + 1))))
    return dict(observed_phrases=len(heard), predicted_cues=len(predicted),
        agreement=sum(row['status']=='agreement' for row in rows),
        disagreement=sum(row['status']=='disagreement' for row in rows),
        unmatched_observations=sum(row['status']=='unmatched_observation' for row in rows),
        unscored_predictions=[dict(index=i, **r) for i,r in enumerate(predicted) if i not in used],
        rows=rows, acceptance_status='not_adjudicated')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('manifest',type=Path)
    args=parser.parse_args();root=args.manifest.resolve().parent
    result={}
    for track in json.loads(args.manifest.read_text()):
        directory=root/track['id']; heard, errors=observations(directory/'gemini-blind')
        methods={}
        for method,file in [('baseline','baseline/lyrics.aligned.json'),('local-repaired','candidate-repeated/lyrics.aligned.json'),
                            ('audio-model','candidate-antigravity/lyrics.antigravity.json'),
                            ('audio-ctc','candidate-antigravity/lyrics.antigravity.aligned.json'),
                            ('audio-only','candidate-antigravity-blind/lyrics.antigravity.aligned.json'),
                            ('audio-confirmed','candidate-antigravity-confirmed/lyrics.antigravity.aligned.json')]:
            path=directory/file
            if path.is_file():
                methods[method]=compare(heard,json.loads(path.read_text())['lines'])
                print(track['id'],method,{k:v for k,v in methods[method].items() if k not in ('rows','unscored_predictions')},flush=True)
        result[track['id']]=dict(methods=methods,observation_errors=errors)
    (root/'performance-audit.json').write_text(json.dumps(result,indent=2)+'\n')

if __name__=='__main__':main()
