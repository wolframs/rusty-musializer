#!/usr/bin/env python3
"""Evidence assertions for knot_check.sh; deliberately separable for negative controls."""
import json
import re
import sys
from pathlib import Path
from PIL import Image, ImageStat, ImageChops

root = Path(sys.argv[1])
for name in ('preview', 'idle', 'reopen', 'minimum'):
    log = (root / f'{name}.log').read_text()
    match = re.search(r'^openai knot:\s+strands=6 segments=160 triangles=23040 weave=fixed energy=([\d.]+).*bands=([\d.,]+)', log, re.M)
    assert match, f'{name}: missing geometry/audio report'
    assert 'bloom=blurred fringe=0.70 grain=0.35' in log, f'{name}: finish unavailable'
    assert 'finish=ready' in log, f'{name}: grain shader unavailable'
    assert 'mismatched-frames=0' in log, f'{name}: framebuffer not restored'
    assert 'output underruns: 0' in log, f'{name}: audio output underrun'
    bands = [float(x) for x in match[2].split(',')]
    assert len(bands) == 6
    if name == 'preview':
        assert float(match[1]) > 0.01 and max(bands)-min(bands) > 0.025, 'no selective audio response'
    if name == 'idle':
        assert float(match[1]) == 0 and max(bands) == 0, 'idle is not quiet'
picker = (root/'minimum.log').read_text()
assert re.search(r'^scene:\s+knot \(OpenAI Knot\)', picker, re.M), 'minimum-size picker did not select knot'
assert 'at=42x260 claimed=0x20000000d' in picker, 'scene tile did not receive the click'
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
print('PASS: geometry, six audio regions, rest, framing, opening, motion and persistence assertions')
