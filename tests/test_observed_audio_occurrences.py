"""New audio wording needs paired original and fresh evidence before rendering."""
import copy,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import AsyncMock,patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools'))
import observed_audio_occurrences as observed
import authored_audio_occurrences as authored
import authored_audio_phrases as phrases
from analysis_io import sha256_file
from test_authored_audio_occurrences import reports,fresh


def voice(values,text='Woo'):
    result=copy.deepcopy(values)
    for report in result:
        for row in report['lines']:row['text']=text
    return result

class ObservedPerformances(unittest.TestCase):
    def test_single_word_adlibs_are_separate_uncertain_performances(self):
        proposed=observed.occurrences(voice(reports()))
        self.assertEqual(len(proposed),2)
        accepted=observed.confirm(proposed,voice(fresh()),[])
        self.assertEqual(len(accepted),2)
        self.assertEqual([round(row['start_seconds'],2) for row in accepted],[10.1,20.1])
        self.assertTrue(all(row['text_scope']=='audio_observed' and row['reference_line_indices']==[]
                            and row['uncertain'] for row in accepted))

    def test_original_and_fresh_crops_must_be_distinct_and_complete(self):
        self.assertEqual(observed.occurrences(voice(reports()[:1])),[])
        self.assertEqual(observed.occurrences(voice([reports()[0],reports()[0]])),[])
        proposed=observed.occurrences(voice(reports()))
        for values in ([],voice(fresh()[:1]),voice([fresh()[0],fresh()[0]])):
            self.assertEqual(observed.confirm(proposed,values,[]),[])
        for flag in ('uncertain','partial_start','partial_end'):
            values=voice(fresh())
            for report in values:
                for row in report['lines']:row[flag]=True
            self.assertEqual(observed.confirm(proposed,values,[]),[])

    def test_wrong_words_wrong_clock_and_shared_fresh_rows_do_not_confirm(self):
        proposed=observed.occurrences(voice(reports()))
        self.assertEqual(observed.confirm(proposed,voice(fresh(),'No'),[]),[])
        shifted=voice(fresh())
        for report in shifted:
            for row in report['lines']:row['start_seconds']+=1;row['end_seconds']+=1
        self.assertEqual(observed.confirm(proposed,shifted,[]),[])
        self.assertEqual(observed.confirm([proposed[0],proposed[0]],voice(fresh()),[]),[])

    def test_existing_caption_owns_a_contained_word_only_in_its_actual_window(self):
        proposed=observed.occurrences(voice(reports()))
        placed=dict(text='We sing woo with you tonight',start_seconds=9,end_seconds=13)
        self.assertTrue(observed.explained(proposed[0],[placed]))
        self.assertFalse(observed.explained(proposed[1],[placed]))
        self.assertEqual(len(observed.confirm(proposed,voice(fresh()),[placed])),1)

    def test_an_owned_source_row_cannot_be_relabelled_as_another_performance(self):
        values=voice(reports());events=observed.occurrences(values)
        placed=dict(text='Different canonical wording',start_seconds=10,end_seconds=12,audio_boundary_evidence=dict(observations=events[0]['observations']))
        self.assertEqual(len(observed.proposals(values,[placed])),1)
        self.assertAlmostEqual(observed.proposals(values,[placed])[0]['start_seconds'],20.05)

    def test_stutter_matching_does_not_collapse_whole_word_repetitions(self):
        self.assertEqual(observed._words('To-to-total'),['total'])
        self.assertEqual(observed._words('Go-go-go the theory'),['go','go','go','the','theory'])

class AuthorizedObservedExtension(unittest.IsolatedAsyncioTestCase):
    async def test_denial_precedes_audio_reads(self):
        with patch.object(observed,'sha256_file',side_effect=AssertionError('must not read')):
            with self.assertRaisesRegex(ValueError,'confirmation'):
                await observed.extend(Path('none'),{}, {},Path('none'),model='gemini-3.8-flash-high',confirmed=False)

    async def test_resume_preserves_authored_ledger_and_split_source_links(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);track=root/'audio';track.write_bytes(b'audio');identity=dict(sha256=sha256_file(track),duration_seconds=30)
            from test_authored_audio_phrases import document,reports as phrase_reports
            originals=phrase_reports()
            for report in originals:
                report['lines'].append(dict(text='Woo',start_seconds=20-report['clip_start_seconds'],end_seconds=22-report['clip_start_seconds'],uncertain=False,partial_start=False,partial_end=False))
            source=dict(audio=identity,provenance=dict(source_kind='antigravity_audio'),source=dict(boundary_observations=originals),lines=[])
            d=document();d['audio']=identity
            d['phrase_splits']=phrases.confirm(phrases.proposals(d,phrase_reports()),phrase_reports())
            saved=copy.deepcopy(d)
            observer=AsyncMock(return_value=voice(fresh()))
            with patch.object(observed.antigravity_audio,'discover',return_value=dict(server=track,harness=track,profile=root)),patch.object(observed.audio,'observe_spans',observer):
                first=await observed.extend(track,source,d,root,model='gemini-3.8-flash-high',confirmed=True)
                second=await observed.extend(track,source,first,root,model='gemini-3.8-flash-high',confirmed=True)
            self.assertEqual(first,second)
            self.assertEqual(d,saved)
            for key in ('lines','unresolved','unmatched','review_flags','phrase_splits'):
                self.assertEqual(first[key],d[key])
            self.assertEqual(len(authored.rendered_lines(first)),3)
            self.assertEqual(observer.await_args_list[0].args[1],observer.await_args_list[1].args[1])
            self.assertNotIn('Woo',json.dumps(observer.await_args_list[0].args[1]))

if __name__=='__main__':unittest.main()
