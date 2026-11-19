#!/usr/bin/python3
"""A window manager that says no, for tools/desktop_scale_check.sh.

  desktop_scale_wm.py refuse          a mapped window keeps the size it has
  desktop_scale_wm.py clamp WxH       a mapped window gets at most WxH

It takes SubstructureRedirect on the root of $DISPLAY (the gate's own Xvfb,
never a real session: it refuses to start without the gate's marker), maps
whatever asks to be mapped, lets windows move, and answers a resize of a
mapped window as its mode says. That is the one thing a bare Xvfb cannot do:
with no window manager every resize is granted at once, so a window that was
refused its size, as a tiling window manager refuses it, was never seen.

Prints `ready` once it owns the root.
"""
import ctypes as C
import os
import struct
import sys

assert os.environ.get('DESKTOP_SCALE_CHECK_DISPLAY') == os.environ.get('DISPLAY'), \
    'run by tools/desktop_scale_check.sh on the Xvfb it started, nowhere else'
mode = sys.argv[1]
limit = tuple(int(value) for value in sys.argv[2].split('x')) if mode == 'clamp' else None
assert mode in ('refuse', 'clamp')

x = C.CDLL('libX11.so.6')
x.XOpenDisplay.restype = C.c_void_p
display = x.XOpenDisplay(None)
assert display
x.XDefaultRootWindow.argtypes = [C.c_void_p]
x.XDefaultRootWindow.restype = C.c_ulong
x.XSelectInput.argtypes = [C.c_void_p, C.c_ulong, C.c_long]
x.XNextEvent.argtypes = [C.c_void_p, C.c_void_p]
x.XMapWindow.argtypes = [C.c_void_p, C.c_ulong]
x.XSync.argtypes = [C.c_void_p, C.c_int]


class Changes(C.Structure):
    _fields_ = [('x', C.c_int), ('y', C.c_int), ('width', C.c_int), ('height', C.c_int),
                ('border_width', C.c_int), ('sibling', C.c_ulong), ('stack_mode', C.c_int)]


x.XConfigureWindow.argtypes = [C.c_void_p, C.c_ulong, C.c_uint, C.POINTER(Changes)]
handler_type = C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)
ignore = handler_type(lambda *_: 0)
x.XSetErrorHandler.argtypes = [handler_type]
x.XSetErrorHandler(ignore)

root = x.XDefaultRootWindow(display)
SUBSTRUCTURE_REDIRECT, SUBSTRUCTURE_NOTIFY = 1 << 20, 1 << 19
x.XSelectInput(display, root, SUBSTRUCTURE_REDIRECT | SUBSTRUCTURE_NOTIFY)
x.XSync(display, 0)
print('ready', flush=True)

MAP_REQUEST, CONFIGURE_REQUEST = 20, 23
CW_X, CW_Y, CW_WIDTH, CW_HEIGHT = 1, 2, 4, 8
mapped = set()
event = C.create_string_buffer(192)
while True:
    x.XNextEvent(display, event)
    kind = struct.unpack_from('i', event, 0)[0]
    if kind == MAP_REQUEST:
        # XMapRequestEvent: type, serial, send_event, display, parent, window.
        window = struct.unpack_from('L', event, 40)[0]
        x.XMapWindow(display, window)
        mapped.add(window)
    elif kind == CONFIGURE_REQUEST:
        # XConfigureRequestEvent: ..., window @40, x y width height border @48,
        # above @72, detail @80, value_mask @88.
        window = struct.unpack_from('L', event, 40)[0]
        cx, cy, width, height, border = struct.unpack_from('5i', event, 48)
        mask = struct.unpack_from('L', event, 88)[0] & (CW_X | CW_Y | CW_WIDTH | CW_HEIGHT)
        if window in mapped:
            if mode == 'refuse':
                mask &= ~(CW_WIDTH | CW_HEIGHT)
            else:
                width, height = min(width, limit[0]), min(height, limit[1])
        if mask:
            changes = Changes(cx, cy, width, height, border, 0, 0)
            x.XConfigureWindow(display, window, mask, C.byref(changes))
    x.XSync(display, 0)
