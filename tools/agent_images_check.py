#!/usr/bin/env python3
"""Pictures in the agent chat, end to end.

Every picture is a synthetic file written here with PIL. A mention comes from
the fake agent's own message, which is the path a real one takes;
`agent-image=` adds the bare entry, as a saved chat holds it, which is also the
only way a missing file reaches the screen. What is pinned is what the capture
shows: the colours inside the reported rect, not the report's word for them.
Colour classifiers allow +-2, because a texture read through a filter is a
blend (docs/HEADLESS_GPU.md), and stay away from quadrant edges and the
rounded corners, which are hairlines.
"""
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image

out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
root = Path.cwd()
failures = []


def check(ok, name, detail=''):
    print(('ok: ' if ok else 'FAIL: ') + name + ('' if ok else f'\n    {detail[:900]}'), flush=True)
    if not ok:
        failures.append(name)


fake = out / 'fake-agent'
fake.write_text(f'#!/bin/sh\nexec /usr/bin/python3 "{root}/tools/fake_agent.py" "$@"\n')
fake.chmod(0o755)
wave = out / 'fixture.wav'
subprocess.run(['cargo', 'run', '--quiet', '--bin', 'make-fixture-wav', '--', str(wave), '4'], check=True)
choices = out / 'agents.json'
# Preferences outlive a run, and a run that leaves Direct would start the next
# one on another view, with the chat off screen.
for leftover in ('state', 'state-look'):
    shutil.rmtree(out / leftover, ignore_errors=True)
env = dict(os.environ, MZ_BIN=str(Path(os.environ.get('MZ_BIN', 'target/debug/musializer')).resolve()),
           MZ_STATE_DIR=str(out / 'state'), SHELL='/bin/sh', MUSIALIZER_AGENT_CHOICES=str(choices),
           MUSIALIZER_AGENT_CLAUDE=str(fake), MUSIALIZER_AGENT_CODEX=str(fake), MUSIALIZER_AGENT_AGY=str(fake))

# Four opaque fills, one of them the mid grey a wrong sRGB step would move most.
RED, GREEN, BLUE, GREY = (214, 40, 32), (32, 196, 72), (40, 72, 224), (128, 128, 128)


def quadrants(size=(640, 360), colours=(RED, GREEN, BLUE, GREY)):
    w, h = size
    pixels = np.zeros((h, w, 3), dtype=np.uint8)
    pixels[:h // 2, :w // 2] = colours[0]
    pixels[:h // 2, w // 2:] = colours[1]
    pixels[h // 2:, :w // 2] = colours[2]
    pixels[h // 2:, w // 2:] = colours[3]
    return Image.fromarray(pixels, 'RGB')


def write_pictures():
    quadrants().save(out / 'sheet.png')
    quadrants().save(out / 'sheet.png-old.png')
    quadrants().save(out / 'photo.jpg', quality=98, subsampling=0)
    quadrants().save(out / 'mislabelled.png', format='JPEG', quality=98, subsampling=0)
    quadrants().save(out / 'still.webp', lossless=True)
    quadrants((360, 640)).save(out / 'portrait.png')
    quadrants((64, 64)).save(out / 'tiny.png')
    grey16 = np.full((360, 640), 0x8080, dtype=np.uint16)
    Image.fromarray(grey16).save(out / 'grey16.png')
    # Left half fully transparent over a colour that must not show; right half
    # half-covered red, to be composited once, on the block's own fill.
    pixels = np.zeros((360, 640, 4), dtype=np.uint8)
    pixels[:, :320] = (255, 255, 255, 0)
    pixels[:, 320:] = (220, 30, 30, 128)
    Image.fromarray(pixels, 'RGBA').save(out / 'alpha.png')
    (out / 'not-image.png').write_text('this is a text file with a picture\'s name\n')
    for n in range(9):
        quadrants().save(out / f's{n}.png')
    for n in range(7):
        quadrants((640, 480)).save(out / f'f{n}.png')


write_pictures()


def run(name, agent='claude', presses=(), say=None, frames=None, size='1920x1080', extra=(), theme=None,
        press_at=None, state=None):
    spec = [f'agent={agent}', f'size={size}', 'play=0']
    if presses:
        spec.append('agent-press=' + '+'.join(presses))
    if press_at is not None:
        spec.append(f'agent-press-at={press_at}')
    if say:
        spec.append(f'agent-say={say}')
    spec.extend(extra)
    first = 40 if press_at is None else press_at
    frames = frames or first + 12 * len(presses) + 40
    png = out / f'{name}.png'
    command = ['tools/headless_run.sh', str(wave), '--ui-scale', '100', '--hud=0']
    if theme:
        command += ['--ui-theme', theme]
    command += ['--ui-probe', ','.join(spec), '--probe-frames', str(frames), '--probe-shot', str(png)]
    launch = dict(env, MZ_STATE_DIR=str(out / state)) if state else env
    done = subprocess.run(command, env=launch, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    (out / f'{name}.log').write_text(done.stdout)
    dock = next((l for l in reversed(done.stdout.splitlines()) if l.startswith('dock:')), '')
    if not dock:
        check(False, f'{name}: a dock line', done.stdout[-900:])
    return dock, done.stdout


def rects(dock, kind):
    return [tuple(int(v) for v in r) for r in
            re.findall(rf'(?<![a-z-]){kind}@(-?\d+),(-?\d+),(\d+),(\d+)', dock)]


def images(dock):
    found = re.search(r'images=\[([^\]]*)\]', dock)
    return found.group(1).split(';') if found and found.group(1) else []


def entries(dock):
    found = re.search(r'image-entries=(\d+)', dock)
    return int(found.group(1)) if found else -1


def large(dock):
    found = re.search(r' large=(\S+)\]?', dock)
    return found.group(1).rstrip(']') if found else ''


def pixels(name):
    return np.asarray(Image.open(out / f'{name}.png').convert('RGB'), dtype=int)


def patch(shot, rect, u0, v0, u1, v1):
    """A part of `rect`, in fractions of it."""
    x, y, w, h = rect
    return shot[y + int(h * v0):y + int(h * v1), x + int(w * u0):x + int(w * u1)]


def filled(area, colour, tolerance=2):
    """Whether a patch is that colour, under renderer rounding."""
    return bool(area.size and (np.abs(area - np.array(colour)).max(axis=2) <= tolerance).mean() >= 0.995)


def worst(area, colour):
    return f'median {np.median(area.reshape(-1, 3), axis=0)} wanted {colour}'


def shows_quadrants(name, rect, label, tolerance=2):
    shot = pixels(name)
    for colour, spot, word in ((RED, (0.08, 0.1, 0.42, 0.4), 'red'), (GREEN, (0.58, 0.1, 0.92, 0.4), 'green'),
                               (BLUE, (0.08, 0.6, 0.42, 0.9), 'blue'), (GREY, (0.58, 0.6, 0.92, 0.9), 'grey')):
        area = patch(shot, rect, *spot)
        check(filled(area, colour, tolerance), f'{label}: its {word} is the file\'s', worst(area, colour))


# --- A mention: the moment this exists for ---
for theme in ('dark', 'light'):
    dock, _ = run(f'picture-{theme}', say='picture', frames=90, theme=theme)
    check(entries(dock) == 1, f'{theme}: the sheet the agent named is one entry', dock)
    check(any(i.startswith('sheet.png:shown:640x360:') for i in images(dock)),
          f'{theme}: the picture itself is drawn, not its placeholder', dock)
    thumb = rects(dock, 'image')[0]
    shows_quadrants(f'picture-{theme}', thumb, f'{theme}: the thumbnail')
    column = rects(dock, 'composer')[0]
    check(thumb[0] >= column[0] and thumb[0] + thumb[2] <= column[0] + column[2],
          f'{theme}: it fits the column', dock)
    check(abs(thumb[3] - thumb[2] * 9 / 16) <= 1, f'{theme}: it keeps its shape', str(thumb))
    caption = rects(dock, 'image-caption')[0]
    check(caption[1] >= thumb[1] + thumb[3] and caption[3] > 8, f'{theme}: its caption sits under it', dock)
    shot = pixels(f'picture-{theme}')
    area = patch(shot, caption, 0, 0, 1, 1)
    check(np.abs(area - area[0, 0]).max() > 60, f'{theme}: the caption is words on the frame', str(caption))

for agent in ('codex', 'agy'):
    dock, _ = run(f'picture-{agent}', agent, say='picture', frames=110)
    check(any(i.startswith('sheet.png:shown:') for i in images(dock)), f'{agent}: a mention shows the same way', dock)

dock, _ = run('codex-viewed', 'codex', say='viewed', frames=110)
check(any(i.startswith('sheet.png:shown:') for i in images(dock)), 'Codex looking at a picture shows it', dock)

# --- From the entry alone, as a saved chat reopens ---
dock, _ = run('reopened', extra=['agent-image=sheet.png'], frames=70)
check(any(i.startswith('sheet.png:shown:') for i in images(dock)), 'the bare entry and the chat\'s folder are enough', dock)
shows_quadrants('reopened', rects(dock, 'image')[0], 'the reopened picture')
dock, _ = run('absolute', extra=[f'agent-image={out}/sheet.png'], frames=70)
check(any(i.startswith('sheet.png:shown:') for i in images(dock)), 'an absolute path draws too', dock)

# --- What is not a picture ---
dock, _ = run('unwritten', say='unwritten', frames=90)
check(entries(dock) == 0 and not rects(dock, 'image') and 'later.png' in dock,
      'a name with no file behind it adds nothing', dock)
dock, log = run('missing', extra=['agent-image=gone.png'], frames=70)
check(images(dock) == ['gone.png:missing'], 'a missing file says so', dock)
area = patch(pixels('missing'), rects(dock, 'image')[0], 0, 0, 1, 1)
check(np.abs(area - area[1, 12]).max() > 90, 'its words are on the frame, not a blank', str(rects(dock, 'image')))
dock, _ = run('broken', say='broken', frames=90)
check(images(dock) == ['not-image.png:undecodable'], 'a text file called .png fails by name', dock)
area = patch(pixels('broken'), rects(dock, 'image')[0], 0, 0, 1, 1)
check(np.abs(area - area[1, 12]).max() > 90, 'its failure is on the frame', dock)

# --- Once per file, again when it changes ---
dock, _ = run('repeat', say='repeat', frames=100)
check(entries(dock) == 1, 'a tool call, its output and the reply naming one file show it once', dock)
os.utime(out / 'sheet.png', (0, 1_700_000_000))
dock, _ = run('rewrite', say='rewrite', frames=150)
check(entries(dock) == 2 and images(dock).count(images(dock)[0]) == 2 and images(dock)[0].startswith('sheet.png:shown'),
      'a file rewritten since it showed shows again', dock)
write_pictures()
dock, _ = run('half', say='half', frames=130)
check(images(dock) and images(dock)[0].startswith('sheet.png-old.png:shown') and entries(dock) == 1,
      'a path cut by the stream is not read until the stream ends', dock)
dock, _ = run('listing', say='listing', frames=100)
check(entries(dock) == 6 and 'Showing 6 of 9 pictures named here' in dock, 'a listing does not fill the chat', dock)

# --- While it decodes ---
dock, _ = run('held', extra=['agent-image=sheet.png', 'agent-image-hold=500'], frames=60)
check(images(dock) == ['sheet.png:decoding'], 'a held decode reports its placeholder', dock)
held_rect = rects(dock, 'image')[0]
area = patch(pixels('held'), held_rect, 0.05, 0.1, 0.95, 0.35)
check(not filled(area, RED) and not filled(area, GREEN), 'the placeholder is not the picture', str(held_rect))
dock, _ = run('released', extra=['agent-image=sheet.png', 'agent-image-hold=45'], frames=75)
check(any(i.startswith('sheet.png:shown') for i in images(dock)), 'released, the picture arrives', dock)
check(rects(dock, 'image')[0] == held_rect, 'a 16:9 picture lands exactly in its placeholder',
      f'{held_rect} {rects(dock, "image")[0]}')
many = '+'.join(f'f{n}.png' for n in range(7))
dock, _ = run('stack-held', extra=[f'agent-image={many}', 'agent-image-hold=500'], frames=60)
held = rects(dock, 'image')
composer = rects(dock, 'composer')[0]
check(held[-1][1] + held[-1][3] <= composer[1] and held[-1][1] > composer[1] - 400,
      'the newest placeholder sits at the bottom of the chat', dock)
check(any(i.endswith(':off-screen') for i in images(dock)), 'pictures scrolled away are not decoded', dock)
dock, _ = run('stack-released', extra=[f'agent-image={many}', 'agent-image-hold=45'], frames=75)
done = rects(dock, 'image')
check(done[-1][1] + done[-1][3] <= composer[1] and done[-1][1] + done[-1][3] >= held[-1][1] + held[-1][3] - 2,
      'pictures of another shape growing from it do not move the bottom', f'{held[-1]} {done[-1]}')
check(images(dock)[-1].startswith('f6.png:shown'), 'the newest is the one drawn', dock)

# --- Formats, shapes and alpha ---
for name, colours in (('photo.jpg', 8), ('mislabelled.png', 8), ('still.webp', 2)):
    label = name.replace('.', '-')
    dock, _ = run(f'format-{label}', extra=[f'agent-image={name}'], frames=75)
    check(any(i.startswith(f'{name}:shown') for i in images(dock)), f'{name} decodes', dock)
    if rects(dock, 'image'):
        shows_quadrants(f'format-{label}', rects(dock, 'image')[0], name, colours)
dock, _ = run('grey16', extra=['agent-image=grey16.png'], frames=70)
check(filled(patch(pixels('grey16'), rects(dock, 'image')[0], 0.1, 0.1, 0.9, 0.9), GREY),
      'a 16-bit PNG comes down to its own grey', dock)
dock, _ = run('portrait', extra=['agent-image=portrait.png'], frames=70)
thumb = rects(dock, 'image')[0]
check(thumb[3] == 360 and abs(thumb[2] - 203) <= 1, 'a portrait stops at the tallest thumbnail, in shape', str(thumb))
shows_quadrants('portrait', thumb, 'the portrait')
dock, _ = run('tiny', extra=['agent-image=tiny.png'], frames=70)
check(rects(dock, 'image')[0][2:] == (64, 64), 'a small picture is not blown up', dock)

for theme in ('dark', 'light'):
    dock, _ = run(f'alpha-{theme}', extra=['agent-image=alpha.png'], frames=70, theme=theme)
    thumb = rects(dock, 'image')[0]
    shot = pixels(f'alpha-{theme}')
    fill = np.median(patch(shot, thumb, 0.05, 0.1, 0.45, 0.9).reshape(-1, 3), axis=0)
    check(filled(patch(shot, thumb, 0.05, 0.1, 0.45, 0.9), fill) and fill[0] < 235 and abs(fill[0] - fill[1]) < 12,
          f'{theme}: a transparent pixel shows the block, not its hidden colour', f'{fill}')
    wanted = np.array((220, 30, 30)) * (128 / 255) + fill * (1 - 128 / 255)
    area = patch(shot, thumb, 0.55, 0.1, 0.95, 0.9)
    check(filled(area, wanted, 3), f'{theme}: half cover composites once over the block', worst(area, wanted))

# --- A narrow window ---
dock, _ = run('narrow', extra=['agent-image=sheet.png'], frames=70, size='1100x720')
thumb = rects(dock, 'image')[0]
check(thumb[0] >= 0 and thumb[0] + thumb[2] <= 1100 and thumb[2] <= 332, 'the narrowest column holds it', str(thumb))
shows_quadrants('narrow', thumb, 'at the narrowest column')

# --- Looking closer ---
for theme in ('dark', 'light'):
    dock, _ = run(f'viewer-{theme}', presses=['image'], extra=['agent-image=sheet.png'], theme=theme)
    check(large(dock) == 'sheet.png:full:640x360', f'{theme}: a press on the thumbnail opens it', dock)
    view = rects(dock, 'viewer')[0]
    check(view[2] >= 1760 and abs(view[3] - view[2] * 9 / 16) <= 2, f'{theme}: large, fit to the window', str(view))
    check(view[0] >= 16 and view[0] + view[2] <= 1904 and view[1] >= 16 and view[1] + view[3] <= 1064
          and abs((view[0] + view[2] / 2) - 960) <= 2, f'{theme}: inside the window, centred', str(view))
    shows_quadrants(f'viewer-{theme}', view, f'{theme}: the large picture')
    check('keyboard=true' in dock, f'{theme}: it holds the keyboard', dock)
    corner = patch(pixels(f'viewer-{theme}'), rects(dock, 'viewer-outside')[0], 0, 0, 1, 1)
    bare, _ = run(f'viewer-closed-{theme}', extra=['agent-image=sheet.png'], frames=80, theme=theme)
    base = np.median(patch(pixels(f'viewer-closed-{theme}'), (0, 0, 8, 8), 0, 0, 1, 1).reshape(-1, 3), axis=0)
    scrim = np.array((0x0A, 0x0B, 0x0E)) * (0xBE / 255) + base * (1 - 0xBE / 255)
    check(filled(corner, scrim, 3), f'{theme}: the window goes behind the scrim', worst(corner, scrim))

dock, _ = run('viewer-stays', presses=['image', 'viewer'], extra=['agent-image=sheet.png'])
check(large(dock).startswith('sheet.png:'), 'a press on the picture leaves it open', dock)
dock, log = run('viewer-under', presses=['image', 'chip-model'], extra=['agent-image=sheet.png'])
check('row:model' not in dock, 'a press through the picture reaches nothing under it', dock)
dock, _ = run('viewer-outside', presses=['image', 'viewer-outside'], extra=['agent-image=sheet.png'])
check(large(dock) == 'closed', 'a press outside closes it', dock)
check('viewer@' not in dock, 'closed, it is off the report too', dock)
check('keyboard=true' in dock, 'the composer has the keyboard back', dock)
dock, _ = run('viewer-idle', presses=['image-caption', 'image', 'viewer-outside'], extra=['agent-image=sheet.png'])
check(large(dock) == 'closed' and 'keyboard=false' in dock,
      'a composer nobody was typing in does not take the keyboard on close', dock)
line = lambda log, name: next((l for l in log.splitlines() if l.startswith(name)), '')
dock, log = run('viewer-escape', presses=['image', 'key:escape'], extra=['agent-image=sheet.png'])
check(large(dock) == 'closed', 'Escape closes it, through the keyboard the dock holds', dock)
check('gesture=none' in line(log, 'timeline:'), 'nothing kept a pointer claim', line(log, 'timeline:'))
dock, _ = run('viewer-reopen', presses=['image', 'key:escape', 'image'], extra=['agent-image=sheet.png'])
check(large(dock).startswith('sheet.png:'), 'it opens again after closing', dock)

# --- The window's own widgets under it ---
dock, log = run('transport', extra=['agent-image=sheet.png'], frames=70)
x, y, w, h = map(int, re.search(r'play=(\d+),(\d+),(\d+),(\d+)', line(log, 'transport:')).groups())
press = [f'click={x + w // 2}x{y + h // 2}', 'click-after=62']
dock, log = run('play-bare', extra=['agent-image=sheet.png', *press], frames=90)
check('claimed=nothing' not in line(log, 'click probe:') and 'claimed=' in line(log, 'click probe:'),
      'with the picture closed, play takes a press', line(log, 'click probe:'))
dock, log = run('play-covered', presses=['image'], extra=['agent-image=sheet.png', *press], frames=90)
check(large(dock).startswith('sheet.png:') and 'claimed=nothing' in line(log, 'click probe:'),
      'with it open, play under it does not', line(log, 'click probe:'))
check('gesture=none' in line(log, 'timeline:'), 'and nothing is left holding the pointer', line(log, 'timeline:'))
lane = ['click=700x960', 'click-after=62']
dock, log = run('lane-covered', presses=['image'], extra=['agent-image=sheet.png', *lane], frames=90)
check('claimed=nothing' in line(log, 'click probe:') and 'gesture=none' in line(log, 'timeline:'),
      'nor does the timeline under it', line(log, 'click probe:') + line(log, 'timeline:'))
dock, log = run('column-left', presses=['image'], press_at=30, state='state-look',
                extra=['agent-image=sheet.png', 'column-switch=look', *press], frames=90)
check(large(dock) == 'closed' and 'claimed=nothing' not in line(log, 'click probe:'),
      'when Direct leaves the column the picture goes with it, and the window has its pointer back',
      dock + line(log, 'click probe:'))

print('agent images:', 'PASS' if not failures else f'FAIL {len(failures)}: {failures}')
sys.exit(1 if failures else 0)
