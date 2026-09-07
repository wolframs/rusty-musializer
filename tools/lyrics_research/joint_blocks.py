#!/usr/bin/env python3
"""Ablate independent line refinement and single-line anchor partitions.

Research only: results do not enter the application's analysis cache. Uses
real decoded PCM without opening an audio output device.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import anchor_block_align
import lyric_anchor_block


def coalesce(blocks: list[dict], maximum_seconds: float = 40.0) -> list[dict]:
    result: list[dict] = []
    for block in blocks:
        if result:
            previous = result[-1]
            contiguous = previous['last_line'] + 1 == block['first_line']
            short = previous['last_line'] - previous['first_line'] + 1 < 4
            bounded = block['window_end'] - previous['window_start'] <= maximum_seconds
            if contiguous and short and bounded:
                previous['last_line'] = block['last_line']
                previous['window_end'] = max(previous['window_end'], block['window_end'])
                previous['kind'] = 'joint-anchors'
                continue
        result.append(dict(block))
    for i, block in enumerate(result):
        block['index'] = i
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--variant', choices=['no-local', 'joint', 'joint-no-stars', 'bounded-local', 'interval-coarse'],
                        required=True)
    parser.add_argument('--track')
    args = parser.parse_args()
    root = args.manifest.resolve().parent
    original = lyric_anchor_block.build_blocks

    def grouped(*a, **kw):
        blocks = original(*a, **kw)
        return blocks if any(b['kind'] == 'section-evidence' for b in blocks) else coalesce(blocks)

    if args.variant not in {'bounded-local', 'interval-coarse'}:
        lyric_anchor_block.coarse_local_refinement_allowed = lambda *_: False
    if args.variant in {'joint', 'joint-no-stars'}:
        lyric_anchor_block.build_blocks = grouped
    initial = {}
    original_align = anchor_block_align.align_block

    def bounded(waveform, rate, lines, block, *a, **kw):
        if block['kind'] == 'coarse-local-evidence':
            position = block['first_line']
            current = initial.get(position, {})
            start = current.get('acoustic_start_seconds')
            end = current.get('acoustic_end_seconds')
            if start is not None and end is not None:
                block = dict(block)
                previous_end = initial.get(position - 1, {}).get('acoustic_end_seconds')
                next_start = initial.get(position + 1, {}).get('acoustic_start_seconds')
                if previous_end is not None and previous_end <= start:
                    block['window_start'] = max(block['window_start'], previous_end - 0.1)
                if next_start is not None and next_start >= end:
                    block['window_end'] = min(block['window_end'], next_start + 0.1)
        result = original_align(waveform, rate, lines, block, *a, **kw)
        if not block['kind'].startswith('global-') and block['kind'] != 'coarse-local-evidence':
            for position, decision in result.items():
                if position not in initial or decision.get('score', 0) > initial[position].get('score', 0):
                    initial[position] = decision
        return result

    if args.variant == 'bounded-local':
        anchor_block_align.align_block = bounded
    original_assemble = lyric_anchor_block.assemble_document

    def interval_assemble(plan, decisions, owners, coarse, **kw):
        from collections import Counter
        counts = Counter(tuple(line['tokens']) for line in plan['lines'])
        adjusted = dict(kw.get('trusted_coarse') or {})
        changes = {}
        for position, line in enumerate(plan['lines']):
            index = line['index']
            proposal = adjusted.get(index)
            decision = decisions.get(position, {})
            start, end = decision.get('acoustic_start_seconds'), decision.get('acoustic_end_seconds')
            if proposal and start is not None and end is not None and counts[tuple(line['tokens'])] == 1:
                if proposal[0] <= start <= proposal[1] + 4 and end <= proposal[1] + 4.15:
                    adjusted[index] = (start, proposal[1])
                    changes[index] = {'original': proposal, 'selection_only': adjusted[index]}
        kw['trusted_coarse'] = adjusted
        document = original_assemble(plan, decisions, owners, coarse, **kw)
        document['research_interval_coarse'] = changes
        return document

    if args.variant == 'interval-coarse':
        lyric_anchor_block.assemble_document = interval_assemble
    for track in json.loads(args.manifest.read_text()):
        if args.track and track['id'] != args.track:
            continue
        output = root / track['id'] / f'candidate-{args.variant}.json'
        if output.exists():
            continue
        source = root / track['id'] / 'candidate-parser'
        whisper = json.loads((source / 'lyrics.whisper.json').read_text())
        coarse = json.loads((source / 'lyrics.sync.json').read_text())
        print('START', track['id'], args.variant, flush=True)
        initial.clear()
        document = anchor_block_align.align(
            Path(track['audio']), whisper, (root / track['id'] / 'reference.txt').read_text(),
            coarse, audio_duration=track['duration_seconds'],
            interior_stars=args.variant != 'joint-no-stars')
        document['research_variant'] = args.variant
        output.write_text(json.dumps(document, indent=2) + '\n')
        print('DONE', track['id'], len(document['lines']), len(document['unresolved']), flush=True)


if __name__ == '__main__':
    main()
