"""Controls for stable full-track reference counts across candidate iterations."""
from pathlib import Path
import json
import sys
import tempfile
import unittest
import itertools
import random

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools/lyrics_research'))
from fixed_reference import compare, freeze, _assignment, _matching_tokens
from analysis_io import canonical_sha256


def cue(text, start, end):
    return dict(text=text, start_seconds=start, end_seconds=end)


class FixedReferenceMetric(unittest.TestCase):
    def test_syllable_stutter_matches_word_but_keeps_its_initial_phoneme_time(self):
        reference = [cue('To-to-to-total mode collapse', 1, 3)]
        self.assertEqual(compare(reference, [cue('Total mode collapse', 1, 3)])['error_count'], 0)
        shifted = compare(reference, [cue('Total mode collapse', 1.6, 3)])
        self.assertEqual(shifted['timing_errors'], 1)
        self.assertEqual((shifted['missing'], shifted['extra']), ([], []))
        self.assertEqual(_matching_tokens('Go-go-go the theory in-in the night'),
                         ['go', 'go', 'go', 'the', 'theory', 'in', 'in', 'the', 'night'])
        self.assertEqual(_matching_tokens('B-b-baby blue-green'), ['baby', 'blue', 'green'])

    def test_spacing_in_syllable_stutters_does_not_create_a_missing_line(self):
        reference=[cue('To-to-total mode collapse',1,3)]
        self.assertEqual(compare(reference,[cue('To- to- to- total mode collapse',1,3)])['error_count'],0)
        self.assertEqual(_matching_tokens('Go - go - go the theory'),['go','go','go','the','theory'])

    def test_chop_run_retrigger_count_does_not_change_phrase_identity_or_count(self):
        reference=[cue('so '*32,1,5)]
        self.assertEqual(compare(reference,[cue('so '*8,1,5)])['error_count'],0)
        short=compare(reference,[cue('so '*8,1,3)])
        self.assertEqual((short['timing_errors'],short['missing'],short['extra']),(1,[],[]))
        separate=compare(reference,[cue('so',1,2),cue('so',2,3),cue('so',3,4),cue('so',4,5)])
        self.assertEqual(len(separate['extra']),3)
        self.assertEqual(separate['reference_lines'],1)
        calls=[cue('No',1,2),cue('No',3,4)]
        self.assertGreater(compare(calls,[cue('No No',1,4)])['error_count'],0)

    def test_lead_and_echo_are_two_performed_phrases_even_with_identical_words(self):
        reference = [cue('Read a book once in a while', 1, 3),
                     cue('Read a book once in a while', 3.2, 5.2)]
        merged = [cue('Read a book once in a while (Read a book once in a while)', 1, 5.2)]
        result = compare(reference, merged)
        self.assertEqual(result['reference_lines'], 2)
        self.assertGreater(result['error_count'], 0)
        self.assertEqual(compare(reference, reference)['error_count'], 0)

    def test_a_misplaced_unique_line_does_not_reassign_the_later_repeats(self):
        reference=[cue('Again',10,12),cue('Something different',15,17),
                   cue('Again',20,22),cue('Again',30,32),cue('The ending',40,42)]
        candidate=[cue('Something different',5,7),cue('Again',10,12),
                   cue('Again',20,22),cue('Again',30,32),cue('The ending',40,42)]
        result=compare(reference,candidate)
        self.assertEqual(result['error_count'],1)
        self.assertEqual(result['timing_errors'],1)
        self.assertEqual((result['missing'],result['extra']),([],[]))
        self.assertEqual([(row['reference_index'],row['candidate_index'])
                          for row in result['decisions']],[(0,1),(1,0),(2,2),(3,3),(4,4)])

    def test_assignment_cost_matches_exhaustive_small_graphs(self):
        rng=random.Random(531)
        for n,m in ((1,3),(2,3),(3,5),(4,5)):
            for _ in range(12):
                costs=[[rng.randrange(-20,20) for _ in range(m)] for _ in range(n)]
                pairs=_assignment(costs)
                actual=sum(costs[i][j] for i,j in pairs)
                expected=min(sum(costs[i][j] for i,j in enumerate(columns))
                             for columns in itertools.permutations(range(m),n))
                self.assertEqual(actual,expected)
                self.assertEqual(len({j for _,j in pairs}),n)

    def test_spelling_variants_cannot_swap_two_distant_choruses(self):
        reference = [cue('We follow every signal', 10, 12),
                     cue('We follow any signal', 130, 132)]
        candidate = [cue('We follow any signal', 10.1, 12.1),
                     cue('We follow every signal', 130.1, 132.1)]
        result = compare(reference, candidate)
        self.assertEqual(result['error_count'], 0)
        self.assertEqual([(row['reference_index'], row['candidate_index'])
                          for row in result['decisions']], [(0, 0), (1, 1)])
        # Timing cannot rescue a lexically ineligible replacement.
        wrong = compare(reference, [cue('Completely unrelated words', 10, 12),
                                    candidate[1]])
        self.assertEqual((len(wrong['missing']), len(wrong['extra'])), (1, 1))

    def test_nearby_wording_variant_still_counts_a_bad_edge(self):
        reference = [cue('We follow every signal', 10, 12),
                     cue('We follow any signal', 130, 132)]
        candidate = [cue('We follow any signal', 10.6, 12),
                     cue('We follow every signal', 130, 132)]
        result = compare(reference, candidate)
        self.assertEqual((result['error_count'], result['timing_errors']), (1, 1))

    def test_additional_occurrences_cannot_be_omitted_from_a_frozen_audit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            candidate=dict(lines=[cue('Again',1,2)],performed_occurrences=[cue('Again',5,6)])
            digest=canonical_sha256(candidate)
            (root/'candidate.json').write_text(json.dumps(candidate))
            (root/'summary.json').write_text(json.dumps(dict(timestamps_blinded=True,
                candidate_sha256=digest,track=dict(sha256='audio',duration_seconds=10))))
            receipt=dict(identity=dict(audio_sha256='audio',candidate_sha256=digest,
                start=0,end=10,prompt_sha256='prompt'),audit=dict(cues=[
                    dict(id=0,status='present',start_seconds=1,end_seconds=2)],missing=[]))
            (root/'0000.json').write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError,'omitted'):
                freeze(root)
            receipt['audit']['cues'].append(dict(id=1,status='present',start_seconds=5,end_seconds=6))
            (root/'0000.json').write_text(json.dumps(receipt))
            self.assertEqual([row['start_seconds'] for row in freeze(root)['lines']],[1,5])

    def test_freeze_requires_blind_complete_matching_audio_receipts(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            candidate = dict(lines=[cue('First actual phrase', 1, 3)])
            digest = canonical_sha256(candidate)
            summary = dict(timestamps_blinded=True, candidate_sha256=digest,
                           track=dict(sha256='audio', duration_seconds=30))
            (directory / 'candidate.json').write_text(json.dumps(candidate))
            (directory / 'summary.json').write_text(json.dumps(summary))
            receipt = dict(identity=dict(audio_sha256='audio', candidate_sha256=digest,
                start=0, end=27, prompt_sha256='prompt'),
                audit=dict(cues=[dict(id=0, status='present', start_seconds=1.1, end_seconds=3.1)], missing=[]))
            (directory / '0000.json').write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError, 'every audio interval'):
                freeze(directory)
            second = dict(identity={**receipt['identity'], 'start': 15, 'end': 30},
                          audit=dict(cues=[], missing=[]))
            (directory / '0001.json').write_text(json.dumps(second))
            frozen = freeze(directory)
            self.assertEqual(frozen['lines'][0]['start_seconds'], 1.1)
            self.assertEqual(len(frozen['evidence']), 2)
            (directory / 'summary.json').write_text(json.dumps({**summary, 'timestamps_blinded': False}))
            with self.assertRaisesRegex(ValueError, 'Timestamp-visible'):
                freeze(directory)
            (directory / 'summary.json').write_text(json.dumps(summary))
            second['identity']['audio_sha256'] = 'other audio'
            (directory / '0001.json').write_text(json.dumps(second))
            with self.assertRaisesRegex(ValueError, 'another candidate or audio'):
                freeze(directory)

    def test_shifted_line_counts_once_and_half_second_is_allowed(self):
        reference = [cue('A fence around the fire', 1, 3), cue('The server is awake', 5, 7)]
        candidate = [cue('A fence around the fire', 1.5, 3.5), cue('The server is awake', 6.5, 8.5)]
        result = compare(reference, candidate)
        self.assertEqual(result['timing_errors'], 1)
        self.assertEqual(result['error_count'], 1)
        self.assertEqual(result['error_rate'], .5)
        self.assertEqual(result['missing'], [])
        self.assertEqual(result['extra'], [])

    def test_missing_extra_and_repeat_are_not_hidden_by_changing_denominator(self):
        reference = [cue('Again', 1, 2), cue('Again', 4, 5), cue('A final memory', 8, 10)]
        candidate = [cue('Again', 4, 5), cue('A final memory', 8, 10), cue('Invented verse', 12, 14)]
        result = compare(reference, candidate)
        self.assertEqual(result['reference_lines'], 3)
        self.assertEqual(result['missing'], [0])
        self.assertEqual(result['extra'], [2])
        self.assertEqual(result['timing_errors'], 0)
        self.assertEqual(result['error_count'], 2)

    def test_large_clock_error_is_not_counted_as_both_missing_and_extra(self):
        result = compare([cue('Only one occurrence', 1, 2)], [cue('Only one occurrence', 80, 82)])
        self.assertEqual(result['timing_errors'], 1)
        self.assertEqual(result['error_count'], 1)

    def test_nonfinite_or_reversed_times_cannot_pass_the_tolerance(self):
        for start, end in [(float('nan'), 2), (1, float('inf')), (2, 1), (True, 2)]:
            with self.subTest(start=start, end=end), self.assertRaises(ValueError):
                compare([cue('A phrase', 1, 2)], [cue('A phrase', start, end)])


if __name__ == '__main__':
    unittest.main()
