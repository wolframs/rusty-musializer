#!/usr/bin/env python3
"""Press the real About controls; every target comes from about: in app reports.
ABOUT_CHECK_NEGATIVE_ONLY=1 runs only the licence-opening assertion, for the
single GPU/CPU negative-control pair. It never changes the application state.
"""
import os
from pathlib import Path
import re
import subprocess
import sys
from PIL import Image

out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
fixture = Path('build/redesign/fixture/Door, No Room.musi').resolve()
if not fixture.exists():
    raise SystemExit('Copy the project fixture as instructed in the task before running About checks.')
env = dict(os.environ, MZ_BIN=os.environ.get('ABOUT_BIN', str(Path('target/debug/musializer').resolve())))
expected_rows = len(re.findall(r'^    Credit \{', Path('crates/musializer-core/src/credits.rs').read_text(), re.M))

def line(log, prefix):
    found = re.search(r'^'+re.escape(prefix)+r':\s*(.*)$', log, re.M)
    assert found, f'missing {prefix}: report'
    return found[1]

def rect(report, key):
    found = re.search(r'\b'+re.escape(key)+r'@(-?\d+),(-?\d+),(\d+),(\d+)', report)
    assert found, f'missing {key} in {report}'
    r = tuple(map(int, found.groups()))
    assert r[2] > 0 and r[3] > 0, (key, r)
    return r

def point(report, key, scale=100):
    x, y, w, h = rect(report, key)
    return f'{(x+w/2)*scale/100:g}x{(y+h/2)*scale/100:g}'

def run(name, about=False, click=None, key=None, size='960x640', theme='dark', scale=100, welcome=False, playing=False, extra=''):
    probe = f'size={size},about={int(about)}'
    if not welcome: probe += f',time=0,play={int(playing)},column=lyrics'
    if click: probe += ',click='+click
    if key: probe += ',key='+key
    if extra: probe += ','+extra
    cmd = ['tools/headless_run.sh']
    if not welcome: cmd += ['--project', str(fixture)]
    cmd += ['--ui-scale', str(scale), '--ui-theme', theme, '--hud=0', '--ui-probe', probe,
            '--probe-frames', '24', '--probe-shot', str(out/(name+'.png'))]
    result = subprocess.run(cmd, env=dict(env, MZ_STATE_DIR=str(out/('state-'+name))),
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    (out/(name+'.log')).write_text(result.stdout)
    assert result.returncode == 0, f'{name}: exit {result.returncode}; see {out/(name+".log")}'
    assert line(result.stdout, 'frames rendered') == '24', f'{name}: app quit before capture'
    report = line(result.stdout, 'about')
    assert f'rows={expected_rows} ' in report, (name, report, expected_rows)
    assert 'scale='+f'{scale/100:.2f}' in line(result.stdout, 'fonts'), f'{name}: requested scale was not drawn'
    return report, result.stdout

def check(condition, message):
    assert condition, message
    print('PASS: '+message, flush=True)

def surface(name, report, scale, theme, size):
    image = Image.open(out/(name+'.png')).convert('RGB')
    check(image.size == tuple(map(int, size.split('x'))), name+': capture dimensions')
    x, y, w, h = rect(report, 'rect')
    check(x >= 0 and y >= 0 and (x+w)*scale/100 <= image.width+1 and (y+h)*scale/100 <= image.height+1,
          name+': modal fits the framebuffer')
    # An opaque interior patch, clear of text, corners, hairlines and shadow.
    # Both rasterizers write the floating surface exactly (HEADLESS_GPU.md).
    gx, gy, gw, gh = rect(report, 'gap')
    px = round((gx+gw/2)*scale/100)
    py = round((gy+gh/2)*scale/100)
    expected = (0x17, 0x1A, 0x1F) if theme == 'dark' else (255, 255, 255)
    patch = [image.getpixel((px+dx, py+dy)) for dx in range(-1, 2) for dy in range(-1, 2)]
    check(all(c == expected for c in patch), name+': visible floating surface uses the appearance role')

def covered_notice_case(opened_log):
    # This empty part of About covers a real close control in the notice tray.
    notice_report = line(opened_log, 'notices')
    covered_point = point(notice_report.replace('close=', 'close@'), 'close')
    covered, covered_log = run('covered-notice', about=True, click=covered_point)
    check('open=true' in covered and line(covered_log, 'notices') == notice_report,
          'a press over the covered notice close leaves the underlying tray unchanged')

if os.environ.get('ABOUT_CHECK_TRAY_ONLY') == '1':
    _, tray_log = run('tray-baseline', about=True)
    covered_notice_case(tray_log)
    raise SystemExit(0)

baseline, base_log = run('baseline')
check('open=false' in baseline, 'About starts closed')
opened, opened_log = run('logo', click=point(baseline, 'logo'))
check('open=true' in opened and 'action=opened' in opened, 'pressing the logo opens About')
licence, _ = run('licence', about=True, click=point(opened, 'licence'))
check('open=true' in licence and 'notice=0 ' in licence and 'action=licence' in licence and 'notice-face=ui missing=0' in licence,
      'pressing the first licence reads the embedded Musializer MIT notice')
if os.environ.get('ABOUT_CHECK_NEGATIVE_ONLY') == '1':
    raise SystemExit(0)

gap, gap_log = run('gap', about=True, click=point(opened, 'gap'))
check('open=true' in gap and 'notice=none' in gap and 'claimed=nothing' in line(gap_log, 'click probe'), 'pressing an interior gap leaves About unchanged')
covered_notice_case(opened_log)
scrim, _ = run('scrim', about=True, click=point(opened, 'scrim'))
check('open=false' in scrim and 'action=scrim' in scrim, 'pressing the scrim closes About')
closed, _ = run('close', about=True, click=point(opened, 'close'))
check('open=false' in closed and 'action=close' in closed, 'pressing × closes About')
escaped, escape_log = run('escape', about=True, key='escape')
check('open=false' in escaped and 'action=escape' in escaped, 'Escape closes About and the app runs all 24 frames')
check(line(escape_log, 'column') == line(base_log, 'column'), 'Escape leaves the Lyrics view unchanged')
check(line(escape_log, 'timeline') == line(base_log, 'timeline'), 'Escape leaves the timeline state unchanged')
welcome, _ = run('welcome', welcome=True)
welcome_open, _ = run('welcome-open', welcome=True, click=point(welcome, 'welcome'))
check('open=true' in welcome_open, 'the quiet welcome About control opens the same surface')
# A live track enters through the logo too; decoded PCM is untouched and --mute
# is injected by headless_run.sh, so no test sound reaches the operator.
live, live_log = run('playing-logo', playing=True, click=point(baseline, 'logo'))
check('open=true' in live and re.search(r'^[1-9]\d* consumed', line(live_log, 'audio frames')),
      'About opens through the logo while music analysis is progressing')
# Page controls are real presses; the report must move and all rows remain
# reachable rather than fitting by shrinking their text.
scrolled, _ = run('scroll', about=True, click=point(opened, 'down'))
match = re.search(r'scroll=(\d+)/(\d+)', scrolled)
check(match and int(match[1]) > 0 and int(match[2]) > int(match[1]), 'the scroll control advances the credits')
for theme in ['dark', 'light']:
    for size in ['960x640', '1280x720']:
        for scale in [100, 150]:
            name = f'{theme}-{size}-{scale}'
            # Existing app policy preserves a 960x640 logical minimum. At
            # 150%, use physical pixels for these same logical window sizes.
            width, height = map(int, size.split('x'))
            physical = f'{width*scale//100}x{height*scale//100}'
            report, _ = run(name, about=True, theme=theme, size=physical, scale=scale)
            check('open=true' in report, name+': About is open')
            surface(name, report, scale, theme, physical)
print('About checks passed.', flush=True)
