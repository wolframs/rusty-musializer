#!/usr/bin/env python3
"""Report-addressed presses, gap controls, persistence, and six-size/look captures.
PIXEL_LOOK_BIN can select a perturbed binary; PIXEL_LOOK_FAST omits extra captures.
All test audio remains intact and process-muted through headless_run.sh.
"""
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys

out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
env = dict(os.environ, MZ_BIN=os.environ.get('PIXEL_LOOK_BIN', str(Path('target/debug/musializer').resolve())))
subprocess.run(['cargo', 'run', '--quiet', '--bin', 'make-fixture-wav', '--', str(out/'fixture.wav'), '8'], check=True, stdout=subprocess.DEVNULL)
for owned in [True, False]:
    show = {'schema':'musializer.treatment/v1', 'title':'Pixel look fixture', 'audio':{'path':'fixture.wav'},
            'finish':{'pixel':{'pixels':144,'palette':'handheld'}},
            'segments':[{'at':0,'scene':'cadence', **({'pixel':{'pixels':144,'palette':'handheld','grid':0}} if owned else {})}]}
    (out/f'{owned}.treatment.json').write_text(json.dumps(show))
failures=[]
def check(ok, message):
    print(('PASS: ' if ok else 'FAIL: ')+message, flush=True)
    if not ok: failures.append(message)
def line(log, label):
    found=re.findall(r'^'+re.escape(label)+r':\s*(.*)$',log,re.M)
    if not found: raise AssertionError(f'missing {label}')
    return found[-1]
def run(name, scope='video', click=None, size='1280x720', theme='dark', owned=True, state=None, reveal=True, typed=None):
    probe=f'size={size},column=look,time=2,play=0'+(f',pixel={scope}' if reveal else '')
    if typed: probe+=f',save-to={out/name}.musi'
    if click: probe+=f',click={click[0]}x{click[1]}'
    cmd=['tools/headless_run.sh','--treatment',str(out/f'{owned}.treatment.json'),'--ui-scale','100','--hud=0',
         '--ui-probe',probe,'--probe-frames','180' if typed else '24','--probe-shot',str(out/f'{name}.png')]
    if theme: cmd+=['--ui-theme',theme]
    run_env=dict(env,MZ_STATE_DIR=str(state or out/f'state-{name}'))
    if typed:
        run_env.update(MZ_BIN=str(Path('tools/pixel_look_input.py').resolve()), PIXEL_LOOK_REAL_BIN=env['MZ_BIN'])
        if typed=='cancel': run_env['PIXEL_LOOK_CANCEL']='1'
    result=subprocess.run(cmd,env=run_env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    (out/f'{name}.log').write_text(result.stdout)
    if result.returncode: raise AssertionError(f'{name}: exit {result.returncode}: {result.stdout[-1500:]}')
    return line(result.stdout,'pixel look'), result.stdout

def rect(report, name):
    match=re.search(r'(?:^|[ ;=])'+re.escape(name)+r'@(-?\d+),(-?\d+),(\d+),(\d+)',report)
    if not match: raise AssertionError(f'{name} missing: {report}')
    return tuple(map(int,match.groups()))
def centre(r): return (r[0]+r[2]//2,r[1]+r[3]//2)
def field(report,name): return re.search(r'(?:^| )'+name+r'=(\S+)',report)[1]

base={}
for scope in ['video','section']:
    report,_=run(f'{scope}-base',scope)
    base[scope]=report
    check(field(report,'scope')==scope, f'{scope}: scope is drawn')
    for index, choice in enumerate(['off','Handheld-green','Handheld-grey','Home-computer','Fantasy-console','Console-colour','Pixels-only']+(['inherit'] if scope=='section' else [])):
        report,log=run(f'{scope}-choice-{index}',scope,centre(rect(base[scope],f'choice-{index}')))
        state=field(report,'video' if scope=='video' else 'section')
        check(state==choice if choice in ['off','inherit'] else state.startswith(choice+':144:'),f'{scope}: palette {choice} reached model: {state}')
        # Each palette has an adjacent gap (the space above its row).
        r=rect(base[scope],f'choice-{index}')
        gap,gaplog=run(f'{scope}-gap-{index}',scope,(r[0]+r[2]//2,r[1]-4))
        check(field(gap,'video')==field(base[scope],'video') and field(gap,'section')==field(base[scope],'section'),f'{scope}: palette {index} gap changes nothing')
        check('claimed=nothing' in line(gaplog,'click probe'), f'{scope}: palette {index} gap claims nothing')
    r=rect(base[scope],'size'); point=(r[0]+r[2]//4,r[1]+r[3]//2)
    expected=math.floor(48+(point[0]-r[0])/r[2]*672+0.5)
    report,_=run(f'{scope}-size',scope,point)
    check(field(report,'pixels')==str(expected),f'{scope}: short edge exactly {expected} pixels')
    report,_=run(f'{scope}-typed',scope,centre(rect(base[scope],'value')),typed='commit')
    check(field(report,'pixels')=='216' and field(report,'typing')=='false',f'{scope}: pressed value chip and native keys commit 216')
    saved=out/f'{scope}-typed.musi'
    saved_doc=json.loads(saved.read_text()) if saved.exists() else {}
    pixel=(saved_doc.get('scene_switches',{}).get('finish',{}).get('pixel',{}) if scope=='video' else (saved_doc.get('scene_switches',{}).get('cues') or [{}])[0].get('pixel',{}))
    check(pixel.get('pixels')==216,f'{scope}: native Ctrl+S persists exact edited count in .musi')
    report,_=run(f'{scope}-typed-cancel',scope,centre(rect(base[scope],'value')),typed='cancel')
    check(field(report,'pixels')=='144' and field(report,'typing')=='false',f'{scope}: Escape discards typed pixel count')
    r=rect(base[scope],'kick-1')
    report,_=run(f'{scope}-kick-on',scope,centre(r))
    check(field(report,'kick')=='0.600,0.500,0.400',f'{scope}: kick cartridge values reached model')
    # Kick off is seeded via the normal treatment rather than a probe write.
    show=json.loads((out/'True.treatment.json').read_text())
    show['finish']['pixel'].update(split=.6,shake=.5,zoom=.4)
    show['segments'][0]['pixel'].update(split=.6,shake=.5,zoom=.4)
    original=(out/'True.treatment.json').read_bytes()
    (out/'True.treatment.json').write_text(json.dumps(show))
    try: report,_=run(f'{scope}-kick-off',scope,centre(rect(base[scope],'kick-0')))
    finally: (out/'True.treatment.json').write_bytes(original)
    check(field(report,'kick')=='0.000,0.000,0.000',f'{scope}: kick off zeros all three values')
    r=rect(base[scope],'kick-0')
    gap,gaplog=run(f'{scope}-kick-gap',scope,(r[0]+r[2]//2,r[1]-4))
    check(field(gap,'kick')=='0.000,0.000,0.000',f'{scope}: kick gap changes nothing')
    check('claimed=nothing' in line(gaplog,'click probe'), f'{scope}: kick gap claims nothing')
    r=rect(base[scope],'value')
    gap,gaplog=run(f'{scope}-size-gap',scope,(r[0]+r[2]//2,r[1]-4))
    check(field(gap,'pixels')=='144',f'{scope}: size gap changes nothing')
    check('claimed=nothing' in line(gaplog,'click probe'), f'{scope}: size gap claims nothing')

report,_=run('scope-to-section','video',centre(rect(base['video'],'scope-1')))
check(field(report,'scope')=='section','press This section changes scope')
report,_=run('scope-to-video','section',centre(rect(base['section'],'scope-0')))
check(field(report,'scope')=='video','press Whole video changes scope')
r=rect(base['video'],'scope-0')
report,gaplog=run('scope-gap','video',(r[0]+r[2]//2,r[1]-5))
check(field(report,'scope')=='video','scope gap changes nothing')
check('claimed=nothing' in line(gaplog,'click probe'),'scope gap claims nothing')
report,_=run('inherited','section',owned=False)
check(field(report,'current')=="The-video's-look" and 'size@' not in report and 'kick-0@' not in report,'inherited section has no owned size or kick control')
for i, theme in enumerate(['dark','light']):
    state=out/f'theme-state-{theme}'
    report,log=run(f'theme-before-{theme}',theme='light' if theme=='dark' else 'dark',state=state)
    report,log=run(f'theme-press-{theme}',click=centre(rect(report,f'theme-{i}')),theme='light' if theme=='dark' else 'dark',state=state)
    check(line(log,'appearance').startswith('black-amber' if i==0 else 'light-blue'),f'{theme}: press changes rendered appearance')
    report,log=run(f'theme-restart-{theme}',theme=None,state=state)
    check(line(log,'appearance').startswith('black-amber' if i==0 else 'light-blue'),f'{theme}: appearance persists across restart')
    prefs_path=state/'config'/'musializer'/'ui.json'
    prefs=json.loads(prefs_path.read_text()) if prefs_path.exists() else {}
    check(prefs.get('theme')==('black-amber' if i==0 else 'light-blue'),f'{theme}: actual preference file matches')
r=rect(base['video'],'theme-0')
report,log=run('theme-gap',click=(r[0]+r[2]//2,r[1]-5))
check(line(log,'appearance').startswith('black-amber'),'appearance gap changes nothing')
check('claimed=nothing' in line(log,'click probe'),'appearance gap claims nothing')
if not os.environ.get('PIXEL_LOOK_FAST'):
    for size in ['960x640','1280x720','1920x1080']:
        for theme in ['dark','light']:
            for scope in ['video','section']:
                report,log=run(f'look-{scope}-{theme}-{size}',scope,size=size,theme=theme)
                viewport=tuple(map(int,re.search(r'viewport=([\d,]+)',line(log,'tune layout'))[1].split(',')))
                area=tuple(map(int,re.search(r'area=([\d,]+)',report)[1].split(',')))
                check(viewport[1]<=area[1] and area[1]+area[3]<=viewport[1]+viewport[3],f'{scope}/{theme}/{size}: entire filter is visible')
                check('ui=Space Grotesk' in line(log,'fonts') and 'icons=Font Awesome' in line(log,'fonts'),f'{scope}/{theme}/{size}: assets reported')
(out/'result.json').write_text(json.dumps({'failures':failures},indent=2))
print(f'{len(failures)} failures; evidence {out}',flush=True)
sys.exit(bool(failures))
