#!/usr/bin/env bash
# Startup appearance, persistence and handoff checked with real X11 input.
# Playback is muted per process, with private display/config and no audio sink.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
OUT="$ROOT/build/welcome-theme-check"
mkdir -p "$OUT/config" "$OUT/data" "$OUT/state" "$OUT/bin"
export ROOT OUT
export WAYLAND_DISPLAY=musializer-welcome-no-compositor
export PULSE_SERVER=unix:/nonexistent/musializer-welcome
export XDG_CONFIG_HOME="$OUT/config" XDG_DATA_HOME="$OUT/data" XDG_STATE_HOME="$OUT/state"
export MUSIALIZER_UI_PREFERENCES="$OUT/ui.json"
export MUSIALIZER_RECOVERY_DIR="$OUT/recovery"
export PATH="$OUT/bin:$ROOT/build/egui-ui-test-tools/root/usr/bin:$PATH"
export LD_LIBRARY_PATH="$ROOT/build/egui-ui-test-tools/root/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
command -v xdotool >/dev/null
cargo build --quiet --bin musializer --bin make-fixture-wav
./target/debug/make-fixture-wav "$OUT/source.wav" 8 > "$OUT/fixture.log"
export MZ_TEST_AUDIO="$OUT/source.wav"
# Return the isolated fixture through the normal picker process boundary.
cat > "$OUT/bin/kdialog" <<'PICKER'
#!/bin/sh
printf '%s\n' "$MZ_TEST_AUDIO"
printf 'opened\n' > "$OUT/picker-called"
PICKER
chmod +x "$OUT/bin/kdialog"
xvfb-run --auto-servernum --server-args='-screen 0 960x640x24 -nolisten tcp' python3 - <<'PY'
import json, os, pathlib, subprocess, time
from PIL import ImageGrab
root=pathlib.Path(os.environ['ROOT']); out=pathlib.Path(os.environ['OUT'])
(out/'result.json').unlink(missing_ok=True)
def xd(*args):
    return subprocess.check_output(['xdotool',*map(str,args)],text=True).strip()
def click(x,y):
    xd('mousemove','--sync',x,y); time.sleep(.1)
    xd('mousedown',1); time.sleep(.15); xd('mouseup',1); time.sleep(.6)
def shot(name):
    im=ImageGrab.grab(xdisplay=os.environ['DISPLAY']); im.save(out/(name+'.png')); return im.convert('RGB')
def launch(theme=None):
    args=[str(root/'target/debug/musializer'),'--mute','--size','960x640','--ui-scale','100']
    if theme: args+=['--ui-theme',theme]
    p=subprocess.Popen(args,stdout=(out/('app-'+str(theme)+'.log')).open('w'),stderr=subprocess.STDOUT)
    for _ in range(100):
        if p.poll() is not None: raise RuntimeError('application exited')
        try:
            wins=xd('search','--onlyvisible','--pid',p.pid).splitlines()
            if wins: xd('windowfocus','--sync',wins[-1]); break
        except subprocess.CalledProcessError: pass
        time.sleep(.1)
    time.sleep(.8); return p
def stop(p):
    p.terminate(); p.wait(timeout=5)
p=launch('light-blue')
try:
    shot('01-welcome-light')
    click(850,40); shot('02-theme-menu')
    click(835,118); im=shot('03-welcome-amber')
    assert json.loads((out/'ui.json').read_text())['theme']=='black-amber'
    assert max(im.getpixel((20,120)))<40
    (out/'picker-called').unlink(missing_ok=True)
    click(184,308); time.sleep(1.5)
    assert (out/'picker-called').exists(), 'Open audio click did not reach the picker'
    xd('keydown','F8'); time.sleep(.15); xd('keyup','F8'); time.sleep(.8)
    im=shot('04-opened-editor-amber')
    assert max(im.getpixel((700,90)))<40, 'editor did not replace the welcome step marker'
    assert max(im.getpixel((530,30)))<40, 'editor reverted theme after opening audio'
    # The editor panel starts at x=522; its border must be visible. Dark scene
    # pixels alone cannot distinguish the editor from the normal workspace.
    assert min(im.getpixel((522,90)))>45, 'Editor did not open'
finally: stop(p)
p=launch()
try:
    im=shot('05-welcome-reloaded')
    assert max(im.getpixel((20,120)))<40, 'startup did not restore theme'
    click(850,40); shot('06-reloaded-menu')
    click(835,76); im=shot('07-welcome-light-again')
    assert json.loads((out/'ui.json').read_text())['theme']=='light-blue'
    assert min(im.getpixel((20,120)))>230
finally: stop(p)
(out/'result.json').write_text(json.dumps({'status':'passed','checks':['welcome theme toggles both ways','selection persisted across restart','opening audio and Editor preserves theme']},indent=2)+'\n')
print('PASS: welcome theme, restart and editor handoff')
PY
