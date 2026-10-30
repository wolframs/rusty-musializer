#!/usr/bin/env bash
# WX-5 follow-ups: real scene posters and inline artist editing.
# Every launch is muted/isolated by headless_run.sh. Points are derived from
# report rectangles, including the gaps. WX5_ONLY=posters|artist|captures narrows
# a negative-control run; WX5_BIN selects its binary. No tool touches ~/Music.
# Source negative controls and GPU/CPU receipts: docs/WX5_FEATURES_EVIDENCE.md.
set -euo pipefail
cd "$(dirname "$0")/.."
export MUSIALIZER_CAPTURE_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:82}"
pgrep -af Xvfb || true
if pgrep -af "^Xvfb ${MUSIALIZER_CAPTURE_DISPLAY} " >/dev/null; then
    echo "display ${MUSIALIZER_CAPTURE_DISPLAY} is already occupied" >&2
    exit 1
fi
export MZ_BIN="${WX5_BIN:-$PWD/target/debug/musializer}"
OUT="${1:-build/wx5-features}"
mkdir -p "$OUT"
export WX5_OUT="$(realpath "$OUT")"
export MZ_STATE_DIR="$WX5_OUT/state"
cargo run --quiet --bin make-fixture-wav -- "$WX5_OUT/fixture.wav" 20 >/dev/null
python3 - <<'PY'
import itertools, json, math, os, re, shutil, subprocess
from pathlib import Path
import numpy as np
from PIL import Image
out = Path(os.environ['WX5_OUT'])
only = os.environ.get('WX5_ONLY', 'all')
failures = []
def check(ok, message):
    print(('ok: ' if ok else 'FAIL: ') + message, flush=True)
    if not ok: failures.append(message)
def line(log, key):
    found = re.findall(r'^' + re.escape(key) + r':\s*(.*)$', log, re.M)
    if not found: raise AssertionError('missing report: ' + key)
    return found[-1]
def rect(log, key, name, sep='@'):
    text = line(log, key)
    m = re.search(re.escape(name) + r'\*?' + re.escape(sep) + r'(\d+),(\d+),(\d+),(\d+)', text)
    if not m: raise AssertionError('missing rect ' + name + ' in ' + text)
    return tuple(map(int, m.groups()))
def centre(r):
    x,y,w,h = r
    return f'{x+w//2}x{y+h//2}'
def run(name, probe, frames=30, project=None, appearance='dark', extra=()):
    cmd = ['tools/headless_run.sh']
    cmd += [str(project)] if project else ['--treatment', str(out/'plain.treatment.json')]
    cmd += ['--ui-scale','100','--ui-theme',appearance,'--probe-frames',str(frames),
            '--ui-probe',probe,'--probe-shot',str(out/(name+'.png')), *extra]
    with (out/(name+'.log')).open('w') as f:
        result = subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT)
    log = (out/(name+'.log')).read_text(errors='replace')
    check(result.returncode == 0 and (out/(name+'.png')).is_file(), name + ' completed and captured')
    return log
plan = {'schema':'musializer.treatment/v1','title':'WX5 feature fixture',
        'audio':{'path':'fixture.wav'}, 'segments':[{'at':0,'scene':'spectrum'}]}
(out/'plain.treatment.json').write_text(json.dumps(plan))

if only in ('all','posters'):
    families = {'sound':['spectrum','pulse','atlas','loom'],
                'geometry':['orbital','pentagram','knot','metaballs','voronoi','glyphs'],
                'atmosphere':['constellation','fluid','aurora','tunnel'],
                'materials':['chrome','iridescent'],'living':['terrarium','clawd'],
                'type-stage':['ascii','cadence','phosphor','stage','threshold'],
                'pixel-cats':['pixel-boot','pixel-shell','pixel-run','pixel-quartet','pixel-chapel','pixel-rave']}
    # The expected scene ids come from the known family contract; remaining
    # families are validated against visible=[...] and dumped scene identities.
    for family in families:
        dump = out/('dump-'+family)
        shutil.rmtree(dump, ignore_errors=True)
        log = run('family-'+family, f'size=960x640,time=3,scene-family={family},posters=settle,poster-dump={dump}')
        picker = line(log,'scene picker')
        states = re.findall(r'([\w-]+):(ready(?:-idle)?|pending|stale|unavailable)@(\d+),(\d+),(\d+),(\d+)', picker)
        names = re.search(r'visible=\[([^]]+)\]',picker)[1].split(',')
        check(names == families[family], family+' retains its exact scene identities')
        check([s[0] for s in states] == names and len(states) == len(names), family+' reports every visible tile state')
        check('offset=0.0/0.0' in picker, family+' fits unscrolled at 960x640')
        screenshot = np.array(Image.open(out/('family-'+family+'.png')).convert('RGB'))
        for name,state,x,y,w,h in states:
            check(state in ('ready','ready-idle'), name+' settled state')
            files = list(dump.glob(f'{name}-picker-{name}-*.png'))
            check(len(files) == 1, name+' has its own scene dump')
            if len(files) != 1: continue
            x,y,w,h = map(int,(x,y,w,h))
            poster = Image.open(files[0]).convert('RGB')
            pw,ph = poster.size
            width = min(w,h*pw/ph); height=width*ph/pw
            dx=x+(w-width)/2; dy=y+(h-height)/2
            # Compare the centre, away from fractional placement and border
            # coverage. ±1 blend rounding is immaterial at this error floor.
            actual = screenshot[math.ceil(dy+2):math.floor(dy+height-2), math.ceil(dx+2):math.floor(dx+width-2)]
            # Sampling through mipmaps is close to area resize, not bit-equal.
            reference = np.array(poster.resize((round(width),round(height)),Image.Resampling.BOX))
            reference = reference[2:2+actual.shape[0],2:2+actual.shape[1]]
            ah=min(len(actual),len(reference)); aw=min(actual.shape[1],reference.shape[1])
            actual=actual[:ah,:aw].astype(float); reference=reference[:ah,:aw].astype(float)
            error=float(np.mean(abs(actual-reference)))
            wrong=[]
            for other in dump.glob('*-picker-*.png'):
                if other==files[0]: continue
                wrong_image=np.array(Image.open(other).convert('RGB').resize((round(width),round(height)),Image.Resampling.BOX))[2:2+ah,2:2+aw].astype(float)
                if wrong_image.shape==actual.shape: wrong.append(float(np.mean(abs(actual-wrong_image))))
            check(error < 24 and (not wrong or error+0.5 < min(wrong)), f'{name} tile matches its own poster (error {error:.2f}, nearest other {min(wrong,default=999):.2f})')
        if family == 'pixel-cats':
            # Exact tile geometry is pinned as well as data flow.
            check(rect(log,'scene picker','pixel-boot') == (168,83,103,104), 'poster tile geometry 103x104 at minimum')
            a=rect(log,'scene picker','pixel-boot'); b=rect(log,'scene picker','pixel-shell')
            gap=f'{a[0]+a[2]+(b[0]-a[0]-a[2])//2}x{a[1]+a[3]//2}'
            clicked=run('tile-click',f'size=960x640,time=3,scene-family={family},posters=settle,click={centre(b)}')
            check('active=pixel-shell' in line(clicked,'scene picker'), 'tile press selects its named scene')
            missed=run('tile-gap',f'size=960x640,time=3,scene-family={family},posters=off,click={gap}')
            check('active=spectrum' in line(missed,'scene picker'), 'tile gap changes no scene')
    for mode,frames in [('off',30),('pending',1)]:
        suffix=',posters=off' if mode=='off' else ''
        log=run('poster-'+mode,'size=960x640,time=3,scene-family=pixel-cats'+suffix,frames)
        states=re.findall(r'pixel-[\w-]+:(\w+)@',line(log,'scene picker'))
        check(states == [('unavailable' if mode=='off' else 'pending')]*6, mode+' is explicit on all six tiles')
    # Ordinary scheduling, while music plays: this is not the unbounded settle.
    log=run('playing-posters','size=1280x720,play=1,scene-family=pixel-cats',180)
    stats=line(log,'posters')
    check('budget=3ms/8ms' in stats, 'the poster service retains its exact 3ms/8ms budgets')
    step=float(re.search(r'step-worst=([\d.]+)',stats)[1]); frame=float(re.search(r'frame-worst=([\d.]+)',stats)[1])
    check(step <= 3+frame+1.5, f'playing budget: step {step:.1f} <= 3 + frame {frame:.1f} + timer slack')
    check(re.findall(r'pixel-[\w-]+:(\w+)@',line(log,'scene picker')) == ['ready']*6, 'six posters arrive under the playing budget')

if only in ('all','artist'):
    # Start with a saved v1 project. Real autosave after each UI commit pins the
    # bytes through the exact user path, not only a model serializer unit test.
    seed=out/'artist.musi'
    subprocess.run(['tools/headless_run.sh','--treatment',str(out/'plain.treatment.json'),'--save-project',str(seed)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True)
    original=seed.read_bytes()
    log=run('artist-base','size=960x640,time=3',project=seed)
    slot=rect(log,'top bar','artist')
    check(slot[2:] == (64,30), 'empty artist invitation has the exact 64x30 seat')
    point=centre(slot)
    editing=run('artist-draft',f'size=960x640,time=3,click={point},artist-text=Space Mono F M T,artist-action=draft',project=seed)
    check('artist-editing=true' in line(editing,'top bar') and 'Space Mono F M T' in line(editing,'top bar'),'slot press opens and holds the draft')
    check('direct*' in line(editing,'top bar') and 'fullscreen' not in line(editing,'top bar'), 'typing leaves the mode and top bar visible')
    gap=f'{slot[0]-4}x{slot[1]+slot[3]//2}'
    missed=run('artist-gap',f'size=960x640,time=3,click={gap},artist-text=Wrong,artist-action=commit',project=seed)
    check('artist-event=probe-refused-no-focus' in line(missed,'top bar') and 'artist-editing=false' in line(missed,'top bar'), 'artist gap cannot open or commit a draft')
    cancelled=run('artist-cancel',f'size=960x640,time=3,click={point},artist-text=Wrong,artist-action=cancel',project=seed)
    check('artist=none' in line(cancelled,'top bar') and 'artist-event=cancelled' in line(cancelled,'top bar') and seed.read_bytes()==original, 'Escape outcome cancels without a metadata write')
    committed=run('artist-commit',f'size=960x640,time=3,click={point},artist-text=Fixture Artist,artist-action=commit',150,project=seed)
    check('artist="Fixture Artist"' in line(committed,'top bar') and not 'artist-editing=true' in line(committed,'top bar'), 'Enter outcome commits artist and releases focus')
    data=json.loads(seed.read_text())
    check(data['schema_version']=='musializer.project/v2' and data.get('metadata',{}).get('artist')=='Fixture Artist','UI commit autosaves the artist as v2')
    point=centre(rect(committed,'top bar','artist'))
    clear=run('artist-clear',f'size=960x640,time=3,click={point},artist-action=clear',150,project=seed)
    check('artist=none' in line(clear,'top bar') and seed.read_bytes()==original, 'clearing in the UI restores byte-identical v1')

if only in ('all','captures'):
    project=Path('build/redesign/fixture/Door, No Room.musi')
    project=project if project.is_file() else None
    for appearance,size in itertools.product(['dark','light'],['960x640','1280x720','1920x1080']):
        stem=appearance+'-'+size
        base=run(stem+'-scenes',f'size={size},time=60,scene-family=pixel-cats,posters=settle',project=project,appearance=appearance)
        point=centre(rect(base,'top bar','artist'))
        editing=run(stem+'-artist',f'size={size},time=60,click={point},artist-text=Chaos Cat Familiar,artist-action=draft',project=project,appearance=appearance)
        bar=line(editing,'top bar'); x,y,w,h=rect(editing,'top bar','artist')
        check('artist-editing=true' in bar and 'Chaos Cat Familiar' in bar,stem+' artist draft drawn')
        check((y,h)==(8,30) and x+w<=int(size.split('x')[0]),stem+' artist field fits the 46px bar')

if failures: raise SystemExit(f'{len(failures)} WX5 feature checks failed; artifacts: {out}')
print('WX5 features: all requested checks passed; artifacts: '+str(out))
PY
