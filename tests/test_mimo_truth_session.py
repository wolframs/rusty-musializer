"""The ground-truth listening session: answers in, tracks.json out, no guessing.

Offline and silent: no audio is decoded, rendered or played, and librosa is
not needed (the generator imports it only inside the functions that analyse
audio). The fixture answer log mimics what tools/listening-lab appends,
including a superseded revision and an incomplete one.
"""

from __future__ import annotations

import copy
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

from mimo_bench import bench_io, ground_truth  # noqa: E402
from mimo_bench import truth_convert as convert  # noqa: E402
from mimo_bench import truth_session as session  # noqa: E402
from mimo_bench import truth_spec as spec  # noqa: E402

FIXTURES = ROOT / "tests" / "fixtures" / "mimo_truth"
PROTOCOL = "mimo-truth-v1-shut-up-cat"


def blank_truth() -> dict:
    return json.loads((ROOT / "tools" / "mimo_bench" / "ground_truth" / "tracks.json")
                      .read_text(encoding="utf-8"))


class ConverterTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.dir = Path(self.temporary.name)
        self.answers = self.dir / "answers"
        self.answers.mkdir()
        self.log_lines = (FIXTURES / f"{PROTOCOL}.answers.jsonl").read_text(
            encoding="utf-8").splitlines()
        self.protocol_file = self.dir / f"{PROTOCOL}.listen.json"
        self.protocol_file.write_text('{"stand-in": "protocol"}', encoding="utf-8")
        self.sidecar = json.loads((FIXTURES / "session.json").read_text(encoding="utf-8"))
        plan = self.sidecar["protocols"][PROTOCOL]
        plan["protocol_path"] = str(self.protocol_file)
        plan["protocol_sha256"] = hashlib.sha256(self.protocol_file.read_bytes()).hexdigest()
        self.write_log(self.log_lines)

    def write_log(self, lines: list[str]) -> None:
        (self.answers / f"{PROTOCOL}.answers.jsonl").write_text(
            "\n".join(lines) + "\n", encoding="utf-8")

    def convert(self, **kwargs):
        return convert.build_truth(self.sidecar, self.answers, blank_truth(), **kwargs)

    def test_the_fixture_converts_to_exact_truth(self) -> None:
        document, _ = self.convert()
        entry = document["tracks"]["shut-up-cat"]
        # revision 2 of the key supersedes the earlier "Can't tell"
        self.assertEqual(entry["key"], {"status": "adjudicated", "tonic": "Eb", "mode": "major"})
        self.assertEqual(entry["meter"], {"status": "adjudicated", "value": "4/4"})
        self.assertEqual(entry["instruments"], {
            "status": "adjudicated",
            "present": ["lead vocal", "kick", "piano"],
            "allowed_extra": ["synth pad"],
            "absent": ["electric guitar"],
        })
        self.assertIn("key note: chorus settles on it", entry["notes"])
        self.assertIn("instruments note: a talkbox, maybe", entry["notes"])
        self.assertIn("meter confidence: clear", entry["notes"])
        self.assertEqual(entry["excerpt"], {"start_seconds": 30.0, "seconds": 100.0})
        # the other track was not in the session and is untouched
        self.assertEqual(document["tracks"]["constellation-whale"],
                         blank_truth()["tracks"]["constellation-whale"])

    def test_converted_truth_ends_the_abstentions(self) -> None:
        document, _ = self.convert()
        track = bench_io.track_by_slug("shut-up-cat")
        before = ground_truth.load(track, authored_document=blank_truth())
        after = ground_truth.load(track, authored_document=document)
        # tempo, form and lyrics depend on build/ data a checkout may lack;
        # only the three listened dimensions are this converter's business.
        listened = {"key", "meter", "instruments"}
        self.assertEqual(listened & set(before.abstaining_dimensions()), listened)
        self.assertEqual(listened & set(after.abstaining_dimensions()), set())
        self.assertEqual((after.key_tonic, after.key_mode, after.meter), ("Eb", "major", "4/4"))

    def test_an_incomplete_last_revision_is_refused(self) -> None:
        self.write_log(self.log_lines[:4])  # instruments only at its incomplete revision 1
        with self.assertRaisesRegex(convert.Refusal, "instruments: saved while incomplete"):
            self.convert()

    def test_an_unanswered_instrument_row_is_refused(self) -> None:
        last = json.loads(self.log_lines[4])
        del last["responses"]["inst-synth-pad"]
        last["revision"] = 3
        self.write_log(self.log_lines + [json.dumps(last)])
        with self.assertRaisesRegex(convert.Refusal, "rows left unanswered: synth pad"):
            self.convert()

    def test_a_missing_question_is_refused_unless_allowed(self) -> None:
        kept = [line for line in self.log_lines if '"question_id":"meter"' not in line]
        self.write_log(kept)
        with self.assertRaisesRegex(convert.Refusal, "meter: not answered yet"):
            self.convert()
        document, _ = self.convert(allow_missing=True)
        self.assertEqual(document["tracks"]["shut-up-cat"]["meter"]["status"], "unadjudicated")
        self.assertEqual(document["tracks"]["shut-up-cat"]["key"]["tonic"], "Eb")

    def test_an_answer_outside_the_options_is_refused(self) -> None:
        last = json.loads(self.log_lines[1])
        last["answer"] = "G · 7/8 fits"
        last["revision"] = 2
        self.write_log(self.log_lines + [json.dumps(last)])
        with self.assertRaisesRegex(convert.Refusal, "not one of the session's options"):
            self.convert()

    def test_no_stable_key_and_cant_tell(self) -> None:
        for answer, status in (("No stable home note: it wanders or never settles", "none"),
                               ("Can't tell", "unadjudicated"),
                               ("None of these sits right", "unadjudicated")):
            record = json.loads(self.log_lines[2])
            record["answer"] = answer
            record["revision"] = 9
            self.write_log(self.log_lines + [json.dumps(record)])
            document, _ = self.convert()
            self.assertEqual(document["tracks"]["shut-up-cat"]["key"],
                             {"status": status, "tonic": None, "mode": None}, answer)

    def test_cant_pick_instruments_keeps_them_abstaining(self) -> None:
        record = json.loads(self.log_lines[4])
        record.update(answer=spec.INSTRUMENTS_CANT, responses={}, revision=3)
        self.write_log(self.log_lines + [json.dumps(record)])
        document, _ = self.convert()
        self.assertEqual(document["tracks"]["shut-up-cat"]["instruments"]["status"],
                         "unadjudicated")

    def test_a_changed_protocol_file_is_refused(self) -> None:
        self.protocol_file.write_text('{"stand-in": "regenerated"}', encoding="utf-8")
        with self.assertRaisesRegex(convert.Refusal, "protocol file changed"):
            self.convert()

    def test_main_writes_nothing_when_it_refuses(self) -> None:
        self.write_log(self.log_lines[:4])
        sidecar_path = self.dir / "session.json"
        sidecar_path.write_text(json.dumps(self.sidecar), encoding="utf-8")
        output = self.dir / "tracks.json"
        original = json.dumps(blank_truth())
        output.write_text(original, encoding="utf-8")
        with mock.patch("sys.stderr"):
            status = convert.main(["--sidecar", str(sidecar_path), "--answers-dir",
                                   str(self.answers), "--output", str(output)])
        self.assertEqual(status, 2)
        self.assertEqual(output.read_text(encoding="utf-8"), original)


class GeneratedProtocolTests(unittest.TestCase):
    """The generator's protocol must pass the Lab's own limits and agree with
    the sidecar it writes, option label for option label."""

    def plan(self) -> dict:
        plan = copy.deepcopy(json.loads((FIXTURES / "session.json").read_text(
            encoding="utf-8"))["protocols"][PROTOCOL])
        plan["meter"]["offered"].append("12/8")
        plan["meter"]["options"]["H · 12/8 fits"] = {"status": "adjudicated", "value": "12/8"}
        plan["audio"] = {name: f"/fixture/{name}.mp3" for name in
                         ("plain", "key-1", "key-2", "key-3",
                          "meter-4-4", "meter-3-4", "meter-6-8", "meter-12-8")}
        plan["instruments"]["rows"] = list(spec.INSTRUMENT_ROWS)
        rest = [name for name in spec.lexicon_terms() if name not in spec.INSTRUMENT_ROWS]
        plan["instruments"]["other_groups"] = spec.chip_groups(rest)
        return plan

    def test_the_protocol_fits_the_lab_and_the_sidecar(self) -> None:
        plan = self.plan()
        protocol = session.build_protocol(bench_io.track_by_slug("shut-up-cat"), plan)
        self.assertEqual(protocol["id"], PROTOCOL)
        self.assertLessEqual(len(protocol["tracks"]), 8)
        ids = {track["id"] for track in protocol["tracks"]}
        questions = {question["id"]: question for question in protocol["questions"]}
        self.assertEqual(set(questions), set(spec.QUESTIONS))
        for question in protocol["questions"]:
            self.assertLessEqual(len(question["tracks"]), 4, question["id"])
            self.assertTrue(set(question["tracks"]) <= ids)
            self.assertTrue(2 <= len(question["options"]) <= 7, question["id"])
            self.assertLessEqual(len(question["feedback"]["fields"]), 32, question["id"])
            for field in question["feedback"]["fields"]:
                self.assertTrue(2 <= len(field["options"]) <= 10, field["id"])
        self.assertEqual(questions["key"]["options"], list(plan["key"]["options"]))
        self.assertEqual(questions["meter"]["options"], list(plan["meter"]["options"]))
        row_ids = [field["id"] for field in questions["instruments"]["feedback"]["fields"]
                   if field["type"] == "single"]
        self.assertEqual(row_ids, [spec.row_field_id(name) for name in spec.INSTRUMENT_ROWS])
        chips = [option["value"]
                 for field in questions["instruments"]["feedback"]["fields"]
                 if field["type"] == "multi" for option in field["options"]]
        self.assertEqual(sorted(chips + list(spec.INSTRUMENT_ROWS)),
                         sorted(spec.lexicon_terms()))

    def test_each_option_names_the_tab_that_plays_it(self) -> None:
        plan = self.plan()
        protocol = session.build_protocol(bench_io.track_by_slug("shut-up-cat"), plan)
        tabs = {session.tab_letter(position): track["label"]
                for position, track in enumerate(protocol["tracks"])}
        for question in protocol["questions"][:2]:
            for option in question["options"]:
                letter, _, rest = option.partition(" · ")
                if not rest:
                    continue
                label = tabs[letter]
                self.assertIn(label.removeprefix("Count "), rest, option)
                self.assertIn(letter_track_id(protocol, letter), question["tracks"], option)

    def test_chip_groups_always_fit_the_lab(self) -> None:
        for count in range(1, 45):
            names = [f"n{index}" for index in range(count)]
            groups = spec.chip_groups(names)
            self.assertEqual([name for group in groups for name in group], names)
            if count >= 2:
                self.assertTrue(all(2 <= len(group) <= 10 for group in groups), count)

    def test_every_row_is_a_lexicon_term(self) -> None:
        self.assertTrue(set(spec.INSTRUMENT_ROWS) <= set(spec.lexicon_terms()))

    def test_downbeat_phase_follows_the_low_end(self) -> None:
        strength = [0.1, 0.1, 0.9, 0.1] * 8
        self.assertEqual(session.downbeat_phase(strength, 4), 2)
        self.assertEqual(session.downbeat_phase([0.9, 0.1, 0.1] * 6, 3), 0)


def letter_track_id(protocol: dict, letter: str) -> str:
    return protocol["tracks"][ord(letter) - ord("A")]["id"]


if __name__ == "__main__":
    unittest.main()
