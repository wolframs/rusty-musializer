#!/usr/bin/env python3
"""Produce a local vocal-stem experiment with an unchanged sample clock.

No playback or remote requests. Downloads the official TorchAudio checkpoint
if absent. Run under the Lyrics alignment interpreter. Original audio remains
the reference: separation can remove or distort real vocals.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from analysis_io import atomic_write_json, sha256_file


def main():
    import numpy as np
    import torch
    import torchaudio

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--track', action='append', help='Repeat to select several track IDs')
    parser.add_argument('--start', type=float, default=0)
    parser.add_argument('--duration', type=float, help='Default: remainder of each selected track')
    parser.add_argument('--device', default='cuda')
    args = parser.parse_args()
    if not math.isfinite(args.start) or args.start < 0:
        parser.error('--start must be finite and nonnegative')
    if args.duration is not None and (not math.isfinite(args.duration) or args.duration <= 0):
        parser.error('--duration must be finite and positive')
    tracks = json.loads(args.manifest.read_text())
    if args.track and set(args.track) - {t['id'] for t in tracks}:
        parser.error('Unknown track ID')
    torch.set_num_threads(4)
    bundle = torchaudio.pipelines.HDEMUCS_HIGH_MUSDB_PLUS
    model = bundle.get_model().to(args.device).eval()
    # Record the exact checkpoint actually loaded by this bundle, not just its name.
    checkpoint = torchaudio.utils._download_asset(bundle._model_path, progress=False)
    checkpoint_sha = sha256_file(checkpoint)
    rate = bundle.sample_rate
    for track in tracks:
        if args.track and track['id'] not in args.track:
            continue
        duration = min(track['duration_seconds'] - args.start,
                       args.duration or track['duration_seconds'])
        if duration <= 0:
            raise ValueError(f"Start is outside track {track['id']}")
        if sha256_file(track['audio']) != track['sha256']:
            raise ValueError(f"Audio changed for track {track['id']}")
        directory = args.output / track['id']
        directory.mkdir(parents=True, exist_ok=True)
        identity = dict(version=1, track=track['id'], audio_sha256=track['sha256'],
                        start_seconds=args.start, duration_seconds=duration,
                        model='HDEMUCS_HIGH_MUSDB_PLUS', checkpoint_sha256=checkpoint_sha,
                        torch=torch.__version__, torchaudio=torchaudio.__version__,
                        device=args.device, sample_rate=rate, chunk_seconds=10,
                        overlap_seconds=2, output_rate=16000)
        receipt = directory / 'identity.json'
        output = directory / 'vocals.wav'
        if receipt.exists() and output.exists():
            previous = json.loads(receipt.read_text())
            if previous.get('request') == identity and previous.get('output_sha256') == sha256_file(output):
                print('CACHED', track['id'], flush=True)
                continue
        raw = subprocess.check_output([
            'ffmpeg', '-v', 'error', '-i', track['audio'], '-ss', str(args.start),
            '-t', str(duration), '-vn', '-ac', '2', '-ar', str(rate),
            '-f', 'f32le', 'pipe:1'], timeout=120)
        mix = torch.from_numpy(np.frombuffer(raw, dtype='<f4').copy().reshape(-1, 2).T)
        mean, std = mix.mean(), mix.std()
        if not torch.isfinite(mix).all() or std <= 1e-8:
            raise ValueError('Empty, nonfinite or effectively silent separation input')
        wave = (mix - mean) / std
        result, weight = torch.zeros_like(wave), torch.zeros(wave.shape[-1])
        chunk, stride, overlap = rate * 10, rate * 8, rate * 2
        with torch.inference_mode():
            for offset in range(0, wave.shape[-1], stride):
                stop = min(wave.shape[-1], offset + chunk)
                sources = model(wave[:, offset:stop][None].to(args.device))[0]
                vocal = sources[model.sources.index('vocals')].cpu() * std
                ramp = torch.ones(stop - offset)
                n = min(overlap, len(ramp))
                if offset:
                    ramp[:n] = torch.linspace(0, 1, n)
                if stop < wave.shape[-1]:
                    ramp[-n:] = torch.linspace(1, 0, n)
                result[:, offset:stop] += vocal * ramp
                weight[offset:stop] += ramp
                if stop == wave.shape[-1]:
                    break
        if torch.any(weight <= 0):
            raise ValueError('Separation left samples uncovered')
        result /= weight
        temporary = directory / 'vocals.tmp.wav'
        subprocess.run([
            'ffmpeg', '-v', 'error', '-y', '-f', 'f32le', '-ar', str(rate),
            '-ac', '2', '-i', 'pipe:0', '-ac', '1', '-ar', '16000', str(temporary)],
            input=result.T.contiguous().numpy().astype('<f4').tobytes(), check=True, timeout=120)
        temporary.replace(output)
        atomic_write_json(receipt, dict(request=identity, output_sha256=sha256_file(output),
            input_frames=mix.shape[-1], separated_frames=result.shape[-1],
            vocal_rms=float(result.square().mean().sqrt()),
            mix_rms=float(mix.square().mean().sqrt()),
            reference_authority='original_mix', status='research_only'))
        print('DONE', track['id'], flush=True)


if __name__ == '__main__':
    main()
