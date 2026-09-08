#!/usr/bin/env python3
"""Author a short scene audition from measured audio; never plays or uploads it.

Choose low-energy cuts near two separated passages, or supply exact --window
START:END bounds after listening. These are measured candidate boundaries, not
claims about phrases, bars, lyrics, or human acceptance. Never replaces a session.
See tools/listening-lab/README.md for the complete creation and playback loop.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

import numpy as np


def candidate_windows(document: dict) -> list[tuple[float, float]]:
    frames = document['frames']
    times = np.array([f['time_seconds'] for f in frames])
    rms = np.array([f['rms'] for f in frames])
    duration = float(document['audio']['duration_seconds'])
    if duration < 35 or len(times) < 10:
        raise ValueError('automatic selection needs at least 35 seconds; use --window')
    hop = float(np.median(np.diff(times)))
    n = max(1, round(0.20 / hop))
    energy = np.convolve(np.pad(rms, (n, n), mode='edge'), np.ones(2*n+1)/(2*n+1), mode='valid')

    def valley(low: float, high: float) -> float:
        indices = np.flatnonzero((times >= low) & (times <= high))
        if not len(indices):
            raise ValueError('no measured frames inside candidate window')
        # Favor an audible breath rather than a single-frame transient trough.
        index = int(indices[np.argmin(energy[indices])])
        return round(float(times[index]), 3)

    result = []
    for fraction in (0.24, 0.64):
        target = min(duration-32, max(2, duration*fraction))
        start = valley(max(0, target-6), target+4)
        end = valley(start+20, min(duration, start+30))
        result.append((start, end))
    return result


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('audio', type=Path)
    p.add_argument('--measured', type=Path, required=True)
    p.add_argument('--id', required=True)
    p.add_argument('--title', required=True)
    p.add_argument('--output-dir', type=Path, default=Path('build/listening-review'))
    p.add_argument('--window', action='append', default=[], metavar='START:END')
    p.add_argument('--scenes', nargs='+', default=['atlas', 'cadence'])
    args = p.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_-]{1,48}', args.id):
        p.error('id must be 1..48 letters, numbers, underscores or hyphens')
    document = json.loads(args.measured.read_text())
    digest = hashlib.sha256(args.audio.read_bytes()).hexdigest()
    if document['audio']['sha256'] != digest:
        p.error('measured audio digest differs; refusing wrong-track questions')
    duration = float(document['audio']['duration_seconds'])
    windows = [tuple(map(float, w.split(':'))) for w in args.window] if args.window else candidate_windows(document)
    for window in windows:
        if len(window) != 2 or not (0 <= window[0] < window[1] <= duration) or window[1]-window[0] > 600:
            p.error('window must be START:END within the track, at most 600 seconds')
    root = Path(__file__).resolve().parent.parent
    defaults = json.loads(subprocess.check_output([
        'cargo', 'run', '--quiet', '-p', 'musializer-core', '--example', 'listening_defaults'
    ], cwd=root, text=True))
    if any(scene not in defaults for scene in args.scenes):
        p.error('unknown scene name')
    output = args.output_dir.resolve()
    protocol_path = output / f'{args.id}.protocol.json'
    evidence_path = output / f'{args.id}.windows.json'
    if protocol_path.exists() or evidence_path.exists():
        p.error('session already exists; choose a new id to preserve answers and provenance')
    items = []
    for start, end in windows:
        for scene in args.scenes:
            question = {
                'atlas': 'Song Atlas: would you keep this visual treatment for this passage?',
                'cadence': 'Cadence: is the motion smooth and would you keep this look?',
            }.get(scene, f'{scene}: would you keep this look for this passage?')
            items.append(dict(id=f'q{len(items)+1:02}', at_seconds=start,
                window=dict(pre=0, post=end-start), question=question, kind='choice',
                options=['keep', 'interesting but needs fixing', 'reject'],
                apply=dict(scene=scene, seed=20260905, snapshots=dict(a=defaults[scene]))))
    protocol = dict(schema='musializer.protocol/v1', title=args.title,
                    audio=dict(path=str(args.audio.resolve()), sha256=digest), items=items)
    output.mkdir(parents=True, exist_ok=True)
    protocol_path.write_text(json.dumps(protocol, indent=2)+'\n')
    evidence_path.write_text(json.dumps(dict(audio_sha256=digest, measured_cache_key=document['cache_key'],
        method='operator-specified' if args.window else '200ms energy valleys; not verified phrase boundaries',
        windows=windows, settings_source='live Rust descriptor defaults',
        acceptance='unreviewed'), indent=2)+'\n')
    subprocess.run(['node', str(root/'tools/listening-lab/scripts/import-cx4.mjs'),
                    '--output-dir', str(output/'browser'), str(protocol_path)], check=True, cwd=root)
    # This is an openly identified new design review, not a blind sampler trial.
    sheet = output/'browser'/f'{args.id}-feedback.listen.json'
    browser = json.loads(sheet.read_text())
    browser['blind'] = False
    browser['instructions'] = 'Judge the new scene in Musializer. Answer in either the app or this sheet. Keep both on the same qNN if using the browser; N advances the app. R replays. These are candidate musical windows; report a bad cut separately from the look.'
    browser['feedback_templates']['cx4-single']['fields'].append(dict(
        id='window_fit', type='single', label='Did the passage begin and end naturally?', required=True,
        options=[dict(value='natural', label='Natural'), dict(value='start', label='Start cuts a phrase'),
                 dict(value='end', label='End cuts a phrase'), dict(value='both', label='Both cuts are poor')]))
    sheet.write_text(json.dumps(browser, indent=2)+'\n')
    print(protocol_path)
    print(f'Browser protocols: {output / "browser"}')


if __name__ == '__main__':
    main()
