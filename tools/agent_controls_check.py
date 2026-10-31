#!/usr/bin/env python3
"""The composer's model, effort and access controls, end to end.

Every choice below is made by pressing the chip and then the menu row, and is
then read back from where it has to land: the fake's own reply, which says what
its protocol told it (`state?`), never the dock's own idea of the selection.
Claude takes a choice live, Codex with its next turn, Antigravity on a new
process, so each has its own case.
"""
import json
import os
import re
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
env = dict(os.environ, MZ_BIN=str(Path(os.environ.get('MZ_BIN', 'target/debug/musializer')).resolve()),
           MZ_STATE_DIR=str(out / 'state'), SHELL='/bin/sh', MUSIALIZER_AGENT_CHOICES=str(choices),
           MUSIALIZER_AGENT_CLAUDE=str(fake), MUSIALIZER_AGENT_CODEX=str(fake), MUSIALIZER_AGENT_AGY=str(fake))


def run(name, agent, presses=(), say=None, frames=None, size='1920x1080'):
    spec = [f'agent={agent}', f'size={size}', 'play=0']
    if presses:
        spec.append('agent-press=' + '+'.join(presses))
    if say:
        spec.append(f'agent-say={say}')
    frames = frames or 40 + 12 * len(presses) + 30
    png = out / f'{name}.png'
    done = subprocess.run(['tools/headless_run.sh', str(wave), '--ui-scale', '100', '--hud=0', '--ui-probe',
                           ','.join(spec), '--probe-frames', str(frames), '--probe-shot', str(png)],
                          env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    (out / f'{name}.log').write_text(done.stdout)
    dock = next((l for l in reversed(done.stdout.splitlines()) if l.startswith('dock:')), '')
    if not dock:
        check(False, f'{name}: a dock line', done.stdout[-900:])
    return dock, done.stdout


def rects(dock):
    return {name: tuple(map(float, r)) for name, *r in
            re.findall(r'([a-z][a-z:.\-0-9]*)@(-?[\d.]+),(-?[\d.]+),([\d.]+),([\d.]+)', dock)}


def has(dock, *words):
    return all(word in dock for word in words)


# --- Claude: live ---
choices.unlink(missing_ok=True)
dock, _ = run('claude-model-menu', 'claude', ['chip-model'])
check(has(dock, 'listed=5', 'chips="Fake Opus|High|Edits"', 'mode=acceptEdits'),
      'Claude lists its models and reports what it applied before any turn', dock)
found = rects(dock)
check(all(f'row:model:{m}' in found for m in ('default', 'fake-opus', 'fake-haiku')),
      'the chip press opens the model menu', dock)
check('users=0' in dock and 'tools=0' in dock, 'the catalogue is not a turn', dock)
closed, _ = run('claude-closed', 'claude')
check(not any(n.startswith('row:') for n in rects(closed)), 'no press, no menu', closed)
shown = np.asarray(Image.open(out / 'claude-model-menu.png').convert('RGB'), dtype=int)
plain = np.asarray(Image.open(out / 'claude-closed.png').convert('RGB'), dtype=int)
x, y, w, h = map(int, found['row:model:fake-opus'])
check(np.abs(shown[y:y+h, x:x+w] - plain[y:y+h, x:x+w]).sum(axis=2).max() > 60,
      'the menu reaches the frame, not only the report')

dock, _ = run('claude-pick-model', 'claude', ['chip-model', 'row:model:fake-haiku'], 'state?')
check(has(dock, 'pick=model:fake-haiku/', '[fake-haiku/'), 'a model pressed reaches Claude', dock)
check('row:' not in dock, 'taking a row closes the menu', dock)

dock, _ = run('claude-no-levels', 'claude', ['chip-model', 'row:model:fake-haiku', 'chip-effort'])
check('row:effort:' not in dock, 'a model without levels has no effort menu', dock)

choices.unlink(missing_ok=True)
dock, _ = run('claude-pick-effort', 'claude', ['chip-effort', 'row:effort:max'], 'state?')
check(has(dock, '/effort:max/', 'chips="Fake Opus|Max|Edits"', '[fake-opus/max/acceptEdits]'),
      'an effort pressed reaches Claude and reads back', dock)

choices.unlink(missing_ok=True)
dock, _ = run('claude-capped', 'claude', ['chip-model', 'row:model:fake-capped', 'chip-effort', 'row:effort:max'], 'state?')
check(has(dock, '/effort:max/', 'chips="Fake Capped|High|Edits"', '[fake-capped/high/acceptEdits]'),
      'the chip shows the level Claude applied, not the one it was asked for', dock)

choices.unlink(missing_ok=True)
dock, _ = run('claude-pick-access', 'claude', ['chip-access', 'row:access:full'], 'state?')
check(has(dock, '/access:full ', 'mode=bypassPermissions', '/bypassPermissions]'),
      'Full access reaches a running Claude', dock)
saved = json.loads(choices.read_text())
check(saved['agents']['claude']['access'] == 'accept-edits', 'Full access is not remembered', str(saved))

choices.unlink(missing_ok=True)
dock, _ = run('claude-refused', 'claude', ['chip-model', 'row:model:fake-refused'], 'state?')
check(has(dock, 'pick=model:-/', '[fake-opus/', 'not available on this plan'),
      'a choice the CLI refuses goes back and says why', dock)

dock, _ = run('claude-gap', 'claude', ['chip-access', 'send'], 'state?')
check('row:' not in dock and 'pick=model:-/effort:-/access:accept-edits' in dock and 'mode=acceptEdits' in dock,
      'a press outside a menu closes it and picks nothing', dock)

dock, _ = run('claude-toggle', 'claude', ['chip-access', 'chip-access'])
check('row:' not in dock, 'a second press on the chip closes its menu', dock)

# --- What a new chat starts on ---
choices.unlink(missing_ok=True)
run('remember', 'claude', ['chip-effort', 'row:effort:low', 'chip-access', 'row:access:supervised'])
dock, _ = run('remembered', 'claude', say='state?', frames=70)
check(has(dock, '/effort:low/access:supervised', '/low/default]'),
      'a new chat starts on the choices last made, on its command line', dock)

choices.write_text(json.dumps({'schema': 'musializer.agent-choices/v1', 'agents': {
    'claude': {'model': 'fake-gone', 'effort': 'max', 'access': 'accept-edits'}}}))
dock, _ = run('stale', 'claude', say='state?', frames=70)
check(has(dock, 'pick=model:-/effort:-/', 'fake-gone is no longer offered', '[fake-opus/'),
      'a remembered model the CLI dropped falls back and says so', dock)

broken = '{"schema": "something else"}'
choices.write_text(broken)
dock, log = run('refused-file', 'claude', ['chip-effort', 'row:effort:low'], 'state?')
check('/low/acceptEdits]' in dock, 'a file that cannot be read leaves the chat on defaults', dock)
check(choices.read_text() == broken and 'not saving over them' in log,
      'a file that cannot be read is not written over', choices.read_text())

# --- Codex: with the next turn ---
choices.unlink(missing_ok=True)
dock, _ = run('codex-default', 'codex', say='state?', frames=70)
check(has(dock, 'listed=2', 'chips="Fake Sol|Low|Edits"', '[fake-sol/low/on-request+user+workspaceWrite]'),
      'Codex lists its visible models and a turn carries the default access', dock)
dock, _ = run('codex-picks', 'codex', ['chip-model', 'row:model:fake-luna', 'chip-effort', 'row:effort:max',
                                       'chip-access', 'row:access:full'], 'state?')
check('[fake-luna/max/never+user+dangerFullAccess]' in dock, 'the next turn carries all three', dock)
choices.unlink(missing_ok=True)
dock, _ = run('codex-auto', 'codex', ['chip-access', 'row:access:auto'], 'state?')
check('on-request+auto_review+workspaceWrite' in dock, 'Auto routes approvals to the reviewer', dock)

# --- Antigravity: on a new process ---
choices.unlink(missing_ok=True)
dock, _ = run('agy-default', 'agy', say='state?', frames=70)
check(has(dock, 'listed=2', '[-/accept-edits/ask]'), 'agy models folds to one row per model', dock)
dock, _ = run('agy-picks', 'agy', ['chip-model', 'row:model:fake-argon', 'chip-access', 'row:access:full'], 'state?',
              frames=140)
check('[fake-argon-medium/accept-edits/skip]' in dock, 'the next message restarts it with the choices', dock)
choices.unlink(missing_ok=True)
dock, _ = run('agy-supervised', 'agy', ['chip-access', 'row:access:supervised'])
check(has(dock, '/access:accept-edits ', 'row:access:supervised'),
      'a row the agent cannot do is drawn, says why, and takes no press', dock)

choices.unlink(missing_ok=True)
dock, _ = run('agy-auto', 'agy', ['chip-access', 'row:access:auto'], 'state?', frames=150)
check(has(dock, '/access:accept-edits ', 'no auto mode on this account'),
      'an account without auto mode puts the choice back and says so', dock)

# --- The narrowest column ---
for name, size, words in (('narrow', '1100x720', False), ('column-1280', '1280x720', True)):
    choices.unlink(missing_ok=True)
    dock, _ = run(name, 'codex', ['chip-access', 'row:access:full', 'chip-model'], size=size)
    found = rects(dock)
    box = found['composer']
    chips = [found[k] for k in ('chip-model', 'chip-effort', 'chip-access', 'send')]
    inside = all(c[0] >= box[0] and c[0] + c[2] <= box[0] + box[2] and c[1] >= box[1]
                 and c[1] + c[3] <= box[1] + box[3] for c in chips)
    apart = all(a[0] + a[2] <= b[0] for a, b in zip(chips, chips[1:]))
    check(inside and apart, f'{name}: every control fits the composer, in order', dock)
    check((found['chip-access'][2] > 40) == words, f'{name}: the access chip keeps its words only with room', dock)
    row = found['row:model:fake-sol']
    check(row[0] >= 0 and row[0] + row[2] <= int(size.split('x')[0]), f'{name}: the menu stays on screen', dock)

print('agent controls:', 'PASS' if not failures else f'FAIL {len(failures)}: {failures}')
sys.exit(1 if failures else 0)
