#!/usr/bin/env python3
"""Evidence assertions for knot_check.sh; deliberately separable for negative controls."""
import json
import re
import sys
from pathlib import Path
from PIL import Image, ImageStat, ImageChops

root = Path(sys.argv[1])
# The drawn knot: form, its petals, the fixed mesh size, the depth-sorted
# weave, then the audio state. 960 segments x (10 tube steps + 2 shadow
# sides) x 2 triangles is 23040 whatever the form.
REPORT = re.compile(
    r'^lumen knot:\s+form=(?P<form>\d) petals=(?P<petals>\d+) winds=\d+ swing=(?P<swing>[\d.]+)'
    r' morph=(?P<morph>[\d.]+) segments=960 triangles=23040 weave=sorted comets=(?P<comets>\d+)'
    r' energy=(?P<energy>[\d.]+).*bands=(?P<bands>[\d.,]+)', re.M)
def report(log):
    return REPORT.search(log)
for name in ('preview', 'idle', 'reopen', 'minimum'):
    log = (root / f'{name}.log').read_text()
    match = report(log)
    assert match, f'{name}: missing geometry/audio report'
    assert 'bloom=blurred fringe=0.70 grain=0.35' in log, f'{name}: finish unavailable'
    assert 'finish=ready' in log, f'{name}: grain shader unavailable'
    assert 'mismatched-frames=0' in log, f'{name}: framebuffer not restored'
    assert 'output underruns: 0' in log, f'{name}: audio output underrun'
    bands = [float(x) for x in match['bands'].split(',')]
    assert len(bands) == 6
    if name == 'preview':
        assert float(match['energy']) > 0.01 and max(bands)-min(bands) > 0.025, 'no selective audio response'
    if name == 'idle':
        assert float(match['energy']) == 0 and max(bands) == 0, 'idle is not quiet'
picker = (root/'minimum.log').read_text()
assert re.search(r'^scene:\s+knot \(Lumen Knot\)', picker, re.M), 'minimum-size picker did not select knot'
assert re.search(r'^click probe:\s+at=\d+x\d+ claimed=0x20000000d$', picker, re.M), \
    'scene tile did not receive the click'
assert 'family=geometry active=knot' in picker, 'grouped picker did not reveal the selected knot'
# Preview includes chrome; inspect the exported scene to avoid counting UI text.
im = Image.open(root / 'portrait.png').convert('RGB')
w, h = im.size
assert (w, h) == (360, 640)
control = Image.open(root / 'response-off.png').convert('RGB')
response_difference = sum(ImageStat.Stat(ImageChops.difference(im, control)).mean) / 3
assert response_difference > 0.5, f'audio response is not visible: MAD={response_difference:.4f}'
print(f'Audio-on versus response-off (same PCM/time): MAD={response_difference:.4f}')
finish_off = Image.open(root/'finish-off.png').convert('RGB')
finish_difference = sum(ImageStat.Stat(ImageChops.difference(im, finish_off)).mean)/3
assert finish_difference > 1.0, f'finish is not visible: MAD={finish_difference:.4f}'
assert 'bloom=off fringe=0.00 grain=0.00' in (root/'finish-off.log').read_text(), 'finish bypass did not disable all controls'
print(f'Finish versus bypass (same PCM/time): MAD={finish_difference:.4f}')
pixels = list(im.convert("RGB").tobytes())
pixels = list(zip(pixels[0::3], pixels[1::3], pixels[2::3]))
bright = [(i % w, i // w) for i, c in enumerate(pixels) if max(c) > 120]
assert len(bright) > w*h*0.025, 'ribbons absent or too dim'
assert min(x for x,y in bright) > 5 and max(x for x,y in bright) < w-5, 'horizontal clipping'
assert min(y for x,y in bright) > 5 and max(y for x,y in bright) < h-5, 'vertical clipping'
# The central opening must remain darker than the material. This catches an
# accidental full-disc fill even if its mesh/report is otherwise valid.
center = im.crop((w//2-12,h//2-12,w//2+12,h//2+12))
assert max(ImageStat.Stat(center).mean) < 110, 'central opening filled'
strong = Image.open(root/'strong.png').convert('RGB')
assert max(ImageStat.Stat(strong.crop((w//2-12,h//2-12,w//2+12,h//2+12))).mean) < 110, 'peak response filled the hole'
for strip in [(0,0,5,h),(w-5,0,w,h),(0,0,w,5),(0,h-5,w,h)]:
    assert max(channel[1] for channel in strong.crop(strip).getextrema()) < 120, 'peak response clipped a lobe'
# A fresh state opens tied, on the six-petal rosette, with light moving.
first = report((root/'preview.log').read_text())
assert first['form'] == '1' and first['petals'] == '6' and first['morph'] == '1.000', 'did not open on the six-petal rosette'
assert int(first['comets']) > 0, 'no comets on a kicking phrase'
# `form` names a knot, and the knot drawn is a different picture.
named = report((root/'form.log').read_text())
assert named and named['form'] == '4' and named['petals'] == '8', 'form 4 did not tie the eight-petal rosette'
form_difference = sum(ImageStat.Stat(ImageChops.difference(im, Image.open(root/'form.png').convert('RGB'))).mean)/3
assert form_difference > 2.0, f'form 4 looks like form 1: MAD={form_difference:.4f}'
print(f'Form 4 versus form 1 (same PCM/time): MAD={form_difference:.4f}')
# The drop: half a second after it the knot is pulled tight; settled in the
# next form by 10 s. Response off, the same PCM moves nothing.
ring = report((root/'drop-ring.log').read_text())
assert ring and float(ring['morph']) < 0.5 and float(ring['swing']) < 0.2, f'the drop did not pull the knot tight: {ring and ring.group(0)}'
settled = report((root/'drop.log').read_text())
assert settled and settled['form'] == '2' and settled['petals'] == '7' and settled['morph'] == '1.000', 'the drop did not move the form on'
still = report((root/'drop-off.log').read_text())
assert still and still['form'] == '1' and still['morph'] == '1.000', 'response off still moved the form'
print(f"Drop: ring swing={ring['swing']} morph={ring['morph']}; settled form {settled['form']}; response-off form {still['form']}")
motion = Image.open(root/'motion.png').convert('RGB')
frames = [motion.crop((x*480,y*270,(x+1)*480,(y+1)*270)) for y in range(4) for x in range(4)]
assert len({f.tobytes() for f in frames}) >= 12, 'motion is frozen'
project = json.loads((root/'knot.musi').read_text())
text = json.dumps(project)
assert 'settings.knot.polish' in text and 'settings.knot.size' in text, 'settings/routes missing from project'
def mappings(value):
    if isinstance(value, dict):
        if value.get('parameter') == 'settings.knot.polish':
            yield value
        for child in value.values():
            yield from mappings(child)
    elif isinstance(value, list):
        for child in value:
            yield from mappings(child)
route, = list(mappings(project))
assert route['source'] == 'rms' and route['output_min'] == 0.25 and route['output_max'] == 0.8, 'saved route changed'
assert re.search(r'^routes:\s+1$', (root/'reopen.log').read_text(), re.M), 'route did not hydrate'
print('PASS: geometry, six audio regions, rest, framing, opening, forms, drop, motion and persistence assertions')
