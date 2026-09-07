"""Repeated performances need their own evidence and must reach the actual lane."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools'))
import authored_audio_occurrences as occurrences
import external_analysis
from analysis_io import sha256_file

TEXT='The stars return again'

def phrase(start, text=TEXT):
    return dict(text=text,start_seconds=start,end_seconds=start+2,
                uncertain=False,partial_start=False,partial_end=False)

def reports():
    return [dict(clip_start_seconds=0,clip_duration_seconds=30,lines=[phrase(10),phrase(20)]),
            dict(clip_start_seconds=5,clip_duration_seconds=25,lines=[phrase(5.1),phrase(15.1)])]

def fresh():
    return [dict(clip_start_seconds=8,clip_duration_seconds=16,clip_sha256='a'*64,
                 lines=[phrase(2.1),phrase(12.1)]),
            dict(clip_start_seconds=8.65,clip_duration_seconds=16,clip_sha256='b'*64,
                 lines=[phrase(1.55),phrase(11.55)])]

class Instances(unittest.TestCase):
    def test_unresolved_inline_reply_can_yield_confirmed_lead_without_changing_sheet(self):
        sheet = TEXT + ' (The stars return again)'
        document = dict(lines=[], unresolved=[dict(reference_line_index=0, text=sheet)])
        events = occurrences.proposals(document, sheet, reports())
        self.assertEqual(len(events), 2)
        self.assertTrue(all(row['text_scope'] == 'authored_inline_lead' for row in events))
        accepted = occurrences.confirm(events, fresh(), [])
        self.assertEqual(len(accepted), 2)
        self.assertTrue(all(row['text'] == TEXT and row['text_scope'] == 'authored_inline_lead'
                            for row in accepted))
        self.assertEqual(document['unresolved'][0]['text'], sheet)
        self.assertEqual(occurrences.proposals(dict(lines=[]), sheet, reports()), [])

    def test_existing_complete_template_owns_lead_wording(self):
        sheet = TEXT + ' (The stars return again)\n' + TEXT
        events = occurrences.proposals(dict(lines=[], unresolved=[dict(reference_line_index=0)]),
                                       sheet, reports())
        self.assertEqual(len(events), 2)
        self.assertTrue(all('text_scope' not in row and row['reference_line_indices'] == [1]
                            for row in events))

    def test_actual_reply_preserves_complete_performance_instead_of_competing_with_lead(self):
        sheet = TEXT + ' (The stars return again)'
        values = reports()
        for report in values:
            lead = report['lines'][0]
            report['lines'] = [lead, phrase(lead['end_seconds'] + .1)]
        events = occurrences.proposals(dict(lines=[], unresolved=[dict(reference_line_index=0)]),
                                       sheet, values)
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]['text'], sheet)
        self.assertNotIn('text_scope', events[0])

    def test_placed_complete_reply_suppresses_contained_lead(self):
        sheet = TEXT + ' (The stars return again)'
        placed = dict(text=sheet, start_seconds=10, end_seconds=14)
        events = occurrences.proposals(dict(lines=[placed], unresolved=[dict(reference_line_index=0)]),
                                       sheet, reports())
        self.assertEqual(len(events), 1)
        self.assertAlmostEqual(events[0]['start_seconds'], 20.05)

    def test_standalone_backing_and_interior_parentheses_are_not_optional_leads(self):
        for sheet in ('(' + TEXT + ')', TEXT + ' (echo) then we fall'):
            self.assertEqual(occurrences.proposals(
                dict(lines=[], unresolved=[dict(reference_line_index=0)]), sheet, reports()), [])

    def test_one_authored_template_can_have_two_performed_instances(self):
        events=occurrences.proposals(dict(lines=[]),TEXT+'\n'+TEXT,reports())
        self.assertEqual(len(events),2)
        self.assertEqual([e['reference_line_indices'] for e in events],[[0,1],[0,1]])
        accepted=occurrences.confirm(events,fresh(),[])
        self.assertEqual(len(accepted),2)
        self.assertAlmostEqual(accepted[0]['start_seconds'],10.1)
        self.assertAlmostEqual(accepted[1]['start_seconds'],20.1)
        self.assertTrue(all(row['uncertain'] for row in accepted))

    def test_existing_placement_owns_only_its_occurrence(self):
        placed=dict(text=TEXT,start_seconds=10,end_seconds=12)
        events=occurrences.proposals(dict(lines=[placed]),TEXT,reports())
        self.assertEqual(len(events),1)
        self.assertAlmostEqual(events[0]['start_seconds'],20.05)

    def test_single_original_crop_cannot_propose_an_extra(self):
        self.assertEqual(occurrences.proposals(dict(lines=[]),TEXT,reports()[:1]),[])

    def test_original_corroboration_does_not_replace_fresh_confirmation(self):
        events=occurrences.proposals(dict(lines=[]),TEXT,reports())
        for values in ([],fresh()[:1],[fresh()[0],fresh()[0]]):
            self.assertEqual(occurrences.confirm(events,values,[]),[])

    def test_wrong_words_clipped_words_or_bad_timing_do_not_confirm(self):
        events=occurrences.proposals(dict(lines=[]),TEXT,reports())
        for change in ({'text':'A completely different song'},{'partial_start':True},
                       {'uncertain':True},{'end_seconds':15.}):
            values=copy.deepcopy(fresh())
            for report in values:
                for row in report['lines']:row.update(change)
            self.assertEqual(occurrences.confirm(events,values,[]),[])

    def test_two_events_cannot_consume_one_fresh_performance(self):
        event=occurrences.proposals(dict(lines=[]),TEXT,reports())[0]
        self.assertEqual(occurrences.confirm([event,copy.deepcopy(event)],fresh(),[]),[])

    def test_confirmation_windows_are_bounded_and_distinct(self):
        events=occurrences.proposals(dict(lines=[]),TEXT,reports())
        spans=occurrences.confirmation_spans(events,30)
        self.assertEqual(len(spans),2)
        self.assertNotEqual(spans[0][0],spans[1][0])
        self.assertTrue(all(0<=start<start+length<=30 and length<=30 for start,length in spans))

    def test_occurrences_reach_bridge_and_counts_without_entering_authored_ledger(self):
        authored=dict(text='Opening line',start_seconds=1,end_seconds=3)
        added=occurrences.confirm(occurrences.proposals(dict(lines=[]),TEXT,reports()),fresh(),[])
        lyrics=dict(lines=[authored],performed_occurrences=added,unresolved=[],review_flags=[],performed_candidates=[])
        plan=dict(audio=dict(sha256='a'*64,duration_seconds=30),sections=[dict(start_seconds=0,end_seconds=30,recommended_scene='spectrum',transition_strength=.5,reasons=[])])
        bridge=external_analysis.build_bridge(plan,lyrics=lyrics)
        rows=[row for row in external_analysis.parse_bridge(bridge) if row[0]=='LYRIC']
        self.assertEqual([int(row[2]) for row in rows],[1000,10100,20100])
        self.assertEqual([row[5] for row in rows],['none','uncertain','uncertain'])
        manifest=external_analysis.build_assist_manifest(mode='lyrics',audio_sha='a'*64,measured_duration=30,
            cache_status={},paths={'manifest':Path('manifest.json')},plan=plan,lyrics=lyrics,semantic=None)
        self.assertEqual(manifest['result_counts']['lyrics'],3)
        self.assertEqual(manifest['result_counts']['lyrics_review_flags'],2)
        self.assertEqual(lyrics['lines'],[authored])

class AuthorizedExtension(unittest.IsolatedAsyncioTestCase):
    async def test_denial_precedes_filesystem_discovery_and_network(self):
        with patch.object(occurrences,'sha256_file',side_effect=AssertionError('read forbidden')):
            with self.assertRaisesRegex(ValueError,'confirmation'):
                await occurrences.extend(Path('none'),{}, {},Path('none'),model='gemini-3.8-flash-high',confirmed=False)

    async def test_extension_preserves_ledger_and_reuses_same_request_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);track=root/'audio';track.write_bytes(b'fixture');server=root/'server';server.write_bytes(b'fixture server');harness=root/'harness';harness.write_bytes(b'fixture harness')
            identity=dict(sha256=sha256_file(track),duration_seconds=30)
            source=dict(audio=identity,provenance=dict(source_kind='antigravity_audio'),
                source=dict(reference=dict(text=TEXT),boundary_observations=reports()),lines=[])
            document=dict(audio=identity,lines=[],unresolved=[dict(reference_line_index=0,text=TEXT)],unmatched=[dict(text=TEXT)],review_flags=[],unreliable_evidence=[],statistics={})
            saved=copy.deepcopy(document)
            observe=AsyncMock(return_value=fresh())
            with patch.object(occurrences.antigravity_audio,'discover',return_value=dict(server=server,harness=harness,profile=root)),patch.object(occurrences.audio,'observe_spans',observe):
                result=await occurrences.extend(track,source,document,root,model='gemini-3.8-flash-high',confirmed=True)
                again=await occurrences.extend(track,source,result,root,model='gemini-3.8-flash-high',confirmed=True)
            self.assertEqual(document,saved)
            for field in ('lines','unresolved','unmatched','review_flags'):
                self.assertEqual(result[field],saved[field])
            self.assertEqual(len(result['performed_occurrences']),2)
            self.assertEqual(result,again)
            self.assertEqual(observe.await_args_list[0].args[1],observe.await_args_list[1].args[1])
            self.assertNotIn(TEXT,json.dumps(observe.await_args_list[0].args[1]))

if __name__=='__main__':unittest.main()
