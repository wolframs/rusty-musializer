#!/usr/bin/python3
"""Run ONLY as MZ_BIN under headless_run.sh; type into a probe-pressed value chip."""
import ctypes as C
import os
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
    time.sleep(.8)
    root=x.XDefaultRootWindow(display)
    children=C.POINTER(C.c_ulong)(); count=C.c_uint(); parent=C.c_ulong(); returned_root=C.c_ulong()
    assert x.XQueryTree(display,root,C.byref(returned_root),C.byref(parent),C.byref(children),C.byref(count))
    assert count.value
    x.XSetInputFocus(display,children[count.value-1],2,0)
    def key(sym, down):
        test.XTestFakeKeyEvent(display,x.XKeysymToKeycode(display,sym),down,0); x.XFlush(display); time.sleep(.06)
    key(0xffe3,1); key(ord('a'),1); key(ord('a'),0); key(0xffe3,0)
    for char in '216': key(ord(char),1); key(ord(char),0)
    sym=0xff1b if os.environ.get('PIXEL_LOOK_CANCEL') else 0xff0d
    key(sym,1); key(sym,0)
    sys.exit(process.wait(timeout=15))
finally:
    if process.poll() is None: process.terminate(); process.wait(timeout=5)
