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
 tables (look=captions)
  12. a voice's hue, ink and name (its lines keep it); removing a voice that
      sings lines asks to confirm; Add a voice is drawn with why it waits; a
      style's anchor, a rename that takes its line along, Remove disabled for
      a style in use and working for one that is not, Add a style; a title
      card's text, handle (gains its @), start time (and a refused one),
      style, Remove asking to confirm, Add at the playhead (and waiting
      while a card covers it).
 follow
  13. following a treatment, the cards are drawn disabled and a press
      changes nothing; Stop following stops following and the card is
      editable again.
 captures
  14. both appearances at 1280x720 (section and video) and at the column's
      minimum width: the card lies inside the list, the faces loaded.

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
"""
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

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
films = {'plain': base_show, 'film': dict(base_show, finish='film'), 'words': words_show}
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


def run(name, probe, frames=150, project='plain', theme='dark', follow=False):
    """One probe run on a fresh copy of a seeded project. Returns (log, saved)."""
    work = out / name
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    if follow:
        shutil.copy(out / 'fixture.wav', work / 'fixture.wav')
        shutil.copy(out / f'{project}.treatment.json', work / 'show.treatment.json')
        source = ['--treatment', str(work / 'show.treatment.json')]
    else:
        # The .musi names its bundled audio by its own stem: keep the names.
        shutil.copy(out / f'{project}.musi', work / f'{project}.musi')
        assets = out / f'{project}.assets'
        if assets.exists():
            shutil.copytree(assets, work / f'{project}.assets')
        source = ['--project', str(work / f'{project}.musi')]
    cmd = ['tools/headless_run.sh', *source, '--ui-scale', '100', '--hud=0', '--ui-theme', theme,
           '--ui-probe', probe, '--probe-frames', str(frames), '--probe-shot', str(work / 'shot.png')]
    result = subprocess.run(cmd, env=dict(env, MZ_STATE_DIR=str(work / 'state')),
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
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
    check(abs(float(field(line(log, 'transport'), 'playhead')) - 7.0) < 0.01, 'the gap beside Play the cut leaves the playhead at 7')

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
    check(field(bw, 'scope') == 'captions' and 'voice-name-0@' in bw and 'style-name-1@' in bw and 'title-text-0@' in bw,
          'look=captions shows the three tables')
    styles0 = [entry['name'] for entry in original['words']['caption_style']['styles']]

    def words(saved):
        return saved['lyrics'], saved['caption_style']['styles'], saved['scene_switches'].get('titles', [])

    def same(saved):
        return words(saved) == words(original['words'])
    bar = rect(bw, 'voice-hue-0')
    point, unit = at_fraction(bar, 0.5)
    log, saved = run('voice-hue', WORDS + click(point), project='words')
    check(abs(words(saved)[0]['voices'][0]['hue'] - round(unit * 360)) < 1e-6, f'voice hue track: exactly {round(unit * 360)} degrees, saved')
    log, saved = run('voice-hue-gap', WORDS + click((point[0], bar[1] + bar[3] // 2 - 15 - 3)), project='words')
    check(same(saved), 'voice hue gap changes nothing')
    log, saved = run('voice-ink', WORDS + click(centre(rect(bw, 'voice-ink-1-3'))), project='words')
    check(words(saved)[0]['voices'][1].get('text_rgba') == 'c8f0ffff', f"a voice ink swatch, saved: {words(saved)[0]['voices'][1]}")
    log, saved = run('voice-name', WORDS + click(centre(rect(bw, 'voice-name-0'))) + ',look-text=mezzo,look-action=commit', project='words')
    lyrics = words(saved)[0]
    check(lyrics['voices'][0]['name'] == 'mezzo' and lyrics['cues'][0].get('voice') == 'mezzo',
          'a renamed voice keeps its lines')
    log, saved = run('voice-remove', WORDS + click(centre(rect(bw, 'voice-name-0-button'))), project='words')
    check(same(saved) and 'voice-0-armed:1-lines' in cards(log), 'removing a voice that sings lines first asks to confirm')
    check('add-voice@disabled:' in bw, 'Add a voice is drawn with why it waits')
    shout = styles0.index('shout')
    top = styles0.index('top')
    log, saved = run('style-anchor', WORDS + click(centre(rect(bw, f'style-{top}-top_left'))), project='words')
    check(words(saved)[1][top].get('anchor') == 'top_left', 'a style anchor chip, saved')
    r = rect(bw, f'style-{top}-top_left')
    log, saved = run('style-anchor-gap', WORDS + click((r[0] + r[2] + 2, r[1] + r[3] // 2)), project='words')
    check(same(saved), 'style anchor gap changes nothing')
    log, saved = run('style-rename', WORDS + click(centre(rect(bw, f'style-name-{shout}'))) + ',look-text=loud,look-action=commit', project='words')
    names = [entry['name'] for entry in words(saved)[1]]
    check('loud' in names and 'shout' not in names and words(saved)[0]['cues'][1].get('style') == 'loud',
          'a renamed style takes the line that wore it along')
    check(f'style-name-{shout}-button@disabled:' in bw, 'a style a line wears cannot be removed (drawn disabled)')
    log, saved = run('style-remove-used', WORDS + click(centre(rect(bw, f'style-name-{shout}-button'))), project='words')
    check(same(saved), 'a press on the disabled Remove changes nothing')
    log, saved = run('style-remove', WORDS + click(centre(rect(bw, f'style-name-{top}-button'))), project='words')
    check([entry['name'] for entry in words(saved)[1]] == [n for n in styles0 if n != 'top'], 'an unused style is removed')
    log, saved = run('style-add', WORDS + click(centre(rect(bw, 'add-style'))), project='words')
    check([entry['name'] for entry in words(saved)[1]] == styles0 + ['style-1'], 'Add a style adds style-1')
    log, saved = run('title-text', WORDS + click(centre(rect(bw, 'title-text-0'))) + ',look-text=Door Fixture,look-action=commit', project='words')
    check(words(saved)[2][0]['title'] == 'Door Fixture', 'a typed title, saved')
    log, saved = run('title-handle', WORDS + click(centre(rect(bw, 'title-handle-0'))) + ',look-text=cats,look-action=commit', project='words')
    check(words(saved)[2][0].get('handle') == '@cats', 'a typed handle gains its @, saved')
    log, saved = run('title-from', WORDS + click(centre(rect(bw, 'title-start-0'))) + ',look-text=0:01.0,look-action=commit', project='words')
    check(words(saved)[2][0]['start_seconds'] == 1.0, 'a typed start time, saved')
    log, saved = run('title-from-bad', WORDS + click(centre(rect(bw, 'title-start-0'))) + ',look-text=3,look-action=commit', project='words')
    check(same(saved) and field(cards(log), 'refused') == 'title-start-0', 'a start after the end is refused and says so')
    log, saved = run('title-style', WORDS + click(centre(rect(bw, 'title-0-style-top'))), project='words')
    check(words(saved)[2][0].get('style') == 'top', 'a title card style chip, saved')
    log, saved = run('title-remove', WORDS + click(centre(rect(bw, 'title-text-0-button'))), project='words')
    check(same(saved), 'removing a title card first asks to confirm')
    log, saved = run('title-add', WORDS + click(centre(rect(bw, 'add-title'))), project='words')
    titles = words(saved)[2]
    check(len(titles) == 2 and titles[1]['start_seconds'] == 3.0 and titles[1]['title'] == 'Look cards fixture'
          and 'artist' not in titles[1], f'Add a title card at the playhead: the project title, its artist by default, saved ({titles[-1:]})')
    covered, _ = run('title-add-covered', WORDS.replace('time=3', 'time=1'), frames=30, project='words')
    check('add-title@disabled:' in cards(covered), 'with a card on screen at the playhead, Add waits and says why')
    log, saved = run('title-add-gap', WORDS + click((rect(bw, 'add-title')[0] + rect(bw, 'add-title')[2] + 4, rect(bw, 'add-title')[1] + 10)), project='words')
    check(same(saved), 'the gap beside Add a title card changes nothing')

if only in ('all', 'follow'):
    log, _ = run('follow-base', SECTION + ',section=2,follow=1', frames=40, follow=True)
    b = cards(log)
    check(field(b, 'editable') == '0' and field(b, 'follow') == 'show.treatment.json' and 'stop-follow@' in b,
          'following, the card names the file and is not editable')
    log, _ = run('follow-press', SECTION + ',section=2,follow=1' + click(centre(rect(b, 'kind-flash'))), frames=40, follow=True)
    check(field(cards(log), 'kind') == 'fade', 'following, a kind press changes nothing')
    log, _ = run('follow-stop', SECTION + ',section=2,follow=1' + click(centre(rect(b, 'stop-follow'))), frames=40, follow=True)
    check(line(log, 'treatment follow') == 'none' and field(cards(log), 'editable') == '1',
          'Stop following stops it, and the card is editable again')

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

(out / 'result.json').write_text(json.dumps({'failures': failures}, indent=2))
print(f'{len(failures)} failures; evidence {out}', flush=True)
sys.exit(bool(failures))
