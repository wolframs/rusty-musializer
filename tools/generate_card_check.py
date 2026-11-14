#!/usr/bin/env python3
"""The section card's Generate, pressed headless (VG H4). Run by
tools/generate_check.sh after its command-line half, against the same fake
provider, on a copy of the project that half saved (`made.musi`: Spectrum,
a generated clip in section 2, Pulse Field).

    tools/generate_card_check.py OUT

Every press aims at a rect from the `look cards:` report line, and every
control has a press into the gap beside it that must change nothing. What it
asserts:

  1. the clip row's Generate... opens the block; a press beside it does not;
  2. a model, a length and a frame toggle each take their press, a press
     beside each changes nothing;
  3. the button carries the price ($0.12) and its first press arms it
     (Send, Back); a press beside it arms nothing;
  4. Send generates: the fake gets one submit, the clip lands in section 2
     and the autosaved project carries it with the card's prompt;
     a project that holds the clip for the request, opened with an empty
     store, places it and sends nothing (H4b);
  5. Cancel stops waiting: the card and the notice say it may still be billed,
     nothing is placed; Generate again picks the same job up (no new submit);
     a Send whose answer is lost says it may have been billed, the next Send
     sends nothing and warns, and a second press after the warning sends once
     more (H4b);
  6. with no key stored, the button is drawn disabled and the card says where
     the key is set; following a treatment, Generate... is disabled;
  7. the Scenes flyout's Generate... opens Look on the section with the block;
  8. captures: dark and light at 1280x1440, and the 300 px column.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

out = Path(sys.argv[1]).resolve()
env = dict(os.environ)
failures: list[str] = []
SECTION = 'size=1280x1440,column=look,time=5,play=0,section=2'
PROMPT = 'a+slow+push+through+fog+toward+a+lit+door'


def check(ok: bool, what: str) -> None:
    print(('ok:   ' if ok else 'FAIL: ') + what)
    if not ok:
        failures.append(what)


def count(name: str) -> int:
    return json.loads((out / 'fake' / 'counts.json').read_text())[name]


def run(name: str, probe: str, frames: int = 90, theme: str = 'dark', key: bool = True,
        follow: Path | None = None, project: str = 'made', source: Path | None = None,
        store: Path | None = None) -> tuple[str, dict]:
    work = out / f'card-{name}'
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    source = source or out
    shutil.copy(source / f'{project}.musi', work / f'{project}.musi')
    shutil.copytree(source / f'{project}.assets', work / f'{project}.assets')
    state = work / 'state'
    if key:
        shutil.copytree(out / 'state' / 'config', state / 'config')
    else:
        state.mkdir()
    source = ['--project', str(work / f'{project}.musi')]
    if follow is not None:
        source += ['--treatment', str(follow)]
    command = ['tools/headless_run.sh', *source, '--ui-scale', '100', '--hud=0', '--ui-theme', theme,
               '--ui-probe', probe, '--probe-frames', str(frames), '--probe-shot', str(work / 'shot.png')]
    extra = {'MUSIALIZER_GENERATED_DIR': str(store)} if store is not None else {}
    result = subprocess.run(command, env=dict(env, MZ_STATE_DIR=str(state), **extra), stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True)
    (work / 'log.txt').write_text(result.stdout)
    if result.returncode:
        raise SystemExit(f'{name}: exit {result.returncode}: {result.stdout[-1500:]}')
    return result.stdout, json.loads((work / f'{project}.musi').read_text())


def line(log: str, label: str) -> str:
    found = re.findall(r'^' + re.escape(label) + r':\s*(.*)$', log, re.M)
    if not found:
        raise SystemExit(f'missing {label}:')
    return found[-1]


def cards(log: str) -> str:
    return line(log, 'look cards')


def rect(report: str, name: str) -> tuple[int, int, int, int]:
    match = re.search(r'(?:^|[ ;=])' + re.escape(name) + r'@(?:disabled:)?(-?\d+),(-?\d+),(\d+),(\d+)', report)
    if not match:
        raise SystemExit(f'{name} missing: {report[:400]}')
    return tuple(map(int, match.groups()))


def disabled(report: str, name: str) -> bool:
    return re.search(r'(?:^|[ ;=])' + re.escape(name) + r'@disabled:', report) is not None


def centre(r) -> tuple[int, int]:
    return (r[0] + r[2] // 2, r[1] + r[3] // 2)


def field(report: str, key: str) -> str | None:
    match = re.search(r'(?:^| )' + re.escape(key) + r'=(\S+)', report)
    return match[1] if match else None


def click(point) -> str:
    return f',click={point[0]}x{point[1]}'


OPEN = f'{SECTION},generate=open,generate-prompt={PROMPT}'

# 1. Generate... opens the block; beside it, nothing.
base, _ = run('closed', SECTION)
generate = rect(cards(base), 'clip-generate')
check(field(cards(base), 'generate') == 'closed' and not disabled(cards(base), 'clip-generate'),
      'the clip row offers Generate..., the block closed')
log, _ = run('open-press', SECTION + click(centre(generate)))
check(field(cards(log), 'generate') == 'open', 'a press on Generate... opens the block')
log, _ = run('open-gap', SECTION + click((generate[0] - 3, generate[1] + generate[3] // 2)))
check(field(cards(log), 'generate') == 'closed', 'a press beside Generate... opens nothing')

# 2. Model, length, a frame: each press takes, each press beside does not.
opened, _ = run('open', OPEN)
report = cards(opened)
check(field(report, 'generate-model') == 'google/veo-3.1-lite' and field(report, 'generate-seconds') == '4'
      and field(report, 'generate-first') == 'true' and field(report, 'generate-last') == 'true',
      'the block opens on Veo 3.1 Lite, 4 s, both frames')
wan = rect(report, 'generate-model-2')
log, _ = run('model', OPEN + click(centre(wan)))
check(field(cards(log), 'generate-model') == 'alibaba/wan-3.0', 'a press on Wan 3.0 chooses it')
veo = rect(report, 'generate-model-0')
log, _ = run('model-gap', OPEN + click((veo[0] + 20, veo[1] + veo[3] + 2)))
check(field(cards(log), 'generate-model') == 'google/veo-3.1-lite', 'a press between two models chooses neither')
eight = rect(report, 'generate-length-8')
log, _ = run('length', OPEN + click(centre(eight)))
check(field(cards(log), 'generate-seconds') == '8', 'a press on 8 s takes it')
six = rect(report, 'generate-length-6')
log, _ = run('length-gap', OPEN + click((six[0] + six[2] + 2, six[1] + six[3] // 2)))
check(field(cards(log), 'generate-seconds') == '4', 'a press between two lengths takes neither')
first = rect(report, 'generate-first')
log, _ = run('first', OPEN + click(centre(first)))
check(field(cards(log), 'generate-first') == 'false' and field(cards(log), 'generate-last') == 'true',
      'a press on the first frame turns it off, the last stays')
log, _ = run('first-gap', OPEN + click((first[0] + 30, first[1] - 9)))
check(field(cards(log), 'generate-first') == 'true', 'a press above the first frame changes nothing')

# 3. The price on the button; the first press arms it.
action = rect(report, 'generate-action')
check(field(report, 'generate-price') == '$0.12' and not disabled(report, 'generate-action'),
      'the button carries the price: $0.12 (4 s of Veo 3.1 Lite at 720p)')
log, _ = run('arm', OPEN + click(centre(action)))
check(field(cards(log), 'generate-armed') == 'true' and 'generate-send@' in cards(log)
      and 'generate-back@' in cards(log), 'the first press arms it: Send and Back')
log, _ = run('arm-gap', OPEN + click((action[0] + 10, action[1] + action[3] + 3)))
check(field(cards(log), 'generate-armed') == 'false', 'a press below the button arms nothing')
armed, _ = run('armed', f'{SECTION},generate=armed,generate-prompt={PROMPT}')
back = rect(cards(armed), 'generate-back')
log, _ = run('back', f'{SECTION},generate=armed,generate-prompt={PROMPT}' + click(centre(back)))
check(field(cards(log), 'generate-armed') == 'false', 'Back disarms it')

# 4. Send generates and places, and the project keeps it.
submits = count('submit')
log, saved = run('send', f'{SECTION},generate=send,generate-prompt={PROMPT}', frames=900)
clips = saved.get('clips', [])
placed = saved['scene_switches']['cues'][1].get('clip', {}).get('clip')
record = next((clip.get('generated') for clip in clips if clip['sha256'] == placed), None)
check(count('submit') == submits + 1, 'Send made one submit')
check(record is not None and record['prompt'] == PROMPT.replace('+', ' ') and record['model'] == 'google/veo-3.1-lite',
      'the clip plays in section 2 and the autosaved project carries its provenance')
check('last=2:Info:' in line(log, 'generate') and 'Estimated $0.12, OpenRouter reported $0.1200' in line(log, 'generate'),
      'the card says it was generated, with the estimate and the reported cost')

# 4b. A project that already holds the clip for the request (H4b), opened
# with an empty store (another machine): placed, and nothing is sent.
TEXT = 'a+text+only+shot+of+a+harbour'
submits = count('submit')
log, saved = run('text', f'{SECTION},generate=send,generate-frames=none,generate-prompt={TEXT}', frames=900)
text_clip = saved['scene_switches']['cues'][1].get('clip', {}).get('clip')
check(count('submit') == submits + 1 and text_clip is not None and text_clip != placed,
      'a text-only Send generates and places its clip')
bundled = out / 'card-text'
project = json.loads((bundled / 'made.musi').read_text())
project['scene_switches']['cues'][1]['clip'] = json.loads((out / 'made.musi').read_text())['scene_switches']['cues'][1]['clip']
(bundled / 'made.musi').write_text(json.dumps(project))
empty = out / 'card-empty-store'
shutil.rmtree(empty, ignore_errors=True)
empty.mkdir()
# Another machine that has read today's prices and made no clips.
shutil.copy(out / 'store' / 'prices.json', empty / 'prices.json')
log, saved = run('bundled', f'{SECTION},generate=send,generate-frames=none,generate-prompt={TEXT}', frames=300,
                 source=bundled, store=empty)
words = line(log, 'generate')
check(count('submit') == submits + 1 and saved['scene_switches']['cues'][1].get('clip', {}).get('clip') == text_clip
      and 'The project already holds the clip made for exactly this request, so nothing was sent.' in words,
      'with no store, the same Send finds the clip the project holds, places it in section 2 and sends nothing')

# 5. Cancel: may still be billed, nothing placed; the next press picks it up.
SLOW = 'a+field+at+night+[slow]'
submits = count('submit')
running, _ = run('running', f'{SECTION},generate=send,generate-prompt={SLOW}', frames=240)
check(re.search(r'running=2:waiting', line(running, 'generate')) is not None and count('submit') == submits + 1,
      'a slow job is submitted once and shows as waiting')
cancel = rect(cards(running), 'generate-cancel')
log, saved = run('cancel', f'{SECTION},generate=send,generate-prompt={SLOW},click-after=200' + click(centre(cancel)),
                 frames=420)
words = line(log, 'generate')
check('last=2:Warning:' in words and 'may still be billed (about $0.12)' in words,
      'Cancel stops waiting and says it may still be billed')
check(saved['scene_switches']['cues'][1]['clip'] == json.loads((out / 'made.musi').read_text())['scene_switches']['cues'][1]['clip'],
      'nothing was placed after Cancel')
check(count('submit') == submits + 1,
      'the application closed with that job running, and the next Send polled it rather than paying again')
(out / 'fake' / 'release').touch()
log, saved = run('pick-up', f'{SECTION},generate=send,generate-prompt={SLOW}', frames=900)
check(count('submit') == submits + 1 and 'nothing new was paid for' in line(log, 'generate'),
      'Generate again picks the job up: no new submit')
(out / 'fake' / 'release').unlink()

# 5b. A Send whose answer never arrives (H4b): "may have been billed", never
# "nothing"; the next Send sends nothing and warns; a second press after the
# warning sends it again, once.
LOST = 'a+lost+answer+in+the+window+[lost-answer-once]'
submits = count('submit')
log, _ = run('lost', f'{SECTION},generate=send,generate-prompt={LOST}', frames=420)
words = line(log, 'generate')
check(count('submit') == submits + 1 and 'last=2:Error:' in words
      and 'its answer never arrived, so it may have been billed (about $0.12)' in words
      and 'Nothing was sent' not in words and field(cards(log), 'generate-unconfirmed') == 'yes',
      'a Send accepted and never answered: one submit, the card says it may have been billed, not "nothing"')
log, _ = run('lost-held', f'{SECTION},generate=send,generate-prompt={LOST}', frames=300)
words = line(log, 'generate')
check(count('submit') == submits + 1 and 'last=2:Warning:' in words
      and 'may have accepted and billed it (about $0.12)' in words and 'Press Generate again' in words
      and field(cards(log), 'generate-unconfirmed') == 'yes',
      'the next Send, in a new window, sends nothing and says the first may have been billed')
log, saved = run('lost-resend', f'{SECTION},generate=send-twice,generate-prompt={LOST}', frames=900)
words = line(log, 'generate')
placed = saved['scene_switches']['cues'][1].get('clip', {}).get('clip')
record = next((clip.get('generated') for clip in saved.get('clips', []) if clip['sha256'] == placed), None)
check(count('submit') == submits + 2 and record is not None and 'lost answer in the window' in record['prompt']
      and 'Estimated $0.12' in words and field(cards(log), 'generate-unconfirmed') == 'no',
      'Send pressed again after the warning sends it once more and places the clip')

# 6. No key; a followed treatment.
log, _ = run('no-key', OPEN, key=False)
check(disabled(cards(log), 'generate-action') and field(cards(log), 'generate-key') == 'none'
      and 'key=none' in line(log, 'generate'), 'with no key the button is disabled and the card says so')
follow_file = out / 'card-follow.treatment.json'
plain_clip = next((out / 'made.assets' / 'clips').glob('*.mp4'))
shutil.copy(plain_clip, out / 'card-follow.mp4')
follow_file.write_text(json.dumps({
    'schema': 'musializer.treatment/v1', 'audio': {'path': 'fixture.wav'},
    'output': {'resolution': '640x360', 'fps': 24},
    'segments': [{'at': 0, 'scene': 'spectrum'}, {'at': 4, 'scene': 'clip', 'clip': 'card-follow.mp4'},
                 {'at': 8, 'scene': 'pulse'}]}))
log, _ = run('follow', SECTION + ',follow=1', follow=follow_file)
check(disabled(cards(log), 'clip-generate'), 'following a treatment, Generate... is disabled')

# 7. The flyout's Generate... opens Look on the section, with the block.
flyout, _ = run('flyout', 'size=1280x1440,column=direct,time=5,play=0,flyout=scenes')
picker = line(flyout, 'scene picker')
match = re.search(r'generate@(-?\d+),(-?\d+),(\d+),(\d+)', picker)
if match is None:
    check(False, f'the flyout reports its Generate: {picker[:300]}')
else:
    seat = tuple(map(int, match.groups()))
    log, _ = run('flyout-press', 'size=1280x1440,column=direct,time=5,play=0,flyout=scenes' + click(centre(seat)))
    check(field(cards(log), 'generate') == 'open' and field(cards(log), 'target') == '2',
          'the flyout Generate... opens Look on section 2 with the block')

# 8. Captures: both appearances, and the narrowest column.
for name, probe, theme in [('capture-dark', OPEN, 'dark'), ('capture-light', OPEN, 'light'),
                           ('capture-narrow', OPEN + ',column-width=300', 'dark')]:
    log, _ = run(name, probe, theme=theme)
    check(field(cards(log), 'generate') == 'open' and (out / f'card-{name}' / 'shot.png').exists(),
          f'{name}: captured (look at {out}/card-{name}/shot.png)')

print()
if failures:
    print(f'generate_card_check: {len(failures)} failed', file=sys.stderr)
    sys.exit(1)
print('generate_card_check: all checks passed')
