#!/usr/bin/env python3
"""Build a research correction candidate from a separately preserved audio audit.

This candidate MUST NOT be scored against the audit that created it. Compare
against blind observations or a fresh, differently cropped audio judgement.
"""
import argparse
import copy
import json
from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from analysis_io import atomic_write_json


def corrected(source, receipts):
    if source.get('performed_occurrences') or source.get('phrase_splits'):
        raise ValueError('This legacy correction tool cannot rewrite an authored ledger with separate performed occurrences or phrase splits')
    result=copy.deepcopy(source)
    decisions={}; additions=[]; rejected=[]
    for index, receipt in enumerate(receipts):
        offset=receipt['identity']['start']
        for decision in receipt['audit']['cues']:
            if decision['id'] in decisions:raise ValueError('Duplicate audit decision')
            decisions[decision['id']]={**decision,'clip_index':index,'offset':offset}
        for row in receipt['audit']['missing']:
            if row['uncertain']:continue
            additions.append(dict(text=row['text'],start_seconds=offset+row['start_seconds'],
                end_seconds=offset+row['end_seconds'],confidence=.5,uncertain=True,
                source_line_indices=[index],audio_review=dict(action='added',receipt=index)))
    if set(decisions)!=set(range(len(source['lines']))):raise ValueError('Incomplete audio audit')
    rows=[]
    for i,row in enumerate(result['lines']):
        decision=decisions[i]
        row['audio_review']=decision
        if decision['status']=='absent':
            rejected.append(row);continue
        if decision['status']=='uncertain':row['uncertain']=True;row['confidence']=.5
        else:
            start=decision['offset']+decision['start_seconds']
            end=decision['offset']+decision['end_seconds']
            if max(abs(start-row['start_seconds']),abs(end-row['end_seconds']))>.5:
                row['start_seconds'],row['end_seconds']=start,end
                row['uncertain']=True;row['confidence']=.5
        rows.append(row)
    result['lines']=sorted(rows+additions,key=lambda r:r['start_seconds'])
    result['source']['audio_review_rejected']=rejected
    result['research_variant']='audio-audited-correction'
    result['acceptance_status']='needs_independent_evaluation'
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('manifest',type=Path)
    args=parser.parse_args();root=args.manifest.resolve().parent
    for t in json.loads(args.manifest.read_text()):
        folder=root/t['id']/'independent-audit'
        if not (folder/'summary.json').exists():continue
        source=json.loads((folder/'candidate.json').read_text())
        receipts=[json.loads(p.read_text()) for p in sorted(folder.glob('????.json'))]
        output=corrected(source,receipts)
        atomic_write_json(root/t['id']/'candidate-audio-reviewed.json',output)
        print(t['id'],len(source['lines']),'->',len(output['lines']))

if __name__=='__main__':main()
