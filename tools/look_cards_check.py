#!/usr/bin/env python3
"""The look cards gate (D): Look's section, video and captions cards, pressed.

Run by tools/look_cards_check.sh, which owns the X server. Checks, each
press aimed at a rect from `look cards:` and each with a press into the gap
beside it that must change nothing (the report, the model and the saved
file):

 section  (fixture: four sections; 2 and 3 Spectrum, 4 Cadence)
   1. a kind chip sets how section 2 arrives; the saved .musi says so; on
      section 4 (a cut) a kind adds a transition and the lane's own counts
      (`scene runs: arriving=`, `scene marks: transitions=`) follow;
   2. the length track sets the exact logarithmic length at the pressed
      point; a typed length commits, Escape keeps the old one, typing with
      nothing focused is refused;
   3. the glide track sets the exact quadratic glide; on section 4 (after
      another scene) the row is disabled and a press on it changes nothing;
   4. a palette chip grades the section (and `graded=` follows); the frame
      on screen changes at once while paused, and the gap press does not;
   5. the strength track on a graded section sets the exact strength;
   6. a caption style chip sets the style the section's lines begin in;
   7. Play the cut seeks before the boundary, plays through the transition
      and pauses after it; the gap press leaves the playhead alone;
   8. Done lets the section go; with none selected, the invitation selects
      the section under the playhead; the scope switch shows the video card;
   9. the lane head's name field opens Look from Direct; the gap beside it
      does not; a threshold section's door steps through scenes by name and
      Fog, written into the section's tuning;
  10. a digit typed into a card value does not select a scene (the control,
      the same key with no field open, does).
 video
  11. intro and outro kinds by name, the outro's length, Film and None,
      a finish member, a toning swatch, a typed highlight colour.
 tables (look=captions; `look-entry=` opens the entry a press works in)
  12. a voice's hue, ink and name (its lines keep it); removing a voice that
      sings lines asks to confirm; a
      style's anchor, a rename that takes its line along, Remove disabled for
      a style in use and working for one that is not, Add a style; a title
      card's text, handle (gains its @), start time (and a refused one),
      style, Remove asking to confirm, Add at the playhead (and waiting
      while a card covers it).
  19. every entry rests as one row; a press on one opens it alone, the gap
      between two opens nothing, its fold lays it to rest; an added style
      or card opens.
  20. Add a voice adds voice-1, open; This line is chosen on the voice that
      sings the line at the playhead; it gives the line to a voice, and on
      the voice that sings it takes it back; the gap
      between This line and Section N changes nothing.
 follow
  13. following a treatment, the cards are drawn disabled and a press
      changes nothing; Stop following stops following and the card is
      editable again.
  23. following, a kind chip's hover gives the follow's reason (D2).
 sequence (D2: a second project, `drop-at=`, `look-commit-at=`)
  15. a title card half-typed in one project, another project dropped in,
      Enter a frame later: the draft died with its project; Enter in the
      frame the other opens: refused where it is applied (`stale=1`). The
      control (no drop) commits the same draft.
  16. a removal armed in one project (a title card's, a voice's) is not
      armed in the other; the control, with no drop, stays armed in view.
  17. Play the cut still pauses at its end with the column on Lyrics; it
      is let go when another project opens and when the user's own Right
      arrow seeks (`key=right`).
 show
  18. selecting a section (section=, a real press on its body) puts the
      playhead just past its arrival (3.150 for section 2 from 7 s); a
      playhead already inside stays; a boundary press does not move it;
      played on past it, the card says whose picture it is and Show N
      returns (the gap beside it does not).
 keys (native XTest presses and keys, this file as MZ_BIN)
  22. a length typed with real keys saves and picks no scene; a title typed
      with T, space and 4 in it saves and moves neither the column, the
      playhead nor the scene; the 4 alone, with no field open, picks one.
 captures
  14. both appearances at 1280x720 (section and video) and at the column's
      minimum width: the card lies inside the list, the faces loaded.
  21. (in 14) every transition kind's glyph is placed beside its name and
      has inked pixels against its chip, at 1280 and at the 300 px column.

NEGATIVE CONTROLS (2026-11-14, GPU path), each a binary built from a perturbed
tree into its own target directory, run as LOOK_CARDS_BIN, then the tree
restored with `git checkout` and shown clean:
  A (11 failures): SetTransition accepted but not applied (every kind, the
    lane's counts, typed and dragged lengths, the intro); the logarithmic and
    quadratic tracks returning the old value (length, glide, outro length);
    `card_typing()` false (the 4 typed into a value picked ASCII Field).
    A fourth perturbation, chips clickable while following, was caught by
    nothing because the follow guard also drops every edit: two guards,
    one perturbed.
  B (10 failures): SetGrade not applied (palette, the lane's grade count,
    strength, and the paused-frame check, which first passed on this binary
    through an autosave's notice card over the preview, mean 3.02, until the
    cards were masked: 0.00); the glide refusal ignored (drawn enabled, the
    press glided); a style rename without its cascade (the line lost its
    style); Add a title card never blocked (it offered itself over a card).

D2 NEGATIVE CONTROLS (2026-11-14, GPU path), each a perturbed debug build
copied aside, run as LOOK_CARDS_BIN, the file restored with `git checkout`
and `git status` shown clean:
  15a (2 failures): `look_cards_frame` without `forget()`: the late Enter
    committed STALE into the other project's title card 0 and autosaved it
    (the review's defect 1 reproduced).
  15b (2 failures): the composition root's owner check made `true ||`: the
    same-frame Enter wrote STALE into the other project (`stale=0`).
  16 (2 failures, title and voice armed=…-0 in the other project): every
    guard off at once, which is what it took: the arm is cleared by the
    project change directly and again through the section-change reset
    `forget()` triggers, it names its row by what it holds, and it lapses a
    frame its row is not drawn. Any one of the first two alone kept the
    check green; with only the row's contents left the stored arm survived
    (reported `armed=title-0`) although the other project's row read
    Remove, so the check pins the stored state, not the label.
  17a (1 failure): the cut tended only while Look is the view: with the
    column on Lyrics it played on to 7.89 s.
  17b (1 failure): the cut kept across a project change: the other track
    was paused at 4.13, the first track's stop.
  17c (1 failure): the user's seek ignored: the cut paused at 4.13 after
    the Right arrow.
  18a (2 failures): the lane's show-on-select seek removed: section=2 and
    a real lane press both left the playhead at 7.000.
  18b (1 failure): Show N pressed but inert: the playhead played on to 5.53.
  19/20a (3 failures): a resting row's press and This line made inert:
    the voice did not open; alto did not take the line; tenor did not
    give it back.
  20b (1 failure): Add a voice adding a second "alto": the model refused
    it and the table stayed alto, tenor.
  21 (4 failures on the GPU and 4 on the CPU path, MZ_GL_LAUNCH=""): the
    glyph drawing made a no-op: 0 inked pixels in each of the 14 squares;
    the real build passed on both renderers.
  22 (2 failures): `card_typing()` made false: "1.25" typed with real keys
    picked Song Atlas; "To Room 4" never reached the title: its T closed
    Look and its space played (playhead 3.6).
  20c/23 (1 failure each, one binary): This line never drawn chosen: the
    marker of the voice that sings the line went missing; `card_hint`
    passing the chip's own words while following: Flash's description.
  Found on the way: the first sequence ran 120 frames and passed against
  both perturbations, because autosave settles 1.5 s after an edit and the
  file was read before it; the control (no drop, must commit) caught that.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path


def drive():
    """Run ONLY as MZ_BIN under headless_run.sh (LOOK_CARDS_DRIVE set): start
    the real binary, then press and type into it with native XTest events,
    the steps the gate wrote into LOOK_CARDS_DRIVE, and close it."""
    import ctypes as C
    assert os.environ['WAYLAND_DISPLAY'] == 'musializer-headless-run-no-compositor'
    assert os.environ['PULSE_SERVER'] == 'unix:/nonexistent/musializer-headless-run-no-audio'
    process = subprocess.Popen([os.environ['LOOK_CARDS_REAL_BIN'], *sys.argv[1:]])
    try:
        x = C.CDLL('libX11.so.6')
        xt = C.CDLL('libXtst.so.6')
        x.XOpenDisplay.restype = C.c_void_p
        d = x.XOpenDisplay(None)
        assert d
        x.XDefaultRootWindow.argtypes = [C.c_void_p]
        x.XDefaultRootWindow.restype = C.c_ulong
        x.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong),
                                 C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]
        x.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
        x.XKeysymToKeycode.argtypes = [C.c_void_p, C.c_ulong]
        x.XKeysymToKeycode.restype = C.c_uint
        x.XFlush.argtypes = [C.c_void_p]
        xt.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        xt.XTestFakeMotionEvent.argtypes = [C.c_void_p, C.c_int, C.c_int, C.c_int, C.c_ulong]
        xt.XTestFakeButtonEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        # An X error (a window that went away) must not end the driver.
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
        root = x.XDefaultRootWindow(d)
        window = None
        # The app's own window: mapped (IsViewable) and window-sized.
        for _ in range(150):
            assert process.poll() is None, 'app ended before its window'
            children = C.POINTER(C.c_ulong)()
            count, parent, returned = C.c_uint(), C.c_ulong(), C.c_ulong()
            if x.XQueryTree(d, root, C.byref(returned), C.byref(parent), C.byref(children), C.byref(count)):
                for index in range(count.value):
                    attributes = Attributes()
                    if (x.XGetWindowAttributes(d, children[index], C.byref(attributes))
                            and attributes.map_state == 2 and attributes.width >= 640):
                        window = children[index]
            if window:
                break
            time.sleep(.1)
        assert window, 'no mapped window'
        # The fixture loads in well under this; keys sent while it decodes
        # would be lost.
        time.sleep(2.0)
        x.XSetInputFocus(d, window, 2, 0)
        x.XFlush(d)

        def key(sym, down):
            xt.XTestFakeKeyEvent(d, x.XKeysymToKeycode(d, sym), down, 0)
            x.XFlush(d)
            time.sleep(.05)

        def press(sym, shift=False):
            if shift:
                key(0xffe1, 1)
            key(sym, 1)
            key(sym, 0)
            if shift:
                key(0xffe1, 0)
        named = {'enter': 0xff0d, 'escape': 0xff1b, 'space': 0x20}
        for step in json.loads(os.environ['LOOK_CARDS_DRIVE']):
            assert process.poll() is None, f'app exited at {step}'
            if 'click' in step:
                xt.XTestFakeMotionEvent(d, 0, *step['click'], 0)
                x.XFlush(d)
                time.sleep(.1)
                xt.XTestFakeButtonEvent(d, 1, 1, 0)
                x.XFlush(d)
                time.sleep(.1)
                xt.XTestFakeButtonEvent(d, 1, 0, 0)
                x.XFlush(d)
                time.sleep(.25)
            elif 'chord' in step:
                key(0xffe3, 1)
                press(ord(step['chord']))
                key(0xffe3, 0)
            elif 'type' in step:
                for char in step['type']:
                    if char == ' ':
                        press(0x20)
                    else:
                        press(ord(char.lower()), shift=char.isupper())
            elif 'key' in step:
                press(named[step['key']])
            elif 'wait' in step:
                time.sleep(step['wait'])

        class ClientMessage(C.Structure):
            _fields_ = [('type', C.c_int), ('serial', C.c_ulong), ('send_event', C.c_int), ('display', C.c_void_p),
                        ('window', C.c_ulong), ('message_type', C.c_ulong), ('format', C.c_int), ('data', C.c_long * 5)]
        x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
        x.XInternAtom.restype = C.c_ulong
        x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.c_void_p]
        event = (C.c_long * 24)()
        message = C.cast(event, C.POINTER(ClientMessage)).contents
        message.type, message.send_event, message.display = 33, 1, d
        message.window = window
        message.message_type = x.XInternAtom(d, b'WM_PROTOCOLS', 0)
        message.format = 32
        message.data[0] = x.XInternAtom(d, b'WM_DELETE_WINDOW', 0)
        x.XSendEvent(d, window, 0, 0, C.byref(event))
        x.XFlush(d)
        print('look-cards-input: native steps delivered', flush=True)
        return process.wait(timeout=60)
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)


if os.environ.get('LOOK_CARDS_DRIVE'):
    sys.exit(drive())

out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
binary = os.environ.get('LOOK_CARDS_BIN') or os.environ.get('MZ_BIN') or str(Path('target/debug/musializer').resolve())
env = dict(os.environ, MZ_BIN=binary)
only = os.environ.get('LOOK_CARDS_ONLY', 'all')
failures = []


def check(ok, message):
    print(('PASS: ' if ok else 'FAIL: ') + message, flush=True)
    if not ok:
        failures.append(message)


subprocess.run(['cargo', 'run', '--quiet', '--bin', 'make-fixture-wav', '--', str(out / 'fixture.wav'), '8'],
               check=True, stdout=subprocess.DEVNULL)
base_show = {
    'schema': 'musializer.treatment/v1', 'title': 'Look cards fixture', 'audio': {'path': 'fixture.wav'},
    'caption_styles': {'top': {'anchor': 'top_center'}, 'shout': {'size_scale': 0.085}},
    'segments': [
        {'at': 0, 'scene': 'spectrum', 'transition': {'kind': 'fade', 'seconds': 1.0}},
        {'at': 2.5, 'scene': 'spectrum', 'transition': 'fade'},
        {'at': 4.0, 'scene': 'spectrum', 'grade': 'night-door', 'glide': 1.0},
        {'at': 5.5, 'scene': 'cadence'},
        {'at': 6.5, 'scene': 'threshold', 'settings': {'beyond': 7}},
    ],
    'outro': {'kind': 'fade', 'seconds': 1.6},
}
words_show = dict(base_show,
                  artist='Fixture Artist',
                  voices={'alto': {'hue': 253, 'text': '#e9e3ff'}, 'tenor': {'hue': 33}},
                  lyrics=[{'at': 1.0, 'until': 2.0, 'text': 'the first line', 'voice': 'alto'},
                          {'at': 2.6, 'until': 3.6, 'text': 'the loud line', 'voice': 'tenor', 'style': 'shout'}],
                  titles=[{'at': 0.5, 'until': 2.0, 'title': 'Fixture Title'}])
# A second project for the sequences (D2): the same song with words of its
# own and a title card 0 of its own, so a stale draft or confirmation from
# `words` has a row of the same index to land on.
other_show = dict(words_show, title='Other fixture',
                  titles=[{'at': 0.5, 'until': 2.0, 'title': 'Other Title'}])
films = {'plain': base_show, 'film': dict(base_show, finish='film'), 'words': words_show, 'other': other_show}
if only in ('all', 'clip', 'captures'):
    # The clip rows (F1c): FFmpeg's testsrc2 burns its frame number into
    # every frame; made here, never committed. a.mp4 30 fps 6 s, b.mp4 square
    # 25 fps 4 s, c.mp4 for an import.
    for name, source, seconds in [('a', 'testsrc2=size=640x360:rate=30', 6), ('b', 'testsrc2=size=360x360:rate=25', 4),
                                  ('c', 'testsrc2=size=480x270:rate=24', 3)]:
        subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-y', '-f', 'lavfi', '-i', source, '-t', str(seconds),
                        '-c:v', 'libx264', '-g', '30', '-pix_fmt', 'yuv420p', str(out / f'{name}.mp4')], check=True)
    clip_segments = [
        {'at': 0, 'scene': 'spectrum'},
        {'at': 2, 'scene': 'clip', 'clip': {'path': 'a.mp4', 'from': 1}},
        {'at': 4, 'scene': 'clip', 'clip': 'a.mp4', 'transition': {'kind': 'fade', 'seconds': 0.6}},
        {'at': 6, 'scene': 'clip', 'clip': {'path': 'b.mp4', 'after': 'loop', 'fit': 'contain'}},
    ]
    films['clip'] = dict({key: value for key, value in base_show.items() if key != 'outro'},
                         title='Clip fixture', segments=clip_segments)
    # Section 3 with an own start where the clock already is (1 + 2 s).
    films['clipown'] = dict(films['clip'], segments=[dict(segment) for segment in clip_segments])
    films['clipown']['segments'][2]['clip'] = {'path': 'a.mp4', 'from': 3}
for name, show in films.items():
    (out / f'{name}.treatment.json').write_text(json.dumps(show))
    with (out / f'seed-{name}.log').open('w') as log:
        subprocess.run(['tools/headless_run.sh', '--treatment', str(out / f'{name}.treatment.json'),
                        '--save-project', str(out / f'{name}.musi')],
                       env=dict(env, MZ_STATE_DIR=str(out / f'state-seed-{name}')),
                       stdout=log, stderr=subprocess.STDOUT, check=True)


def line(log, label):
    found = re.findall(r'^' + re.escape(label) + r':\s*(.*)$', log, re.M)
    if not found:
        raise AssertionError(f'missing {label}:')
    return found[-1]


def copy_project(project, work):
    # The .musi names its bundled audio by its own stem: keep the names.
    shutil.copy(out / f'{project}.musi', work / f'{project}.musi')
    assets = out / f'{project}.assets'
    if assets.exists():
        shutil.copytree(assets, work / f'{project}.assets')


def run(name, probe, frames=150, project='plain', theme='dark', follow=False, also=(), drive=None, prepare=None):
    """One probe run on a fresh copy of a seeded project. Returns (log, saved).

    `also` names projects copied beside it, for a `drop=` of a second one;
    `prepare(work)` changes the copy before the launch."""
    work = out / name
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    for other in also:
        copy_project(other, work)
    if follow:
        shutil.copy(out / 'fixture.wav', work / 'fixture.wav')
        shutil.copy(out / f'{project}.treatment.json', work / 'show.treatment.json')
        for clip_file in out.glob('*.mp4'):
            shutil.copy(clip_file, work / clip_file.name)
        source = ['--treatment', str(work / 'show.treatment.json')]
    else:
        copy_project(project, work)
        source = ['--project', str(work / f'{project}.musi')]
    if prepare is not None:
        prepare(work)
    cmd = ['tools/headless_run.sh', *source, '--ui-scale', '100', '--hud=0', '--ui-theme', theme,
           '--ui-probe', probe, '--probe-frames', str(frames), '--probe-shot', str(work / 'shot.png')]
    run_env = dict(env, MZ_STATE_DIR=str(work / 'state'))
    if drive is not None:
        # Native keys and presses (D2): this file, as MZ_BIN, starts the real
        # binary and drives it with XTest.
        run_env.update(MZ_BIN=str(Path(__file__).resolve()), LOOK_CARDS_REAL_BIN=binary,
                       LOOK_CARDS_DRIVE=json.dumps(drive))
    result = subprocess.run(cmd, env=run_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    (work / 'log.txt').write_text(result.stdout)
    if result.returncode:
        raise AssertionError(f'{name}: exit {result.returncode}: {result.stdout[-1500:]}')
    saved = json.loads((work / f'{project}.musi').read_text()) if not follow else {}
    return result.stdout, saved


def cards(log):
    return line(log, 'look cards')


def rect(report, name):
    match = re.search(r'(?:^|[ ;=])' + re.escape(name) + r'@(?:disabled:)?(-?\d+),(-?\d+),(\d+),(\d+)', report)
    if not match:
        raise AssertionError(f'{name} missing: {report[:300]}')
    return tuple(map(int, match.groups()))


def centre(r):
    return (r[0] + r[2] // 2, r[1] + r[3] // 2)


def field(report, key):
    match = re.search(r'(?:^| )' + re.escape(key) + r'=(\S+)', report)
    return match[1] if match else None


def click(point):
    return f',click={point[0]}x{point[1]}'


def claimed_nothing(log):
    return 'claimed=nothing' in line(log, 'click probe')


def cue(saved, index):
    return saved['scene_switches']['cues'][index]


SECTION = 'size=1280x720,column=look,time=3,play=0'
original = {name: json.loads((out / f'{name}.musi').read_text()) for name in films}


def unchanged(saved, project='plain'):
    return saved['scene_switches'] == original[project]['scene_switches']


def transition_seconds(unit):
    # core::ui::tune_groups::look_cards::transition_seconds, restated.
    seconds = 0.05 * (30.0 / 0.05) ** min(max(unit, 0.0), 1.0)
    return min(max(round(seconds * 100) / 100, 0.05), 30.0)


def glide_seconds(unit):
    unit = min(max(unit, 0.0), 1.0)
    return min(max(round(30.0 * unit * unit * 20) / 20, 0.0), 30.0)


def at_fraction(bar, fraction):
    x = bar[0] + round(bar[2] * fraction)
    return (x, bar[1] + bar[3] // 2), (x - bar[0]) / bar[2]


if only in ('all', 'section'):
    s2, _ = run('base-s2', SECTION + ',section=2', frames=30)
    s3, _ = run('base-s3', SECTION + ',section=3', frames=30)
    s4, _ = run('base-s4', SECTION + ',section=4', frames=30)
    b2, b3, b4 = cards(s2), cards(s3), cards(s4)
    check(field(b2, 'target') == '2' and field(b2, 'kind') == 'fade' and field(b2, 'seconds') == '0.60',
          f'section 2 selected by section=2 arrives by fade 0.60: {b2[:120]}')

    # 1. kinds
    # Long enough for the 3.1 s cut it plays to end and pause.
    log, saved = run('kind-flash', SECTION + ',section=2' + click(centre(rect(b2, 'kind-flash'))), frames=420)
    check(field(cards(log), 'kind') == 'flash', 'kind chip: flash reaches the report')
    playhead = float(field(line(log, 'transport'), 'playhead'))
    check(4.1 <= playhead <= 4.7 and 'playing=false' in line(log, 'resume'),
          f'a kind picked while paused plays its cut and pauses after it: playhead {playhead:.3f}')
    check(cue(saved, 1)['transition']['kind'] == 'flash' and abs(cue(saved, 1)['transition']['seconds'] - 0.6) < 1e-6,
          f"kind chip: the autosaved .musi says flash 0.6: {cue(saved, 1).get('transition')}")
    r = rect(b2, 'kind-flash')
    log, saved = run('kind-gap', SECTION + ',section=2' + click((r[0] + r[2] + 2, r[1] + r[3] // 2)))
    check(field(cards(log), 'kind') == 'fade' and unchanged(saved) and claimed_nothing(log),
          'kind gap: the 4 px between two chips changes nothing')
    before_runs, before_marks = line(s4, 'scene runs'), line(s4, 'scene marks')
    log, saved = run('kind-wipe-s4', SECTION + ',section=4' + click(centre(rect(b4, 'kind-wipe'))))
    check(field(cards(log), 'kind') == 'wipe' and cue(saved, 3).get('transition', {}).get('kind') == 'wipe',
          'kind chip on a cut: section 4 now arrives by wipe, saved')
    check(field(line(log, 'scene runs'), 'arriving') == str(int(field(before_runs, 'arriving')) + 1)
          and field(line(log, 'scene marks'), 'transitions') == str(int(field(before_marks, 'transitions')) + 1),
          f"the lane follows: arriving {field(before_runs, 'arriving')} -> {field(line(log, 'scene runs'), 'arriving')}")

    # 2. length
    bar = rect(b2, 'length')
    point, unit = at_fraction(bar, 0.25)
    expected = transition_seconds(unit)
    log, saved = run('length', SECTION + ',section=2' + click(point))
    check(field(cards(log), 'seconds') == f'{expected:.2f}', f'length track at {unit:.3f}: exactly {expected:.2f} s ({field(cards(log), "seconds")})')
    check(abs(cue(saved, 1)['transition']['seconds'] - expected) < 1e-6, 'length: saved')
    row_top = bar[1] + bar[3] // 2 - 15
    log, saved = run('length-gap', SECTION + ',section=2' + click((point[0], row_top - 3)))
    check(field(cards(log), 'seconds') == '0.60' and unchanged(saved), 'length gap: the space above the row changes nothing')
    value = centre(rect(b2, 'length-value'))
    log, saved = run('length-typed', SECTION + ',section=2' + click(value) + ',look-text=1.25,look-action=commit')
    check(field(cards(log), 'seconds') == '1.25' and field(cards(log), 'typing') == 'none'
          and abs(cue(saved, 1)['transition']['seconds'] - 1.25) < 1e-6, 'typed length 1.25 commits and saves')
    log, saved = run('length-cancel', SECTION + ',section=2' + click(value) + ',look-text=9,look-action=cancel')
    check(field(cards(log), 'seconds') == '0.60' and unchanged(saved), 'Escape keeps the old length')
    log, _ = run('length-nofocus', SECTION + ',section=2' + click((point[0], row_top - 3)) + ',look-text=9,look-action=commit', frames=30)
    check('probe=refused-no-focus' in cards(log) and field(cards(log), 'seconds') == '0.60',
          'typing with nothing focused is refused and says so')

    # 3. glide
    bar = rect(b2, 'glide')
    point, unit = at_fraction(bar, 0.5)
    expected = glide_seconds(unit)
    log, saved = run('glide', SECTION + ',section=2' + click(point))
    check(field(cards(log), 'glide') == f'{expected:.2f}' and abs(cue(saved, 1).get('glide', 0) - expected) < 1e-6,
          f'glide track at {unit:.3f}: exactly {expected:.2f} s, saved')
    log, saved = run('glide-gap', SECTION + ',section=2' + click((point[0], bar[1] + bar[3] // 2 + 15 + 5)))
    check(field(cards(log), 'glide') == '0.00' and unchanged(saved), 'glide gap: the space under the row changes nothing')
    check('glide@disabled:' in b4, 'glide after another scene is drawn disabled')
    log, saved = run('glide-disabled', SECTION + ',section=4' + click(centre(rect(b4, 'glide'))))
    check(field(cards(log), 'glide') == '0.00' and unchanged(saved), 'a press on the disabled glide changes nothing')

    # 4. palettes, and the frame on screen
    before_graded = field(line(s2, 'scene runs'), 'graded')
    log, saved = run('palette', SECTION + ',section=2' + click(centre(rect(b2, 'palette-ember'))))
    check(field(cards(log), 'grade') == 'ember:0.82:125,0,32' and cue(saved, 1).get('grade', {}).get('map') is not None,
          f"palette chip: ember at 0.82, saved ({field(cards(log), 'grade')})")
    check(field(line(log, 'scene runs'), 'graded') == str(int(before_graded) + 1), 'the lane counts the new grade')
    r = rect(b2, 'palette-ember')
    gap_log, saved_gap = run('palette-gap', SECTION + ',section=2' + click((r[0] - 2, r[1] + r[3] // 2)))
    check(field(cards(gap_log), 'grade') == 'none' and unchanged(saved_gap) and claimed_nothing(gap_log), 'palette gap changes nothing')
    try:
        from PIL import Image, ImageChops, ImageStat
        frame = re.search(r'pixels (\d+),(\d+),(\d+),(\d+)', line(log, 'preview frame')).groups()
        x, y, w, h = map(int, frame)
        box = (x, y, x + w, y + h)

        # Notice cards sit over the preview, and an edit's autosave can post
        # one (the recovery warning) that the gap run does not: blank every
        # card either run drew, in both, so only the picture is compared.
        cards_drawn = []
        for name in ('palette', 'palette-gap', 'kind-gap'):
            notices = line((out / name / 'log.txt').read_text(), 'notices')
            cards_drawn += [tuple(map(int, m)) for m in re.findall(r'card=(\d+),(\d+),(\d+),(\d+)', notices)]

        def shot(name):
            image = Image.open(out / name / 'shot.png').convert('RGB')
            for cx, cy, cw, ch in cards_drawn:
                image.paste((0, 0, 0), (cx - 8, cy - 8, cx + cw + 8, cy + ch + 8))
            return image.crop(box)
        graded = sum(ImageStat.Stat(ImageChops.difference(shot('palette'), shot('palette-gap'))).mean) / 3
        same = sum(ImageStat.Stat(ImageChops.difference(shot('kind-gap'), shot('palette-gap'))).mean) / 3
        # Calibrated 2026-11-14 on the GPU path with the cards masked: Ember
        # over the fixture's dark Spectrum 3.46, the grade dropped 0.00.
        check(graded > 1.5 and graded > 4 * same + 0.5,
              f'paused, the graded frame differs from the ungraded one at once (mean {graded:.2f}; two ungraded {same:.2f})')
    except ImportError:
        check(False, 'Pillow is needed for the frame check')

    # 5. strength
    bar = rect(b3, 'map')
    point, unit = at_fraction(bar, 0.5)
    expected = round(unit * 100) / 100
    log, saved = run('map', SECTION + ',section=3' + click(point))
    check(field(cards(log), 'grade') == f'night-door:{expected:.2f}:125,0,32', f"strength track: exactly {expected:.2f} ({field(cards(log), 'grade')})")
    check(abs(cue(saved, 2)['grade']['map'] - expected) < 1e-6, 'strength: saved')
    log, saved = run('map-gap', SECTION + ',section=3' + click((point[0], bar[1] + bar[3] // 2 - 15 - 3)))
    check(field(cards(log), 'grade') == 'night-door:0.82:125,0,32' and unchanged(saved), 'strength gap changes nothing')

    # 6. caption style
    log, saved = run('style', SECTION + ',section=2' + click(centre(rect(b2, 'style-top'))))
    check(field(cards(log), 'style') == 'top' and cue(saved, 1).get('caption_style') == 'top', 'style chip: section 2 begins its lines in top, saved')
    r = rect(b2, 'style-top')
    log, saved = run('style-gap', SECTION + ',section=2' + click((r[0] - 2, r[1] + r[3] // 2)))
    check(field(cards(log), 'style') == 'base' and unchanged(saved), 'style gap changes nothing')

    # 7. play the cut, from after the section
    play = rect(b2, 'play')
    log, _ = run('play', 'size=1280x720,column=look,time=7,play=0,section=2' + click(centre(play)), frames=420)
    playhead = float(field(line(log, 'transport'), 'playhead'))
    check(4.1 <= playhead <= 4.7 and 'playing=false' in line(log, 'resume') and field(cards(log), 'cut') == 'none',
          f'Play the cut seeks to 1.0, plays through the fade and pauses after 4.1: playhead {playhead:.3f}')
    log, _ = run('play-gap', 'size=1280x720,column=look,time=7,play=0,section=2' + click((play[0] - 3, play[1] + play[3] // 2)), frames=60)
    # Selecting section 2 put the playhead at 3.15, just past its arrival (18).
    check(abs(float(field(line(log, 'transport'), 'playhead')) - 3.15) < 0.01, 'the gap beside Play the cut leaves the playhead where the selection put it')

    # 8. done, invitation, scope
    log, _ = run('done', SECTION + ',section=2' + click(centre(rect(b2, 'done'))), frames=30)
    check(field(cards(log), 'target') == 'none' and 'invite@' in cards(log), 'Done lets the section go; the invitation shows')
    none_log, _ = run('base-none', SECTION, frames=30)
    log, _ = run('invite', SECTION + click(centre(rect(cards(none_log), 'invite'))), frames=30)
    check(field(cards(log), 'target') == '2', 'the invitation selects the section under the playhead (2 at 3 s)')
    log, _ = run('scope', SECTION + ',section=2' + click(centre(rect(b2, 'scope-1'))), frames=30)
    check(field(cards(log), 'scope') == 'video', 'the scope switch shows the video card')
    r = rect(b2, 'scope-1')
    log, _ = run('scope-gap', SECTION + ',section=2' + click((r[0] + r[2] // 2, r[1] - 6)), frames=30)
    check(field(cards(log), 'scope') == 'section', 'the gap above the switch changes nothing')

    # 9. the lane's name field opens Look
    direct, _ = run('base-direct', 'size=1280x720,column=direct,time=3,play=0,section=2', frames=30)
    head = line(direct, 'lane head')
    name = tuple(map(int, re.search(r'name=(\d+),(\d+),(\d+),(\d+)', head).groups()))
    log, _ = run('name-field', 'size=1280x720,column=direct,time=3,play=0,section=2' + click(centre(name)), frames=30)
    check(field(line(log, 'column'), 'view') == 'look' and field(cards(log), 'target') == '2',
          'the selected section\'s name field opens its card in Look')
    log, _ = run('name-gap', 'size=1280x720,column=direct,time=3,play=0,section=2' + click((name[0] + name[2] + 3, name[1] + name[3] // 2)), frames=30)
    check(field(line(log, 'column'), 'view') == 'direct', 'the gap beside the name field leaves Direct')

    # 9b. a threshold section's door, by name
    s5, _ = run('base-s5', SECTION + ',section=5', frames=30)
    b5 = cards(s5)
    check(field(b5, 'beyond') == 'constellation' and 'beyond-next@' in b5, 'a threshold section shows its door onto Constellation')
    log, saved = run('beyond-next', SECTION + ',section=5' + click(centre(rect(b5, 'beyond-next'))))
    check(field(cards(log), 'beyond') not in ('constellation', 'fog', None)
          and cue(saved, 4).get('settings') != cue(original['plain'], 4).get('settings'),
          f"the door's next scene: {field(cards(log), 'beyond')}, saved in the section's tuning")
    log, saved = run('beyond-fog', SECTION + ',section=5' + click(centre(rect(b5, 'beyond-fog'))))
    check(field(cards(log), 'beyond') == 'fog', 'Fog: the doorway shows its own fog')
    r = rect(b5, 'beyond-previous')
    log, saved = run('beyond-gap', SECTION + ',section=5' + click((r[0] + r[2] + 2, r[1] + r[3] // 2)))
    check(field(cards(log), 'beyond') == 'constellation' and unchanged(saved), 'the gap beside the door stepper changes nothing')

    # 10. digits typed into a value do not select a scene
    def scene_of(log):
        return field(line(log, 'tune layout'), 'scene')
    log, _ = run('digit-typing', SECTION + ',section=2' + click(value) + ',key=4', frames=80)
    check(scene_of(log) == 'spectrum' and field(cards(log), 'typing') == 'length',
          f'a 4 typed into the length value keeps the scene: {scene_of(log)}')
    # The control presses the scene's name in Look's header: not a control.
    log, _ = run('digit-control', SECTION + ',section=2' + click((1000, 112)) + ',key=4', frames=80)
    check(scene_of(log) != 'spectrum', f'control: the same 4 with no field open picks a scene ({scene_of(log)})')

if only in ('all', 'video'):
    # Tall enough that the whole card is inside the list without a scroll.
    VIDEO = 'size=1280x1100,column=look,time=3,play=0,look=video'
    v, _ = run('base-video', VIDEO, frames=30)
    bv = cards(v)
    check(field(bv, 'scope') == 'video' and field(bv, 'intro') == 'fade:1.00' and field(bv, 'outro') == 'fade:1.60',
          f'look=video shows the video card: {bv[:160]}')
    log, saved = run('outro-iris', VIDEO + click(centre(rect(bv, 'outro-iris'))))
    check(field(cards(log), 'outro') == 'iris:1.60' and saved['scene_switches']['outro']['kind'] == 'iris'
          and field(line(log, 'scene marks'), 'outro') == 'iris', 'outro chip: iris, saved, and the lane says so')
    r = rect(bv, 'outro-iris')
    log, saved = run('outro-gap', VIDEO + click((r[0] + r[2] + 2, r[1] + r[3] // 2)))
    check(field(cards(log), 'outro') == 'fade:1.60' and unchanged(saved) and claimed_nothing(log), 'outro gap changes nothing')
    log, saved = run('intro-flash', VIDEO + click(centre(rect(bv, 'intro-flash'))))
    check(field(cards(log), 'intro') == 'flash:1.00' and cue(saved, 0)['transition']['kind'] == 'flash', 'intro chip: the first section opens by flash, saved')
    bar = rect(bv, 'outro-length')
    point, unit = at_fraction(bar, 0.75)
    expected = transition_seconds(unit)
    log, saved = run('outro-length', VIDEO + click(point))
    check(field(cards(log), 'outro') == f'fade:{expected:.2f}' and abs(saved['scene_switches']['outro']['seconds'] - expected) < 1e-6,
          f'outro length at {unit:.3f}: exactly {expected:.2f} s, saved')
    log, saved = run('finish-film', VIDEO + click(centre(rect(bv, 'finish-film'))))
    check(field(cards(log), 'finish').startswith('0.45,0.74,0.016,0.85,0.50,0.45,0.40,')
          and saved['scene_switches'].get('finish', {}).get('bloom') == 0.45, 'Film: the film finish, saved')
    fv, _ = run('base-video-film', VIDEO, frames=30, project='film')
    log, saved = run('finish-none', VIDEO + click(centre(rect(cards(fv), 'finish-none'))), project='film')
    check(field(cards(log), 'finish').startswith('0.00,0.74,0.012,0.00,0.00,0.00,0.00,') and 'finish' not in saved['scene_switches'],
          'None: no finish, and none written')
    r = rect(bv, 'finish-film')
    log, saved = run('finish-gap', VIDEO + click((r[0] - 2, r[1] + r[3] // 2)))
    check(unchanged(saved) and claimed_nothing(log), 'finish gap changes nothing')
    bar = rect(bv, 'vignette')
    point, unit = at_fraction(bar, 0.75)
    expected = round(unit * 100) / 100
    log, saved = run('vignette', VIDEO + click(point))
    check(abs(saved['scene_switches'].get('finish', {}).get('vignette', -1) - expected) < 1e-6,
          f'vignette track: exactly {expected:.2f}, saved')
    # The toning colours and the bloom's shape act only with their strength
    # up: on the plain fixture they are drawn disabled and a press does nothing.
    check('shadow@disabled:' in bv and 'bloom-threshold@disabled:' in bv, 'with toning and bloom off their colours and shape are disabled')
    log, saved = run('shadow-off', VIDEO + click(centre(rect(bv, 'shadow'))))
    check(unchanged(saved) and field(cards(log), 'finish') == field(bv, 'finish'), 'a press on the disabled shadow tone changes nothing')
    bf = cards(fv)
    log, saved = run('shadow', VIDEO + click(centre(rect(bf, 'shadow-2'))), project='film')
    check('#2a1030' in field(cards(log), 'finish') and saved['scene_switches'].get('finish', {}).get('shadow_rgb') == '2a1030',
          'a shadow swatch sets the toning shadow, saved')
    r = rect(bf, 'shadow-2')
    log, saved = run('shadow-gap', VIDEO + click((r[0] - 2, r[1] + r[3] // 2)), project='film')
    check(unchanged(saved, 'film'), 'the gap between two swatches changes nothing')
    log, saved = run('highlight-typed', VIDEO + click(centre(rect(bf, 'highlight-value'))) + ',look-text=#123456,look-action=commit', project='film')
    check(field(cards(log), 'finish').endswith('#123456') and saved['scene_switches'].get('finish', {}).get('highlight_rgb') == '123456',
          'a typed highlight colour commits and saves')

if only in ('all', 'tables'):
    WORDS = 'size=1280x1440,column=look,time=3,play=0,look=captions'
    w, saved0 = run('base-words', WORDS, frames=30, project='words')
    bw = cards(w)
    styles0 = [entry['name'] for entry in original['words']['caption_style']['styles']]
    shout = styles0.index('shout')
    top = styles0.index('top')
    # D2: every entry rests as one row; the open one shows its controls.
    check(field(bw, 'scope') == 'captions' and field(bw, 'open') == 'none'
          and all(f'entry-{name}@' in bw for name in ('voice-0', 'voice-1', f'style-{shout}', 'title-0'))
          and 'voice-hue-0@' not in bw and 'title-text-0@' not in bw,
          'look=captions shows the three tables, every entry resting as one row')
    bases = {}

    def entry(name):
        """The captions card with entry NAME open, and the spec that opens it."""
        if name not in bases:
            log, _ = run(f'base-words-{name}', WORDS + f',look-entry={name}', frames=30, project='words')
            bases[name] = cards(log)
        return bases[name], WORDS + f',look-entry={name}'

    def words(saved):
        return saved['lyrics'], saved['caption_style']['styles'], saved['scene_switches'].get('titles', [])

    def same(saved):
        return words(saved) == words(original['words'])

    # 19. a press on a resting row opens it (and only it); the gap between
    #     two rows opens nothing; the fold closes it.
    log, _ = run('entry-open', WORDS + click(centre(rect(bw, 'entry-voice-1'))), frames=30, project='words')
    check(field(cards(log), 'open') == 'voice-1' and 'voice-hue-1@' in cards(log) and 'voice-hue-0@' not in cards(log),
          'a press on a resting voice opens it, and only it')
    r0, r1 = rect(bw, 'entry-voice-0'), rect(bw, 'entry-voice-1')
    log, _ = run('entry-gap', WORDS + click((r0[0] + r0[2] // 2, (r0[1] + r0[3] + r1[1]) // 2)), frames=30, project='words')
    check(field(cards(log), 'open') == 'none' and claimed_nothing(log), 'the gap between two resting rows opens nothing')
    bv1, spec_v1 = entry('voice-1')
    log, _ = run('entry-fold', spec_v1 + click(centre(rect(bv1, 'fold-voice-1'))), frames=30, project='words')
    check(field(cards(log), 'open') == 'none' and 'entry-voice-1@' in cards(log), 'the fold on an open entry lays it to rest')

    bv0, spec_v0 = entry('voice-0')
    bar = rect(bv0, 'voice-hue-0')
    point, unit = at_fraction(bar, 0.5)
    log, saved = run('voice-hue', spec_v0 + click(point), project='words')
    check(abs(words(saved)[0]['voices'][0]['hue'] - round(unit * 360)) < 1e-6, f'voice hue track: exactly {round(unit * 360)} degrees, saved')
    log, saved = run('voice-hue-gap', spec_v0 + click((point[0], bar[1] + bar[3] // 2 - 15 - 3)), project='words')
    check(same(saved), 'voice hue gap changes nothing')
    log, saved = run('voice-ink', spec_v1 + click(centre(rect(bv1, 'voice-ink-1-3'))), project='words')
    check(words(saved)[0]['voices'][1].get('text_rgba') == 'c8f0ffff', f"a voice ink swatch, saved: {words(saved)[0]['voices'][1]}")
    log, saved = run('voice-name', spec_v0 + click(centre(rect(bv0, 'voice-name-0'))) + ',look-text=mezzo,look-action=commit', project='words')
    lyrics = words(saved)[0]
    check(lyrics['voices'][0]['name'] == 'mezzo' and lyrics['cues'][0].get('voice') == 'mezzo',
          'a renamed voice keeps its lines')
    log, saved = run('voice-remove', spec_v0 + click(centre(rect(bv0, 'voice-name-0-button'))), project='words')
    check(same(saved) and 'voice-0-armed:1-lines' in cards(log), 'removing a voice that sings lines first asks to confirm')
    # 20. Add a voice (D2): a new voice with a hue of its own, open; at 3 s
    #     the line sung is "the loud line" (tenor): tenor's This line is drawn
    #     chosen and alto's is not; alto's takes the line, tenor's gives it
    #     back to no voice.
    log, saved = run('voice-add', WORDS + click(centre(rect(bw, 'add-voice'))), project='words')
    voices = words(saved)[0]['voices']
    check([v['name'] for v in voices] == ['alto', 'tenor', 'voice-1'] and field(cards(log), 'open') == 'voice-2',
          f"Add a voice adds voice-1, open for editing ({[v['name'] for v in voices]}, open={field(cards(log), 'open')})")
    r = rect(bw, 'add-voice')
    log, saved = run('voice-add-gap', WORDS + click((r[0] + r[2] + 4, r[1] + r[3] // 2)), project='words')
    check(same(saved), 'the gap beside Add a voice changes nothing')
    check('voice-1-line-sung' in bv1 and 'voice-0-line-sung' not in bv0,
          'the voice that sings the line at the playhead has This line chosen; the other does not')
    log, saved = run('voice-line-give', spec_v0 + click(centre(rect(bv0, 'voice-0-line'))), project='words')
    check(words(saved)[0]['cues'][1].get('voice') == 'alto' and words(saved)[0]['cues'][0].get('voice') == 'alto',
          f"alto's This line takes the line at the playhead ({words(saved)[0]['cues'][1].get('voice')})")
    log, saved = run('voice-line-take', spec_v1 + click(centre(rect(bv1, 'voice-1-line'))), project='words')
    check('voice' not in words(saved)[0]['cues'][1] and words(saved)[0]['cues'][1]['text'] == 'the loud line',
          f"tenor's chosen This line takes the line back from it ({words(saved)[0]['cues'][1]})")
    a_, b_ = rect(bv0, 'voice-0-line'), rect(bv0, 'voice-0-section')
    log, saved = run('voice-line-gap', spec_v0 + click(((a_[0] + a_[2] + b_[0]) // 2, a_[1] + a_[3] // 2)), project='words')
    check(same(saved), 'the gap between This line and Section N changes nothing')

    bs, spec_s = entry(f'style-{top}')
    log, saved = run('style-anchor', spec_s + click(centre(rect(bs, f'style-{top}-top_left'))), project='words')
    check(words(saved)[1][top].get('anchor') == 'top_left', 'a style anchor chip, saved')
    r = rect(bs, f'style-{top}-top_left')
    log, saved = run('style-anchor-gap', spec_s + click((r[0] + r[2] + 2, r[1] + r[3] // 2)), project='words')
    check(same(saved), 'style anchor gap changes nothing')
    bsh, spec_sh = entry(f'style-{shout}')
    log, saved = run('style-rename', spec_sh + click(centre(rect(bsh, f'style-name-{shout}'))) + ',look-text=loud,look-action=commit', project='words')
    names = [entry_['name'] for entry_ in words(saved)[1]]
    check('loud' in names and 'shout' not in names and words(saved)[0]['cues'][1].get('style') == 'loud',
          'a renamed style takes the line that wore it along')
    check(f'style-name-{shout}-button@disabled:' in bsh, 'a style a line wears cannot be removed (drawn disabled)')
    log, saved = run('style-remove-used', spec_sh + click(centre(rect(bsh, f'style-name-{shout}-button'))), project='words')
    check(same(saved), 'a press on the disabled Remove changes nothing')
    log, saved = run('style-remove', spec_s + click(centre(rect(bs, f'style-name-{top}-button'))), project='words')
    check([entry_['name'] for entry_ in words(saved)[1]] == [n for n in styles0 if n != 'top'], 'an unused style is removed')
    log, saved = run('style-add', WORDS + click(centre(rect(bw, 'add-style'))), project='words')
    check([entry_['name'] for entry_ in words(saved)[1]] == styles0 + ['style-1'] and field(cards(log), 'open') == f'style-{len(styles0)}',
          'Add a style adds style-1, open for editing')
    bt, spec_t = entry('title-0')
    log, saved = run('title-text', spec_t + click(centre(rect(bt, 'title-text-0'))) + ',look-text=Door Fixture,look-action=commit', project='words')
    check(words(saved)[2][0]['title'] == 'Door Fixture', 'a typed title, saved')
    log, saved = run('title-handle', spec_t + click(centre(rect(bt, 'title-handle-0'))) + ',look-text=cats,look-action=commit', project='words')
    check(words(saved)[2][0].get('handle') == '@cats', 'a typed handle gains its @, saved')
    log, saved = run('title-from', spec_t + click(centre(rect(bt, 'title-start-0'))) + ',look-text=0:01.0,look-action=commit', project='words')
    check(words(saved)[2][0]['start_seconds'] == 1.0, 'a typed start time, saved')
    log, saved = run('title-from-bad', spec_t + click(centre(rect(bt, 'title-start-0'))) + ',look-text=3,look-action=commit', project='words')
    check(same(saved) and field(cards(log), 'refused') == 'title-start-0', 'a start after the end is refused and says so')
    log, saved = run('title-style', spec_t + click(centre(rect(bt, 'title-0-style-top'))), project='words')
    check(words(saved)[2][0].get('style') == 'top', 'a title card style chip, saved')
    log, saved = run('title-remove', spec_t + click(centre(rect(bt, 'title-text-0-button'))), project='words')
    check(same(saved), 'removing a title card first asks to confirm')
    log, saved = run('title-add', WORDS + click(centre(rect(bw, 'add-title'))), project='words')
    titles = words(saved)[2]
    check(len(titles) == 2 and titles[1]['start_seconds'] == 3.0 and titles[1]['title'] == 'Look cards fixture'
          and 'artist' not in titles[1] and field(cards(log), 'open') == 'title-1',
          f'Add a title card at the playhead: the project title, its artist by default, saved, open ({titles[-1:]})')
    covered, _ = run('title-add-covered', WORDS.replace('time=3', 'time=1'), frames=30, project='words')
    check('add-title@disabled:' in cards(covered), 'with a card on screen at the playhead, Add waits and says why')
    log, saved = run('title-add-gap', WORDS + click((rect(bw, 'add-title')[0] + rect(bw, 'add-title')[2] + 4, rect(bw, 'add-title')[1] + 10)), project='words')
    check(same(saved), 'the gap beside Add a title card changes nothing')

if only in ('all', 'sequence'):
    # The review's defects (D2, 2026-11-14): state a card holds must belong to
    # one project, and Play the cut must keep its promise off the Look view.
    WORDS = 'size=1280x1440,column=look,time=3,play=0,look=captions'
    TITLE0, VOICE0 = WORDS + ',look-entry=title-0', WORDS + ',look-entry=voice-0'
    bw = cards(run('seq-base', TITLE0, frames=30, project='words')[0])
    bv = cards(run('seq-base-voice', VOICE0, frames=30, project='words')[0])

    def dropped(name):
        # Relative to the checkout, where headless_run.sh runs: the spec has
        # a 256-byte limit and the absolute path alone is half of it.
        return os.path.relpath(out / name / 'other.musi')

    def titles_of(name, project):
        return json.loads((out / name / f'{project}.musi').read_text())['scene_switches'].get('titles', [])

    # 15. a title half-typed in one project, another project dropped in, then
    #     Enter: the draft died with its project (frame 60, a frame after);
    #     and an Enter in the very frame the other opens is refused where it
    #     is applied (`stale=`), because the edit is addressed to its project.
    # The control: the same draft and Enter with no other project commits.
    # 240 frames: autosave settles 1.5 s after the edit at frame 60.
    log, saved = run('seq-draft-control', TITLE0 + click(centre(rect(bw, 'title-text-0')))
                     + ',look-text=STALE,look-action=draft,look-commit-at=60', frames=240, project='words')
    check(saved['scene_switches']['titles'][0]['title'] == 'STALE', 'control: with no project change the same draft and Enter commit')
    for name, commit in [('seq-draft-late', 60), ('seq-draft-same', 30)]:
        log, saved = run(name, TITLE0 + click(centre(rect(bw, 'title-text-0')))
                         + f',look-text=STALE,look-action=draft,drop={dropped(name)},drop-at=30,look-commit-at={commit}',
                         frames=240, project='words', also=('other',))
        other = titles_of(name, 'other')
        check('current 1 "Other fixture"' in line(log, 'tracks'), f"{name}: the other project opened and is current ({line(log, 'tracks')})")
        check(other == original['other']['scene_switches']['titles'] and saved['scene_switches']['titles'] == original['words']['scene_switches']['titles'],
              f'{name}: a draft typed in one project never lands on the other\'s title card 0 ({[t["title"] for t in other]})')
        check(field(cards(log), 'typing') == 'none' and line(log, 'project').endswith('(clean)'),
              f"{name}: nothing is left typing and the opened project was never dirtied ({line(log, 'project')})")
        if commit == 60:
            check('probe=refused-no-focus' in cards(log), f'{name}: the late Enter finds no field (the draft died with its project)')
        else:
            check(field(cards(log), 'stale') == '1', f'{name}: the same-frame edit was refused as addressed to the other project (stale={field(cards(log), "stale")})')

    # 16. an armed removal belongs to one row of one project: armed in one,
    #     the other dropped in, the other's card 0 and voice 0 are not armed
    #     (one press there would have removed them). The control, with no
    #     drop, is still armed at the end: the row stayed in view.
    for button, armed, spec, base in [('title-text-0-button', 'title-0', TITLE0, bw), ('voice-name-0-button', 'voice-0', VOICE0, bv)]:
        log, saved = run(f'seq-arm-control-{armed}', spec + click(centre(rect(base, button))), frames=60, project='words')
        check(field(cards(log), 'armed') == armed and saved['scene_switches']['titles'] == original['words']['scene_switches']['titles'],
              f'control: a press on Remove ({armed}) arms it and removes nothing, and it stays armed while in view')
        name = f'seq-arm-{armed}'
        log, _ = run(name, spec + click(centre(rect(base, button))) + f',drop={dropped(name)},drop-at=30',
                     frames=60, project='words', also=('other',))
        check('current 1 "Other fixture"' in line(log, 'tracks') and field(cards(log), 'armed') == 'none',
              f'{name}: armed in one project, nothing is armed in the other (armed={field(cards(log), "armed")})')

    # 17. Play the cut keeps its promise off Look, and only its own: pressed
    #     on section 2 (paused at 7 s: it seeks to 1.0 and should pause
    #     after 4.1), then the column switched to Lyrics on frame 40, it still
    #     pauses there; another project opened on frame 40 lets it go (the
    #     report says cut=none and the new track is not paused at the old
    #     track's stop); the user's own Right arrow on frame 45 lets it go too
    #     (playback runs on past the stop).
    PLAY = 'size=1280x720,column=look,time=7,play=0,section=2'
    s2, _ = run('seq-play-base', PLAY, frames=30)
    play = centre(rect(cards(s2), 'play'))
    log, _ = run('seq-play-lyrics', PLAY + click(play) + ',column-switch=lyrics', frames=420)
    playhead = float(field(line(log, 'transport'), 'playhead'))
    check(field(line(log, 'column'), 'view') == 'lyrics' and 4.1 <= playhead <= 4.7 and 'playing=false' in line(log, 'resume'),
          f'Play the cut, then Lyrics on frame 40: it still pauses after the cut (playhead {playhead:.3f})')
    name = 'seq-play-drop'
    log, _ = run(name, PLAY + click(play) + f',drop={dropped(name)},drop-at=40', frames=400, also=('other',))
    # `resume: playing=` reads the stream after the loop has ended, false
    # either way: the playhead is the evidence. The other track starts from
    # 0 on frame 40; the old cut paused it as it crossed 4.1.
    playhead = float(field(line(log, 'transport'), 'playhead'))
    check('current 1 "Other fixture"' in line(log, 'tracks') and field(cards(log), 'cut') == 'none' and playhead > 5.0,
          f"Play the cut, then another project: the cut is let go, the new track not paused at its stop (playhead {playhead:.3f}, {line(log, 'resume')})")
    # 260 frames: past the stop, short of the 8 s fixture's end.
    log, _ = run('seq-play-nudge', PLAY + click(play) + ',key=right', frames=260)
    playhead = float(field(line(log, 'transport'), 'playhead'))
    check(field(cards(log), 'cut') == 'none' and playhead > 5.0,
          f'Play the cut, then the user\'s own Right arrow: the cut lets go and playback runs on (playhead {playhead:.3f})')

if only in ('all', 'show'):
    # 18. One section at a time (D2): selecting a section puts the playhead
    #     in it, once it has arrived, so the frame, Look's head and the
    #     tuning under the card are the section the card edits, 50 ms past
    #     the arrival so a seek never lands on the boundary. Section 2
    #     starts at 2.5 and arrives by a 0.6 s fade: 3.150 exactly.
    def playhead_of(log):
        return float(field(line(log, 'transport'), 'playhead'))
    log, _ = run('show-select', 'size=1280x720,column=look,time=7,play=0,section=2', frames=30)
    now = re.search(r'now=run \S+ section (\d+)', line(log, 'scene runs'))
    check(abs(playhead_of(log) - 3.15) < 0.0005 and now and now[1] == '2' and field(line(log, 'scene runs'), 'selected') == '2',
          f"section=2 from 7 s: the playhead goes to 3.150, just past section 2's arrival, and the playhead's section is the selected one ({playhead_of(log):.3f}, {now and now[0]})")
    log, _ = run('show-inside', 'size=1280x720,column=look,time=3,play=0,section=2', frames=30)
    check(abs(playhead_of(log) - 3.0) < 0.0005, f'control: a playhead already in the section stays ({playhead_of(log):.3f})')
    lane = tuple(map(int, re.search(r'lane=(\d+),(\d+),(\d+),(\d+)', line(log, 'scene runs')).groups()))

    def lane_x(seconds):
        return round(lane[0] + seconds / 8.0 * lane[2])
    body = (lane_x(4.75), lane[1] + lane[3] // 2)
    log, _ = run('show-lane-click', 'size=1280x720,column=look,time=7,play=0' + click(body), frames=30)
    now = re.search(r'now=run \S+ section (\d+)', line(log, 'scene runs'))
    check(field(line(log, 'scene runs'), 'selected') == '3' and abs(playhead_of(log) - 4.05) < 0.0005 and now and now[1] == '3',
          f"a real press on section 3 in the lane selects it and puts the playhead 50 ms past its cut, in it: 4.050 ({playhead_of(log):.3f}, {now and now[0]})")
    log, _ = run('show-lane-boundary', 'size=1280x720,column=look,time=7,play=0' + click((lane_x(4.0), body[1])), frames=30)
    check(field(line(log, 'scene runs'), 'selected') in ('2', '3') and abs(playhead_of(log) - 7.0) < 0.0005,
          f"a press on a boundary (a retime) selects and leaves the playhead at 7 ({playhead_of(log):.3f}, selected={field(line(log, 'scene runs'), 'selected')})")
    # Played on into section 3: the card says the picture is 3's, and Show 2
    # puts it back (pressed on frame 120, while playing).
    base, _ = run('show-elsewhere', 'size=1280x720,column=look,time=3,play=1,section=2', frames=120)
    check('show@' in cards(base) and playhead_of(base) > 4.0,
          f'played past section 2, the card says so and offers Show 2 ({playhead_of(base):.3f})')
    log, _ = run('show-press', 'size=1280x720,column=look,time=3,play=1,section=2,click-after=120' + click(centre(rect(cards(base), 'show'))), frames=150)
    check(playhead_of(log) < 4.0 and 'show@' not in cards(log), f'Show 2 puts the playhead back in section 2 ({playhead_of(log):.3f})')
    r = rect(cards(base), 'show')
    log, _ = run('show-gap', 'size=1280x720,column=look,time=3,play=1,section=2,click-after=120' + click((r[0] - 3, r[1] + r[3] // 2)), frames=150)
    check(playhead_of(log) > 4.0 and 'show@' in cards(log), f'the gap beside Show changes nothing ({playhead_of(log):.3f})')

if only in ('all', 'keys'):
    # 22. Real keys (D2): every check above types through the probe's text
    #     seam; here native XTest presses and keys go through GLFW. The
    #     length value: a real press, Ctrl+A, "1.25", Enter: saved 1.25, and
    #     its digits picked no scene. A title card: "To Room 4" (a T that
    #     opens Look, a space that plays, a 4 that picks a scene, all typed
    #     into the field instead), Enter: saved, still paused, still Look.
    #     The control: the same keys with no field open do pick a scene.
    s2 = cards(run('keys-base', SECTION + ',section=2', frames=30)[0])
    value = centre(rect(s2, 'length-value'))
    log, saved = run('keys-length', SECTION + ',section=2', frames=900,
                     drive=[{'click': value}, {'chord': 'a'}, {'type': '1.25'}, {'key': 'enter'}, {'wait': 2.5}])
    check('native steps delivered' in log and abs(cue(saved, 1)['transition']['seconds'] - 1.25) < 1e-6
          and field(line(log, 'tune layout'), 'scene') == 'spectrum',
          f"real keys: 1.25 typed into the length value is saved and picks no scene ({cue(saved, 1)['transition']}, scene {field(line(log, 'tune layout'), 'scene')})")
    TITLE0 = 'size=1280x1440,column=look,time=3,play=0,look=captions,look-entry=title-0'
    bt = cards(run('keys-title-base', TITLE0, frames=30, project='words')[0])
    log, saved = run('keys-title', TITLE0, frames=900, project='words',
                     drive=[{'click': centre(rect(bt, 'title-text-0'))}, {'chord': 'a'}, {'type': 'To Room 4'}, {'key': 'enter'}, {'wait': 2.5}])
    playhead = float(field(line(log, 'transport'), 'playhead'))
    # Read without raising: a T that reached the shell closes Look, and its
    # `tune layout:` line with it.
    scene = re.search(r'^tune layout:\s+scene=(\S+)', log, re.M)
    scene = scene[1] if scene else 'no Look'
    check(saved['scene_switches']['titles'][0]['title'] == 'To Room 4' and abs(playhead - 3.0) < 0.001
          and field(line(log, 'column'), 'view') == 'look' and scene == 'spectrum',
          f"real keys: 'To Room 4' typed into a title is saved; no key reached play, Look or the scenes (playhead {playhead:.3f}, view {field(line(log, 'column'), 'view')}, scene {scene})")
    log, _ = run('keys-control', SECTION + ',section=2', frames=900, drive=[{'type': '4'}, {'wait': 0.5}])
    check(field(line(log, 'tune layout'), 'scene') != 'spectrum',
          f"control: the same 4 with no field open picks a scene ({field(line(log, 'tune layout'), 'scene')})")

if only in ('all', 'follow'):
    log, _ = run('follow-base', SECTION + ',section=2,follow=1', frames=40, follow=True)
    b = cards(log)
    check(field(b, 'editable') == '0' and field(b, 'follow') == 'show.treatment.json' and 'stop-follow@' in b,
          'following, the card names the file and is not editable')
    log, _ = run('follow-press', SECTION + ',section=2,follow=1' + click(centre(rect(b, 'kind-flash'))), frames=40, follow=True)
    check(field(cards(log), 'kind') == 'fade', 'following, a kind press changes nothing')
    # 23. (D2) a disabled control says why: hovered while following, a kind
    #     chip's words are the follow's reason, not the kind's description.
    log, _ = run('follow-hover', SECTION + ',section=2,follow=1,hover=%dx%d' % centre(rect(b, 'kind-flash')), frames=40, follow=True)
    check('Following a treatment' in line(log, 'tooltip'), f"following, a kind chip's hover says why it waits ({line(log, 'tooltip')})")
    log, _ = run('follow-stop', SECTION + ',section=2,follow=1' + click(centre(rect(b, 'stop-follow'))), frames=40, follow=True)
    check(line(log, 'treatment follow') == 'none' and field(cards(log), 'editable') == '1',
          'Stop following stops it, and the card is editable again')

if only in ('all', 'clip'):
    # 24-33. A Video Clip section's clip (F1c), on the clip fixture: sections
    # 2 and 3 play a.mp4 (2 from 1 s, 3 going on from it), 4 plays b.mp4
    # (loop, contain). The playhead at 5 s is inside section 3: clip time
    # 1 + 3 = 4.0 s, frame 120.
    CLIP = 'size=1280x720,column=look,time=5,play=0'
    a_sha = original['clip']['clips'][0]['sha256']
    b_sha = original['clip']['clips'][1]['sha256']

    def clip_frame(log):
        """The clip frame the preview drew, and whether it was the exact one."""
        found = re.search(r'drew frame (\d+)/\d+ (EXACT|INEXACT)', line(log, 'clip'))
        return (int(found[1]), found[2] == 'EXACT') if found else (None, False)

    def clip_unchanged(saved, project='clip'):
        return unchanged(saved, project) and saved.get('clips') == original[project].get('clips')

    c3, _ = run('clip-s3', CLIP + ',section=3', project='clip', frames=60)
    c4, _ = run('clip-s4', CLIP + ',section=4', project='clip', frames=60)
    c1, _ = run('clip-s1', CLIP + ',section=1', project='clip', frames=30)
    b3, b4, b1 = cards(c3), cards(c4), cards(c1)
    frame3 = clip_frame(c3)
    check(field(b3, 'clip') == a_sha[:8] and field(b3, 'clip-from') == 'continues-2' and field(b3, 'clip-at') == '3.0000'
          and frame3 == (120, True),
          f"section 3 reads a.mp4 going on from section 2 at 3.00 s; the preview drew frame 120 exactly: "
          f"{field(b3, 'clip')} {field(b3, 'clip-from')} {field(b3, 'clip-at')} {frame3}")
    # 33. Only a Video Clip section has clip rows.
    check(field(b1, 'clip') == '-' and 'clip-choice-' not in b1 and 'clip-import@' not in b1,
          'a Spectrum section shows none of the clip rows')

    # 24. which clip: a press on a library row plays that clip here.
    log, saved = run('clip-pick-a', CLIP + ',section=4' + click(centre(rect(b4, 'clip-choice-0'))), project='clip')
    check(field(cards(log), 'clip') == a_sha[:8] and cue(saved, 3)['clip'].get('clip') == a_sha
          and cue(saved, 3)['clip'].get('after') == 'loop',
          f"a press on a.mp4's row plays it in section 4, saved; its end stays loop: {cue(saved, 3).get('clip')}")
    first, second = rect(b4, 'clip-choice-0'), rect(b4, 'clip-choice-1')
    log, saved = run('clip-pick-gap', CLIP + ',section=4' + click((first[0] + 20, first[1] + first[3] + 2)), project='clip')
    check(field(cards(log), 'clip') == b_sha[:8] and clip_unchanged(saved) and claimed_nothing(log),
          'the gap between two clip rows changes nothing')
    check('(chosen)' in re.search(r'clip-choice-1@[^;]*', b4)[0] and 'thumbs=' in line(c4, 'posters'),
          'the clip section 4 plays is the chosen row')

    # 25. continue -> own start: pinned where the clock is, the frame does not move.
    own = rect(b3, 'start-own')
    log, saved = run('clip-own', CLIP + ',section=3' + click(centre(own)), project='clip', frames=150)
    frame = clip_frame(log)
    check(field(cards(log), 'clip-from') == 'own' and abs(cue(saved, 2)['clip'].get('from', -1) - 3.0) < 1e-9,
          f"Own start pins section 3 at 3.0 s, saved: {cue(saved, 2).get('clip')}")
    check(frame == frame3, f'Own start draws the same clip frame: {frame} against {frame3}')
    cont = rect(b3, 'start-continue')
    log, saved = run('clip-own-gap', CLIP + ',section=3' + click((cont[0] + cont[2] + 2, cont[1] + cont[3] // 2)), project='clip')
    check(field(cards(log), 'clip-from') == 'continues-2' and clip_unchanged(saved) and claimed_nothing(log),
          'the gap between Continue and Own start changes nothing')

    # 26. own start (unchanged number) -> continue: the frame does not move.
    o3, _ = run('clipown-s3', CLIP + ',section=3', project='clipown', frames=60)
    before = clip_frame(o3)
    log, saved = run('clipown-continue', CLIP + ',section=3' + click(centre(rect(cards(o3), 'start-continue'))),
                     project='clipown', frames=150)
    check(field(cards(o3), 'clip-from') == 'own' and field(cards(log), 'clip-from') == 'continues-2'
          and 'from' not in cue(saved, 2)['clip'],
          f"Continue drops section 3's own start, saved: {cue(saved, 2).get('clip')}")
    check(clip_frame(log) == before == (120, True), f'Continue draws the same clip frame: {clip_frame(log)} against {before}')
    # 27. Continue is not offered where the section before plays another clip.
    check('start-continue@disabled:' in b4, 'Continue is drawn disabled on section 4 (section 3 plays a.mp4)')

    # 28. the number: a press on the From track and a typed one move the frame.
    bar = rect(b3, 'clip-from')
    point, unit = at_fraction(bar, 0.25)
    log, saved = run('clip-from', CLIP + ',section=3' + click(point), project='clip', frames=150)
    start = cue(saved, 2)['clip'].get('from')
    expected = round(unit * 5.99 * 100) / 100
    check(start is not None and abs(start - expected) < 0.0051 and clip_frame(log)[0] == int((start + 1.0) * 30 + 1e-6),
          f'the From track at {unit:.3f} sets {expected:.2f} s and the frame follows: saved {start}, frame {clip_frame(log)}')
    check(int(field(line(log, 'posters'), 'refreshed')) >= 1,
          f"the section's poster is drawn again for the new start: {field(line(log, 'posters'), 'refreshed')}")
    log, saved = run('clip-from-gap', CLIP + ',section=3' + click((point[0], bar[1] + bar[3] // 2 - 15 - 3)), project='clip')
    check(field(cards(log), 'clip-from') == 'continues-2' and clip_unchanged(saved), 'the space above the From row changes nothing')
    value = centre(rect(b3, 'clip-from-value'))
    log, saved = run('clip-from-typed', CLIP + ',section=3' + click(value) + ',look-text=2.5,look-action=commit',
                     project='clip', frames=150)
    check(abs(cue(saved, 2)['clip'].get('from', -1) - 2.5) < 1e-9 and clip_frame(log) == (105, True),
          f"a typed 2.5 starts section 3 at 2.5 s: frame {clip_frame(log)} (2.5 + 1 s = 105)")

    # 29. after its end, by name.
    log, saved = run('clip-bounce', CLIP + ',section=4' + click(centre(rect(b4, 'after-bounce'))), project='clip')
    check(field(cards(log), 'clip-after') == 'bounce' and cue(saved, 3)['clip'].get('after') == 'bounce',
          f"Bounce: section 4 bounces, saved: {cue(saved, 3).get('clip')}")
    loop, bounce = rect(b4, 'after-loop'), rect(b4, 'after-bounce')
    log, saved = run('clip-after-gap', CLIP + ',section=4' + click((loop[0] + loop[2] + 2, loop[1] + loop[3] // 2)), project='clip')
    check(field(cards(log), 'clip-after') == 'loop' and clip_unchanged(saved) and claimed_nothing(log),
          'the gap between Loop and Bounce changes nothing')

    # 30. fit.
    log, saved = run('clip-cover', CLIP + ',section=4' + click(centre(rect(b4, 'fit-cover'))), project='clip')
    check(field(cards(log), 'clip-fit') == 'cover' and 'fit' not in cue(saved, 3)['clip'],
          f"Cover: section 4 fills the frame; the default is not written: {cue(saved, 3).get('clip')}")
    cover = rect(b4, 'fit-cover')
    log, saved = run('clip-fit-gap', CLIP + ',section=4' + click((cover[0] + cover[2] + 2, cover[1] + cover[3] // 2)), project='clip')
    check(field(cards(log), 'clip-fit') == 'contain' and clip_unchanged(saved) and claimed_nothing(log),
          'the gap between Cover and Contain changes nothing')
    glyphs = re.findall(r'(?:after|fit)-\w+-glyph@', b4)
    check(len(glyphs) == 5, f'every end and fit chip has its glyph beside its name ({len(glyphs)})')

    # 31. Import clip (the flyout's import, landing here), and Generate drawn
    #     disabled with nothing behind it.
    log, saved = run('clip-import', CLIP + ',section=2' + click(centre(rect(b3, 'clip-import'))) + f",clip-pick={out / 'c.mp4'}",
                     project='clip', frames=90)
    names = [entry['name'] for entry in saved.get('clips', [])]
    c_sha = next((entry['sha256'] for entry in saved.get('clips', []) if entry['name'] == 'c.mp4'), None)
    check(names == ['a.mp4', 'b.mp4', 'c.mp4'] and c_sha and cue(saved, 1)['clip'].get('clip') == c_sha
          and field(cards(log), 'clip-library') == '3',
          f'Import clip adds c.mp4 and section 2 plays it, saved: {names}')
    imp, gen = rect(b3, 'clip-import'), rect(b3, 'clip-generate')
    log, saved = run('clip-import-gap', CLIP + ',section=2' + click((imp[0] + imp[2] + 2, imp[1] + imp[3] // 2)) + f",clip-pick={out / 'c.mp4'}",
                     project='clip')
    check(clip_unchanged(saved) and claimed_nothing(log), 'the gap between Import and Generate changes nothing')
    log, saved = run('clip-generate', CLIP + ',section=2' + click(centre(gen)) + f",clip-pick={out / 'c.mp4'}", project='clip')
    check('clip-generate@disabled:' in b3 and clip_unchanged(saved) and field(cards(log), 'clip-library') == '2',
          'Generate is drawn disabled and a press on it changes nothing')

    # 32. a clip whose file is gone says so, in the placeholder's words.
    def lose_a(work):
        for path in (work / 'clip.assets' / 'clips').glob(a_sha + '.*'):
            path.unlink()
    log, _ = run('clip-missing', CLIP + ',section=3', project='clip', frames=60, prepare=lose_a)
    words = re.search(r'clip-words="([^"]*)"', cards(log))
    placeholder = re.search(r'PLACEHOLDER "([^"]*)"', line(log, 'clip'))
    check(words and placeholder and words[1] == placeholder[1] and 'the file is missing' in words[1],
          f"a missing clip says why, as the placeholder does: {words and words[1]!r} / {placeholder and placeholder[1]!r}")
    check('clip-import@' in cards(log) and 'clip-import@disabled' not in cards(log), 'Import is the way out')

    # 34. Play the cut over a clip section: from 2.5 through the fade to 5.6.
    log, _ = run('clip-play', CLIP + ',section=3' + click(centre(rect(b3, 'play'))), project='clip', frames=420)
    playhead = float(field(line(log, 'transport'), 'playhead'))
    check(5.5 <= playhead <= 6.0 and 'playing=false' in line(log, 'resume') and clip_frame(log)[0] is not None,
          f'Play the cut plays through section 3\'s fade and pauses: playhead {playhead:.3f}, clip {clip_frame(log)}')

    # 35. following a treatment, the clip rows are disabled and say why.
    log, _ = run('clip-follow-base', CLIP + ',section=4,follow=1', project='clip', follow=True, frames=60)
    followed = cards(log)
    log, _ = run('clip-follow', CLIP + ',section=4,follow=1' + click(centre(rect(followed, 'after-bounce'))),
                 project='clip', follow=True, frames=60)
    report = cards(log)
    check('clip-import@disabled:' in followed and 'start-own@disabled:' in followed and field(followed, 'editable') == '0'
          and field(report, 'clip-after') == 'loop' and field(report, 'follow') == 'show.treatment.json',
          f"following, the clip rows are drawn disabled and a press on Bounce changes nothing ({field(report, 'clip-after')})")

if only in ('all', 'captures'):
    for theme in ['dark', 'light']:
        for scope, probe in [('section', SECTION + ',section=3'), ('video', SECTION + ',look=video'),
                             ('narrow', 'size=960x640,column=look,column-width=300,time=3,play=0,section=3')]:
            log, _ = run(f'capture-{scope}-{theme}', probe, frames=30, theme=theme)
            b = cards(log)
            area = tuple(map(int, field(b, 'area').split(',')))
            viewport = tuple(map(int, re.search(r'viewport=([\d,]+)', line(log, 'tune layout'))[1].split(',')))
            check(area[1] >= viewport[1] and area[0] >= viewport[0] and area[0] + area[2] <= viewport[0] + viewport[2],
                  f'{scope}/{theme}: the card lies inside the list ({field(b, "area")} in {viewport})')
            check('ui=Space Grotesk' in line(log, 'fonts'), f'{scope}/{theme}: the UI face loaded')
            if scope in ('section', 'narrow'):
                # 21. every kind's glyph drew (D2): inked pixels in its square,
                #     against the chip's own fill beside it. Opaque fills and
                #     anti-aliased strokes: a count, not exact colours.
                from PIL import Image
                image = Image.open(out / f'capture-{scope}-{theme}' / 'shot.png').convert('RGB')
                px = image.load()
                weak = []
                glyphs = re.findall(r'kind-(\w+)-glyph@(\d+),(\d+),(\d+),(\d+)', b)
                for kind, gx, gy, gw, gh in glyphs:
                    gx, gy, gw, gh = map(int, (gx, gy, gw, gh))
                    chip = rect(b, f'kind-{kind}')
                    ground = px[chip[0] + 4, chip[1] + chip[3] // 2]
                    inked = sum(1 for yy in range(gy, gy + gh) for xx in range(gx, gx + gw)
                                if sum(abs(a - c) for a, c in zip(px[xx, yy], ground)) > 60)
                    if inked < 8:
                        weak.append(f'{kind}:{inked}')
                check(len(glyphs) == 14 and not weak, f'{scope}/{theme}: all 14 kind glyphs drew ({len(glyphs)} placed, weak {weak})')

if only in ('all', 'captures'):
    # 36. (F1c) the clip rows in both appearances, at 1280 and at the 300 px
    #     column (a tall window, so the whole card is on screen): inside the list, the section's poster and every clip's
    #     middle frame drawn (posters=settle waits for both), and the end and
    #     fit glyphs inked against their chips.
    from PIL import Image
    for theme in ['dark', 'light']:
        for scope, probe in [('clip', 'size=1280x720,column=look,time=5,play=0,section=3,posters=settle'),
                             ('clip-narrow', 'size=960x1200,column=look,column-width=300,time=5,play=0,section=3,posters=settle')]:
            log, _ = run(f'capture-{scope}-{theme}', probe, frames=40, theme=theme, project='clip')
            b = cards(log)
            area = tuple(map(int, field(b, 'area').split(',')))
            viewport = tuple(map(int, re.search(r'viewport=([\d,]+)', line(log, 'tune layout'))[1].split(',')))
            check(area[0] >= viewport[0] and area[0] + area[2] <= viewport[0] + viewport[2],
                  f'{scope}/{theme}: the card lies inside the list ({field(b, "area")} in {viewport})')
            thumbs = re.findall(r'clip-thumb-\d+=(\w+)', b)
            check(field(b, 'clip-poster') == 'ready' and thumbs == ['ready', 'ready'],
                  f"{scope}/{theme}: the section's poster and both clips' frames drew ({field(b, 'clip-poster')}, {thumbs})")
            image = Image.open(out / f'capture-{scope}-{theme}' / 'shot.png').convert('RGB')
            px = image.load()
            weak = []
            for name, gx, gy, gw, gh in re.findall(r'((?:after|fit)-\w+)-glyph@(\d+),(\d+),(\d+),(\d+)', b):
                gx, gy, gw, gh = map(int, (gx, gy, gw, gh))
                chip = rect(b, name)
                ground = px[chip[0] + 4, chip[1] + chip[3] // 2]
                inked = sum(1 for yy in range(gy, gy + gh) for xx in range(gx, gx + gw)
                            if sum(abs(a - c) for a, c in zip(px[xx, yy], ground)) > 60)
                if inked < 8:
                    weak.append(f'{name}:{inked}')
            check(not weak, f'{scope}/{theme}: every end and fit glyph drew (weak {weak})')
            # The poster is a picture, not the empty well: many colours in it.
            poster = rect(b, 'clip-poster')
            well = image.crop((poster[0] + 2, poster[1] + 2, poster[0] + poster[2] - 2, poster[1] + poster[3] - 2))
            colours = len({well.getpixel((x, y)) for x in range(well.width) for y in range(well.height)})
            check(colours > 50, f"{scope}/{theme}: the poster holds a picture ({colours} colours)")

(out / 'result.json').write_text(json.dumps({'failures': failures}, indent=2))
print(f'{len(failures)} failures; evidence {out}', flush=True)
sys.exit(bool(failures))
