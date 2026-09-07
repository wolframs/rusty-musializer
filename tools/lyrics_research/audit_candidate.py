#!/usr/bin/env python3
"""Ask Gemini to audit every active cue and every audio interval independently.

The reported rate is a model-audited estimate, not human-adjudicated ground
truth. Uncertain judgements prevent an acceptance claim. No output is applied
back to the candidate; inputs, prompts and raw receipts are preserved.
"""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import antigravity_audio
from authored_audio_occurrences import rendered_lines
from analysis_io import atomic_write_json, canonical_sha256

PROMPT = '''Listen to the audio directly and audit the proposed captions below.
They are fallible proposals, NOT a transcript to assume is correct. Check every
scored cue against the actual performance, including held vowels, fast speech,
backing voices, repeated lines and instrumental gaps. No tools. Do not estimate
speech times from reading speed. Times are relative to this clip's first sample.
For each scored cue: is it one actually performed phrase at approximately this
location, or an extra/duplicate? If present, give its actual first and last vocal
phoneme time, excluding reverb. Use uncertain if you cannot judge reliably, or
if either boundary is outside the clip. Preserve the proposal's line grouping
when the words are actually heard, even if you would punctuate them differently.
Unscored context cues are supplied to prevent counting them again as missing.
Also report any performed lyric phrase whose START falls in the audit interval
and which NONE of the supplied scored/context cues represents. An ad-lib or
repeat counts; production directions do not. Do not count a phrase as missing
just because its proposed time is wrong; mark its scored cue's actual timing.
Return only JSON, with every scored cue id exactly once:
{"cues":[{"id":0,"status":"present","start_seconds":1.0,"end_seconds":2.0,
"reason":"short explanation"}],"missing":[{"text":"heard missing phrase",
"start_seconds":3.0,"end_seconds":4.0,"uncertain":false}],"notes":[]}
Allowed cue status: present, absent, uncertain. Absent/uncertain need no times.
Be exact about uncertainty; this is testing a 0.5-second timing tolerance.
'''
BLIND_TIMING = '''No proposed timestamps are supplied. Locate each phrase from
the audio itself. Caption order is context only; it may contain duplicate or
invented phrases. Do not derive any timestamp from list position or reading speed.
'''


def parse(raw, ids, length, core):
    raw = raw.strip()
    if raw.startswith('```') and raw.endswith('```'):
        raw = raw.split('\n', 1)[1].rsplit('```', 1)[0]
    value = json.loads(raw)
    if not isinstance(value, dict) or not isinstance(value.get('cues'), list) or not isinstance(value.get('missing'), list):
        raise ValueError('Audit needs cues and missing arrays')
    if sorted(r['id'] for r in value['cues']) != sorted(ids):
        raise ValueError('Audit must judge every owned cue exactly once')
    for row in value['cues']:
        if row.get('status') not in ('present', 'absent', 'uncertain'):
            raise ValueError('Invalid audit status')
    for row in value['cues'] + value['missing']:
        if row.get('status', 'present') == 'present':
            start, end = row.get('start_seconds'), row.get('end_seconds')
            if any(type(x) not in (int, float) for x in (start, end)) or not 0 <= start < end <= length + .02:
                raise ValueError('Invalid audit timing')
    for row in value['missing']:
        if not isinstance(row.get('text'), str) or not row['text'].strip() or type(row.get('uncertain')) is not bool:
            raise ValueError('Invalid missing phrase')
        if not core[0] <= row['start_seconds'] < core[1]:
            raise ValueError('Missing phrase belongs to another audit interval')
    return value


def summarize(rows, receipts):
    counts = dict(present=0, timing_errors=0, extra=0, missing=0, uncertain=0)
    for receipt in receipts:
        offset = receipt['identity']['start']
        for row in receipt['audit']['cues']:
            if row['status'] == 'uncertain':counts['uncertain'] += 1
            elif row['status'] == 'absent':counts['extra'] += 1
            else:
                counts['present'] += 1
                proposed = rows[row['id']]
                if max(abs(row[f'{edge}_seconds'] + offset - proposed[f'{edge}_seconds']) for edge in ('start','end')) > .5:
                    counts['timing_errors'] += 1
        for row in receipt['audit']['missing']:
            counts['uncertain' if row['uncertain'] else 'missing'] += 1
    denominator = counts['present'] + counts['missing']
    error_count = counts['timing_errors'] + counts['extra'] + counts['missing']
    return counts, denominator, error_count


async def run(args):
    root = args.manifest.resolve().parent
    paths = antigravity_audio.discover()
    semaphore = asyncio.Semaphore(2)
    for track in json.loads(args.manifest.read_text()):
        if args.track and args.track != track['id']:
            continue
        directory = root / track['id'] / args.output_dir
        directory.mkdir(exist_ok=True)
        source = root / track['id'] / args.candidate
        candidate = json.loads(source.read_text())
        rows = rendered_lines(candidate)
        atomic_write_json(directory / 'candidate.json', candidate)
        length = track['duration_seconds']
        async def audit(index, core_start):
            core_end = min(length, core_start + 18)
            start, end = max(0, core_start - 3), min(length, core_end + 9)
            supplied = [dict(id=i, text=row['text'],
                start_seconds=row['start_seconds'] - start,
                end_seconds=row['end_seconds'] - start,
                scored=core_start <= row['start_seconds'] < core_end)
                for i, row in enumerate(rows) if row['end_seconds'] > start - 3 and row['start_seconds'] < end + 3]
            ids = [row['id'] for row in supplied if row['scored']]
            core = (core_start - start, core_end - start)
            if not args.include_timestamps:
                for row in supplied:
                    del row['start_seconds']
                    del row['end_seconds']
            prompt = PROMPT + ('' if args.include_timestamps else BLIND_TIMING) + '\n' + json.dumps(dict(audit_interval=core, proposed_cues=supplied))
            identity = dict(audio_sha256=track['sha256'], candidate_sha256=canonical_sha256(candidate),
                            prompt_sha256=canonical_sha256(prompt), start=start, end=end,
                            model='gemini-3.8-flash-high')
            receipt = directory / f'{index:04}.json'
            if receipt.exists():
                saved = json.loads(receipt.read_text())
                if saved['identity'] == identity:
                    return saved
            async with semaphore:
                clip = await asyncio.to_thread(subprocess.check_output, [
                    'ffmpeg', '-v', 'error', '-i', track['audio'], '-ss', str(start), '-t', str(end-start),
                    '-vn', '-ac', '1', '-ar', '16000', '-map_metadata', '-1', '-f', 'wav', 'pipe:1'], timeout=60)
                for attempt in range(2):
                    response = await antigravity_audio.ask(SimpleNamespace(**paths,
                        model=identity['model'], timeout=180), clip, prompt)
                    try:
                        parsed = parse(response['response'], ids, end-start, core)
                        break
                    except (ValueError, KeyError, TypeError) as error:
                        atomic_write_json(directory / f'{index:04}.failed-{attempt}.json',
                            dict(identity=identity, response=response['response'], error=str(error)))
                        if attempt:
                            raise
                result = dict(identity=identity, clip_sha256=hashlib.sha256(clip).hexdigest(),
                    agent=response['agent'], model=response['model'], session_id=response['session_id'],
                    response=response['response'], audit=parsed)
                atomic_write_json(receipt, result)
                print(track['id'], 'audit', index, flush=True)
                return result
        tasks = [asyncio.create_task(audit(i, float(start))) for i, start in enumerate(range(0, int(length)+1, 18)) if start < length]
        try:
            receipts = await asyncio.gather(*tasks)
        finally:
            for task in tasks:
                if not task.done():task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
        counts, denominator, error_count = summarize(rows, receipts)
        atomic_write_json(directory/'summary.json', dict(track=track, counts=counts,
            proposed_cues=len(rows), estimated_reference_lines=denominator,
            model_audited_error_rate=error_count/denominator if denominator else None,
            timestamps_blinded=not args.include_timestamps,
            acceptance_status='model_audit_not_adjudicated', candidate_sha256=canonical_sha256(candidate)))
        print('DONE', track['id'], counts, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--track')
    parser.add_argument('--candidate', default='candidate-antigravity/lyrics.antigravity.aligned.json')
    parser.add_argument('--output-dir', default='independent-audit')
    parser.add_argument('--include-timestamps', action='store_true',
                        help='Negative-control protocol only; can bias timing judgements')
    args=parser.parse_args()
    asyncio.run(run(args))

if __name__ == '__main__':main()
