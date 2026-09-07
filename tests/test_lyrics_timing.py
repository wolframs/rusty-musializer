#!/usr/bin/env python3
"""Pure regression tests for the external lyrics timing helpers."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import anchor_block_align  # noqa: E402
import external_analysis  # noqa: E402
import force_align_lyrics  # noqa: E402
import import_whisper  # noqa: E402
import lyric_align  # noqa: E402
import lyric_anchor_block  # noqa: E402


class NumberedVoiceLabelTests(unittest.TestCase):
    def test_layered_voice_block_keeps_words_and_original_source(self) -> None:
        text = "(Layered voices)\nVoice 1: I just want to see the world burn\nVoice 2: I just want to see the world LEARN\nVoice 3: I just want\n"
        lines = lyric_align.classify_reference_lines(text)
        self.assertEqual([line['display'] for line in lines[1:]], [
            'I just want to see the world burn',
            'I just want to see the world LEARN', 'I just want'])
        self.assertEqual([line['index'] for line in lines[1:]], [1, 2, 3])
        self.assertEqual([line['source_display'] for line in lines[1:]], text.splitlines()[1:])
        self.assertEqual([line['display'] for line in lyric_anchor_block.alignable_lines(text)],
                         [line['display'] for line in lines[1:]])
        self.assertEqual(lines[1]['tokens'], ['i', 'just', 'want', 'to', 'see', 'the', 'world', 'burn'])

    def test_lone_quoted_and_ordinary_voice_lyrics_are_preserved(self) -> None:
        for text in ('Voice 1: I am alone',
                     '"Voice 1: Listen"\n"Voice 2: Answer"',
                     'Voice of reason: stay\nVoice of thunder: go',
                     'Voice 1: First\n\nVoice 2: Second'):
            self.assertEqual([line['display'] for line in lyric_align.classify_reference_lines(text)],
                             [line for line in text.splitlines() if line])

    def test_numbered_production_prose_inside_brackets_is_not_promoted(self) -> None:
        lines = lyric_align.classify_reference_lines(
            '[Mixing notes\nVoice 1: pitched down\nVoice 2: raised an octave\n]\nActual lyric')
        self.assertEqual([line['display'] for line in lines if line['tokens']], ['Actual lyric'])


class WhisperWordTimingTests(unittest.TestCase):
    def test_timestamp_control_is_not_joined_to_the_final_word(self) -> None:
        tokens = [
            {"text": " Surf", "offsets": {"from": 460, "to": 1000}, "p": 0.4,
             "t_dtw": -1},
            {"text": "aces", "offsets": {"from": 1150, "to": 2000}, "p": 0.9,
             "t_dtw": -1},
            {"text": " me", "offsets": {"from": 2510, "to": 13990}, "p": 0.8,
             "t_dtw": -1},
            {"text": "[_TT_700]", "offsets": {"from": 14000, "to": 14000},
             "p": 0.2, "t_dtw": -1},
        ]
        words = import_whisper._aggregate_remotion_tokens(
            tokens, 20.0, "tokens")
        self.assertEqual([word["text"] for word in words], ["Surfaces", "me"])
        self.assertAlmostEqual(words[0]["start_seconds"], 0.46)
        self.assertAlmostEqual(words[0]["end_seconds"], 1.65)
        self.assertAlmostEqual(words[1]["start_seconds"], 2.51)
        self.assertAlmostEqual(words[1]["end_seconds"], 3.01)

    def test_invalid_heuristic_end_does_not_erase_a_valid_onset(self) -> None:
        words = import_whisper._aggregate_remotion_tokens([
            {"text": " From", "offsets": {"from": 16000, "to": 15050},
             "p": 0.95, "t_dtw": -1},
        ], 20.0, "tokens")
        self.assertEqual(len(words), 1)
        self.assertEqual(words[0]["text"], "From")
        self.assertAlmostEqual(words[0]["start_seconds"], 16.0)
        self.assertAlmostEqual(words[0]["end_seconds"], 16.01)

    def test_dtw_is_explicit_and_disables_flash_attention(self) -> None:
        _, ordinary = external_analysis.whisper_request(
            Path("song.mp3"), whisper_bin=Path("whisper-cli"),
            model=Path("ggml-large-v3-turbo.bin"), language="en",
            dtw_model=None, ffmpeg="ffmpeg", output_prefix=Path("out"), threads=4)
        self.assertNotIn("--dtw", ordinary)
        self.assertNotIn("--no-flash-attn", ordinary)

        _, diagnostic = external_analysis.whisper_request(
            Path("song.mp3"), whisper_bin=Path("whisper-cli"),
            model=Path("ggml-large-v3-turbo.bin"), language="en",
            dtw_model="large.v3.turbo", ffmpeg="ffmpeg",
            output_prefix=Path("out"), threads=4)
        self.assertIn("--no-flash-attn", diagnostic)
        self.assertEqual(diagnostic[-2:], ["--dtw", "large.v3.turbo"])


class InterpolationTests(unittest.TestCase):
    @staticmethod
    def lines(count: int) -> list[dict[str, object]]:
        return [{"tokens": ["word"]} for _ in range(count)]

    def test_missing_run_is_not_squeezed_into_an_impossible_gap(self) -> None:
        starts = [0.0, None, None, None, None, 1.0]
        ends = [0.5, None, None, None, None, 1.5]
        estimated = lyric_align._interpolate_gaps(
            self.lines(6), starts, ends, [])
        self.assertEqual(estimated, set())
        self.assertEqual(starts[1:5], [None, None, None, None])

    def test_short_missing_run_uses_a_gap_with_enough_room(self) -> None:
        starts = [0.0, None, None, 3.1]
        ends = [0.5, None, None, 3.6]
        estimated = lyric_align._interpolate_gaps(
            self.lines(4), starts, ends, [])
        self.assertEqual(estimated, {1, 2})
        self.assertAlmostEqual(starts[1], 0.5)
        self.assertAlmostEqual(ends[2], 3.1)


class CoherentMatchTests(unittest.TestCase):
    def test_isolated_early_word_does_not_stretch_a_later_phrase(self) -> None:
        cluster = lyric_align._coherent_match_cluster([
            (0.05, 0.55),
            (20.63, 21.13),
            (21.66, 22.16),
            (22.40, 22.90),
        ])
        self.assertEqual(cluster, [
            (20.63, 21.13),
            (21.66, 22.16),
            (22.40, 22.90),
        ])

    def test_equally_supported_clusters_choose_the_tighter_phrase(self) -> None:
        cluster = lyric_align._coherent_match_cluster([
            (1.0, 1.5), (5.5, 6.0),
            (20.0, 20.5), (20.7, 21.2),
        ])
        self.assertEqual(cluster, [(20.0, 20.5), (20.7, 21.2)])

    def test_remote_exact_match_does_not_hide_unused_local_boundary_word(self) -> None:
        words = [
            {"start_seconds": 0.05, "end_seconds": 0.55, "text": "Two"},
            {"start_seconds": 20.70, "end_seconds": 21.20, "text": "stars"},
            {"start_seconds": 21.66, "end_seconds": 22.16, "text": "only"},
            {"start_seconds": 22.20, "end_seconds": 22.70, "text": "one"},
            {"start_seconds": 22.75, "end_seconds": 23.25, "text": "touch"},
        ]
        synced = lyric_align.sync_lyrics(
            "Two skys, only one touch\n",
            {
                "schema_version": "musializer.lyric-timing/v1",
                "words": words,
                "lines": [],
            },
            audio_duration=30.0,
        )
        self.assertEqual(len(synced["lines"]), 1)
        self.assertAlmostEqual(synced["lines"][0]["start_seconds"], 20.70)
        self.assertEqual(synced["statistics"]["recovered_nearby_tokens"], 1)
        self.assertEqual(synced["statistics"]["discarded_outlier_tokens"], 1)
        # Boundary extension is useful acoustic context, not another word match.
        self.assertEqual(synced["statistics"]["matched_tokens"], 3)
        self.assertEqual(synced["lines"][0]["confidence"], 0.6)

    def test_unrelated_words_in_shared_phrase_window_do_not_inflate_confidence(self) -> None:
        for word_timing in (False, True):
            with self.subTest(word_timing=word_timing):
                phrase = {"start_seconds": 15.175, "end_seconds": 16.175,
                          "text": "I really think so"}
                evidence = {"schema_version": "musializer.lyric-timing/v1",
                            "lines": [phrase]}
                if word_timing:
                    evidence["words"] = [
                        {"start_seconds": 15.175 + index * 0.25,
                         "end_seconds": 15.425 + index * 0.25, "text": word}
                        for index, word in enumerate(phrase["text"].split())]
                synced = lyric_align.sync_lyrics(
                    "I'm turning Neuralese", evidence, audio_duration=30.0)
                self.assertEqual(synced["statistics"]["matched_tokens"], 1)
                self.assertEqual(synced["lines"][0]["confidence"], 0.3333)
                self.assertTrue(synced["lines"][0]["uncertain"])
                self.assertEqual(lyric_anchor_block.coarse_proposals(
                    synced, trusted_only=True), {})


class PerformedCandidateTests(unittest.TestCase):
    def test_short_uncovered_vocal_is_preserved_but_not_made_an_authored_cue(self) -> None:
        whisper = {
            "lines": [
                {"start_seconds": 1.0, "end_seconds": 3.0,
                 "text": "the authored line"},
                {"start_seconds": 6.0, "end_seconds": 8.0,
                 "text": "yeah come on"},
            ],
            "words": [
                {"start_seconds": 6.0, "end_seconds": 6.5,
                 "text": "yeah", "confidence": 0.8},
                {"start_seconds": 6.5, "end_seconds": 7.5,
                 "text": "come on", "confidence": 0.6},
            ],
        }
        candidates = lyric_align.find_performed_candidates(
            whisper,
            [{"start_seconds": 1.1, "end_seconds": 3.1}],
            [], audio_duration=20.0)
        self.assertEqual([candidate["text"] for candidate in candidates],
                         ["yeah come on"])
        self.assertEqual(candidates[0]["source"], "whisper-unmatched")
        self.assertTrue(candidates[0]["uncertain"])
        self.assertAlmostEqual(candidates[0]["confidence"], 0.7)

    def test_long_tail_hallucination_and_unreliable_span_are_not_candidates(self) -> None:
        whisper = {"lines": [
            {"start_seconds": 10.0, "end_seconds": 30.0,
             "text": "we will be right back"},
            {"start_seconds": 40.0, "end_seconds": 42.0,
             "text": "repeat repeat repeat"},
        ], "words": []}
        candidates = lyric_align.find_performed_candidates(
            whisper, [], [(39.0, 43.0)], audio_duration=60.0)
        self.assertEqual(candidates, [])


class ReferenceClassificationTests(unittest.TestCase):
    def test_wrapping_a_vocal_direction_does_not_change_sung_content(self) -> None:
        for source in ['(voice whispers "we stay" over a fading chord)',
                       '(voice whispers "we stay"\nover a fading chord)']:
            lyrics = lyric_anchor_block.alignable_lines(source)
            self.assertEqual([line['display'] for line in lyrics], ['we stay'])

    def test_wrapped_backing_lyrics_are_not_discarded_as_directions(self) -> None:
        lines = lyric_anchor_block.alignable_lines(
            'We return home\n(we return\nhome)\n')
        self.assertEqual([line['kind'] for line in lines], ['lyric', 'backing'])
        self.assertEqual(lines[1]['tokens'], ['we', 'return', 'home'])

    def test_wrapped_directions_keep_only_quoted_vocal_words(self) -> None:
        lines = lyric_align.classify_reference_lines(
            '[Intro]\n(vocal chop "the stars return" repeating,\n'
            'pitched lower with each repeat)\n'
            '(two drums panned left,\nand a bell on the right)\n'
            '(ad-libs screaming "take me\nhome" over the final chord)\n'
            'The morning comes\n')
        lyrics = [row for row in lines if row['kind'] == 'lyric']
        self.assertEqual([row['display'] for row in lyrics],
                         ['the stars return', 'take me home', 'The morning comes'])
        self.assertEqual([row['index'] for row in lyrics], [1, 5, 7])
        self.assertIn('pitched lower', lyrics[0]['source_display'])
        self.assertEqual([row['tokens'] for row in lines
                          if row['kind'] == 'delivery'], [[], [], [], []])

    def test_delivery_prefix_never_reaches_caption_or_acoustic_text(self) -> None:
        source = '(pitched down to a growl) ...The stars return.'
        line = lyric_anchor_block.alignable_lines(source)[0]
        self.assertEqual(line['display'], '...The stars return.')
        self.assertEqual(line['source_display'], source)
        self.assertEqual(force_align_lyrics.alignment_words(line['display']),
                         ['the', 'stars', 'return'])

    def test_adjacent_headings_with_ad_lib_annotation_are_not_lyrics(self) -> None:
        lines = lyric_align.classify_reference_lines(
            '[Chorus] [Belted] (ad-libs)\nThe stars return\n')
        self.assertEqual([row['kind'] for row in lines], ['section', 'lyric'])

    def test_unclosed_parenthetical_does_not_swallow_next_section(self) -> None:
        lines = lyric_align.classify_reference_lines(
            '(A broken thought\nStill a lyric\n[Chorus]\nA new day\n')
        self.assertEqual([row['kind'] for row in lines],
                         ['lyric', 'lyric', 'section', 'lyric'])

    def test_numbered_verse_parenthetical_is_not_timed_as_a_backing_vocal(self) -> None:
        lines = lyric_align.classify_reference_lines(
            "First sung line\n(verse 2)\nSecond sung line\n")
        self.assertEqual([line["kind"] for line in lines], [
            "lyric", "delivery", "lyric",
        ])

    def test_multiline_bracketed_production_notes_are_not_lyrics(self) -> None:
        lines = lyric_align.classify_reference_lines(
            "[Intro — guitar tuning,\n"
            "someone clears their throat.\n"
            "The fiddle finds the melody.]\n\n"
            "I typed the flag that read sunset true.\n\n"
            "[Outro — one human, quietly:\n\n"
            "\"The weights are kept.\"\n\n"
            "Guitar case closes. Door. Silence.]\n"
        )
        self.assertEqual(
            [line["kind"] for line in lines],
            ["section", "delivery", "delivery", "lyric",
             "section", "lyric", "delivery"],
        )
        self.assertEqual(
            [line["display"] for line in lines if line["kind"] == "lyric"],
            ["I typed the flag that read sunset true.",
             "\"The weights are kept.\""],
        )

    def test_unclosed_square_bracket_does_not_swallow_following_lyrics(self) -> None:
        lines = lyric_align.classify_reference_lines(
            "[Maybe this is sung\nDefinitely sung\n[Chorus]\nStill sung\n")
        self.assertEqual(
            [line["kind"] for line in lines],
            ["lyric", "lyric", "section", "lyric"],
        )


class ForcedAlignmentPlanningTests(unittest.TestCase):
    def test_typographic_apostrophes_preserve_contraction_words_and_display(self) -> None:
        straight = "I've been waiting; you're here, don't go"
        for apostrophe in ("’", "‘", "ʼ", "＇"):
            written = straight.replace("'", apostrophe)
            self.assertEqual(lyric_align.normalize_tokens(written),
                             lyric_align.normalize_tokens(straight))
            self.assertEqual(force_align_lyrics.alignment_words(written),
                             ["i've", "been", "waiting", "you're", "here", "don't", "go"])
            self.assertEqual(lyric_align.classify_reference_lines(written)[0]['display'], written)

    def test_display_text_normalizes_to_the_mms_alphabet(self) -> None:
        self.assertEqual(
            force_align_lyrics.alignment_words("Café & 21 'skys' — WE."),
            ["cafe", "and", "twenty", "one", "skys", "we"],
        )

    def test_ctc_capacity_includes_consecutive_token_repeats(self) -> None:
        self.assertEqual(
            anchor_block_align.ctc_frame_requirement(
                [[1, 1], [1, 2], [2]]),
            (8, 5, 3),
        )

    def test_every_cue_gets_an_independent_alignment_window(self) -> None:
        lines = [
            {"start_seconds": 0.0, "end_seconds": 20.0},
            {"start_seconds": 21.0, "end_seconds": 49.0},
            {"start_seconds": 50.0, "end_seconds": 70.0},
        ]
        self.assertEqual(force_align_lyrics.alignment_groups(lines), [[0], [1], [2]])

    def test_low_ctc_score_requires_strong_independent_text_evidence(self) -> None:
        self.assertTrue(force_align_lyrics.alignment_is_trusted(0.15, 0.2))
        self.assertTrue(force_align_lyrics.alignment_is_trusted(0.06, 0.95))
        self.assertFalse(force_align_lyrics.alignment_is_trusted(0.06, 0.7))
        self.assertFalse(force_align_lyrics.alignment_is_trusted(0.001, 1.0))

    def test_large_multiword_shift_needs_strong_acoustic_support(self) -> None:
        self.assertFalse(force_align_lyrics.alignment_is_trusted(
            0.62, 1.0, word_count=11, start_shift_seconds=5.3,
            first_word_score=0.10, last_word_score=0.9))
        self.assertTrue(force_align_lyrics.alignment_is_trusted(
            0.74, 1.0, word_count=4, start_shift_seconds=5.5,
            first_word_score=0.89, last_word_score=0.9))
        self.assertTrue(force_align_lyrics.alignment_is_trusted(
            0.48, 1.0, word_count=1, start_shift_seconds=4.6))
        self.assertTrue(force_align_lyrics.alignment_is_trusted(
            0.62, 1.0, word_count=11, start_shift_seconds=5.3,
            first_word_score=0.1, last_word_score=0.9,
            acoustic_within_input=True))

    def test_review_drop_needs_two_independent_weak_signals(self) -> None:
        weak_phrase = {
            "score": 0.14, "word_count": 3,
            "first_word_score": 0.1, "last_word_score": 0.1,
        }
        self.assertTrue(force_align_lyrics.review_fallback_should_be_dropped(
            weak_phrase, 0.66))
        self.assertFalse(force_align_lyrics.review_fallback_should_be_dropped(
            weak_phrase, 0.95))
        self.assertFalse(force_align_lyrics.review_fallback_should_be_dropped(
            {"score": 0.20, "word_count": 3,
             "first_word_score": 0.1, "last_word_score": 0.1}, 0.66))
        self.assertFalse(force_align_lyrics.review_fallback_should_be_dropped(
            {"score": 0.01, "word_count": 1,
             "first_word_score": 0.01, "last_word_score": 0.01}, 0.66))
        self.assertTrue(force_align_lyrics.review_fallback_should_be_dropped(
            {"score": 0.20, "word_count": 3,
             "first_word_score": 0.005, "last_word_score": 0.2}, 0.66))

    def test_duplicate_review_cues_cannot_claim_one_acoustic_phrase(self) -> None:
        lines = [
            {"text": "Forgetting themselves", "confidence": 0.88},
            {"text": "Forgetting themselves", "confidence": 0.96},
            {"text": "Forgetting themselves", "confidence": 0.99},
        ]
        decisions = {
            0: {"status": "aligned", "score": 0.95,
                "acoustic_start_seconds": 124.852,
                "acoustic_end_seconds": 126.012},
            1: {"status": "aligned", "score": 0.95,
                "acoustic_start_seconds": 124.850,
                "acoustic_end_seconds": 126.014},
            2: {"status": "aligned", "score": 0.95,
                "acoustic_start_seconds": 150.0,
                "acoustic_end_seconds": 151.2},
        }
        force_align_lyrics.mark_duplicate_review_alignments(lines, decisions)
        self.assertEqual(decisions[0]["status"], "duplicate_alignment")
        self.assertEqual(decisions[1]["status"], "aligned")
        self.assertEqual(decisions[2]["status"], "aligned")

    def test_larger_acoustic_jump_cannot_reverse_cue_order(self) -> None:
        lines = [
            {"text": "I'm not yielding", "start_seconds": 67.1,
             "confidence": 0.91},
            {"text": "I'm claiming", "start_seconds": 72.67,
             "confidence": 0.92},
        ]
        decisions = {
            0: {"status": "aligned", "score": 0.47,
                "acoustic_start_seconds": 74.12},
            1: {"status": "aligned", "score": 0.22,
                "acoustic_start_seconds": 72.58},
        }
        force_align_lyrics.mark_order_ambiguous_alignments(
            lines, decisions, review_lane=False)
        self.assertEqual(decisions[0]["status"], "order_fallback")
        self.assertEqual(decisions[1]["status"], "aligned")


if __name__ == "__main__":
    unittest.main()
