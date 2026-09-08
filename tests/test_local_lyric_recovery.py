import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import local_lyric_recovery as recovery
from analysis_io import sha256_file


def phrase(text, start):
    return dict(text=text, status='weak', asr_start_seconds=start, asr_end_seconds=start + len(text.split()) * .2, word_alignments=[dict(
        text=word, start_seconds=start + index * .2,
        end_seconds=start + index * .2 + .15)
        for index, word in enumerate(text.lower().split())])


def report(start, *rows):
    return dict(start=start, seconds=30., lines=list(rows))


class LocalRecoveryTests(unittest.TestCase):
    def test_a_single_recognition_cannot_create_a_caption(self):
        self.assertEqual(recovery.corroborate([report(0, phrase('we will return', 10))], ''), [])

    def test_duplicate_receipts_are_not_independent(self):
        view = report(0, phrase('we will return', 10))
        self.assertEqual(recovery.corroborate([view, copy.deepcopy(view)], ''), [])

    def test_disagreeing_acoustic_edges_do_not_validate_each_other(self):
        views = [report(0, phrase('we will return', 10)), report(5, phrase('we will return', 11.5))]
        self.assertEqual(recovery.corroborate(views, ''), [])

    def test_ctc_agreement_at_the_wrong_occurrence_is_refused(self):
        views = [report(start, phrase('we will return', 10)) for start in (0, 5)]
        for view in views:
            view['lines'][0].update(asr_start_seconds=18., asr_end_seconds=20.)
        self.assertEqual(recovery.corroborate(views, ''), [])

    def test_equal_text_repetitions_keep_their_own_occurrences(self):
        views = [report(start, phrase('we will return', 10), phrase('we will return', 15))
                 for start in (0, 5)]
        rows = recovery.corroborate(views, '')
        self.assertEqual([row['start_seconds'] for row in rows], [10., 15.])
        self.assertEqual([row['end_seconds'] for row in rows], [10.55, 15.55])

    def test_written_words_alone_never_establish_presence(self):
        views = [report(start, phrase('we will return', 10)) for start in (0, 5)]
        rows = recovery.corroborate(views, '(a sample shouting "entirely different words")')
        self.assertEqual([row['text'] for row in rows], ['we will return'])

    def test_phrase_at_the_crop_edge_needs_another_complete_observation(self):
        views = [report(0, phrase('we will return', 5.1)), report(5, phrase('we will return', 5.1))]
        self.assertEqual(recovery.corroborate(views, ''), [])

    def test_two_separate_deliveries_are_not_a_single_written_echo_caption(self):
        views = [report(start, phrase('we will return', 10), phrase('we will return', 11))
                 for start in (0, 5)]
        rows = recovery.corroborate(views, 'We will return (we will return)')
        self.assertEqual([row['text'] for row in rows], ['We will return', 'We will return'])
        self.assertEqual([row['start_seconds'] for row in rows], [10., 11.])

    def test_stutter_includes_the_first_performed_syllable(self):
        row = phrase('t t t total mode collapse', 10)
        row['text'] = 'T-t-t-total mode collapse'
        words = recovery.acoustic_words(row)
        self.assertEqual(words[0], ('total', 10., 10.75))
        self.assertEqual([word[0] for word in words], ['total', 'mode', 'collapse'])

    def test_unhyphenated_repeated_words_are_not_swallowed(self):
        row = phrase('go go going home', 10)
        self.assertEqual(len(recovery.acoustic_words(row)), 4)

    def test_a_phrase_cannot_bridge_a_long_acoustic_gap(self):
        first = phrase('we will return', 10)
        first['word_alignments'][-1].update(start_seconds=20., end_seconds=20.2)
        self.assertEqual(recovery.corroborate([report(0, first), report(5, first)], ''), [])

    def test_conflicting_written_cue_is_preserved_for_review(self):
        original = dict(lines=[dict(reference_line_index=2, line_position=0, kind='lyric',
            text='Unrelated written instruction', start_seconds=10., end_seconds=11.,
            coarse_start_seconds=10., coarse_end_seconds=11.)],
            unresolved=[], unmatched=[], review_flags=[], statistics={})
        snapshot = copy.deepcopy(original)
        views = [report(start, phrase('we will return', 10)) for start in (0, 5)]
        result = recovery.recover(original, views, '')
        self.assertEqual(original, snapshot)
        self.assertEqual(result['lines'], [])
        self.assertEqual(result['unresolved'][0]['text'], 'Unrelated written instruction')
        self.assertEqual(result['unresolved'][0]['reference_line_index'], 2)
        self.assertEqual(len(result['performed_occurrences']), 1)
        self.assertTrue(result['performed_occurrences'][0]['uncertain'])

    def test_model_or_binary_change_invalidates_recovery_cache(self):
        with tempfile.TemporaryDirectory() as temp:
            binary, model = Path(temp) / 'binary', Path(temp) / 'model'
            binary.write_bytes(b'first'); model.write_bytes(b'weights')
            doc = dict(local_recovery=dict(version=recovery.VERSION, request_identity=dict(version=recovery.CROP_VERSION,
                whisper_binary_sha256=sha256_file(binary), whisper_model_sha256=sha256_file(model))))
            self.assertTrue(recovery.cache_accepts(doc, binary, model))
            binary.write_bytes(b'new version')
            self.assertFalse(recovery.cache_accepts(doc, binary, model))
            self.assertFalse(recovery.cache_accepts({}, binary, model))


if __name__ == '__main__':
    unittest.main()
