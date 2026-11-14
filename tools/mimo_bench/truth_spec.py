"""What the ground-truth listening session asks, shared by its two halves.

``truth_session.py`` (needs librosa) writes the session: audition audio, one
Listening Lab protocol per excerpt and a sidecar that maps every answer the
operator can give to a ground-truth value. ``truth_convert.py`` (standard
library only) reads the Lab's answer log back through that sidecar into
``ground_truth/tracks.json``. Both import their vocabulary from here, so an
option label can never mean one thing to the generator and another to the
converter.
"""

from __future__ import annotations

from pathlib import Path

from .bench_io import BENCH_ROOT, DATA_ROOT
from .scorers import INSTRUMENT_LEXICON

SESSION_VERSION = "musializer.mimo-bench-truth-session/v1"
SIDECAR_SCHEMA = "musializer.mimo-bench-truth-sidecar/v1"

SESSION_ROOT = BENCH_ROOT / "truth-session"
PROTOCOLS_DIR = SESSION_ROOT / "browser"
AUDIO_DIR = SESSION_ROOT / "audio"
SIDECAR_PATH = SESSION_ROOT / "session.json"
# Where tools/listening-lab writes answers by default (its vite config resolves
# ../../build/listening-lab/answers from its own directory).
ANSWERS_DIR = DATA_ROOT / "build" / "listening-lab" / "answers"

QUESTION_KEY = "key"
QUESTION_METER = "meter"
QUESTION_INSTRUMENTS = "instruments"
QUESTIONS = (QUESTION_KEY, QUESTION_METER, QUESTION_INSTRUMENTS)

# The three states of an instrument row, and where each lands in tracks.json.
STATE_HEAR = "hear"
STATE_MAYBE = "maybe"
STATE_ABSENT = "absent"
INSTRUMENT_STATES = (
    (STATE_HEAR, "I hear it"),
    (STATE_MAYBE, "Maybe"),
    (STATE_ABSENT, "Not there"),
)
STATE_TO_LIST = {STATE_HEAR: "present", STATE_MAYBE: "allowed_extra",
                 STATE_ABSENT: "absent"}

INSTRUMENTS_DONE = "I went through the list"
INSTRUMENTS_CANT = "I can't pick instruments out here"

# The rows everyone answers, most likely first for AI-made pop and electronic
# music, which both excerpts are. Deliberately NOT ordered by what MiMo said:
# ordering the truth checklist by the claims of the model under test would
# anchor the truth to the thing it is meant to check. Lexicon terms MiMo did
# claim for a track and that are not here are promoted into that track's rows
# (unmarked), so every claim it made can be scored for precision.
INSTRUMENT_ROWS: tuple[str, ...] = (
    "lead vocal",
    "backing vocal",
    "kick",
    "snare",
    "clap",
    "hi-hat",
    "cymbal",
    "synth bass",
    "bass guitar",
    "synth pad",
    "synth lead",
    "synth pluck",
    "piano",
    "electric guitar",
    "strings",
    "vocal chop",
    "noise",
    "percussion",
)

# Short plain-language hints for rows a non-engineer may not recognise.
INSTRUMENT_HINTS: dict[str, str] = {
    "noise": "risers, whooshes, white-noise sweeps",
    "vocal chop": "cut-up vocal snippets used as an instrument",
    "synth pluck": "short plucked synth notes",
    "synth pad": "a soft sustained synth bed",
    "synth lead": "a synth playing a melody or arpeggio",
    "percussion": "shaker, tambourine, congas, cowbell…",
    "cymbal": "crash or ride",
    "drum machine": "programmed / electronic drums as a whole",
    "drum kit": "a played acoustic drum kit as a whole",
    "mallets": "marimba, vibraphone, glockenspiel, kalimba…",
}

# The Lab allows at most ten chips per multi field; the remainder of the
# lexicon is split into groups of at most this many.
OTHER_GROUP_SIZE = 10


def chip_groups(names: list[str]) -> list[list[str]]:
    """Split names into balanced groups the Lab accepts (2..10 chips each)."""
    if not names:
        return []
    count = -(-len(names) // OTHER_GROUP_SIZE)
    size = -(-len(names) // count)
    groups = [names[start:start + size] for start in range(0, len(names), size)]
    if len(groups) > 1 and len(groups[-1]) < 2:
        groups[-2].extend(groups.pop())
    return groups


def lexicon_terms() -> tuple[str, ...]:
    return tuple(INSTRUMENT_LEXICON)


def row_field_id(canonical: str) -> str:
    return "inst-" + "".join(ch if ch.isalnum() else "-" for ch in canonical)


def protocol_id(slug: str) -> str:
    return f"mimo-truth-v1-{slug}"


def protocol_path(slug: str, root: Path = PROTOCOLS_DIR) -> Path:
    return root / f"{protocol_id(slug)}.listen.json"


__all__ = [
    "ANSWERS_DIR",
    "AUDIO_DIR",
    "INSTRUMENTS_CANT",
    "INSTRUMENTS_DONE",
    "INSTRUMENT_HINTS",
    "INSTRUMENT_ROWS",
    "INSTRUMENT_STATES",
    "OTHER_GROUP_SIZE",
    "PROTOCOLS_DIR",
    "QUESTIONS",
    "QUESTION_INSTRUMENTS",
    "QUESTION_KEY",
    "QUESTION_METER",
    "SESSION_ROOT",
    "SESSION_VERSION",
    "SIDECAR_PATH",
    "SIDECAR_SCHEMA",
    "STATE_TO_LIST",
    "chip_groups",
    "lexicon_terms",
    "protocol_id",
    "protocol_path",
    "row_field_id",
]
