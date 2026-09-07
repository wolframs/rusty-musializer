#!/usr/bin/env python3
"""Independent short-context Whisper evidence; decode only, never playback."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import external_analysis
from analysis_io import atomic_write_json, sha256_file


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--track')
    args = parser.parse_args()
    root = args.manifest.resolve().parent
    binary, model = external_analysis._default_whisper_paths()
    if binary is None or model is None:
        parser.error('Whisper runtime unavailable')
    env = external_analysis._safe_local_env()
    model_hash = sha256_file(model)
    for track in json.loads(args.manifest.read_text()):
        if args.track and track['id'] != args.track:
            continue
        directory = root / track['id'] / 'whisper-chunks'
        directory.mkdir(exist_ok=True)
        for start in range(0, int(track['duration_seconds']), 15):
            output = directory / f'{start:04d}.json'
            if output.exists():
                continue
            seconds = min(18, track['duration_seconds'] - start)
            print('START', track['id'], start, flush=True)
            with tempfile.TemporaryDirectory(prefix='musializer-whisper-research-') as temp:
                wav = Path(temp) / 'clip.wav'
                prefix = Path(temp) / 'result'
                subprocess.run(['ffmpeg', '-v', 'error', '-nostdin', '-i', track['audio'],
                                '-ss', str(start), '-t', str(seconds), '-vn', '-ac', '1',
                                '-ar', '16000', '-map_metadata', '-1', str(wav)],
                               env=env, check=True, timeout=60)
                _, command = external_analysis.whisper_request(
                    wav, whisper_bin=binary, model=model, language='en', dtw_model=None,
                    ffmpeg='ffmpeg', output_prefix=prefix)
                command[command.index('-f') + 1] = str(wav)
                with output.with_suffix('.log').open('w') as log:
                    subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   check=True, timeout=180)
                raw = json.loads(prefix.with_suffix('.json').read_text())
                normalized = external_analysis.normalize_whisper(
                    raw, audio_sha256=track['sha256'], audio_duration=seconds,
                    model=model.name, prefer_dtw=False)
                normalized['research'] = dict(clip_start_seconds=start,
                                               clip_duration_seconds=seconds,
                                               model_sha256=model_hash,
                                               source='independent-whisper-chunk')
                atomic_write_json(output.with_suffix('.raw.json'), raw)
                atomic_write_json(output, normalized)
            print('DONE', track['id'], start, flush=True)


if __name__ == '__main__':
    main()
