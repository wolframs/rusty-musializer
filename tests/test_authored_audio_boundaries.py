"""Original-crop matching must preserve authored words and distinct occurrences."""
import copy
import json
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import authored_audio_boundaries as audio


def phrase(text, start, end, **kw):
    return dict(text=text, start_seconds=start, end_seconds=end,
                partial_start=False, partial_end=False, uncertain=False, **kw)


def clip(offset, rows):
    return dict(clip_start_seconds=offset, clip_duration_seconds=10, lines=rows)


def fixture():
    text='A quiet voice follows me'
    document=dict(lines=[dict(text=text, reference_line_index=4,
        start_seconds=5., end_seconds=9.)], unresolved=[dict(text='Not found')])
    reports=[clip(0,[phrase(text,5.1,8.1)]),clip(2,[
        phrase('A quiet voice',3.2,4.5),phrase('follows me',4.6,6.2)])]
    return document,reports


class Boundaries(unittest.TestCase):
    def test_split_and_whole_views_fuse_without_changing_supplied_words(self):
        doc,reports=fixture();original=copy.deepcopy(doc);result=audio.refine(doc,reports)
        row=result['lines'][0]
        self.assertEqual(row['text'],doc['lines'][0]['text'])
        self.assertEqual((row['start_seconds'],row['end_seconds']),(5.1,8.2))
        self.assertEqual(result['unresolved'],doc['unresolved'])
        self.assertEqual(doc,original)
        self.assertEqual(result['audio_boundary_refinement']['refined'],1)

    def test_single_or_duplicate_crop_cannot_corroborate_itself(self):
        doc,reports=fixture()
        for evidence in (reports[:1],[reports[0],copy.deepcopy(reports[0])]):
            result=audio.refine(doc,evidence)
            self.assertEqual(result['lines'],doc['lines'])

    def test_interior_words_do_not_inherit_the_surrounding_phrase_boundaries(self):
        doc,reports=fixture();doc['lines'][0]['text']='quiet voice'
        self.assertEqual(audio.refine(doc,reports)['lines'],doc['lines'])

    def test_clipped_or_uncertain_view_does_not_corroborate(self):
        doc,reports=fixture()
        for flag in ('partial_start','partial_end','uncertain'):
            evidence=copy.deepcopy(reports);evidence[0]['lines'][0][flag]=True
            self.assertEqual(audio.refine(doc,evidence)['lines'],doc['lines'])

    def test_disagreeing_clocks_and_distant_occurrences_keep_original_timing(self):
        doc,reports=fixture();reports[1]['lines'][-1]['end_seconds']+=1
        self.assertEqual(audio.refine(doc,reports)['lines'],doc['lines'])
        doc,reports=fixture();doc['lines'][0]['start_seconds']=0
        self.assertEqual(audio.refine(doc,reports)['lines'],doc['lines'])

    def test_two_cues_cannot_consume_the_same_heard_occurrence(self):
        doc,reports=fixture();doc['lines'].append({**doc['lines'][0],'reference_line_index':5})
        result=audio.refine(doc,reports)
        self.assertEqual(result['lines'],doc['lines'])
        self.assertEqual(result['audio_boundary_refinement']['collisions'],[0,1])

    def test_distinct_repetitions_keep_distinct_occurrences(self):
        doc,reports=fixture();doc['lines'].append({**doc['lines'][0],
            'reference_line_index':5,'start_seconds':10.,'end_seconds':14.})
        for report in reports:
            report['lines'] += [dict(row, start_seconds=row['start_seconds']+5,
                end_seconds=row['end_seconds']+5) for row in list(report['lines'])]
            report['clip_duration_seconds']=20
        result=audio.refine(doc,reports)
        self.assertEqual(result['audio_boundary_refinement']['refined'],2)
        self.assertEqual([r['end_seconds'] for r in result['lines']],[8.2,13.2])

    def test_long_silent_gap_cannot_join_two_halves(self):
        doc,reports=fixture();reports[1]['lines'][-1]['start_seconds']+=3
        self.assertEqual(audio.refine(doc,reports)['lines'],doc['lines'])

    def test_boundary_refinement_cannot_reverse_the_authored_path(self):
        doc,reports=fixture();doc['lines'].append(dict(text='Other lyric line',
            reference_line_index=5,start_seconds=5.05,end_seconds=9.0))
        result=audio.refine(doc,reports)
        self.assertEqual(result['lines'],doc['lines'])
        self.assertEqual(result['audio_boundary_refinement']['declined_order'],[0])

    def test_review_link_follows_the_refined_cue_and_keeps_original_warning(self):
        doc,reports=fixture();doc['review_flags']=[dict(reference_line_index=4,
            start_seconds=5.,end_seconds=9.,reason='cross-view disagreement')]
        result=audio.refine(doc,reports);flag=result['review_flags'][0]
        self.assertEqual((flag['start_seconds'],flag['end_seconds']),(5.1,8.2))
        self.assertIn('before audio boundary refinement',flag['reason'])

    def test_emitted_audit_fields_are_declared_in_the_shipped_schema(self):
        doc,reports=fixture();result=audio.refine(doc,reports)
        schema=json.loads((Path(__file__).resolve().parents[1]/'schemas/lyric-sync-v1.schema.json').read_text())
        summary=result['audio_boundary_refinement']
        self.assertEqual(set(summary)-set(schema['properties']['audio_boundary_refinement']['properties']),set())
        evidence=result['lines'][0]['audio_boundary_evidence']
        declared=schema['$defs']['audio_boundary_evidence']['properties']
        self.assertEqual(set(evidence)-set(declared),set())
        for view in evidence['observations']:
            self.assertEqual(set(view)-set(declared['observations']['items']['properties']),set())

if __name__=='__main__':unittest.main()
