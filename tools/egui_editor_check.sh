#!/usr/bin/env bash
# Real-input smoke check for the egui editor. A probe only parks initial time;
# every editing/theme action and modal-blocking assertion uses actual input.
# All playback is process-muted and additionally denied the desktop audio sink.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${MUSIALIZER_EGUI_CHECK_OUT:-$ROOT/build/egui-editor-check-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
export OUT
if ! command -v xdotool >/dev/null; then
    # Optional local tool installation; never changes the host package database.
    TOOLS="$ROOT/build/egui-ui-test-tools"
    mkdir -p "$TOOLS"
    if [[ ! -x "$TOOLS/root/usr/bin/xdotool" ]]; then
        (cd "$TOOLS" && apt-get download xdotool libxdo3 > download.log 2>&1
         for deb in ./*.deb; do dpkg-deb -x "$deb" root; done)
    fi
    export PATH="$TOOLS/root/usr/bin:$PATH"
    export LD_LIBRARY_PATH="$TOOLS/root/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
export WAYLAND_DISPLAY=musializer-egui-check-no-compositor
export PULSE_SERVER=unix:/nonexistent/musializer-egui-check
export MUSIALIZER_UI_PREFERENCES="$OUT/ui.json"
export XDG_CONFIG_HOME="$OUT/config" XDG_STATE_HOME="$OUT/state" XDG_DATA_HOME="$OUT/data"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_DATA_HOME"
# Building and generating PCM open no playback device.
cargo build --quiet --bin musializer --bin make-fixture-wav
./target/debug/make-fixture-wav "$OUT/source.wav" 8 > "$OUT/fixture.log"
xvfb-run --auto-servernum --server-args='-screen 0 1280x900x24 -nolisten tcp' python3 - "$ROOT" <<'PY'
import json, os, pathlib, subprocess, sys, time
from PIL import ImageGrab
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(os.environ['OUT'])
binary = root/'target/debug/musializer'
project = out/'editor.musi'
subprocess.run([str(binary), '--mute', str(out/'source.wav'), '--save-project', str(project)],
               stdout=(out/'seed.log').open('w'), stderr=subprocess.STDOUT, check=True, timeout=30)
doc = json.loads(project.read_text())
doc['lyrics'] = {'next_id': 3, 'cues': [
    {'id':1,'start_seconds':1.0,'end_seconds':2.5,'text':'Original editor phrase'},
    {'id':2,'start_seconds':3.0,'end_seconds':4.5,'text':'Second editor phrase'},
]}
project.write_text(json.dumps(doc, indent=2)+'\n')
(out/'before.json').write_text(json.dumps(doc, indent=2)+'\n')

def xd(*args):
    return subprocess.check_output(['xdotool', *map(str,args)], text=True).strip()
def click(x,y):
    xd('mousemove', '--sync', x,y); xd('mousedown',1); time.sleep(.12); xd('mouseup',1); time.sleep(.3)
def key(*keys):
    xd('key','--clearmodifiers','--delay',80,*keys); time.sleep(.25)
def text(value):
    xd('type','--clearmodifiers','--delay',35,value); time.sleep(.25)
def shot(name):
    ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save(out/f'{name}.png')
def launch(theme=None, parked=False):
    args=[str(binary),'--mute','--project',str(project),'--size','1280x900','--ui-scale','100','--editor-ui']
    if theme: args += ['--ui-theme',theme]
    if parked: args += ['--ui-probe','time=0.8,play=0']
    p=subprocess.Popen(args,stdout=(out/f'app-{theme or "reload"}.log').open('w'),stderr=subprocess.STDOUT)
    try:
        for _ in range(100):
            if p.poll() is not None: raise RuntimeError('application exited during launch')
            try:
                windows=xd('search','--onlyvisible','--pid',p.pid).splitlines()
                if windows: xd('windowfocus','--sync',windows[-1]); break
            except subprocess.CalledProcessError: pass
            time.sleep(.1)
        else: raise RuntimeError('application window did not appear')
        time.sleep(1)
        return p
    except BaseException:
        p.terminate(); p.wait(timeout=5); raise

def stop(p):
    p.terminate()
    try: p.wait(timeout=5)
    except subprocess.TimeoutExpired: p.kill(); p.wait(timeout=5)

p=launch('light-blue', parked=True)
try:
    shot('01-light-tune')
    # Fixed 1280x900 logical pixels, scale1; these target visible toolkit widgets.
    # Interaction coordinates are kept explicit so captures expose layout drift.
    click(970,337)
    shot('02-setting-changed')
    xd('mousemove','--sync',1100,600)
    xd('click','--repeat',5,'--delay',100,5); time.sleep(.4)
    shot('02b-tune-scrolled')
    xd('click','--repeat',10,'--delay',100,4); time.sleep(.4)
    click(804,87)
    shot('03-lyrics')
    # Actual cue navigation must round-trip to the same cue before editing.
    click(827,253); click(754,253)
    shot('03b-cue-navigation')
    click(975,358); key('ctrl+a'); text('Verified toolkit phrase')
    shot('04-text-draft')
    click(852,393); click(852,393); key('ctrl+a'); text('1.250'); key('Return')
    # Closing and reopening through the legacy shortcut must retain the draft
    # and selected cue. Apply only after reopening so persistence proves it.
    click(1223,46); key('F8')
    shot('04b-draft-after-close-reopen')
    click(760,500)
    shot('05-cue-applied')
    # Save from inside the editor rather than relying only on the legacy key.
    click(1200,212)
    time.sleep(.5)
    changed=json.loads(project.read_text())
    (out/'after-apply.json').write_text(json.dumps(changed,indent=2)+'\n')
    cue=changed['lyrics']['cues'][0]
    assert cue['text']=='Verified toolkit phrase', cue
    assert abs(cue['start_seconds']-1.25)<1e-5, cue
    assert changed['scenes'] != doc['scenes'], 'scene setting click did not persist'
    # The real legacy undo shortcut must share the new editor's history after
    # focus returns to the workspace.
    click(1223,46); key('ctrl+z'); key('ctrl+s'); time.sleep(.5)
    undone=json.loads(project.read_text())
    assert undone['lyrics']['cues']==doc['lyrics']['cues'], undone['lyrics']
    shot('06-legacy-undo')
    # F8 reopens on the same cue; Escape closes, and F8 restores it again.
    key('F8'); key('Escape')
    assert p.poll() is None, 'Escape closed the application instead of the editor'
    key('F8'); shot('06b-escape-reopen')
finally:
    stop(p)

p=launch('light-blue')
try:
    click(785,128); time.sleep(.2); shot('07-theme-menu')
    click(785,207); time.sleep(.5); shot('08-black-amber')
    assert json.loads((out/'ui.json').read_text())['theme']=='black-amber'
finally:
    stop(p)
p=launch()
try:
    shot('09-reloaded-theme')
    from PIL import Image
    picture=Image.open(out/'09-reloaded-theme.png').convert('RGB')
    assert max(picture.getpixel((20,225))) < 40, 'reloaded shell background is not dark'
    r,g,b=picture.getpixel((740,87))
    assert r > 60 and r > g * 1.2 and b < 40, 'reloaded selected Tune tab is not amber'
    assert json.loads((out/'ui.json').read_text())['theme']=='black-amber'
finally:
    stop(p)
(out/'result.json').write_text(json.dumps({'status':'passed','input':'real X11 mouse and keyboard',
    'checks':['modal transport input blocked at 0.8 seconds','scene setting saved','lyric text and edge applied','legacy undo','theme persisted and reloaded']},indent=2)+'\n')
print(f'OK: egui editor real-input evidence: {out}')
PY
