"""Missing lyrics need independent occurrence evidence, not a forced CTC path."""
import copy
import json
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools'))
import authored_audio_boundaries as audio
import lyric_anchor_block


def row(text,start,end):
    return dict(text=text,start_seconds=start,end_seconds=end,
                uncertain=False,partial_start=False,partial_end=False)


def fixture():
    text='A quiet voice follows me'
    missing=dict(reference_line_index=1,line_position=1,kind='lyric',text=text,
        reason='global_localization_disagreement',abstained=True,
        acoustic_start_seconds=25.,acoustic_end_seconds=28.)
    doc=dict(lines=[dict(reference_line_index=0,text='Opening lyric line',start_seconds=1.,end_seconds=2.),
                   dict(reference_line_index=2,text='Following lyric line',start_seconds=12.,end_seconds=13.)],
        unresolved=[missing],unmatched=[dict(reference_line_index=1,text=text,reason=missing['reason'])],
        review_flags=[dict(reference_line_index=1,text=text,flag='unresolved')],
        statistics=dict(reference_lines=3,matched_lines=2,unmatched_lines=1,unresolved_lines=1,abstained_lines=1,review_flagged_lines=1))
    reports=[dict(clip_start_seconds=0,clip_duration_seconds=15,lines=[row(text,5.1,8.1)]),
             dict(clip_start_seconds=2,clip_duration_seconds=13,lines=[row(text,3.2,6.2)])]
    return doc,reports


class Recovery(unittest.TestCase):
    def test_recovers_supplied_words_and_preserves_prior_rejection_for_review(self):
        doc,reports=fixture();saved=copy.deepcopy(doc);result=audio.recover(doc,reports,audio_duration=15)
        self.assertEqual(doc,saved)
        recovered=result['lines'][1]
        self.assertEqual(recovered['text'],doc['unresolved'][0]['text'])
        self.assertAlmostEqual(recovered['start_seconds'],5.15)
        self.assertAlmostEqual(recovered['end_seconds'],8.15)
        self.assertTrue(recovered['uncertain']);self.assertTrue(recovered['review_flagged'])
        self.assertEqual(recovered['audio_occurrence_evidence']['previous_alignment'],doc['unresolved'][0])
        self.assertEqual((result['unresolved'],result['unmatched']),([],[]))
        self.assertEqual(result['statistics']['matched_lines'],3)
        self.assertEqual(result['statistics']['abstained_lines'],0)
        self.assertEqual(result['review_flags'][0]['flag'],'audio_occurrence_recovered')
        lyric_anchor_block.validate_full_coverage(result,[dict(index=i) for i in range(3)])

    def test_competing_single_view_cannot_lose_to_a_better_sampled_repeat(self):
        doc,reports=fixture();reports.append(dict(clip_start_seconds=8,clip_duration_seconds=7,
            lines=[row(doc['unresolved'][0]['text'],1,2)]))
        result=audio.recover(doc,reports,audio_duration=15)
        self.assertEqual(result['unresolved'],doc['unresolved'])
        self.assertEqual(result['audio_occurrence_recovery']['declined_ambiguous'],[1])

    def test_single_duplicate_or_uncertain_crops_cannot_recover(self):
        doc,reports=fixture()
        uncertain=copy.deepcopy(reports);uncertain[1]['lines'][0]['uncertain']=True
        for evidence in (reports[:1],[reports[0],reports[0]],uncertain):
            self.assertEqual(audio.recover(doc,evidence,audio_duration=15)['unresolved'],doc['unresolved'])

    def test_competing_end_is_ambiguous_even_when_the_start_agrees(self):
        doc,reports=fixture();reports.append(dict(clip_start_seconds=1,clip_duration_seconds=14,
            lines=[row(doc['unresolved'][0]['text'],4.15,9.2)]))
        result=audio.recover(doc,reports,audio_duration=15)
        self.assertEqual(result['unresolved'],doc['unresolved'])
        self.assertEqual(result['audio_occurrence_recovery']['declined_ambiguous'],[1])

    def test_existing_neighbour_bounds_are_not_replaced_by_coarse_guesses(self):
        doc,reports=fixture();doc['lines'][0]['start_seconds']=9
        self.assertEqual(audio.recover(doc,reports,audio_duration=15)['unresolved'],doc['unresolved'])

    def test_an_existing_unfused_cue_still_owns_its_performance(self):
        doc,reports=fixture();doc['lines'][1].update(text=doc['unresolved'][0]['text'],start_seconds=5.25)
        result=audio.recover(doc,reports,audio_duration=15)
        self.assertEqual(result['unresolved'],doc['unresolved'])
        self.assertEqual(result['audio_occurrence_recovery']['declined_competing'],[1])

    def test_two_missing_lines_cannot_claim_one_occurrence(self):
        doc,reports=fixture();doc['lines'][1]['reference_line_index']=3
        doc['unresolved'].append(dict(doc['unresolved'][0],reference_line_index=2,line_position=2))
        result=audio.recover(doc,reports,audio_duration=15)
        self.assertEqual(result['unresolved'],doc['unresolved'])
        self.assertEqual(result['audio_occurrence_recovery']['declined_competing'],[1,2])

    def test_recovered_lines_cannot_reverse_each_other(self):
        doc,reports=fixture();doc['lines'][1]['reference_line_index']=3
        doc['unresolved'].append(dict(doc['unresolved'][0],reference_line_index=2,line_position=2))
        doc['unresolved'][0]['text']='Golden dreams return'
        for report in reports:
            offset=report['clip_start_seconds'];report['lines'].append(row('Golden dreams return',9-offset,10-offset))
        result=audio.recover(doc,reports,audio_duration=15)
        self.assertEqual(result['unresolved'],doc['unresolved'])
        self.assertEqual(result['audio_occurrence_recovery']['declined_order'],[1,2])

    def test_auditable_recovery_fields_are_in_the_schema(self):
        doc,reports=fixture();result=audio.recover(doc,reports,audio_duration=15)
        schema=json.loads((Path(__file__).resolve().parents[1]/'schemas/lyric-sync-v1.schema.json').read_text())
        self.assertFalse(set(result['audio_occurrence_recovery'])-set(schema['properties']['audio_occurrence_recovery']['properties']))
        self.assertFalse(set(result['lines'][1])-set(schema['$defs']['line']['properties']))
        self.assertIn(result['review_flags'][0]['flag'],schema['properties']['review_flags']['items']['properties']['flag']['enum'])

if __name__=='__main__':unittest.main()
