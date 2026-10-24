#!/usr/bin/env python3
"""Check scene families and the website-inspired gallery through the real app.

All app launches use headless_run.sh: private display/config, process mute,
and an unreachable audio sink. Synthetic PCM and generated media stay in build/.
--negative-control disables response in both compared treatments; it must fail.
"""
from __future__ import annotations
import argparse
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import wave
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
NEW = ('chrome', 'aurora', 'metaballs', 'tunnel', 'voronoi', 'iridescent')


def command(argv, log=None, timeout=120):
    with (log.open('w') if log else open(os.devnull, 'w')) as stream:
        process = subprocess.Popen([str(arg) for arg in argv], cwd=ROOT,
                                   stdout=stream, stderr=subprocess.STDOUT,
                                   start_new_session=True)
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


def compare(out, a, b, floor, luma=False):
    """PSNR between two captures, refused at or below `floor`.

    `luma=True` compares BT.709 luma only. That is the right measure when one
    side is a decoded video frame: the encoder stores colour at half
    resolution (4:2:0), so a scene whose look *is* saturated fine colour
    bands (Iridescent Pool's thin film) loses decibels in chroma that say
    nothing about whether the same picture was drawn. Measured 2026-10-24:
    the pool's still against its own video frame was 31-32 dB in RGB and
    39.7 dB in luma; a replay that drew a different moment moves luma too.
    """
    x, y = image(out/a), image(out/b)
    if luma:
        weights = np.array([0.2126, 0.7152, 0.0722])
        x, y = x @ weights, y @ weights
    mse = np.mean((x-y)**2)
    psnr = 10*math.log10(255**2/mse) if mse else math.inf
    assert psnr > floor, f'{out.name}: {a}/{b} replay changed ({psnr:.2f} dB)'
    return round(psnr, 3) if math.isfinite(psnr) else 'identical'


def frame(video, target, index):
    command(['ffmpeg', '-v', 'error', '-i', video, '-vf', f'select=eq(n\\,{index})',
             '-frames:v', '1', '-y', target])


def check_scene(base, name, encoder, negative):
    out = base/name
    out.mkdir(parents=True, exist_ok=True)
    for mode, response in [('music', 0 if negative else 1), ('ambient', 0)]:
        doc = dict(schema='musializer.treatment/v1', audio=dict(path=str(base/'pulse.wav')),
                   output=dict(resolution='640x360', fps=30, quality='high'), seed=17,
                   scenes={name: dict(settings=dict(response=response))},
                   segments=[dict(at=0, scene=name)])
        (out/f'{mode}.treatment.json').write_text(json.dumps(doc))
    app(out, 'music', '--treatment', out/'music.treatment.json',
        '--still', '2', out/'motion.png', '--still', '4', out/'music.png',
        '--save-project', out/'scene.musi', '--encoder', encoder,
        '--render-window', '0', '5', '--render', out/'full.mp4')
    app(out, 'ambient', '--treatment', out/'ambient.treatment.json',
        '--still', '4', out/'ambient.png')
    a = image(out/'music.png')
    mad = float(np.mean(np.abs(a-image(out/'ambient.png'))))
    assert mad > 0.7, f'{name}: music response missing (MAD {mad:.4f})'
    motion = float(np.mean(np.abs(a-image(out/'motion.png'))))
    assert motion > 1, f'{name}: motion missing (MAD {motion:.4f})'
    assert a.std() > 10, f'{name}: flat/dark frame'
    log = (out/'music.log').read_text()
    line = next(line for line in log.splitlines() if line.startswith(f'{name}:'))
    assert ('shader=drawn' in line or 'surface=shader' in line) and 'REFUSED' not in line, line
    app(out, 'reopen', out/'scene.musi', '--still', '4', out/'reopen.png',
        '--encoder', encoder, '--render-window', '4', '1', '--render', out/'clip.mp4')
    app(out, 'portrait', '--treatment', out/'music.treatment.json',
        '--resolution', '360x640', '--still', '4', out/'portrait.png')
    portrait = image(out/'portrait.png')
    assert portrait.shape == (640, 360, 3) and portrait.std() > 10, f'{name}: portrait lost the scene'
    frame(out/'full.mp4', out/'video.png', 120)
    frame(out/'clip.mp4', out/'clip.png', 0)
    result = dict(response_mad=round(mad, 3), motion_mad=round(motion, 3),
                  project=compare(out, 'music.png', 'reopen.png', 60),
                  still_video_db=compare(out, 'music.png', 'video.png', 33, luma=True),
                  full_clip_db=compare(out, 'video.png', 'clip.png', 33, luma=True))
    print(name, json.dumps(result), flush=True)
    return result


def picker_report(log):
    return next(line for line in log.read_text().splitlines() if line.startswith('scene picker:'))


def check_picker(base, vocabulary):
    out = base/'picker'
    out.mkdir(exist_ok=True)
    groups = {}
    for scene in vocabulary['scenes']:
        groups.setdefault(scene['family'], []).append(scene['name'])
    assert len(groups) == 6
    # Click probes settle three frames, then press/hold/release. Leave a
    # following frame for the deferred command to redraw its new selection.
    for family, members in groups.items():
        app(out, family, base/'pulse.wav', '--size', '960x640', '--hud=0',
            '--probe-frames', '12', '--probe-shot', out/f'{family}.png',
            '--ui-probe', f'play=0,sidebar=240,scene-family={family}')
        report = picker_report(out/f'{family}.log')
        assert f'family={family} active=spectrum' in report, report
        assert 'offset=0.0/0.0' in report, f'{family}: does not fit minimum picker'
        visible = re.search(r'visible=\[(.*?)\]', report)[1].split(',')
        assert visible == members, f'{family}: unreachable scenes {members} / {visible}'
        width, height, x, y = map(int, re.search(r'viewport=(\d+)x(\d+)@(\d+),(\d+)', report).groups())
        rows = math.ceil(len(members)/2)
        tile_h = max(24, min(44, (height-4*(rows-1))/rows))
        tile_w = (width-4)/2
        for i, scene in enumerate(members):
            cx = round(x+(i%2)*(tile_w+4)+tile_w/2)
            cy = round(y+(i//2)*(tile_h+4)+tile_h/2)
            app(out, f'click-{scene}', base/'pulse.wav', '--size', '960x640', '--hud=0',
                '--probe-frames', '12', '--probe-shot', out/f'click-{scene}.png',
                '--ui-probe', f'play=0,sidebar=240,scene-family={family},click={cx}x{cy}')
            actual = picker_report(out/f'click-{scene}.log')
            assert f'active={scene} ' in actual, f'tile {scene} did not select it: {actual}'
        print(f'picker {family}: all {len(members)} tiles clicked at 960x640 / 240px sidebar', flush=True)
    # One real tab click proves family navigation reaches the same state as the
    # parked probe, without changing playback or silently selecting a scene.
    # Materials is fourth in the two-row tab strip.
    width, _, x, y = map(int, re.search(r'viewport=(\d+)x(\d+)@(\d+),(\d+)', report).groups())
    cx = round(x+(width-8)/6)
    cy = y-52+26+11
    app(out, 'category-click', base/'pulse.wav', '--size', '960x640', '--hud=0',
        '--probe-frames', '12', '--probe-shot', out/'category-click.png',
        '--ui-probe', f'play=0,sidebar=240,click={cx}x{cy}')
    assert 'family=materials active=spectrum' in picker_report(out/'category-click.log')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT/'build/scene-gallery-check')
    parser.add_argument('--negative-control', action='store_true')
    parser.add_argument('--picker-only', action='store_true')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    # A sustained multiband phrase with a breathing envelope exercises bass,
    # mids and treble at the photographed moment. A sweep alone has already
    # left the bass region at t=4 and cannot test bass-driven geometry there.
    rate=44100
    t=np.arange(rate*6,dtype=float)/rate
    envelope=0.35+0.65*(0.5+0.5*np.sin(2*np.pi*0.5*t+0.7))
    frequencies=[55,82.41,110,164.81,220,329.63,440,659.26,880,1318.51,1760,2640,4000,6200,9000,12000]
    pcm=np.zeros_like(t)
    for i,freq in enumerate(frequencies):
        pcm+=np.sin(2*np.pi*freq*t+i*0.37)*(0.048 if i<12 else 0.022)
    pcm*=envelope
    kick_phase=np.mod(t+0.09,0.5)
    pcm+=0.16*np.sin(2*np.pi*62*t)*np.exp(-kick_phase/0.07)
    stereo=np.stack([pcm,pcm*0.85],axis=1)
    with wave.open(str(out/'pulse.wav'),'wb') as wav:
        wav.setnchannels(2);wav.setsampwidth(2);wav.setframerate(rate)
        wav.writeframes((np.clip(stereo,-0.95,0.95)*32767).astype('<i2').tobytes())
    os.environ['MZ_STATE_DIR'] = str(out/'isolated-state')
    os.environ['XDG_STATE_HOME'] = str(out/'isolated-state'/'app-state')
    probe = subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'color=s=256x256:d=0.1',
                            '-frames:v', '1', '-c:v', 'h264_nvenc', '-f', 'null', '-'], capture_output=True)
    encoder = 'nvenc' if probe.returncode == 0 else 'x264'
    print(f'encoder={encoder}', flush=True)
    if not args.picker_only:
        results = {name: check_scene(out, name, encoder, args.negative_control) for name in NEW}
        (out/'measurements.json').write_text(json.dumps(results, indent=2)+'\n')
    app(out, 'vocabulary', '--vocabulary')
    vocabulary=(out/'vocabulary.log').read_text()
    check_picker(out, json.loads(vocabulary[vocabulary.index('{'):]))
    print('PASS: grouped picker and native scene gallery', flush=True)


if __name__ == '__main__':
    main()
