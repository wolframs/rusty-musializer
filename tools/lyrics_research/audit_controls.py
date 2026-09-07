#!/usr/bin/env python3
"""Can the audio auditor detect a known timestamp perturbation? No edits applied."""
import argparse,asyncio,json,subprocess,sys
from pathlib import Path
from types import SimpleNamespace
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import antigravity_audio
from authored_audio_occurrences import rendered_lines
from analysis_io import atomic_write_json
from audit_candidate import PROMPT,parse

async def run(args):
 root=args.manifest.resolve().parent;t=next(t for t in json.loads(args.manifest.read_text()) if t['id']==args.track)
 folder=root/args.track/'independent-audit';source=json.loads((folder/'candidate.json').read_text())
 rows=rendered_lines(source)
 receipt=json.loads((folder/f'{args.clip:04}.json').read_text());start,end=receipt['identity']['start'],receipt['identity']['end']
 core_start=args.clip*18;core_end=min(t['duration_seconds'],core_start+18);core=(core_start-start,core_end-start)
 supplied=[dict(id=i,text=r['text'],start_seconds=r['start_seconds']-start+args.shift,
  end_seconds=r['end_seconds']-start+args.shift,scored=core_start<=r['start_seconds']<core_end)
  for i,r in enumerate(rows) if r['end_seconds']>start-3 and r['start_seconds']<end+3]
 ids=[r['id'] for r in supplied if r['scored']]
 if args.blind:
  for row in supplied:del row['start_seconds'];del row['end_seconds']
 prompt=PROMPT+'\n'+json.dumps(dict(audit_interval=core,proposed_cues=supplied))
 clip=subprocess.check_output(['ffmpeg','-v','error','-i',t['audio'],'-ss',str(start),'-t',str(end-start),'-vn','-ac','1','-ar','16000','-map_metadata','-1','-f','wav','pipe:1'],timeout=60)
 response=await antigravity_audio.ask(SimpleNamespace(**antigravity_audio.discover(),model='gemini-3.8-flash-high',timeout=180),clip,prompt)
 result=dict(shift=args.shift,blind=args.blind,clip=args.clip,response=response['response'],agent=response['agent'],session_id=response['session_id'])
 try:result['audit']=parse(response['response'],ids,end-start,core)
 except Exception as error:result['error']=str(error)
 atomic_write_json(folder/f'control-{args.clip}-{args.shift}-{args.blind}.json',result)
 for r in result.get('audit',{}).get('cues',[]):
  if r['status']=='present':
   expected=rows[r['id']];delta=max(abs(r[f'{edge}_seconds']+start-expected[f'{edge}_seconds']-args.shift) for edge in ('start','end'))
   print(r['id'],round(delta,3),r.get('reason'))
  else:print(r)

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('manifest',type=Path);p.add_argument('--track',default='01');p.add_argument('--clip',type=int,default=1);p.add_argument('--shift',type=float,default=1.5);p.add_argument('--blind',action='store_true');asyncio.run(run(p.parse_args()))
if __name__=='__main__':main()
