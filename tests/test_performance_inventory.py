"""Performance-led spelling must not invent words, events or timing."""
import copy,hashlib,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools/lyrics_research'))
import performance_inventory as p

def reference(text):
    return dict(text=text,source='fixture',sha256=hashlib.sha256(text.encode()).hexdigest())

def document(*texts):
    return dict(schema_version='musializer.lyric-review/v1',source={},lines=[
        dict(text=text,start_seconds=i*3.,end_seconds=i*3.+2.,uncertain=True,
             confidence=.5,source_line_indices=[i]) for i,text in enumerate(texts)])

class PerformanceInventory(unittest.TestCase):
    def test_exact_spelling_keeps_performed_count_times_and_source_observations(self):
        source=document("I've been waiting",'unwritten adlib',"I've been waiting")
        original=copy.deepcopy(source)
        result=p.project(source,reference('I’ve been waiting, for you\nAn unperformed written line'))
        self.assertEqual(source,original)
        self.assertEqual([r['text'] for r in result['lines']],['I’ve been waiting,','unwritten adlib','I’ve been waiting,'])
        for before,after in zip(source['lines'],result['lines']):
            self.assertEqual({k:v for k,v in before.items() if k!='text'},
                             {k:v for k,v in after.items() if k!='text'})
        self.assertEqual(result['source']['local_wording']['unmatched_cue_indices'],[1])
        self.assertEqual(result['source']['local_wording']['matches'][0]['original_audio_text'],"I've been waiting")

    def test_fuzzy_expected_words_cannot_replace_performed_variants(self):
        source=document('You built the box before you built the box')
        result=p.project(source,reference('You built the box before you built the question'))
        self.assertEqual(result['lines'],source['lines'])
        self.assertEqual(result['source']['local_wording']['matches'],[])

    def test_ambiguous_spelling_and_production_notes_remain_audio_wording(self):
        source=document('take the road','dramatic pause')
        result=p.project(source,reference('Take the road!\nTake the road?\n[dramatic pause]'))
        self.assertEqual(result['lines'],source['lines'])

    def test_reference_hash_and_input_lane_are_required(self):
        sheet=reference('some words');sheet['text']='changed'
        with self.assertRaisesRegex(ValueError,'changed'):p.project(document('some words'),sheet)
        with self.assertRaisesRegex(ValueError,'review inventory'):p.project({},reference('some words'))

if __name__=='__main__':unittest.main()
