#!/usr/bin/python3
"""Native input in each reviewed text surface must leave cues and playhead intact.

Run from the checkout with MUSIALIZER_CAPTURE_DISPLAY=:88. Every app process
is launched by headless_run.sh, including this file's isolated XTest driver.
The fixture is a disposable copy; nothing in ~/Music is opened. Set MZ_BIN to
an intentionally perturbed build for the negative control.
"""
import ctypes as C
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time


def drive():
    assert os.environ['WAYLAND_DISPLAY'] == 'musializer-headless-run-no-compositor'
    assert os.environ['PULSE_SERVER'] == 'unix:/nonexistent/musializer-headless-run-no-audio'
    process = subprocess.Popen([os.environ['FOCUS_REAL_BIN'], *sys.argv[1:]])
    try:
        x = C.CDLL('libX11.so.6')
        xt = C.CDLL('libXtst.so.6')
        x.XOpenDisplay.restype = C.c_void_p
        d = x.XOpenDisplay(None)
        assert d
        x.XDefaultRootWindow.argtypes = [C.c_void_p]
        x.XDefaultRootWindow.restype = C.c_ulong
        x.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]
        x.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
        x.XKeysymToKeycode.argtypes = [C.c_void_p, C.c_ulong]
        x.XKeysymToKeycode.restype = C.c_uint
        x.XFlush.argtypes = [C.c_void_p]
        xt.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        xt.XTestFakeMotionEvent.argtypes = [C.c_void_p, C.c_int, C.c_int, C.c_int, C.c_ulong]
        xt.XTestFakeButtonEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        time.sleep(.8)
        root = x.XDefaultRootWindow(d)
        children = C.POINTER(C.c_ulong)()
        count, parent, returned = C.c_uint(), C.c_ulong(), C.c_ulong()
        assert x.XQueryTree(d, root, C.byref(returned), C.byref(parent), C.byref(children), C.byref(count))
        assert count.value
        x.XSetInputFocus(d, children[count.value - 1], 2, 0)

        def key(sym, down):
            xt.XTestFakeKeyEvent(d, x.XKeysymToKeycode(d, sym), down, 0)
            x.XFlush(d)
            time.sleep(.045)

        def press(sym):
            key(sym, 1)
            key(sym, 0)

        def chord(sym):
            key(0xffe3, 1)
            press(ord(sym))
            key(0xffe3, 0)

        def click(point):
            xt.XTestFakeMotionEvent(d, 0, *point, 0)
            x.XFlush(d)
            time.sleep(.08)
            xt.XTestFakeButtonEvent(d, 1, 1, 0)
            x.XFlush(d)
            time.sleep(.08)
            xt.XTestFakeButtonEvent(d, 1, 0, 0)
            x.XFlush(d)
            time.sleep(.18)

        # Loading the copied full track can outlast window creation. Wait for
        # its first painted frame instead of delivering keys during decoding.
        from PIL import Image, ImageGrab
        ready_box = tuple(json.loads(os.environ.get('FOCUS_READY_BOX', '[10,10,32,32]')))
        ready = Image.open(os.environ['FOCUS_BASE_SHOT']).convert('RGB').crop(ready_box).tobytes()
        for _ in range(200):
            painted = ImageGrab.grab(xdisplay=os.environ['DISPLAY']).convert('RGB').crop(ready_box).tobytes()
            # Screenshot readback and X11 capture can round the logo's
            # antialiased/gradient pixels by one; a black loading frame is far
            # outside this mean-distance bound.
            if sum(abs(a - b) for a, b in zip(painted, ready)) <= 3 * len(ready):
                break
            assert process.poll() is None, 'app ended before its first frame'
            time.sleep(.1)
        else:
            ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save(os.environ['FOCUS_INPUT_SHOT'] + '.ready-failed.png')
            raise AssertionError('app never painted its ready mark')
        time.sleep(.5)
        points = json.loads(os.environ['FOCUS_POINTS'])
        if points.get('entry'):
            click(points['entry'])
        if os.environ['FOCUS_SURFACE'] != 'terminal':
            for char in 'focus':
                press(ord(char))
            chord('a')
        # Asymmetric arrows/punctuation cannot cancel an escaped shortcut.
        for letter in 'cxvz':
            chord(letter)
        for sym in (ord('.'), ord('.'), ord(','), 0xff53, 0xff53, 0xff51, 0xffff):
            press(sym)
        if os.environ['FOCUS_SURFACE'] == 'chat':
            # A long prompt and Shift+Enter must remain inside the composer.
            for _ in range(16):
                key(0xffe1, 1)
                press(0xff0d)
                key(0xffe1, 0)
                press(ord('x'))
        from PIL import ImageGrab
        assert process.poll() is None, f'app exited during typing: {process.returncode}'
        ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save(os.environ['FOCUS_INPUT_SHOT'])
        # Cancel the local draft, release egui focus with an outside click,
        # then persist through the real global Save chord.
        press(0xff1b)
        click(points['outside'])
        chord('s')
        time.sleep(.4)
        # Ask GLFW to close cleanly after Save rather than burning hundreds
        # of idle frames on a software rasterizer. The app still prints its
        # final report and reaps its dock; the frame limit is the fallback.
        class ClientMessage(C.Structure):
            _fields_ = [('type', C.c_int), ('serial', C.c_ulong),
                        ('send_event', C.c_int), ('display', C.c_void_p),
                        ('window', C.c_ulong), ('message_type', C.c_ulong),
                        ('format', C.c_int), ('data', C.c_long * 5)]
        x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
        x.XInternAtom.restype = C.c_ulong
        x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.c_void_p]
        event = (C.c_long * 24)()
        message = C.cast(event, C.POINTER(ClientMessage)).contents
        message.type, message.send_event, message.display = 33, 1, d
        message.window = children[count.value - 1]
        message.message_type = x.XInternAtom(d, b'WM_PROTOCOLS', 0)
        message.format = 32
        message.data[0] = x.XInternAtom(d, b'WM_DELETE_WINDOW', 0)
        x.XSendEvent(d, message.window, 0, 0, C.byref(event))
        x.XFlush(d)
        print('focus-input: native keys delivered to ' + os.environ['FOCUS_SURFACE'], flush=True)
        return process.wait(timeout=60)
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)


def line(log, label):
    values = re.findall(r'^' + re.escape(label) + r':\s*(.*)$', log, re.M)
    assert values, f'missing {label}'
    return values[-1]


def rect(log, name):
    found = re.search(re.escape(name) + r'@(-?\d+),(-?\d+),(\d+),(\d+)', log)
    assert found, f'no {name} rect in {log}'
    x, y, w, h = map(int, found.groups())
    return [x + w // 2, y + h // 2]


def run_check():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else 'build/review-fixes/focus').resolve()
    out.mkdir(parents=True, exist_ok=True)
    binary = str(Path(os.environ.get('MZ_BIN', 'target/debug/musializer')).resolve())
    env = dict(os.environ, MZ_BIN=binary, MUSIALIZER_AGENT_CLAUDE='/nonexistent/mz-focus-agent')
    cases = {
        'artist': 'column=direct',
        'chat': 'column=direct,agent=claude',
        'terminal': 'column=direct,dock-focus=1',
        'look': 'column=look',
        'pixel': 'column=look,pixel=video',
        'lyric': 'column=lyrics',
    }
    failures = []
    for surface, probe in cases.items():
        if os.environ.get('FOCUS_ONLY') and surface not in os.environ['FOCUS_ONLY'].split(','):
            continue
        fixture = out / surface / 'fixture'
        shutil.copytree('build/redesign/fixture', fixture, dirs_exist_ok=True)
        project = fixture / 'Door, No Room.musi'
        doc = json.loads(project.read_text())
        # Guarantee an editable pixel value and visible artist slot; cue data
        # stays the copied fixture's, so every cue is pinned in the comparison.
        doc['metadata']['artist'] = 'Focus fixture'
        doc['schema_version'] = 'musializer.project/v2'
        # Use an existing pixel treatment's shape through the project field.
        doc['scene_switches'].setdefault('finish', {})['pixel'] = dict(pixels=144, match='none', colours=[], levels=8, **dict.fromkeys(('dither', 'grid', 'scanlines', 'curvature', 'glow', 'split', 'shake', 'zoom'), 0))
        project.write_text(json.dumps(doc))
        before = doc['lyrics']['cues']
        base_cmd = ['tools/headless_run.sh', '--project', str(project), '--ui-scale', '100', '--hud=0',
                    '--ui-probe', f'size=1920x1080,time=3,play=0,lyric=1,{probe}']
        baseline = subprocess.run([*base_cmd, '--probe-frames', '24', '--probe-shot', str(out / f'{surface}-base.png')], env=dict(env, MZ_STATE_DIR=str(out / surface / 'base-state')), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (out / f'{surface}-base.log').write_text(baseline.stdout)
        assert baseline.returncode == 0, baseline.stdout[-2000:]
        log = baseline.stdout
        entry = None
        if surface == 'artist': entry = rect(line(log, 'top bar'), 'artist')
        if surface == 'chat':
            entry = rect(line(log, 'dock'), 'entry')
        if surface == 'look': entry = rect(line(log, 'tune layout'), 'value')
        if surface == 'pixel': entry = rect(line(log, 'pixel look'), 'value')
        if surface == 'lyric': entry = rect(line(log, 'lyrics column'), 'text')
        # Report-derived preview centre, away from every editing control.
        frame = re.search(r'preview frame:\s*.*?pixels (\d+),(\d+),(\d+),(\d+)', log)
        if not frame:
            frame = re.search(r'preview frame:\s*(\d+),(\d+),(\d+),(\d+)', log)
        assert frame, line(log, 'preview frame')
        x, y, w, h = map(int, frame.groups())
        driven_env = dict(env, MZ_BIN=str(Path(__file__).resolve()), FOCUS_REAL_BIN=binary,
                          FOCUS_SURFACE=surface, FOCUS_POINTS=json.dumps({'entry': entry, 'outside': [x + w // 2, y + h // 2]}),
                          FOCUS_BASE_SHOT=str(out / f'{surface}-base.png'), FOCUS_INPUT_SHOT=str(out / f'{surface}-typing.png'), MZ_STATE_DIR=str(out / surface / 'drive-state'))
        if surface == 'terminal':
            col = re.search(r'rect=(\d+),(\d+),(\d+),(\d+)', line(log, 'column'))
            cx, cy, cw, ch = map(int, col.groups())
            # This patch is terminal ground only after frame 20 opens its pane.
            driven_env['FOCUS_READY_BOX'] = json.dumps([cx + 12, cy + ch - 200, cx + 28, cy + ch - 184])
        saved_before = project.stat().st_mtime_ns
        driven = subprocess.run([*base_cmd, '--probe-frames', '600' if surface == 'chat' else '360', '--probe-shot', str(out / f'{surface}.png')], env=driven_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (out / f'{surface}.log').write_text(driven.stdout)
        cues = json.loads(project.read_text())['lyrics']['cues']
        playhead = re.search(r'playhead=(\S+)', line(driven.stdout, 'transport'))
        ok = driven.returncode == 0 and cues == before and playhead and playhead[1] == '3.000' and project.stat().st_mtime_ns != saved_before
        print(('PASS' if ok else 'FAIL') + f': {surface}: cues unchanged={cues == before}; playhead={playhead[1] if playhead else "missing"}', flush=True)
        if surface == 'chat':
            multiline = 'draft-lines=17' in line(driven.stdout, 'dock')
            print(('PASS' if multiline else 'FAIL') + ': Shift+Enter retains exactly 17 draft lines', flush=True)
            ok = ok and multiline
        if not ok: failures.append(surface)
    (out / 'result.json').write_text(json.dumps({'failures': failures}))
    return bool(failures)


if __name__ == '__main__':
    sys.exit(drive() if os.environ.get('FOCUS_SURFACE') else run_check())
