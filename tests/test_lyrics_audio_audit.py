"""The operator's full-line error metric, with model uncertainty kept explicit."""
import json
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools/lyrics_research'))
from audit_candidate import parse, summarize

class AuditMetric(unittest.TestCase):
    def test_both_wrong_boundaries_count_once_and_half_second_is_allowed(self):
        rows=[dict(start_seconds=10.,end_seconds=12.) for _ in range(4)]
        report=dict(identity=dict(start=8.),audit=dict(cues=[
            dict(id=0,status='present',start_seconds=2.5,end_seconds=4.5),
            dict(id=1,status='present',start_seconds=3.,end_seconds=5.),
            dict(id=2,status='absent'),dict(id=3,status='uncertain')],
            missing=[dict(text='A missing line',start_seconds=6.,end_seconds=7.,uncertain=False)]))
        counts,denominator,errors=summarize(rows,[report])
        self.assertEqual(counts,dict(present=2,timing_errors=1,extra=1,missing=1,uncertain=1))
        self.assertEqual(denominator,3)
        self.assertEqual(errors,3)

    def test_omitted_or_duplicated_judgements_cannot_pass_as_full_coverage(self):
        for ids in ([0],[0,0,1],[0,1,2]):
            with self.assertRaises(ValueError):
                parse(json.dumps(dict(cues=[dict(id=i,status='uncertain') for i in ids],missing=[])),[0,1],30,(3,21))

    def test_missing_phrase_must_belong_to_the_current_audio_interval(self):
        with self.assertRaises(ValueError):
            parse(json.dumps(dict(cues=[],missing=[dict(text='Already covered',start_seconds=2,end_seconds=4,uncertain=False)])),[],30,(3,21))

if __name__=='__main__':unittest.main()
