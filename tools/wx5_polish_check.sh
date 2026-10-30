#!/usr/bin/env bash
# WX-5 polish: isolated captures and report-derived presses on :81.
# Usage: tools/wx5_polish_check.sh [output-dir]
# MZ_BIN can name a source-perturbed negative-control binary.
# MZ_GL_LAUNCH="" selects llvmpipe. Never starts a real agent or model.
# WX5_POLISH_REPLAY=1 rechecks completed PNG/report pairs without launching.
set -euo pipefail
cd "$(dirname "$0")/.."
export MUSIALIZER_CAPTURE_DISPLAY="${MUSIALIZER_CAPTURE_DISPLAY:-:81}"
export MZ_XVFB_DISPLAY="$MUSIALIZER_CAPTURE_DISPLAY"
pgrep -af Xvfb || true
python3 - "${1:-build/wx5-polish}" <<'PY'
import ast, hashlib, json, os, re, subprocess, sys, threading
from pathlib import Path
import numpy as np
from PIL import Image
root = Path.cwd()
out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
env = os.environ.copy()
env['MZ_BIN'] = str(Path(env.get('MZ_BIN', 'target/debug/musializer')).resolve())
env['MZ_STATE_DIR'] = str(out / 'state')
env['SHELL'] = '/bin/sh'
replay=os.environ.get('WX5_POLISH_REPLAY')=='1'
fake = out / 'fake-agent'
fake.write_text(f'#!/bin/sh\nexec /usr/bin/python3 "{root}/tools/fake_agent.py" "$@"\n')
fake.chmod(0o755)
env['MUSIALIZER_AGENT_CLAUDE'] = str(fake)
wave = out / 'fixture.wav'
subprocess.run(['cargo', 'run', '--quiet', '--bin', 'make-fixture-wav', '--', str(wave), '8'], check=True)
show = {'schema':'musializer.treatment/v1','title':'Polish fixture','audio':{'path':'fixture.wav'},
        'segments':[{'at':0,'scene':'spectrum'},{'at':2,'scene':'loom'},{'at':4,'scene':'cadence'}]}
treatment = out / 'show.treatment.json'
treatment.write_text(json.dumps(show))
protocol = out / 'test.protocol.json'
protocol.write_text(json.dumps({'schema':'musializer.protocol/v1','title':'Polish listening fixture',
    'audio':{'path':str(wave),'sha256':hashlib.sha256(wave.read_bytes()).hexdigest()},
    'items':[{'id':f'q{i}','at_seconds':t,'window':{'pre':1,'post':1},'question':'Keep this look?',
              'kind':'choice','options':['keep','change']} for i,t in enumerate([2,4,6])]}))
recent = out / 'recent.json'
project = root / 'build/redesign/fixture/Door, No Room.musi'
assert project.is_file(), 'copy the supplied fixture as instructed before running'
recent.write_text(json.dumps({'schema':'musializer.recent-projects/v1',
    'projects':[{'path':str(project),'name':'Door, No Room','opened_unix':None}]}))
env['MUSIALIZER_RECENT_PROJECTS'] = str(recent)
failures=[]
def check(ok, name):
    print(('ok: ' if ok else 'FAIL: ') + name, flush=True)
    if not ok: failures.append(name)
def line(log, name):
    return next((l.split(':',1)[1].strip() for l in reversed(log.splitlines()) if l.startswith(name+':')), '')
def rect(log, name):
    m=re.search(re.escape(name)+r'[@=]([\d.-]+),([\d.-]+),([\d.-]+),([\d.-]+)', log)
    assert m, (name, log[-1200:])
    return tuple(map(float,m.groups()))
def point(r):
    k=current_scale/100
    return f'{(r[0]+r[2]/2)*k:g}x{(r[1]+r[3]/2)*k:g}'
def capture(name, probe, args=(), extra=None, frames=30):
    e=env | (extra or {})
    png=out/f'{name}.png'; log=out/f'{name}.log'
    command=['tools/headless_run.sh',*map(str,args),'--ui-scale',str(current_scale),
        '--ui-theme',current_theme,'--ui-probe',f'size={current_size},{probe}',
        '--probe-frames',str(frames),'--probe-shot',str(png)]
    if not replay:
        with log.open('w') as f:
            result=subprocess.run(command, env=e, stdout=f, stderr=subprocess.STDOUT)
        check(result.returncode==0, name+' launch')
    else:
        check('Window closed successfully' in log.read_text(), name+' completed capture')
    text=log.read_text()
    assert png.is_file(), log
    with Image.open(png) as shot:
        check(shot.format=='PNG' and shot.size==tuple(map(int,current_size.split('x'))), name+' PNG dimensions')
        pixels=np.array(shot.convert('RGB'))
    return text, pixels
def crop(im,r):
    x,y,w,h=r; k=current_scale/100
    return im[round(y*k):round((y+h)*k),round(x*k):round((x+w)*k)]
def direct_check(tag, recut):
    live=out/'live.treatment.json'; live.write_text(json.dumps(show))
    def rewrite():
        updated=json.loads(json.dumps(show))
        if recut: updated['segments'][2]['at']=5
        else: updated['segments'][2]['scene']='loom'
        tmp=out/'live.tmp'; tmp.write_text(json.dumps(updated)); tmp.replace(live)
    writer=threading.Timer(3.5,rewrite)
    if not replay: writer.start()
    try:
        direct,di=capture(('direct-recut-' if recut else 'direct-applied-')+tag,'column=direct,play=0,agent=claude,follow=1',
            ['--treatment',live],frames=420)
    finally:
        if not replay: writer.join()
    dock=line(direct,'dock')
    if recut:
        check('Window applied' in dock and not re.search(r'\b\d+ sections? changed',dock) and re.search(r'· \d+ s ago',dock), 'recut omits uncertain count and keeps age '+tag)
    else:
        check('1 section changed' in dock and re.search(r'· \d+ s ago',dock), 'honest treatment count and age '+tag)
    check('notice@' in dock, 'apply note actually drawn '+tag)
def ai_check(tag):
    dialog,dim=capture('ai-settings-'+tag,'column=assist,play=0',['--treatment',treatment],
        {'MUSIALIZER_ASSIST_SETTINGS_OPEN':'routing','MUSIALIZER_ASSIST_SETTINGS_SCROLL':'10000'})
    ready=list({l.split('@')[0]:l for l in dialog.splitlines() if l.startswith('assist settings ready:')}.values())
    check(bool(ready), 'READY rows reported '+tag)
    long=[]
    for entry in ready:
        text=ast.literal_eval(re.search(r'text=(".*?") lines=',entry).group(1))
        lines=ast.literal_eval(entry.split(' lines=')[1])
        check(' '.join(lines)==text.upper(), 'READY reason unabridged '+tag+' '+text)
        if text=='Not implemented in this build':
            long.append(entry)
            check(lines==['NOT IMPLEMENTED IN','THIS BUILD'], 'READY exact two lines '+tag)
            boundary=tuple(map(float,re.search(r'@([\d.-]+),([\d.-]+),([\d.-]+),([\d.-]+)',entry).groups()))
            check(boundary[2:]==(130.,30.), 'READY exact seat '+tag)
            textpixels=crop(dim,(boundary[0]+12,boundary[1]+14,boundary[2]-12,13))
            background=np.array((23,26,31) if current_theme=='dark' else (255,255,255))
            coverage=np.abs(textpixels.astype(int)-background).sum(axis=2)
            check(np.count_nonzero(coverage>80)>=12, 'READY second line actually visible '+tag)
    check(bool(long), 'unbuilt route reason visible '+tag)
# The exact values below are independent pins, not copied from the report.
# Fill checks inspect opaque interiors. Text uses coverage against its
# background, allowing antialiased glyphs and renderer rounding.
matrix=[('960x640',100),('1280x720',100),('1440x960',150)]
if os.environ.get('WX5_POLISH_FAST')=='1': matrix=matrix[:1]
for current_size,current_scale in matrix:
  for current_theme in ['dark','light']:
    tag=f'{current_theme}-{current_size}-{current_scale}'
    if os.environ.get('WX5_POLISH_ONLY')=='direct':
        direct_check(tag, False)
        direct_check(tag, True)
        continue
    if os.environ.get('WX5_POLISH_ONLY')=='ai':
        ai_check(tag)
        continue
    surface=(17,19,23) if current_theme=='dark' else (247,247,248)
    hover=(29,33,39) if current_theme=='dark' else (239,239,242)
    pennant=(181,154,223) if current_theme=='dark' else (117,80,154)
    welcome,im=capture('welcome-'+tag,'notices=all')
    primary=rect(welcome,'open-audio'); tray=rect(welcome,'tray')
    expected_y=449 if current_size!='1280x720' else 465
    check(tray[1]==expected_y, 'welcome seat '+tag)
    cards=re.findall(r'card=([\d.,-]+)',line(welcome,'notices'))
    check(bool(cards), 'welcome shows a queued notice '+tag)
    for card in cards:
        x,y,w,h=map(float,card.split(','))
        check(y>=tray[1] and y+h<=tray[1]+tray[3], 'notice stays in tray '+tag)
        check(y>=primary[1]+primary[3], 'notice clears primary '+tag)
    row=rect(welcome,'recent')
    hovered,him=capture('recent-hover-'+tag,'hover='+point(row))
    strip=(row[0]-6,row[1]+4,2,row[3]-8)
    check(np.all(crop(him,strip)==surface), 'no hover stripe '+tag)
    inside=(row[0]+2,row[1]+row[3]-2,6,1)
    check(np.all(crop(him,inside)==hover), 'square hover fill '+tag)
    # A real close press, then a press in the gap below the primary actions.
    notices=line(welcome,'notices')
    close=rect(notices,'close')
    dismissed,_=capture('notice-dismiss-'+tag,'notices=all,click='+point(close))
    oldids=set(re.findall(r'id=(\d+)',notices)); newids=set(re.findall(r'id=(\d+)',line(dismissed,'notices')))
    check(bool(oldids-newids), 'notice close takes press '+tag)
    opened,_=capture('welcome-primary-'+tag,'notices=all,click='+point(primary))
    check('claimed=0x600000001' in line(opened,'click probe') and 'queued=5' in line(opened,'notices'), 'primary takes press with notices '+tag)
    gap=(primary[0],primary[1]+primary[3]+2,primary[2],4)
    gaplog,_=capture('welcome-gap-'+tag,'notices=all,click='+point(gap))
    check(line(gaplog,'notices').split('cards=')[0]==notices.split('cards=')[0], 'welcome gap changes nothing '+tag)
    proto,pim=capture('protocol-'+tag,'column=closed,play=0', ['--protocol',protocol])
    mask=np.all(pim==pennant,axis=2)
    n=np.count_nonzero(mask)
    columns=np.flatnonzero(mask.any(axis=0))
    groups=np.split(columns,np.where(np.diff(columns)>1)[0]+1) if columns.size else []
    check(len(groups)==3 and n>=30*(current_scale/100)**2, 'protocol opaque pennants '+tag)
    ai_check(tag)
    for source,args,expected in [
      ('fresh',[],390),
      ('band',['--route','loom.weight:band:2:0:1:0.4:2.2:smoothstep'],418)]:
        tune,tim=capture('route-'+source+'-'+tag,'column=look,play=1,route=loom.weight',
            [wave,'--scene','loom',*args])
        route=line(tune,'route editor'); layout=line(tune,'tune layout')
        card=rect(route,'card')
        check(f'row={expected}px' in route and card[3]==expected, 'route exact card height '+source+' '+tag)
        if source=='fresh':
            band=rect(layout,'band')
            clicked,_=capture('route-click-'+tag,'column=look,play=1,route=loom.weight,click='+point(band),
                [wave,'--scene','loom'])
            check('row=418px source=Band committed=0' in line(clicked,'route editor'),
                'route Band press changes actual height '+tag)
        # The layout's editor starts with the same setting@card.
        check(','.join(f'{v:.0f}' for v in card) in layout.split(' editor=')[-1], 'route report agrees with draw '+source+' '+tag)
    direct_check(tag, False)
    direct_check(tag, True)
print(f'{len(failures)} polish checks failed; captures in {out}',flush=True)
sys.exit(bool(failures))
PY
