#!/usr/bin/env python3
"""Align independently heard phrases inside their actual short audio crops.

This experiment asks whether audio-language phrase discovery plus local joint
CTC fixes occurrence errors. It does not claim the discovery model is reference
truth, and its output is not an application bridge.
"""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import sys
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import anchor_block_align
import lyric_align
import ctc_window_align


def main() -> None:
    import torch
    import torchaudio
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--track', required=True)
    parser.add_argument('--constrained', action='store_true')
    args = parser.parse_args()
    root = args.manifest.resolve().parent
    track = next(t for t in json.loads(args.manifest.read_text()) if t['id'] == args.track)
    waveform, rate = anchor_block_align._load_audio(Path(track['audio']))
    bundle = torchaudio.pipelines.MMS_FA
    model = bundle.get_model(with_star=True).to('cuda').eval()
    tokenizer, aligner = bundle.get_tokenizer(), bundle.get_aligner()
    cues, diagnostics = [], []
    for path in sorted((root / track['id'] / 'gemini-blind').glob('*.json')):
        report = json.loads(path.read_text())
        raw = report['response'].strip()
        if raw.startswith('```'):
            raw = raw.split('\n', 1)[1].rsplit('```', 1)[0]
        response = json.loads(raw)
        lines = [dict(display=r['text'], tokens=lyric_align.normalize_tokens(r['text']))
                 for r in response['lines']]
        if not lines:
            continue
        offset = report['clip_start_seconds']
        length = report['clip_duration_seconds']
        block = dict(index=0, first_line=0, last_line=len(lines) - 1,
                     window_start=offset, window_end=offset + length,
                     kind='audio-model-observed-phrases')
        windowed = [None]
        for i, observed in enumerate(response['lines']):
            if i:
                windowed.append(None)
            words = anchor_block_align.alignment_words(observed['text'])
            windowed.extend([(max(0, observed['start_seconds'] - 0.75),
                              min(length, observed['end_seconds'] + 0.75))] * len(words))
        windowed.append(None)

        def constrained_aligner(emission, tokens):
            frame_count = len(emission)
            bounds = [None if window is None else (
                max(0, math.floor(window[0] / length * frame_count)),
                min(frame_count, math.ceil(window[1] / length * frame_count)))
                for window in windowed]
            flat = [token for word in tokens for token in word]
            token_bounds = [window for word, window in zip(tokens, bounds) for _ in word]
            spans = iter(ctc_window_align.align(emission.numpy(), flat, token_bounds))
            return [[SimpleNamespace(start=a, end=b, score=s)
                     for a, b, s in [next(spans) for _ in word]] for word in tokens]

        try:
            decisions = anchor_block_align.align_block(
                waveform, rate, lines, block, model, tokenizer,
                constrained_aligner if args.constrained else aligner, interior_stars=True)
        except ValueError as error:
            diagnostics.append(dict(evidence=str(path), error=str(error)))
            continue
        for i, observed in enumerate(response['lines']):
            decision = decisions.get(i, {})
            if decision.get('status') not in {'aligned', 'weak'}:
                diagnostics.append(dict(evidence=str(path), observed=observed, decision=decision))
                continue
            # Cropped phrases are not complete reference lines or captions.
            if (observed['start_seconds'] >= 15
                    or (offset > 0 and observed['start_seconds'] <= 0.15)
                    or observed['end_seconds'] >= length - 0.15):
                continue
            cue = dict(text=observed['text'],
                       start_seconds=decision['acoustic_start_seconds'],
                       end_seconds=decision['acoustic_end_seconds'],
                       word_alignments=decision.get('word_alignments', []),
                       evidence=str(path), text_authority='audio-model-proposal',
                       model_start_seconds=offset + observed['start_seconds'],
                       model_end_seconds=offset + observed['end_seconds'],
                       uncertain=observed.get('uncertain', False))
            cues.append(cue)
    variant = 'gemini-constrained' if args.constrained else 'gemini-phrases'
    output = {'audio': track, 'research_variant': variant,
              'lines': sorted(cues, key=lambda row: row['start_seconds']),
              'diagnostics': diagnostics, 'acceptance_status': 'not_adjudicated'}
    path = root / track['id'] / f'candidate-{variant}.json'
    path.write_text(json.dumps(output, indent=2) + '\n')
    print(path, len(cues))


if __name__ == '__main__':
    main()
