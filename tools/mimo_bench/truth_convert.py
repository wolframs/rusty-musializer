#!/usr/bin/env python3
"""Turn the listening session's answers into the benchmark's ground truth.

    python3 tools/mimo_bench/truth_convert.py            # check, then write tracks.json
    python3 tools/mimo_bench/truth_convert.py --dry-run  # print what would be written
    python3 tools/mimo_bench/run.py score                # re-score with the new truth

Reads the session sidecar written by ``truth_session.py`` and the Listening
Lab's append-only answer logs, takes the **last revision** of each question,
and writes key, metre and instruments into ``ground_truth/tracks.json``:

* a chosen candidate → ``adjudicated`` with its value;
* "no stable home note" → key ``none`` (a claimed key then counts as false);
* "can't tell" / "none of these" / "can't pick instruments out" →
  ``unadjudicated``, so that dimension keeps abstaining;
* instrument rows → ``present`` / ``allowed_extra`` / ``absent``; ticked
  extra chips → ``present``; the free-text line → ``notes``.

It **refuses** rather than guesses: a question saved while still incomplete,
an instrument list with an unanswered row, an answer that is not one of the
session's options, a protocol file that changed since the session was built,
or (without ``--allow-missing``) a question nobody answered. Nothing is
written when anything is refused. Standard library only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path
from typing import Any

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    __package__ = "mimo_bench"

from .bench_io import GROUND_TRUTH_PATH, atomic_write_json  # noqa: E402
from . import truth_spec as spec  # noqa: E402


class Refusal(Exception):
    """The answers cannot be turned into truth without guessing."""


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def latest_answers(path: Path) -> dict[str, dict[str, Any]]:
    """The last revision of every question in one Lab answer log."""
    latest: dict[str, dict[str, Any]] = {}
    if not path.is_file():
        return latest
    lines = path.read_text(encoding="utf-8").split("\n")
    for index, line in enumerate(lines):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            if index == len(lines) - 1:   # a torn tail, as the Lab itself tolerates
                continue
            raise Refusal(f"{path}: line {index + 1} is not JSON")
        if record.get("schema") != "musializer.listening-answer/v1":
            raise Refusal(f"{path}: line {index + 1} is not a listening answer")
        question = str(record.get("question_id"))
        previous = latest.get(question)
        if previous is None or int(record.get("revision", 0)) >= int(previous.get("revision", 0)):
            latest[question] = record
    return latest


def _require_complete(protocol: str, question: str, record: dict[str, Any]) -> None:
    if record.get("complete") is False:
        raise Refusal(f"{protocol} / {question}: saved while incomplete "
                      f"(a required follow-up is unanswered); finish it in the Lab")


def _option(protocol: str, question: str, record: dict[str, Any],
            options: dict[str, dict[str, Any]]) -> dict[str, Any]:
    answer = record.get("answer")
    if answer not in options:
        raise Refusal(f"{protocol} / {question}: answer {answer!r} is not one of the "
                      f"session's options {sorted(options)}")
    return options[answer]


def _confidence(record: dict[str, Any]) -> str | None:
    value = (record.get("responses") or {}).get("confidence")
    return str(value) if value else None


def convert_key(protocol: str, record: dict[str, Any], plan: dict[str, Any]) -> dict[str, Any]:
    _require_complete(protocol, spec.QUESTION_KEY, record)
    chosen = _option(protocol, spec.QUESTION_KEY, record, plan["key"]["options"])
    entry = {"status": chosen["status"], "tonic": chosen.get("tonic"),
             "mode": chosen.get("mode")}
    if chosen["status"] == "adjudicated" and not (entry["tonic"] and entry["mode"]):
        raise Refusal(f"{protocol} / key: the sidecar's option carries no tonic and mode")
    return entry


def convert_meter(protocol: str, record: dict[str, Any], plan: dict[str, Any]) -> dict[str, Any]:
    _require_complete(protocol, spec.QUESTION_METER, record)
    chosen = _option(protocol, spec.QUESTION_METER, record, plan["meter"]["options"])
    if chosen["status"] == "adjudicated" and not chosen.get("value"):
        raise Refusal(f"{protocol} / meter: the sidecar's option carries no value")
    return {"status": chosen["status"], "value": chosen.get("value")}


def convert_instruments(protocol: str, record: dict[str, Any],
                        plan: dict[str, Any]) -> dict[str, Any]:
    _require_complete(protocol, spec.QUESTION_INSTRUMENTS, record)
    answer = record.get("answer")
    if answer == spec.INSTRUMENTS_CANT:
        return {"status": "unadjudicated", "present": [], "allowed_extra": [], "absent": []}
    if answer != spec.INSTRUMENTS_DONE:
        raise Refusal(f"{protocol} / instruments: answer {answer!r} is not one of the "
                      f"session's options")
    responses = record.get("responses") or {}
    lists: dict[str, list[str]] = {"present": [], "allowed_extra": [], "absent": []}
    unanswered = []
    for canonical in plan["instruments"]["rows"]:
        state = responses.get(spec.row_field_id(canonical))
        if state is None:
            unanswered.append(canonical)
            continue
        if state not in spec.STATE_TO_LIST:
            raise Refusal(f"{protocol} / instruments: row {canonical!r} has state {state!r}")
        lists[spec.STATE_TO_LIST[state]].append(canonical)
    if unanswered:
        raise Refusal(f"{protocol} / instruments: rows left unanswered: "
                      f"{', '.join(unanswered)}")
    allowed_extra = {name for group in plan["instruments"]["other_groups"] for name in group}
    for index in range(1, len(plan["instruments"]["other_groups"]) + 1):
        for name in responses.get(f"others-{index}") or []:
            if name not in allowed_extra:
                raise Refusal(f"{protocol} / instruments: chip {name!r} is not in the session")
            if name not in lists["present"]:
                lists["present"].append(name)
    if not lists["present"]:
        raise Refusal(f"{protocol} / instruments: nothing marked as heard; use "
                      f"\"{spec.INSTRUMENTS_CANT}\" if that is the answer")
    return {"status": "adjudicated", **lists}


def build_truth(
    sidecar: dict[str, Any],
    answers_dir: Path,
    current: dict[str, Any],
    *,
    allow_missing: bool = False,
    check_protocol_files: bool = True,
) -> tuple[dict[str, Any], list[str]]:
    """The new tracks.json document and a line per decision, or ``Refusal``."""
    if sidecar.get("schema") != spec.SIDECAR_SCHEMA:
        raise Refusal(f"session sidecar schema is {sidecar.get('schema')!r}, "
                      f"expected {spec.SIDECAR_SCHEMA!r}")
    document = json.loads(json.dumps(current))
    tracks = document.setdefault("tracks", {})
    log: list[str] = []
    problems: list[str] = []
    stamp = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    for protocol, plan in sidecar["protocols"].items():
        slug = plan["slug"]
        if check_protocol_files:
            path = Path(plan["protocol_path"])
            if not path.is_file() or _sha256(path) != plan["protocol_sha256"]:
                problems.append(f"{protocol}: the protocol file changed or vanished since "
                                f"the session was built; its answers cannot be trusted "
                                f"to mean the sidecar's options")
                continue
        answers_path = answers_dir / f"{protocol}.answers.jsonl"
        try:
            latest = latest_answers(answers_path)
        except Refusal as refusal:
            problems.append(str(refusal))
            continue
        entry = tracks.setdefault(slug, {})
        converters = ((spec.QUESTION_KEY, "key", convert_key),
                      (spec.QUESTION_METER, "meter", convert_meter),
                      (spec.QUESTION_INSTRUMENTS, "instruments", convert_instruments))
        notes: list[str] = []
        for question, field, converter in converters:
            record = latest.get(question)
            if record is None:
                if not allow_missing:
                    problems.append(f"{protocol} / {question}: not answered yet")
                else:
                    log.append(f"{slug} {field}: not answered, left as it was")
                continue
            try:
                value = converter(protocol, record, plan)
            except Refusal as refusal:
                problems.append(str(refusal))
                continue
            entry[field] = value
            confidence = _confidence(record)
            note = str(record.get("note") or "").strip()
            if confidence:
                notes.append(f"{field} confidence: {confidence}")
            if note:
                notes.append(f"{field} note: {note}")
            log.append(f"{slug} {field}: {json.dumps(value, ensure_ascii=False)}")
        if notes:
            prior = str(entry.get("notes") or "").strip()
            listening = "Listening session " + stamp + ": " + "; ".join(notes)
            entry["notes"] = (prior + " " if prior else "") + listening
        entry["source"] = {
            "session_version": sidecar.get("session_version"),
            "protocol": protocol,
            "protocol_sha256": plan.get("protocol_sha256"),
            "answers_sha256": (_sha256(answers_path) if answers_path.is_file() else None),
            "converted_at": stamp,
        }
    if problems:
        raise Refusal("\n".join(problems))
    return document, log


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--sidecar", type=Path, default=spec.SIDECAR_PATH)
    parser.add_argument("--answers-dir", type=Path, default=spec.ANSWERS_DIR)
    parser.add_argument("--output", type=Path, default=GROUND_TRUTH_PATH)
    parser.add_argument("--allow-missing", action="store_true",
                        help="convert the questions that are answered and leave the rest")
    parser.add_argument("--dry-run", action="store_true")
    options = parser.parse_args(argv)

    if not options.sidecar.is_file():
        print(f"no session at {options.sidecar}; build it with truth_session.py",
              file=sys.stderr)
        return 2
    sidecar = json.loads(options.sidecar.read_text(encoding="utf-8"))
    current = json.loads(options.output.read_text(encoding="utf-8"))
    try:
        document, log = build_truth(sidecar, options.answers_dir, current,
                                    allow_missing=options.allow_missing)
    except Refusal as refusal:
        print("refusing to write ground truth:", file=sys.stderr)
        for line in str(refusal).splitlines():
            print(f"  {line}", file=sys.stderr)
        return 2
    for line in log:
        print(line)
    if options.dry_run:
        print("dry run: nothing written")
        return 0
    atomic_write_json(options.output, document)
    print(f"written to {options.output}; now run `tools/mimo_bench/run.py score`")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
