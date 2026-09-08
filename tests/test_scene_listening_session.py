"""Candidate cuts must land in sustained breaths, not isolated sample dips."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
from scene_listening_session import candidate_windows


class WindowTests(unittest.TestCase):
    def test_sustained_valleys_win_over_a_single_frame_dip(self):
        frames = []
        for i in range(2000):
            t = i * 0.05
            rms = 0.08 if any(abs(t-center) < 0.4 for center in (22, 47, 63, 87)) else 0.8
            if i in (400, 850):
                rms = 0.0
            frames.append(dict(time_seconds=t, rms=rms))
        windows = candidate_windows(dict(frames=frames, audio=dict(duration_seconds=100)))
        for (start, end), (expected_start, expected_end) in zip(windows, [(22, 47), (63, 87)]):
            self.assertLess(abs(start-expected_start), 0.4)
            self.assertLess(abs(end-expected_end), 0.4)
            self.assertGreaterEqual(end-start, 20)
            self.assertLessEqual(end-start, 30)

    def test_short_track_requires_authored_bounds(self):
        with self.assertRaisesRegex(ValueError, 'use --window'):
            candidate_windows(dict(frames=[dict(time_seconds=i, rms=0.3) for i in range(20)],
                                   audio=dict(duration_seconds=20)))


if __name__ == '__main__':
    unittest.main()
