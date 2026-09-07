"""Independent phrase references retain coverage and ambiguity evidence."""
import copy,json,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools/lyrics_research'))
import performed_reference as p

def row(text,start,end,**flags):
    return dict(text=text,start_seconds=start,end_seconds=end,uncertain=False,
                partial_start=False,partial_end=False,**flags)

def fixture():
    track=dict(duration_seconds=30,sha256='a'*64)
    identity=dict(audio_sha256=track['sha256'],duration_seconds=30,spans=[(0.,21.),(15.,15.)])
    values=[[row('lead phrase',1.,3.),row('echo phrase',3.2,5.2),row('boundary phrase',17.8,19.)],
            [row('later phrase',5.,7.)]]
    reports=[dict(identity=identity,clip_start_seconds=offset,clip_duration_seconds=length,
                  clip_sha256='b'*64,lines=rows,response=json.dumps(dict(lines=rows,notes=[])))
             for (offset,length),rows in zip(identity['spans'],values)]
    return track,identity,reports

class ReferenceCoverage(unittest.TestCase):
    def test_short_retriggers_are_selected_for_audio_review_without_merging_echoes(self):
        reports=[dict(lines=[row('so',1,1.2),row('so so',1.4,1.8)]),
                 dict(lines=[row('I really think so',1,2),row('I really think so',2.2,3.2)]),
                 dict(lines=[row('yes',1,1.2),row('yes',4,4.2)])]
        saved=copy.deepcopy(reports)
        self.assertEqual(p.chop_grouping_intervals(reports),[0])
        self.assertEqual(reports,saved)

    def test_chop_run_reference_retains_actual_original_and_revised_request_identities(self):
        track,identity,reports=fixture()
        original=copy.deepcopy(reports)
        grouping={**identity,'phase':'chop-review','spans':[identity['spans'][0]]}
        revised=copy.deepcopy(reports)
        revised[0]['identity']=grouping
        revised[0]['lines']=[row('so so so',1,3)]
        revised[0]['response']=json.dumps(dict(lines=revised[0]['lines'],notes=[]))
        result=p.freeze(track,identity,revised,regrouping_identity=grouping,regrouping_indices=[0])
        self.assertEqual([r['text'] for r in result['lines']],['so so so','later phrase'])
        self.assertEqual(result['regrouping_indices'],[0])
        self.assertNotEqual(result['coverage'][0]['request_identity_sha256'],
                            result['coverage'][1]['request_identity_sha256'])
        self.assertEqual(reports,original)
        revised[0]['identity']=identity
        with self.assertRaisesRegex(ValueError,'another audio request'):
            p.freeze(track,identity,revised,regrouping_identity=grouping,regrouping_indices=[0])

    def test_chop_review_cannot_replace_another_interval_or_track(self):
        track,identity,reports=fixture()
        grouping={**identity,'phase':'chop-review','spans':[identity['spans'][0]]}
        for indices,changed in (([1],grouping),([0,0],grouping),
                                ([0],{**grouping,'audio_sha256':'another track'})):
            with self.assertRaisesRegex(ValueError,'selected audio intervals'):
                p.freeze(track,identity,reports,regrouping_identity=changed,regrouping_indices=indices)

    def test_adjacent_audio_resolves_ownership_without_editing_frozen_reference(self):
        track,identity,reports=fixture()
        reports[1]['lines'].append(row('boundary phrase',2.9,4.1))
        reports[1]['lines'].sort(key=lambda r:r['start_seconds'])
        reports[1]['response']=json.dumps(dict(lines=reports[1]['lines'],notes=[]))
        reference=p.freeze(track,identity,reports);saved=copy.deepcopy(reference)
        result=p.resolve_ownership(reference,reports)
        self.assertEqual(reference,saved)
        self.assertEqual(result['uncertain'],[])
        self.assertEqual(len(result['lines']),4)
        self.assertEqual(len(result['ownership_resolution']['resolved']),1)
        self.assertEqual(result['acceptance_status'],'not_adjudicated')
        result['lines'][0]['text']='changed reference'
        with self.assertRaisesRegex(ValueError,'original audio receipts'):
            p.resolve_ownership(result,reports)

    def test_ownership_requires_agreeing_unique_complete_adjacent_view(self):
        for change in ('wording','timing','partial','duplicate'):
            track,identity,reports=fixture()
            peer=row('boundary phrase',2.9,4.1)
            if change=='wording':peer['text']='different phrase'
            if change=='timing':peer['end_seconds']=5.1
            if change=='partial':peer['partial_start']=True
            reports[1]['lines'].append(peer)
            if change=='duplicate':reports[1]['lines'].append(copy.deepcopy(peer))
            reports[1]['lines'].sort(key=lambda r:r['start_seconds'])
            reports[1]['response']=json.dumps(dict(lines=reports[1]['lines'],notes=[]))
            reference=p.freeze(track,identity,reports)
            result=p.resolve_ownership(reference,reports)
            self.assertEqual(result['uncertain'],reference['uncertain'])
            self.assertEqual(result['ownership_resolution']['resolved'],[])

    def test_full_coverage_and_separate_performed_phrases(self):
        result=p.freeze(*fixture())
        self.assertEqual([r['text'] for r in result['lines']],['lead phrase','echo phrase','later phrase'])
        self.assertEqual(result['lines'][-1]['start_seconds'],20)
        self.assertEqual(result['uncertain'][0]['reasons'],['interval_ownership'])
        self.assertEqual([(r['start_seconds'],r['end_seconds']) for r in result['coverage']],[(0,18),(18,30)])
        self.assertEqual(result['acceptance_status'],'not_adjudicated')

    def test_cached_json_identity_and_track_identity_are_checked(self):
        track,identity,reports=fixture()
        self.assertEqual(p.freeze(track,identity,reports),p.freeze(track,identity,json.loads(json.dumps(reports))))
        track['sha256']='different audio'
        with self.assertRaisesRegex(ValueError,'another track'):p.freeze(track,identity,reports)

    def test_missing_receipt_wrong_clip_and_changed_raw_response_are_refused(self):
        for change in ('missing','offset','parsed'):
            track,identity,reports=fixture()
            if change=='missing':reports.pop()
            elif change=='offset':reports[1]['clip_start_seconds']=16
            else:reports[0]['lines'][0]['text']='altered words'
            with self.assertRaises(ValueError):p.freeze(track,identity,reports)

    def test_uncertain_and_clipped_phrases_are_not_discarded(self):
        track,identity,reports=fixture()
        for flag in ('uncertain','partial_start','partial_end'):
            values=copy.deepcopy(reports)
            values[0]['lines'][0][flag]=True
            values[0]['response']=json.dumps(dict(lines=values[0]['lines'],notes=[]))
            result=p.freeze(track,identity,values)
            self.assertEqual(result['uncertain'][0]['reasons'],[flag])
            self.assertEqual(result['uncertain'][0]['text'],'lead phrase')

if __name__=='__main__':unittest.main()
