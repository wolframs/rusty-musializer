#!/usr/bin/env python3
"""Replay preserved audio observations and compare boundary estimators offline.

No provider process or network request is launched. This runs under the local
alignment interpreter and retains both the CTC and fused outputs for comparison.
"""
import argparse
import copy
import json
from pathlib import Path
import statistics
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import antigravity_lyrics
from analysis_io import atomic_write_json, canonical_sha256


def fuse(line):
    row=copy.deepcopy(line)
    observed=[x for x in row.get('timing_observations',[]) if x['complete']]
    for field in ('start_seconds','end_seconds'):
        points=[x[field] for x in observed]
        # With two separately cropped observations, the acoustic result is a
        # third view. A single audio observation stays a proposal; averaging it
        # with a forced CTC path would add false precision without corroboration.
        if len(points)>1:
            points.append(row[field])
        if points:
            row[field]=statistics.median(points)
    return row


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest',type=Path);parser.add_argument('--track')
    args=parser.parse_args();root=args.manifest.resolve().parent
    for track in json.loads(args.manifest.read_text()):
        if args.track and track['id']!=args.track:continue
        directory=root/track['id']; original=directory/'candidate-antigravity/lyrics.antigravity.json'
        if not original.exists():continue
        source=json.loads(original.read_text());identity=source['provenance']['request_settings']
        folder=original.parent/'antigravity-clips'/canonical_sha256(identity)
        reports=[]
        for i,(start,length) in enumerate(identity['spans']):
            report=json.loads((folder/f'{i:04}.json').read_text())
            assert report['identity']==identity
            report['lines']=antigravity_lyrics.parse_response(report['response'],length)
            reports.append(report)
        rows,unresolved=antigravity_lyrics.merge_observations(reports,track['duration_seconds'])
        for row in rows:
            row['source_line_indices']=row.pop('evidence_indices')
            row['confidence']=0.5 if row['uncertain'] else 0.8
        source['lines']=rows
        source['source']['unresolved_clipped_phrases']=unresolved
        source['source']['sha256']=canonical_sha256(reports)
        replay=directory/'candidate-audio-replay.input.json';atomic_write_json(replay,source)
        acoustic=antigravity_lyrics.align(Path(track['audio']),replay,directory/'candidate-audio-replay-ctc.json')
        for row in acoustic['lines']:
            decision=row['forced_alignment']
            for edge in ('start','end'):
                key=f'{edge}_seconds'
                row[key]=decision.get(f'acoustic_{key}', decision[f'input_{key}'])
        acoustic['lines'].sort(key=lambda row:row['start_seconds'])
        atomic_write_json(directory/'candidate-audio-replay-ctc.json',acoustic)
        fused=copy.deepcopy(acoustic)
        fused['lines']=sorted([fuse(row) for row in acoustic['lines']],key=lambda r:r['start_seconds'])
        fused['research_variant']='median-observations-and-ctc'
        atomic_write_json(directory/'candidate-audio-replay-fused.json',fused)
        print(track['id'],len(fused['lines']),flush=True)

if __name__=='__main__':main()
