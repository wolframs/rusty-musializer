#!/usr/bin/python3
"""Run ONLY as MZ_BIN under headless_run.sh; type into a probe-pressed value chip."""
import ctypes as C
import os
import json
from pathlib import Path
import subprocess
import sys
import time

assert os.environ.get('WAYLAND_DISPLAY') == 'musializer-headless-run-no-compositor'
assert os.environ.get('PULSE_SERVER') == 'unix:/nonexistent/musializer-headless-run-no-audio'
process = subprocess.Popen([os.environ['PIXEL_LOOK_REAL_BIN'], *sys.argv[1:]])
try:
    x = C.CDLL('libX11.so.6')
    test = C.CDLL('libXtst.so.6')
    x.XOpenDisplay.restype = C.c_void_p
    display = x.XOpenDisplay(None)
    assert display
    x.XDefaultRootWindow.argtypes=[C.c_void_p]; x.XDefaultRootWindow.restype=C.c_ulong
    x.XQueryTree.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.POINTER(C.c_ulong)),C.POINTER(C.c_uint)]
    x.XSetInputFocus.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_ulong]
    x.XKeysymToKeycode.argtypes=[C.c_void_p,C.c_ulong]; x.XKeysymToKeycode.restype=C.c_uint
    x.XFlush.argtypes=[C.c_void_p]
    test.XTestFakeKeyEvent.argtypes=[C.c_void_p,C.c_uint,C.c_int,C.c_ulong]
    # An X error (a window that went away, one not mapped yet) must not end
    # the driver: Xlib's default handler exits the process (docs/TRAPS.md).
    handler_type=C.CFUNCTYPE(C.c_int,C.c_void_p,C.c_void_p)
    ignore=handler_type(lambda *_: 0)
    x.XSetErrorHandler.argtypes=[handler_type]
    x.XSetErrorHandler(ignore)
    class Attributes(C.Structure):
        _fields_=[('x',C.c_int),('y',C.c_int),('width',C.c_int),('height',C.c_int),
                  ('border_width',C.c_int),('depth',C.c_int),('visual',C.c_void_p),
                  ('root',C.c_ulong),('class_',C.c_int),('bit_gravity',C.c_int),
                  ('win_gravity',C.c_int),('backing_store',C.c_int),('backing_planes',C.c_ulong),
                  ('backing_pixel',C.c_ulong),('save_under',C.c_int),('colormap',C.c_ulong),
                  ('map_installed',C.c_int),('map_state',C.c_int),('all_event_masks',C.c_long),
                  ('your_event_mask',C.c_long),('do_not_propagate_mask',C.c_long),
                  ('override_redirect',C.c_int),('screen',C.c_void_p)]
    x.XGetWindowAttributes.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(Attributes)]
    root=x.XDefaultRootWindow(display)
    window=None
    # The app's own window: mapped (IsViewable) and window-sized.
    for _ in range(150):
        assert process.poll() is None, 'app ended before its window'
        children=C.POINTER(C.c_ulong)(); count=C.c_uint(); parent=C.c_ulong(); returned_root=C.c_ulong()
        if x.XQueryTree(display,root,C.byref(returned_root),C.byref(parent),C.byref(children),C.byref(count)):
            for index in range(count.value):
                attributes=Attributes()
                if (x.XGetWindowAttributes(display,children[index],C.byref(attributes))
                        and attributes.map_state==2 and attributes.width>=640):
                    window=children[index]
        if window: break
        time.sleep(.1)
    assert window, 'no mapped window'
    # The window maps about 0.4 s before the first frame, and the probe's
    # press resolves five frames after that, which is when the value chip
    # opens its field. Keys sent earlier are lost: a Ctrl+A before the field
    # exists selects nothing and "216" lands after "144". Typing 0.8 s after
    # launch raced that and lost whenever the first frame came later than
    # ~0.85 s, on any commit (2026-11-15). Wait well past it; the gate gives
    # a typed run frames enough to outlast the keys.
    time.sleep(2.0)
    x.XSetInputFocus(display,window,2,0)
    x.XFlush(display)
    def key(sym, down):
        test.XTestFakeKeyEvent(display,x.XKeysymToKeycode(display,sym),down,0); x.XFlush(display); time.sleep(.06)
    def chord(letter):
        key(0xffe3,1); key(ord(letter),1); key(ord(letter),0); key(0xffe3,0)
    if os.environ.get('PIXEL_LOOK_LYRIC_INPUT'):
        controls=json.loads(os.environ['PIXEL_LOOK_LYRIC_INPUT'])
        test.XTestFakeMotionEvent.argtypes=[C.c_void_p,C.c_int,C.c_int,C.c_int,C.c_ulong]
        test.XTestFakeButtonEvent.argtypes=[C.c_void_p,C.c_uint,C.c_int,C.c_ulong]
        def click(name):
            px,py=controls[name]
            test.XTestFakeMotionEvent(display,0,px,py,0); x.XFlush(display); time.sleep(.08)
            test.XTestFakeButtonEvent(display,1,1,0); x.XFlush(display); time.sleep(.08)
            test.XTestFakeButtonEvent(display,1,0,0); x.XFlush(display); time.sleep(.15)
        click('next'); click('prev'); click('text')
        chord('a')
        for char in 'verified column phrase': key(ord(char),1); key(ord(char),0)
        click('start'); chord('a')
        for char in '1.250': key(ord(char),1); key(ord(char),0)
        key(0xff0d,1); key(0xff0d,0)
        click('apply'); chord('s'); time.sleep(.4)
        project=Path(os.environ['PIXEL_LOOK_LYRIC_PROJECT'])
        project.with_suffix('.after-apply.json').write_bytes(project.read_bytes())
        chord('z'); chord('s'); time.sleep(.4)
    else:
        key(0xffe3,1); key(ord('a'),1); key(ord('a'),0); key(0xffe3,0)
        for char in '216': key(ord(char),1); key(ord(char),0)
        sym=0xff1b if os.environ.get('PIXEL_LOOK_CANCEL') else 0xff0d
        key(sym,1); key(sym,0)
        # The field has released focus: save through the workspace's real chord.
        key(0xffe3,1); key(ord('s'),1); key(ord('s'),0); key(0xffe3,0)
    sys.exit(process.wait(timeout=20))
finally:
    if process.poll() is None: process.terminate(); process.wait(timeout=5)
