"""Synthetic controls for position-specific acoustic alignment constraints."""
import sys
import importlib.util
import warnings
from pathlib import Path
import unittest

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import ctc_window_align


class WindowAlignmentTests(unittest.TestCase):
    def test_wrong_occurrence_wins_without_the_window_control(self):
        probabilities = np.full((10, 3), 0.005)
        probabilities[:, 0] = 0.99
        for frame, label, confidence in [(1, 1, 0.999), (2, 2, 0.999),
                                         (6, 1, 0.8), (7, 2, 0.8)]:
            probabilities[frame, 0] = 1 - confidence
            probabilities[frame, label] = confidence
        emissions = np.log(probabilities)
        unconstrained = ctc_window_align.align(emissions, [1, 2], [None, None])
        constrained = ctc_window_align.align(emissions, [1, 2], [(5, 9), (5, 9)])
        self.assertEqual([span[:2] for span in unconstrained], [(1, 2), (2, 3)])
        self.assertEqual([span[:2] for span in constrained], [(6, 7), (7, 8)])

    def test_repeated_character_requires_intervening_blank(self):
        emissions = np.log([[0.1, 0.9], [0.9, 0.1], [0.1, 0.9]])
        spans = ctc_window_align.align(emissions, [1, 1], [None, None])
        self.assertEqual([span[:2] for span in spans], [(0, 1), (2, 3)])
        with self.assertRaisesRegex(ValueError, 'No CTC path'):
            ctc_window_align.align(emissions[:2], [1, 1], [None, None])

    def test_incompatible_windows_abstain(self):
        emissions = np.log(np.full((5, 3), 1 / 3))
        with self.assertRaisesRegex(ValueError, 'No CTC path'):
            ctc_window_align.align(emissions, [1, 2], [(3, 5), (0, 2)])

    def test_nan_is_not_an_alignment(self):
        with self.assertRaisesRegex(ValueError, 'NaN'):
            ctc_window_align.align([[0, float('nan')]], [1], [None])

    @unittest.skipUnless(importlib.util.find_spec('torchaudio'), 'run under the alignment runtime for the differential control')
    def test_unconstrained_spans_match_torchaudio_on_random_emissions(self):
        import torch
        import torchaudio
        rng = np.random.default_rng(73991)
        with warnings.catch_warnings():
            warnings.simplefilter('ignore')
            for frames in (6, 12, 30, 70):
                for count in (1, 2, 3):
                    for _ in range(20):
                        labels = rng.integers(1, 5, count)
                        logits = torch.tensor(rng.normal(size=(frames, 5)), dtype=torch.float32).log_softmax(-1)
                        path, scores = torchaudio.functional.forced_align(
                            logits[None], torch.tensor(labels[None], dtype=torch.int32), blank=0)
                        expected = torchaudio.functional.merge_tokens(path[0], scores[0].exp(), blank=0)
                        actual = ctc_window_align.align(logits.numpy(), labels, [None] * count)
                        self.assertEqual([(span.start, span.end) for span in expected], [span[:2] for span in actual])



if __name__ == '__main__':
    unittest.main()
