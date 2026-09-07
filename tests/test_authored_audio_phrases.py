"""A compound written line becomes separate cues only with separate audio evidence."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools'))
import authored_audio_phrases as p
import authored_audio_occurrences as o
import external_analysis as app
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools/lyrics_research'))
import apply_audio_audit
from analysis_io import sha256_file

TEXT='Read a book once in a while'

def row(start):
    return dict(text=TEXT,start_seconds=start,end_seconds=start+2,
                uncertain=False,partial_start=False,partial_end=False)

def reports():
    return [dict(clip_start_seconds=0,clip_duration_seconds=25,clip_sha256='a'*64,
                 lines=[row(10),row(12.2)]),
            dict(clip_start_seconds=5,clip_duration_seconds=20,clip_sha256='b'*64,
                 lines=[row(5.1),row(7.3)])]

def document():
    return dict(lines=[dict(text=TEXT+' ('+TEXT+')',start_seconds=10,end_seconds=14.2,
                           reference_line_index=7)],unresolved=[],unmatched=[],review_flags=[],
                unreliable_evidence=[],statistics={})

class PhraseSplits(unittest.TestCase):
    def test_ordinary_sentence_can_contain_two_performed_phrases(self):
        d=document();d['lines'][0]['text']='"Read a book once in a while, then let the music play"'
        heard=reports()
        for report in heard:
            report['lines'][0]['text']='Read a book once in a while'
            report['lines'][1]['text']='then let the music play'
        saved=copy.deepcopy(d)
        proposed=p.proposals(d,heard)
        self.assertEqual(len(proposed),1)
        self.assertEqual([part['text'] for part in proposed[0]['phrases']],
                         ['"Read a book once in a while,"','"then let the music play"'])
        d['phrase_splits']=p.confirm(proposed,heard)
        self.assertEqual(len(o.rendered_lines(d)),2)
        self.assertEqual(d['lines'],saved['lines'])
        self.assertEqual(p.proposals(saved,heard[:1]),[])

    def test_written_comma_or_one_split_observation_cannot_establish_a_seam(self):
        d=document();d['lines'][0]['text']='Read a book once in a while, then let the music play'
        heard=reports()
        for report in heard:
            report['lines'][0]['text']=d['lines'][0]['text']
            report['lines'][0]['end_seconds']=report['lines'][1]['end_seconds']
            report['lines'].pop()
        self.assertEqual(p.proposals(d,heard),[])
        partial=reports()[0]
        partial['lines'][1]['text']='then let the music play'
        heard[0]=partial
        self.assertEqual(p.proposals(d,heard),[])

    def test_multiple_supported_seams_remain_an_unresolved_grouping(self):
        texts=['Read a book','Let music play','Hear every word']
        d=document();d['lines'][0].update(text=' '.join(texts),end_seconds=16.4)
        heard=reports()
        for report in heard:
            first=report['lines'][0]['start_seconds']
            report['lines']=[dict(row(first+i*2.2),text=text) for i,text in enumerate(texts)]
        self.assertGreaterEqual(len(p.partitions(d['lines'][0],heard)),2)
        self.assertEqual(p.proposals(d,heard),[])

    def test_separate_phrases_reach_bridge_and_manifest_without_rewriting_authored_line(self):
        d=document();saved=copy.deepcopy(d)
        splits=p.confirm(p.proposals(d,reports()),reports())
        self.assertEqual(len(splits),1)
        d['phrase_splits']=splits
        rendered=o.rendered_lines(d)
        self.assertEqual([round(r['start_seconds'],2) for r in rendered],[10.05,12.25])
        self.assertEqual(d['lines'],saved['lines'])
        self.assertTrue(all(r['uncertain'] for r in rendered))
        plan=dict(audio=dict(sha256='a'*64,duration_seconds=25),sections=[dict(start_seconds=0,end_seconds=25,recommended_scene='spectrum',transition_strength=.5,reasons=[])])
        bridge=app.parse_bridge(app.build_bridge(plan,lyrics=d))
        self.assertEqual(len([row for row in bridge if row[0]=='LYRIC']),2)
        manifest=app.build_assist_manifest(mode='lyrics',audio_sha='a'*64,measured_duration=25,
            cache_status={},paths={'manifest':Path('manifest.json')},plan=plan,lyrics=d,semantic=None)
        self.assertEqual(manifest['result_counts']['lyrics'],2)
        self.assertEqual(manifest['result_counts']['lyrics_review_flags'],2)

    def test_one_native_crop_or_one_fresh_crop_cannot_split(self):
        d=document()
        self.assertEqual(p.proposals(d,reports()[:1]),[])
        proposed=p.proposals(d,reports())
        for fresh in ([],reports()[:1],[reports()[0],reports()[0]]):
            self.assertEqual(p.confirm(proposed,fresh),[])

    def test_missing_or_uncertain_reply_keeps_compound_cue(self):
        proposed=p.proposals(document(),reports())
        for change in ('missing','uncertain','shifted'):
            fresh=reports()
            for report in fresh:
                if change=='missing':report['lines'].pop()
                elif change=='uncertain':report['lines'][1]['uncertain']=True
                else:report['lines'][1]['end_seconds']+=1
            self.assertEqual(p.confirm(proposed,fresh),[])

    def test_shared_performance_cannot_supply_two_parents(self):
        d=document();d['performed_occurrences']=[dict(d['lines'][0])]
        self.assertEqual(p.proposals(d,reports()),[])
        proposed=p.proposals(document(),reports())
        self.assertEqual(p.confirm(proposed+proposed,reports()),[])

    def test_stale_or_duplicate_split_cannot_silently_replace_a_cue(self):
        d=document();d['phrase_splits']=p.confirm(p.proposals(d,reports()),reports())
        for change in ('edit','duplicate','index'):
            changed=copy.deepcopy(d)
            if change=='edit':changed['lines'][0]['end_seconds']+=.1
            elif change=='duplicate':changed['phrase_splits']*=2
            else:changed['phrase_splits'][0]['source_index']=-1
            with self.assertRaisesRegex(ValueError,'unchanged source cue'):
                o.rendered_lines(changed)

    def test_legacy_correction_refuses_split_rendering_indices(self):
        d=document();d['phrase_splits']=p.confirm(p.proposals(d,reports()),reports())
        with self.assertRaisesRegex(ValueError,'phrase splits'):
            apply_audio_audit.corrected(d,[])

    def test_occurrence_namespace_is_independent_from_authored_index(self):
        d=document();d['performed_occurrences']=d.pop('lines');d['lines']=[]
        splits=p.confirm(p.proposals(d,reports()),reports())
        self.assertEqual(splits[0]['source_kind'],'performed_occurrences')
        self.assertEqual(splits[0]['source_index'],0)
        d['phrase_splits']=splits
        self.assertEqual(len(o.rendered_lines(d)),2)

class AuthorizedSplits(unittest.IsolatedAsyncioTestCase):
    async def test_denial_precedes_audio_read(self):
        with patch.object(p,'sha256_file',side_effect=AssertionError('must not read')):
            with self.assertRaisesRegex(ValueError,'confirmation'):
                await p.split(Path('none'),{}, {},Path('none'),model='gemini-3.8-flash-high',confirmed=False)

    async def test_fresh_protocol_is_audio_only_and_resume_preserves_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);track=root/'audio';track.write_bytes(b'audio')
            identity=dict(sha256=sha256_file(track),duration_seconds=25)
            source=dict(audio=identity,provenance=dict(source_kind='antigravity_audio'),
                        source=dict(boundary_observations=reports()),lines=[])
            d=document();d['audio']=identity;saved=copy.deepcopy(d)
            observer=AsyncMock(return_value=reports())
            paths=dict(server=track,harness=track,profile=root)
            with patch.object(p.antigravity_audio,'discover',return_value=paths),patch.object(p.audio,'observe_spans',observer):
                first=await p.split(track,source,d,root,model='gemini-3.8-flash-high',confirmed=True)
                second=await p.split(track,source,first,root,model='gemini-3.8-flash-high',confirmed=True)
            self.assertEqual(first,second)
            self.assertEqual(d,saved)
            self.assertEqual(first['lines'],d['lines'])
            request=observer.await_args_list[0].args[1]
            self.assertEqual(request,observer.await_args_list[1].args[1])
            self.assertNotIn(TEXT,json.dumps(request))

if __name__=='__main__':unittest.main()
