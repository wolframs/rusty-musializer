#!/usr/bin/python3
"""Real keys for tools/desktop_scale_check.sh. Run ONLY as MZ_BIN under headless_run.sh.

Starts the real binary (DESKTOP_SCALE_REAL_BIN) with the arguments it was
given, waits for its window, then sends native X events through XTest:

  DESKTOP_SCALE_POINTER=X,Y   park the pointer there first (window pixels)
  DESKTOP_SCALE_KEYS=plus,plus,minus,zero,equal
                              each one a Ctrl chord, by keysym, so the key
                              that is pressed is the one the display's
                              keyboard layout labels with that character

`--ui-probe key=scale-up` sets the shell's flags after the keys are read, so
it cannot tell whether the key a user presses is the key the shell listens
to. On a German layout it was not (docs/DIVERGENCES.md, DS).
"""
import ctypes as C
import os
import subprocess
import sys
import time

assert os.environ.get('WAYLAND_DISPLAY') == 'musializer-headless-run-no-compositor'
assert os.environ.get('PULSE_SERVER') == 'unix:/nonexistent/musializer-headless-run-no-audio'
KEYSYMS = {'plus': 0x2b, 'minus': 0x2d, 'zero': 0x30, 'equal': 0x3d}
CONTROL_L = 0xffe3
keys = [name for name in os.environ.get('DESKTOP_SCALE_KEYS', '').split(',') if name]
assert all(name in KEYSYMS for name in keys), keys
process = subprocess.Popen([os.environ['DESKTOP_SCALE_REAL_BIN'], *sys.argv[1:]])
try:
    x = C.CDLL('libX11.so.6')
    test = C.CDLL('libXtst.so.6')
    x.XOpenDisplay.restype = C.c_void_p
    display = x.XOpenDisplay(None)
    assert display
    x.XDefaultRootWindow.argtypes = [C.c_void_p]
    x.XDefaultRootWindow.restype = C.c_ulong
    x.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong),
                             C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]
    x.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
    x.XKeysymToKeycode.argtypes = [C.c_void_p, C.c_ulong]
    x.XKeysymToKeycode.restype = C.c_uint
    x.XFlush.argtypes = [C.c_void_p]
    test.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
    test.XTestFakeMotionEvent.argtypes = [C.c_void_p, C.c_int, C.c_int, C.c_int, C.c_ulong]
    # An X error (a window that went away, one not mapped yet) must not end
    # the driver: Xlib's default handler exits the process (docs/TRAPS.md).
    handler_type = C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)
    ignore = handler_type(lambda *_: 0)
    x.XSetErrorHandler.argtypes = [handler_type]
    x.XSetErrorHandler(ignore)

    class Attributes(C.Structure):
        _fields_ = [('x', C.c_int), ('y', C.c_int), ('width', C.c_int), ('height', C.c_int),
                    ('border_width', C.c_int), ('depth', C.c_int), ('visual', C.c_void_p),
                    ('root', C.c_ulong), ('class_', C.c_int), ('bit_gravity', C.c_int),
                    ('win_gravity', C.c_int), ('backing_store', C.c_int), ('backing_planes', C.c_ulong),
                    ('backing_pixel', C.c_ulong), ('save_under', C.c_int), ('colormap', C.c_ulong),
                    ('map_installed', C.c_int), ('map_state', C.c_int), ('all_event_masks', C.c_long),
                    ('your_event_mask', C.c_long), ('do_not_propagate_mask', C.c_long),
                    ('override_redirect', C.c_int), ('screen', C.c_void_p)]
    x.XGetWindowAttributes.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(Attributes)]
    root = x.XDefaultRootWindow(display)
    window = None
    origin = (0, 0)
    # The app's own window: mapped (IsViewable) and window-sized.
    for _ in range(150):
        assert process.poll() is None, 'app ended before its window'
        children = C.POINTER(C.c_ulong)()
        count = C.c_uint()
        parent = C.c_ulong()
        returned_root = C.c_ulong()
        if x.XQueryTree(display, root, C.byref(returned_root), C.byref(parent), C.byref(children), C.byref(count)):
            for index in range(count.value):
                attributes = Attributes()
                if (x.XGetWindowAttributes(display, children[index], C.byref(attributes))
                        and attributes.map_state == 2 and attributes.width >= 640):
                    window = children[index]
                    origin = (attributes.x, attributes.y)
        if window:
            break
        time.sleep(.1)
    assert window, 'no mapped window'
    # The window maps before the first frame; keys sent then are lost
    # (docs/TRAPS.md, "A native-key driver that types on a clock"). Wait well
    # past it; the gate gives the run frames enough to outlast the keys.
    time.sleep(2.0)
    x.XSetInputFocus(display, window, 2, 0)
    x.XFlush(display)
    pointer = os.environ.get('DESKTOP_SCALE_POINTER')
    if pointer:
        # A window nothing placed sits where raylib centred it, not at the
        # root's corner: a point given in the window is moved by its origin.
        px, py = (int(value) for value in pointer.split(','))
        test.XTestFakeMotionEvent(display, 0, origin[0] + px, origin[1] + py, 0)
        x.XFlush(display)
        time.sleep(.4)

    def key(sym, down):
        code = x.XKeysymToKeycode(display, sym)
        assert code, f'no key carries keysym {sym:#x} on this layout'
        test.XTestFakeKeyEvent(display, code, down, 0)
        x.XFlush(display)
        time.sleep(.08)

    for name in keys:
        key(CONTROL_L, 1)
        key(KEYSYMS[name], 1)
        key(KEYSYMS[name], 0)
        key(CONTROL_L, 0)
        # A scale change rebuilds the font atlas and may resize the window.
        time.sleep(.5)
    sys.exit(process.wait(timeout=30))
finally:
    if process.poll() is None:
        process.terminate()
        process.wait(timeout=5)
