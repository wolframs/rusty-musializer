#!/usr/bin/env python3
"""Saved agent chats, end to end.

Every launch is a new process, so whatever a run sees was read back from the
store, and "resumed" is what the fake says its CLI was told, never the dock's
own idea. Chats are kept per project; the second track is the control for that.
"""
import fcntl
import json
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path

from PIL import Image

out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
root = Path.cwd()
failures = []


def check(ok, name, detail=''):
    print(('ok: ' if ok else 'FAIL: ') + name + ('' if ok else f'\n    {detail[:1000]}'), flush=True)
    if not ok:
        failures.append(name)


fake = out / 'fake-agent'
fake.write_text(f'#!/bin/sh\nexec /usr/bin/python3 "{root}/tools/fake_agent.py" "$@"\n')
fake.chmod(0o755)
song = out / 'song.wav'
other = out / 'other.wav'
for wave in (song, other):
    subprocess.run(['cargo', 'run', '--quiet', '--bin', 'make-fixture-wav', '--', str(wave), '4'], check=True)
store = out / 'chats'
shutil.rmtree(store, ignore_errors=True)
choices = out / 'agents.json'
env = dict(os.environ, MZ_BIN=str(Path(os.environ.get('MZ_BIN', 'target/debug/musializer')).resolve()),
           MZ_STATE_DIR=str(out / 'state'), SHELL='/bin/sh', MUSIALIZER_AGENT_CHOICES=str(choices),
           MUSIALIZER_CHATS=str(store), MUSIALIZER_AGENT_CLAUDE=str(fake), MUSIALIZER_AGENT_CODEX=str(fake),
           MUSIALIZER_AGENT_AGY=str(fake))


def run(name, spec, track=song, frames=None, extra=None):
    presses = next((s for s in spec if s.startswith('agent-press=')), '')
    frames = frames or 40 + 12 * (presses.count('+') + bool(presses)) + 40
    done = subprocess.run(['tools/headless_run.sh', str(track), '--ui-scale', '100', '--hud=0', '--ui-probe',
                           ','.join(['size=1920x1080', 'play=0', *spec]), '--probe-frames', str(frames),
                           '--probe-shot', str(out / f'{name}.png')],
                          env=env | (extra or {}), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    (out / f'{name}.log').write_text(done.stdout)
    dock = next((l for l in reversed(done.stdout.splitlines()) if l.startswith('dock:')), '')
    if not dock:
        check(False, f'{name}: a dock line', done.stdout[-900:])
    return dock


def files():
    return sorted(store.glob('*/*.json'))


def has(dock, *words):
    return all(word in dock for word in words)


def fresh():
    for path in files():
        path.unlink()


def edit(path, **changes):
    chat = json.loads(path.read_text())
    chat.update(changes)
    path.write_text(json.dumps(chat))


# --- Claude: kept, listed, reopened, resumed ---
choices.unlink(missing_ok=True)
dock = run('claude-first', ['agent=claude', 'agent-press=chip-access+row:access:full', 'agent-say=hello there'])
saved = files()
check(len(saved) == 1, 'a chat with words in it is kept', str(saved))
chat = json.loads(saved[0].read_text())
check(chat['title'] == 'hello there' and chat['session'] == 'fake-session' and chat['access'] == 'full'
      and chat['agent'] == 'claude' and chat['project'] == str(song),
      'the file holds the title, session, access and project', str(chat)[:600])
kinds = [entry['kind'] for entry in chat['entries']]
check(kinds[0] == 'user' and 'tool' in kinds and 'assistant' in kinds, 'the file holds the conversation', str(kinds))
check(stat.S_IMODE(saved[0].stat().st_mode) == 0o600 and stat.S_IMODE(saved[0].parent.stat().st_mode) == 0o700,
      'the chat is readable by its owner alone')

dock = run('claude-listed', ['dock=1'])
check(has(dock, 'tabs=0', 'earlier=1 ', 'earlier:0@'), 'a new launch lists it for the same project', dock)
dock = run('other-project', ['dock=1'], track=other)
check('earlier=0 ' in dock and 'earlier:0@' not in dock, 'another project does not list it', dock)

edit(saved[0], draft='half a thought')
dock = run('claude-reopen', ['agent-press=earlier:0', 'agent-say=resume?'])
check(has(dock, 'tabs=1', 'users=2', 'resumed=fake-session', 'access:full', 'mode=bypassPermissions'),
      'a press reopens it, resuming the session on its own settings', dock)
check(len(files()) == 1, 'a reopened chat stays one file', str(files()))
chat = json.loads(saved[0].read_text())
check(sum(e['kind'] == 'user' for e in chat['entries']) == 2 and chat['title'] == 'hello there',
      'what is said after reopening is kept in the same chat', str(chat)[:400])

elsewhere = out / 'elsewhere'
elsewhere.mkdir(exist_ok=True)
edit(saved[0], cwd=str(elsewhere))
dock = run('claude-cwd', ['agent-press=earlier:0', 'agent-say=resume?'])
check('cwd=elsewhere' in dock, 'a chat runs where it ran, wherever the project is opened from', dock)
edit(saved[0], cwd=str(out))

dock = run('claude-open-not-listed', ['agent-press=earlier:0+head-plus'])
check('head-plus@' in dock and 'past:0@' not in dock, 'an open chat is not offered again', dock)

edit(saved[0], draft='half a thought')
dock = run('claude-draft', ['agent-press=earlier:0'])
check('draft-lines=1' in dock, 'the draft comes back', dock)
edit(saved[0], draft='')

# Closing the window keeps what happened after the last turn ended.
fresh()
# Slowly, so the prompt lands after the save the message itself caused.
run('asked', ['agent=claude', 'agent-say=ask me slowly'], frames=240)
chat = json.loads(files()[0].read_text())
check(any(e['kind'] == 'permission' and e['allowed'] is None for e in chat['entries']),
      'a prompt still open when the window closes is kept', str(chat['entries'])[:500])
dock = run('asked-reopen', ['agent-press=earlier:0'])
check(has(dock, 'prompts=0', 'The chat ended before you answered'), 'and comes back as a record, not a prompt', dock)

# A chat with no words leaves nothing behind.
fresh()
run('no-words', ['agent=claude'])
check(files() == [], 'a chat nothing was said to is not kept', str(files()))

# --- Codex and Antigravity ---
fresh()
run('codex-first', ['agent=codex', 'agent-say=hello codex'])
dock = run('codex-reopen', ['agent-press=earlier:0', 'agent-say=resume?'])
check(has(dock, 'users=2', 'resumed=fake-thread'), 'Codex resumes its thread', dock)

fresh()
run('agy-first', ['agent=agy', 'agent-say=hello agy'])
dock = run('agy-reopen', ['agent-press=earlier:0', 'agent-say=resume?'])
check(has(dock, 'users=2', 'resumed=fake-conversation', 'lines=2'),
      'Antigravity resumes its conversation, without the standing instructions again', dock)

# --- A CLI that lost the conversation ---
for agent, name in (('claude', 'Claude'), ('codex', 'Codex'), ('agy', 'Antigravity')):
    fresh()
    run(f'{agent}-first', [f'agent={agent}', 'agent-say=hello'])
    edit(files()[0], session='lost-session')
    dock = run(f'{agent}-lost', ['agent-press=earlier:0', 'agent-say=still there'], frames=130)
    check(has(dock, 'no longer has this conversation', 'users=2', 'last="fake reply to: still there'),
          f'{name}: a lost conversation says so and the next message starts over', dock)

# --- What the store refuses ---
fresh()
run('keep', ['agent=claude', 'agent-say=keep me'])
kept = files()[0]
broken = kept.parent / 'broken.json'
broken.write_text('{')
dock = run('refused', ['dock=1'])
check(has(dock, 'earlier=1 ', 'refused=1'), 'a file it cannot read is counted, not listed', dock)
check(broken.read_text() == '{', 'and left alone')
broken.unlink()

# --- Another window has it ---
lock = os.open(kept.with_suffix('.lock'), os.O_CREAT | os.O_WRONLY, 0o600)
fcntl.flock(lock, fcntl.LOCK_EX)
before = kept.read_text()
dock = run('other-window', ['agent-press=earlier:0'])
check(has(dock, 'tabs=0', 'open in another window'), 'a chat open in another window stays there and says so', dock)
dock = run('other-window-forget', ['agent-press=forget:0+forget-yes:0'])
check(kept.read_text() == before and 'open in another window' in dock, 'and cannot be forgotten from here', dock)
os.close(lock)

listed = run('listed-then-edited', ['agent-press=earlier:0'])
check('tabs=1' in listed, 'the lock is free again once the other window lets go', listed)

# --- Forgetting ---
run('second', ['agent=claude', 'agent-say=forget me'])
newest = [p for p in files() if 'forget me' in p.read_text()][0]
dock = run('forget-keep', ['agent-press=forget:0+forget-no:0'])
check(len(files()) == 2 and 'forget-yes' not in dock, 'Keep keeps it', dock)
dock = run('forget-no-confirm', ['agent-press=forget:0'])
check(len(files()) == 2 and 'forget-yes:0@' in dock, 'the × asks first', dock)
dock = run('forget', ['agent-press=forget:0+forget-yes:0'])
check(files() == [kept] and not newest.exists() and 'earlier=1 ' in dock, 'Forget removes that chat only', dock)

# --- The head's + with a chat open ---
dock = run('head', ['agent=codex', 'agent-press=head-plus+past:0'])
check(has(dock, 'tabs=2', 'Claude', 'Codex'), 'the + menu reopens a chat beside an open one', dock)

# --- A picture ---
edit(kept, entries=json.loads(kept.read_text())['entries'] + [{'kind': 'image', 'path': 'sheet.png'}])
dock = run('image', ['agent-press=earlier:0'])
check('image@' in dock, 'a picture entry comes back', dock)
Image.new('RGB', (64, 36), (200, 40, 40)).save(elsewhere / 'sheet.png')
edit(kept, cwd=str(elsewhere))
dock = run('image-cwd', ['agent-press=earlier:0'])
check('sheet.png:' in dock and 'sheet.png:missing' not in dock, "it is found in the chat's own folder", dock)

# --- A probe run keeps nothing unless told where ---
scratch = out / 'xdg'
quiet = {k: v for k, v in env.items() if k != 'MUSIALIZER_CHATS'}
done = subprocess.run(['tools/headless_run.sh', str(song), '--ui-scale', '100', '--hud=0', '--ui-probe',
                       'agent=claude,agent-say=hello,play=0', '--probe-frames', '70'],
                      env=quiet | {'XDG_DATA_HOME': str(scratch)}, stdout=subprocess.PIPE,
                      stderr=subprocess.STDOUT, text=True)
check(done.returncode == 0 and 'replies=1' in done.stdout and not scratch.exists(),
      'a probe run with no named place writes nothing', done.stdout[-400:])

print('saved chats:', 'PASS' if not failures else f'FAIL {len(failures)}: {failures}')
sys.exit(1 if failures else 0)
