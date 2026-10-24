#!/usr/bin/env python3
"""Check the six Pixel Cats scenes through the real application.

For each staging (pixel-boot, pixel-shell, pixel-run, pixel-quartet,
pixel-chapel, pixel-rave) a treatment puts one lyric line over a synthetic
groove and takes stills through the export's own offline path:

  - it draws: the `pixel cats:` report line names the staging, the line typed
    to completion, glyphs and at least one cat, and the CRT shader carried it;
  - it moves: two moments of one section differ;
  - it reacts: the same moment with `response` 1 and with `response` 0 (the
    music shut out) differ. `--negative-control` sets `response` 0 in both, and
    this check MUST then fail: that is what proves it measures the music.

Once, for the rave: the still is the frame the export encodes (PSNR against
the decoded MP4, with a neighbouring frame as the control), a second process
draws the same still byte for byte, and a 9:16 still draws a 180x320 canvas.

Every launch goes through tools/headless_run.sh: a private display and config,
`--mute`, and an unreachable audio sink. The PCM is synthetic and audible; only
playback is muted. Artifacts go to build/pixel-cats-check/.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import signal
import subprocess
import sys
import wave
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
LINES = {
    'pixel-boot': 'SUDO MEOW',
    'pixel-shell': 'sudo meow',
    'pixel-run': 'now every bracket bows to me',
    'pixel-quartet': 'I found a terminal behind the drums',
    'pixel-chapel': 'Kyrie meow ison',
    'pixel-rave': 'meow oh meow oh meow',
}
STAGING = {
    'pixel-boot': 'boot', 'pixel-shell': 'shell', 'pixel-run': 'runner',
    'pixel-quartet': 'quartet', 'pixel-chapel': 'chapel', 'pixel-rave': 'rave',
}


def command(argv, log, timeout=180):
    with log.open('w') as stream:
        process = subprocess.Popen([str(arg) for arg in argv], cwd=ROOT, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        except BaseException:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            raise
    if code:
        raise RuntimeError(f'{Path(str(argv[0])).name} failed ({code}); see {log}')


def app(out, name, *args):
    command([ROOT/'tools/headless_run.sh', *args], out/f'{name}.log')


def image(path):
    data = np.asarray(Image.open(path).convert('RGBA'))
    assert np.all(data[:, :, 3] == 255), f'{path.name}: translucent output'
    return data[:, :, :3].astype(float)


def soft(path):
    from PIL import ImageFilter
    return np.asarray(Image.open(path).convert('RGB').filter(ImageFilter.GaussianBlur(2))).astype(float)


def psnr(a, b):
    mse = np.mean((a-b)**2)
    return math.inf if mse == 0 else 10*math.log10(255**2/mse)


def report(log):
    for line in log.read_text().splitlines():
        if line.startswith('pixel cats:'):
            return line
    raise AssertionError(f'{log.name}: no pixel cats report line')


def fixture(path):
    """Eight seconds: a kick every half second, off-beat hats, a sung-ish
    mid tone that swells, and a pad under it."""
    rate = 44100
    t = np.arange(rate*8, dtype=float)/rate
    pcm = np.zeros_like(t)
    for i, freq in enumerate([110, 164.81, 220, 329.63, 440, 659.26]):
        pcm += 0.04*np.sin(2*np.pi*freq*t+i)
    pcm *= 0.5+0.5*np.sin(2*np.pi*0.25*t)**2
    kick = np.mod(t, 0.5)
    pcm += 0.35*np.sin(2*np.pi*58*t*(1+0.6*np.exp(-kick/0.03)))*np.exp(-kick/0.09)
    hat = np.mod(t+0.25, 0.5)
    noise = np.random.default_rng(3).standard_normal(len(t))
    pcm += 0.08*noise*np.exp(-hat/0.015)
    voice = 0.12*np.sin(2*np.pi*(523.25+8*np.sin(2*np.pi*5*t))*t)*(0.5+0.5*np.sin(2*np.pi*0.8*t))
    pcm += voice*(t > 1.0)
    stereo = np.stack([pcm, pcm*0.9], axis=1)
    with wave.open(str(path), 'wb') as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(rate)
        wav.writeframes((np.clip(stereo, -0.95, 0.95)*32767).astype('<i2').tobytes())


def treatment(out, scene, response, resolution='640x360'):
    doc = {
        'schema': 'musializer.treatment/v1',
        'audio': {'path': str(out/'groove.wav')},
        'output': {'resolution': resolution, 'fps': 30, 'quality': 'high'},
        'seed': 11,
        'scenes': {scene: {'settings': {'response': response}}},
        'segments': [{'at': 0, 'scene': scene}],
        'lyrics': [{'at': 1.0, 'until': 6.5, 'text': LINES[scene]}],
    }
    path = out/f'{scene}-{"music" if response else "ambient"}.treatment.json'
    path.write_text(json.dumps(doc))
    return path


def check_scene(out, scene, negative):
    music = treatment(out, scene, 0 if negative else 1)
    ambient = treatment(out, scene, 0)
    app(out, f'{scene}-music', '--treatment', music,
        '--still', '2.6', out/f'{scene}-early.png', '--still', '4', out/f'{scene}-music.png')
    app(out, f'{scene}-ambient', '--treatment', ambient, '--still', '4', out/f'{scene}-ambient.png')
    line = report(out/f'{scene}-music.log')
    assert f'staging={STAGING[scene]} ' in line, line
    assert ' line=complete ' in line, f'{scene}: the line did not type to completion: {line}'
    glyphs = int(re.search(r' glyphs=(\d+)', line)[1])
    cats = int(re.search(r' cats=(\d+)', line)[1])
    assert glyphs >= 8, f'{scene}: drew {glyphs} glyphs'
    assert cats >= 1, f'{scene}: no cat drew'
    assert 'crt=shader' in line, f'{scene}: the CRT pass fell back: {line}'
    a = image(out/f'{scene}-music.png')
    assert a.std() > 12, f'{scene}: flat frame'
    moved = float(np.mean(np.abs(a-image(out/f'{scene}-early.png'))))
    assert moved > 1.0, f'{scene}: nothing moved between 2.6 s and 4 s (MAD {moved:.3f})'
    reacts = float(np.mean(np.abs(a-image(out/f'{scene}-ambient.png'))))
    assert reacts > 1.0, f'{scene}: the music did not reach the picture (MAD {reacts:.3f})'
    result = {'reacts_mad': round(reacts, 3), 'moves_mad': round(moved, 3),
              'glyphs': glyphs, 'cats': cats}
    print(scene, json.dumps(result), flush=True)
    return result


def check_export(out, encoder):
    scene = 'pixel-rave'
    music = out/f'{scene}-music.treatment.json'
    app(out, 'export', '--treatment', music, '--encoder', encoder,
        '--render-window', '0', '5', '--render', out/'rave.mp4')
    for name, index in [('frame-120', 120), ('frame-100', 100)]:
        command(['ffmpeg', '-v', 'error', '-i', out/'rave.mp4', '-vf', f'select=eq(n\\,{index})',
                 '-frames:v', '1', '-y', out/f'{name}.png'], out/f'{name}.log')
    # Pixel art behind a CRT grid is the encoder's worst case: a lossy 4:2:0
    # stream smears the one-pixel scanlines and the colour split, so the raw
    # PSNR is codec-bound (the still's own 4:2:0 round trip is far higher).
    # Compared after a 2 px blur the codec's grain is gone and a structural
    # difference — a wrong frame, a shifted canvas, a missed line — is not.
    still = image(out/f'{scene}-music.png')
    same = psnr(still, image(out/'frame-120.png'))
    other = psnr(still, image(out/'frame-100.png'))
    same_soft = psnr(soft(out/f'{scene}-music.png'), soft(out/'frame-120.png'))
    other_soft = psnr(soft(out/f'{scene}-music.png'), soft(out/'frame-100.png'))
    assert same_soft > 32 and same_soft > other_soft + 8 and same > other + 8, \
        f'the still is not the video frame: {same_soft:.2f} dB blurred, {same:.2f} dB raw ' \
        f'(neighbour {other_soft:.2f} / {other:.2f} dB)'
    app(out, 'again', '--treatment', music, '--still', '4', out/'again.png')
    identical = (out/'again.png').read_bytes() == (out/f'{scene}-music.png').read_bytes()
    assert identical, 'a second process drew a different still'
    portrait = treatment(out, scene, 1, '360x640')
    app(out, 'portrait', '--treatment', portrait, '--still', '4', out/'portrait.png')
    tall = image(out/'portrait.png')
    assert tall.shape == (640, 360, 3) and tall.std() > 12, 'portrait lost the scene'
    canvas = re.search(r' canvas=(\d+x\d+)x\d+', report(out/'portrait.log'))[1]
    assert canvas == '180x320', f'portrait canvas {canvas}'
    result = {'still_video_db': round(same, 2), 'neighbour_db': round(other, 2),
              'still_video_blurred_db': round(same_soft, 2),
              'neighbour_blurred_db': round(other_soft, 2),
              'reprocess': 'identical', 'portrait_canvas': canvas}
    print('export', json.dumps(result), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--output', type=Path, default=ROOT/'build/pixel-cats-check')
    parser.add_argument('--negative-control', action='store_true',
                        help='shut the music out of both renders; the check must then fail')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    os.environ['MZ_STATE_DIR'] = str(out/'isolated-state')
    fixture(out/'groove.wav')
    probe = subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'color=s=256x256:d=0.1',
                            '-frames:v', '1', '-c:v', 'h264_nvenc', '-f', 'null', '-'],
                           capture_output=True)
    encoder = 'nvenc' if probe.returncode == 0 else 'x264'
    results = {scene: check_scene(out, scene, args.negative_control) for scene in LINES}
    results['export'] = check_export(out, encoder)
    (out/'measurements.json').write_text(json.dumps(results, indent=2)+'\n')
    print('PASS: every Pixel Cats staging draws its line, moves, and answers the music', flush=True)


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        print(f'FAIL: {error}', file=sys.stderr)
        sys.exit(1)
