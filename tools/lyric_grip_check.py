#!/usr/bin/env python3
"""Press/drag the default lyric grip; distinguish a fixed-band cap from no input."""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

out = Path(sys.argv[1] if len(sys.argv) > 1 else 'build/lyric-grip').resolve()
out.mkdir(parents=True, exist_ok=True)
subprocess.run(['cargo', 'run', '--quiet', '--bin', 'make-fixture-wav', '--',
                str(out / 'fixture.wav'), '8'], check=True, stdout=subprocess.DEVNULL)
env = os.environ | {'MZ_BIN': str(Path(os.environ.get('MZ_BIN', 'target/debug/musializer')).resolve())}
failures = []


def check(ok, message):
    print(('ok: ' if ok else 'FAIL: ') + message, flush=True)
    if not ok:
        failures.append(message)


def run(name, theme, probe, state=None):
    state = state or out / (name + '-state')
    e = env | {'MZ_STATE_DIR': str(state)}
    cmd = ['tools/headless_run.sh', str(out / 'fixture.wav'), '--ui-theme', theme,
           '--probe-frames', '30', '--ui-probe', 'size=1280x720,play=1' + (',' + probe if probe else ''),
           '--probe-shot', str(out / (name + '.png'))]
    with (out / (name + '.log')).open('w') as log:
        subprocess.run(cmd, env=e, stdout=log, stderr=subprocess.STDOUT, check=True)
    log = (out / (name + '.log')).read_text()
    report = re.findall(r'^lyrics: (.*)$', log, re.M)[-1]
    rect = tuple(map(int, re.search(r'lane rect=(\d+),(\d+),(\d+),(\d+)', report).groups()))
    check('resizing=false' in report, name + ' releases resize ownership')
    prefs = state / 'config/musializer/ui.json'
    saved = json.loads(prefs.read_text()) if prefs.exists() else {}
    return rect, saved


for theme in ('dark', 'light'):
    base, _ = run(theme + '-base', theme, '')
    x, y, w, h = base
    check(h == 49, theme + ' default lane is exactly 49 px')
    start = f'{x+w//2}x{y+h-4}'
    for direction, target, expected, requested in [('up', 30, 30, 30), ('down', 66, 49, 66)]:
        name = theme + '-' + direction
        state = out / (name + '-state')
        # Reset only this check's owned config so repetitions start at defaults.
        prefs = state / 'config/musializer/ui.json'
        prefs.unlink(missing_ok=True)
        rect, saved = run(name, theme, f'hover={start},left-drag={x+w//2}x{y+target}', state)
        check(rect[3] == expected, f'{name}: press/drag gives exactly {expected} px')
        check(saved.get('lyric_lane_height') == requested, f'{name}: persists request {requested} px')
        reopened, _ = run(name + '-reopen', theme, '', state)
        check(reopened[3] == expected, f'{name}: reopening preserves drawn {expected} px')
    gap, saved = run(theme + '-gap', theme,
                     f'hover={x+w//2}x{y-10},left-drag={x+w//2}x{y+30}')
    check(gap[3] == 49 and saved.get('lyric_lane_height') is None,
          theme + ' press outside grip does not resize or persist a lane request')
    large, saved = run(theme + '-expanded', theme,
                      'timeline-height=330')
    # Discover the enlarged lane before deriving its press point.
    lx, ly, lw, lh = large
    expanded, saved = run(theme + '-expanded-drag', theme,
                         f'timeline-height=330,hover={lx+lw//2}x{ly+lh-4},left-drag={lx+lw//2}x{ly+66}')
    check(expanded[3] == 66 and saved.get('lyric_lane_height') == 66,
          theme + ' enlarged band seats an exact 66 px drag from its reported grip')

print(f'{len(failures)} lyric grip checks failed; captures in {out}', flush=True)
sys.exit(bool(failures))
