"""The scorers. Every dimension has a defined rule; none of them is a vibe.

Three kinds of rule appear here and the distinction is the whole design:

**Checked against a measurement or an adjudication.** Tempo, form, and lyric
position. These have a number to compare, so they are scored as correct /
octave-equivalent / wrong / absent, or as an error in seconds.

**Checked against an operator-authored list.** Key, meter, instruments. The
list is not in this repository's power to derive, so until it is filled in the
scorer returns ``abstain`` and the plan command says how many dimensions are
abstaining. An abstention is reported, never silently counted as a pass.

**Not checkable at all — so measured as consistency and concreteness.** Feel
and texture have no truth. What can be measured is whether repeated runs of
the same cell say the same thing (inter-run vocabulary agreement, and the
spread of the numeric fields), and whether the prose consists of statements
that could be checked at all. The concreteness rule is spelled out in
``concreteness`` and depends on lexicons that are versioned and hashed, so a
later session can see exactly what "concrete" meant when a number was
produced.
"""

from __future__ import annotations

import math
import re
import statistics
from dataclasses import dataclass, field
from typing import Any, Iterable, Sequence

from .bench_io import canonical_sha256
from .ground_truth import LyricTruth, TrackTruth

SCORER_VERSION = "musializer.mimo-bench-scorers/v1"

# ---------------------------------------------------------------------------
# Lexicons. These are the judgement calls in this file; they are hashed into
# every score document so a number can be traced to the vocabulary that made it.
# ---------------------------------------------------------------------------

#: canonical instrument name -> the surface forms that count as naming it.
INSTRUMENT_LEXICON: dict[str, tuple[str, ...]] = {
    "drum kit": ("drum kit", "drums", "drum set", "acoustic drums", "live drums"),
    "drum machine": ("drum machine", "programmed drums", "808", "909", "707",
                     "electronic drums", "sequenced drums"),
    "kick": ("kick", "kick drum", "bass drum"),
    "snare": ("snare", "snare drum", "rimshot"),
    "hi-hat": ("hi-hat", "hihat", "hi hat", "hats", "closed hat", "open hat"),
    "clap": ("clap", "handclap", "hand clap", "claps"),
    "cymbal": ("cymbal", "crash", "ride cymbal", "ride"),
    "percussion": ("percussion", "shaker", "tambourine", "conga", "bongo",
                   "woodblock", "cowbell"),
    "bass guitar": ("bass guitar", "electric bass", "fingered bass", "slap bass"),
    "synth bass": ("synth bass", "sub bass", "808 bass", "bass synth"),
    "double bass": ("double bass", "upright bass", "contrabass"),
    "electric guitar": ("electric guitar", "guitar", "lead guitar", "rhythm guitar",
                        "distorted guitar", "clean guitar"),
    "acoustic guitar": ("acoustic guitar", "steel string guitar", "nylon guitar",
                        "classical guitar"),
    "piano": ("piano", "grand piano", "upright piano", "acoustic piano"),
    "electric piano": ("electric piano", "rhodes", "wurlitzer", "wurli", "ep"),
    "organ": ("organ", "hammond", "hammond organ", "church organ"),
    "synth pad": ("synth pad", "pad", "pads", "string pad", "warm pad"),
    "synth lead": ("synth lead", "lead synth", "saw lead", "square lead", "arp",
                   "arpeggiator", "arpeggio synth"),
    "synth pluck": ("pluck", "plucks", "pluck synth"),
    "strings": ("strings", "string section", "violin", "viola", "cello",
                "orchestral strings"),
    "brass": ("brass", "trumpet", "trombone", "horn section", "french horn", "tuba"),
    "woodwind": ("woodwind", "flute", "clarinet", "oboe", "bassoon", "recorder"),
    "saxophone": ("saxophone", "sax", "tenor sax", "alto sax", "baritone sax"),
    "harp": ("harp",),
    "mallets": ("marimba", "vibraphone", "vibes", "xylophone", "glockenspiel",
                "kalimba", "celesta"),
    "accordion": ("accordion", "melodeon", "concertina"),
    "harmonica": ("harmonica", "harp mouth", "blues harp"),
    "banjo": ("banjo",),
    "mandolin": ("mandolin",),
    "ukulele": ("ukulele", "uke"),
    "lead vocal": ("lead vocal", "lead vocals", "vocal", "vocals", "singer",
                   "voice", "male vocal", "female vocal"),
    "backing vocal": ("backing vocal", "backing vocals", "harmony vocal",
                      "backup vocals", "vocal harmonies", "choir", "chorus vocals"),
    "vocal chop": ("vocal chop", "vocal chops", "chopped vocal", "vox chop"),
    "spoken word": ("spoken word", "spoken vocal", "narration", "speech", "rap",
                    "rapping"),
    "noise": ("white noise", "noise sweep", "riser", "downlifter", "noise wash"),
    "field recording": ("field recording", "foley", "ambience", "room tone",
                        "environmental sound"),
    "turntable": ("turntable", "scratch", "scratching", "vinyl scratch"),
    "sampler": ("sampler", "sample", "sampled loop", "chopped sample"),
    "theremin": ("theremin",),
    "bells": ("bell", "bells", "tubular bells", "chimes"),
}

#: terms that make a sentence a musical statement rather than an impression.
MUSIC_TERM_LEXICON: tuple[str, ...] = (
    "major", "minor", "dorian", "mixolydian", "phrygian", "lydian", "locrian",
    "aeolian", "ionian", "pentatonic", "chromatic", "diatonic", "modal",
    "tonic", "dominant", "subdominant", "cadence", "perfect cadence",
    "plagal", "suspended", "sus2", "sus4", "seventh", "ninth", "eleventh",
    "diminished", "augmented", "triad", "arpeggio", "inversion", "voicing",
    "syncopation", "swing", "shuffle", "triplet", "polyrhythm", "downbeat",
    "upbeat", "backbeat", "off-beat", "offbeat", "half-time", "double-time",
    "bar", "bars", "beat", "beats", "meter", "metre", "time signature",
    "verse", "chorus", "bridge", "pre-chorus", "prechorus", "intro", "outro",
    "breakdown", "drop", "build", "refrain", "coda", "hook", "turnaround",
    "sidechain", "side-chain", "compression", "compressor", "limiter",
    "reverb", "delay", "tape delay", "plate reverb", "chorus effect", "flanger",
    "phaser", "saturation", "distortion", "bitcrush", "bit-crush", "eq",
    "high-pass", "low-pass", "high pass", "low pass", "filter sweep", "cutoff",
    "resonance", "stereo width", "panned", "panning", "mono", "stereo",
    "transient", "attack", "sustain", "release", "decay", "envelope",
    "vibrato", "tremolo", "portamento", "glissando", "staccato", "legato",
    "unison", "octave", "detune", "detuned", "fifth", "fourth", "third",
    "quantized", "grid", "groove", "pocket", "tempo", "bpm", "key",
)

#: the words that make a sentence generic when nothing else in it is concrete.
GENERIC_TERMS: tuple[str, ...] = (
    "nice", "good", "great", "interesting", "beautiful", "amazing", "lovely",
    "powerful", "emotional", "moving", "atmospheric", "vibe", "vibes", "feel",
    "feeling", "mood", "energy", "energetic", "catchy", "pleasant", "cool",
    "dreamy", "epic", "chill", "smooth", "rich", "lush", "deep", "warm",
    "cold", "dark", "bright", "uplifting", "haunting", "evocative", "unique",
    "captivating", "immersive", "engaging", "compelling", "stunning",
)

SECTION_LABELS: tuple[str, ...] = (
    "intro", "verse", "pre-chorus", "prechorus", "chorus", "bridge",
    "breakdown", "drop", "build", "build-up", "buildup", "outro", "coda",
    "interlude", "hook", "refrain", "solo", "instrumental", "post-chorus",
    "middle eight", "middle 8", "tag", "vamp", "fill",
)

STOPWORDS: frozenset[str] = frozenset("""
a an the and or but if then than that this these those there here of in on at to
from by with without into over under as is are was were be been being it its it's
you your i my we our they their he she his her not no yes so very quite rather
just also too much many more most some any all each every both few own same such
can could will would shall should may might must do does did done have has had
about across after against along around before behind below beneath beside between
beyond during except for inside like near off once out outside past since through
throughout till toward towards until up upon within while what which who whom whose
when where why how
""".split())

LEXICON_SHA256 = canonical_sha256({
    "instruments": {name: list(forms) for name, forms in INSTRUMENT_LEXICON.items()},
    "music_terms": list(MUSIC_TERM_LEXICON),
    "generic_terms": list(GENERIC_TERMS),
    "section_labels": list(SECTION_LABELS),
})


# ---------------------------------------------------------------------------
# Text normalization
# ---------------------------------------------------------------------------

_WORD = re.compile(r"[a-z0-9']+")
_WHITESPACE = re.compile(r"\s+")
_SENTENCE = re.compile(r"(?<=[.!?;])\s+|\n+")


def normalize(text: str) -> str:
    return _WHITESPACE.sub(" ", text.replace("’", "'").lower()).strip()


def tokens(text: str) -> list[str]:
    return _WORD.findall(normalize(text))


def content_tokens(text: str) -> list[str]:
    return [token for token in tokens(text) if token not in STOPWORDS and len(token) > 2]


def sentences(text: str) -> list[str]:
    parts = [part.strip() for part in _SENTENCE.split(text or "")]
    return [part for part in parts if part]


# ---------------------------------------------------------------------------
# Claim extraction
# ---------------------------------------------------------------------------

_BPM = re.compile(
    r"(\d{2,3}(?:\.\d+)?)\s*(?:bpm|beats\s+per\s+minute)", re.IGNORECASE)
_METER = re.compile(r"\b([2-9]|1[0-9])\s*/\s*(2|4|8|16)\b")
_KEY = re.compile(
    r"\b([A-G])\s?([#b♯♭]?)\s*"
    r"(major|minor|maj|min|dorian|mixolydian|phrygian|lydian|locrian|aeolian|ionian)\b",
    re.IGNORECASE)
_KEY_OF = re.compile(r"\bkey\s+of\s+([A-G])\s?([#b♯♭]?)\b")
_QUOTED = re.compile(r"[\"“‘']([^\"“”‘’']{3,120})"
                     r"[\"”’']")
_CLOCK = re.compile(r"\b(\d{1,2}):([0-5]\d)(?:\.(\d+))?\b")
_SECONDS = re.compile(
    r"\b(?:at|around|about|near|from)?\s*(\d{1,3}(?:\.\d+)?)\s*"
    r"(?:s\b|sec\b|secs\b|seconds\b)", re.IGNORECASE)

ENHARMONIC: dict[str, str] = {
    "c": "C", "b#": "C", "c#": "C#", "db": "C#", "d": "D", "d#": "D#",
    "eb": "D#", "e": "E", "fb": "E", "f": "F", "e#": "F", "f#": "F#",
    "gb": "F#", "g": "G", "g#": "G#", "ab": "G#", "a": "A", "a#": "A#",
    "bb": "A#", "b": "B", "cb": "B",
}

MODE_ALIASES: dict[str, str] = {
    "maj": "major", "major": "major", "ionian": "major",
    "min": "minor", "minor": "minor", "aeolian": "minor",
    "dorian": "dorian", "mixolydian": "mixolydian", "phrygian": "phrygian",
    "lydian": "lydian", "locrian": "locrian",
}


def normalize_pitch(value: str | None) -> str | None:
    if not value:
        return None
    cleaned = value.strip().replace("♯", "#").replace("♭", "b").lower()
    cleaned = cleaned.replace("-sharp", "#").replace("-flat", "b")
    match = re.match(r"^([a-g])\s*([#b]?)", cleaned)
    if not match:
        return None
    return ENHARMONIC.get(match.group(1) + match.group(2))


def normalize_mode(value: str | None) -> str | None:
    if not value:
        return None
    cleaned = value.strip().lower()
    for alias, canonical in MODE_ALIASES.items():
        if cleaned.startswith(alias):
            return canonical
    return cleaned or None


def parse_seconds(text: str) -> list[float]:
    """Every time in a fragment, from `m:ss` and from `NN s` forms."""
    found: list[float] = []
    for match in _CLOCK.finditer(text):
        fraction = float(f"0.{match.group(3)}") if match.group(3) else 0.0
        found.append(int(match.group(1)) * 60 + int(match.group(2)) + fraction)
    for match in _SECONDS.finditer(text):
        found.append(float(match.group(1)))
    return found


@dataclass
class Claims:
    """What one response asserts, however it was shaped."""

    source: str                                   # "structured" | "free-text"
    text: str = ""
    tempo_bpm: list[float] = field(default_factory=list)
    meters: list[str] = field(default_factory=list)
    keys: list[tuple[str | None, str | None]] = field(default_factory=list)
    instruments: list[str] = field(default_factory=list)
    unknown_instrument_terms: list[str] = field(default_factory=list)
    lyric_moments: list[tuple[float, str]] = field(default_factory=list)
    sections: list[tuple[float, str]] = field(default_factory=list)
    descriptors: list[str] = field(default_factory=list)
    numeric: dict[str, float] = field(default_factory=dict)
    uncertain: list[str] = field(default_factory=list)


def canonical_instrument(term: str) -> str | None:
    cleaned = normalize(term)
    for canonical, forms in INSTRUMENT_LEXICON.items():
        if cleaned == canonical or cleaned in forms:
            return canonical
    # A multi-word claim such as "warm analogue synth pad" still names a pad.
    for canonical, forms in INSTRUMENT_LEXICON.items():
        for form in (canonical,) + forms:
            if re.search(rf"\b{re.escape(form)}\b", cleaned):
                return canonical
    return None


def instruments_in_text(text: str) -> tuple[list[str], list[str]]:
    """Canonical instruments named anywhere in prose, longest form first.

    Returns ``(canonical, unmatched_candidates)``. The second list is only
    populated by the structured path, where the model gave an explicit name
    the lexicon does not know; prose cannot distinguish "a name we do not
    know" from "not an instrument name at all".
    """
    haystack = normalize(text)
    found: list[str] = []
    for canonical, forms in INSTRUMENT_LEXICON.items():
        for form in sorted((canonical,) + forms, key=len, reverse=True):
            if re.search(rf"\b{re.escape(form)}\b", haystack):
                found.append(canonical)
                break
    return found, []


def claims_from_structured(document: dict[str, Any]) -> Claims:
    claims = Claims(source="structured")
    prose_parts = [
        str(document.get("summary") or ""),
        str(document.get("harmony_notes") or ""),
        str(document.get("production_notes") or ""),
    ]
    for instrument in (document.get("instruments") or []):
        if isinstance(instrument, dict):
            prose_parts.append(f"{instrument.get('name', '')} {instrument.get('timbre', '')}")
    prose_parts.extend(str(item) for item in (document.get("texture") or []))
    prose_parts.extend(str(item) for item in (document.get("feel") or []))
    claims.text = "\n".join(part for part in prose_parts if part).strip()

    tempo = document.get("tempo_bpm")
    if isinstance(tempo, (int, float)) and not isinstance(tempo, bool):
        claims.tempo_bpm.append(float(tempo))
    meter = document.get("meter")
    if isinstance(meter, str) and meter.strip() and meter.strip().lower() != "unknown":
        claims.meters.append(meter.strip())
    key, mode = document.get("key"), document.get("mode")
    if isinstance(key, str) and key.strip() and key.strip().lower() != "unknown":
        # A `key` of "F# minor" carries the mode too; the explicit field wins.
        embedded = _KEY.search(key)
        tonic = normalize_pitch(key)
        embedded_mode = normalize_mode(embedded.group(3)) if embedded else None
        claims.keys.append((tonic, normalize_mode(mode) or embedded_mode))
    elif isinstance(mode, str) and mode.strip():
        claims.keys.append((None, normalize_mode(mode)))

    for instrument in (document.get("instruments") or []):
        if not isinstance(instrument, dict):
            continue
        name = str(instrument.get("name") or "")
        canonical = canonical_instrument(name)
        if canonical:
            claims.instruments.append(canonical)
        elif name.strip():
            claims.unknown_instrument_terms.append(name.strip())

    for moment in (document.get("lyric_moments") or []):
        if not isinstance(moment, dict):
            continue
        seconds, phrase = moment.get("seconds"), moment.get("phrase")
        if isinstance(seconds, (int, float)) and not isinstance(seconds, bool) \
                and isinstance(phrase, str) and phrase.strip():
            claims.lyric_moments.append((float(seconds), phrase.strip()))

    for section in (document.get("sections") or []):
        if not isinstance(section, dict):
            continue
        start, label = section.get("start_seconds"), section.get("label")
        if isinstance(start, (int, float)) and not isinstance(start, bool):
            claims.sections.append((float(start), str(label or "")))

    claims.descriptors = _descriptor_terms(
        list(document.get("texture") or []) + list(document.get("feel") or []))
    for name in ("energy", "tension", "valence"):
        value = document.get(name)
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            claims.numeric[name] = float(value)
    claims.uncertain = [str(item) for item in (document.get("uncertain") or [])]
    # Deduplicate while keeping first-mention order.
    claims.instruments = list(dict.fromkeys(claims.instruments))
    return claims


def claims_from_text(text: str) -> Claims:
    claims = Claims(source="free-text", text=text or "")
    claims.tempo_bpm = [float(match.group(1)) for match in _BPM.finditer(text or "")]
    claims.meters = [f"{match.group(1)}/{match.group(2)}"
                     for match in _METER.finditer(text or "")]
    lowered = normalize(text or "")
    if "common time" in lowered and "4/4" not in claims.meters:
        claims.meters.append("4/4")
    if "waltz" in lowered and "3/4" not in claims.meters:
        claims.meters.append("3/4")
    for match in _KEY.finditer(text or ""):
        claims.keys.append((
            normalize_pitch(match.group(1) + match.group(2)),
            normalize_mode(match.group(3)),
        ))
    if not claims.keys:
        for match in _KEY_OF.finditer(text or ""):
            claims.keys.append((normalize_pitch(match.group(1) + match.group(2)), None))
    claims.instruments, claims.unknown_instrument_terms = instruments_in_text(text or "")
    claims.lyric_moments = _quoted_moments(text or "")
    claims.sections = _sections_in_text(text or "")
    claims.descriptors = _descriptor_terms(sentences(text or ""))
    return claims


def _descriptor_terms(fragments: Iterable[str]) -> list[str]:
    seen: list[str] = []
    for fragment in fragments:
        for token in content_tokens(str(fragment)):
            if token not in seen:
                seen.append(token)
    return seen


QUOTE_TIME_WINDOW_CHARACTERS = 120


def timed_positions(text: str) -> list[tuple[int, float]]:
    """Every time in the text, with the character offset it was written at."""
    found: list[tuple[int, float]] = []
    for match in _CLOCK.finditer(text):
        fraction = float(f"0.{match.group(3)}") if match.group(3) else 0.0
        found.append((match.start(),
                      int(match.group(1)) * 60 + int(match.group(2)) + fraction))
    for match in _SECONDS.finditer(text):
        found.append((match.start(), float(match.group(1))))
    found.sort()
    return found


def _quoted_moments(text: str) -> list[tuple[float, str]]:
    """Quoted phrases paired with the *nearest* time, by character distance.

    Nearest rather than last: prose routinely lists two quotations in one
    sentence, each with its own second, and taking whichever time came last
    would attribute both phrases to the second one. That mistake is invisible
    in a spot check and would show up only as a mysteriously large median
    error for the free-text arms.

    A phrase with no time inside the window is still recorded, at ``nan``, so
    the lyric scorer can separate "wrong time" from "no time given at all" —
    the second is prompt compliance, not accuracy.
    """
    times = timed_positions(text)
    moments: list[tuple[float, str]] = []
    for match in _QUOTED.finditer(text):
        phrase = match.group(1).strip()
        if len(tokens(phrase)) < 2:
            continue
        candidates = [
            (min(abs(position - match.start()), abs(position - match.end())), seconds)
            for position, seconds in times
            if not (match.start() <= position < match.end())
        ]
        near = [candidate for candidate in candidates
                if candidate[0] <= QUOTE_TIME_WINDOW_CHARACTERS]
        moments.append((min(near)[1] if near else float("nan"), phrase))
    return moments


def _sections_in_text(text: str) -> list[tuple[float, str]]:
    found: list[tuple[float, str]] = []
    for line in re.split(r"[\n]+", text):
        lowered = normalize(line)
        label = next((name for name in SECTION_LABELS
                      if re.search(rf"\b{re.escape(name)}\b", lowered)), None)
        if label is None:
            continue
        times = parse_seconds(line)
        if times:
            found.append((times[0], label))
    return found


# ---------------------------------------------------------------------------
# Objective dimensions
# ---------------------------------------------------------------------------

TEMPO_TOLERANCE = 0.04          # 4 %, generous enough for a rounded claim
OCTAVE_FACTORS = (0.25, 0.5, 1.0, 2.0, 4.0)
SECTION_TOLERANCE_SECONDS = 3.0
LYRIC_MATCH_THRESHOLD = 0.7
LYRIC_MIN_TOKENS = 3


def _tempo_verdict(claim: float, truth: float) -> str | None:
    for factor in OCTAVE_FACTORS:
        target = truth * factor
        if target <= 0:
            continue
        if abs(claim - target) / target <= TEMPO_TOLERANCE:
            return "exact" if factor == 1.0 else "octave"
    return None


def score_tempo(claims: Claims, truth: TrackTruth) -> dict[str, Any]:
    """Octave equivalence is mandatory, not generosity — and it is bounded.

    Two things this scorer refuses to paper over. First, the repository's own
    tempo estimate for both benchmark excerpts is a *sub-multiple* of the felt
    tempo, so a claim of 112.5 against a measured 56.25 is the model agreeing;
    the factor is recorded so a systematic halving is visible rather than
    hidden inside a pass. Second, the estimate is ambiguous, so the reference
    set is the whole ranked candidate list, and the verdict names which
    candidate matched and at what rank. A claim that only matches the fourth
    candidate at four times its rate is technically accepted and obviously
    weaker evidence than one matching the argmax exactly, and the report can
    tell them apart because both facts are in the record.
    """
    references: dict[str, float] = {}
    if isinstance(truth.measured_bpm, (int, float)) and truth.measured_bpm:
        references["measured"] = float(truth.measured_bpm)
    for rank, candidate in enumerate(truth.excerpt_bpm_candidates, 1):
        if candidate:
            references[f"excerpt#{rank}"] = float(candidate)
    if not references and isinstance(truth.excerpt_bpm, (int, float)) and truth.excerpt_bpm:
        references["excerpt#1"] = float(truth.excerpt_bpm)
    if not references:
        return {"status": "abstain", "reason": "no measured tempo",
                "claims": len(claims.tempo_bpm)}
    if not claims.tempo_bpm:
        return {"status": "absent", "claims": 0, "references": references}

    order = ("exact", "octave", "wrong")
    verdicts: list[dict[str, Any]] = []
    for claim in claims.tempo_bpm:
        best: dict[str, Any] = {
            "claim": claim, "verdict": "wrong", "against": None,
            "ratio": claim / references[next(iter(references))],
        }
        for name, value in references.items():
            verdict = _tempo_verdict(claim, value)
            if verdict is None:
                continue
            if order.index(verdict) < order.index(best["verdict"]):
                best = {"claim": claim, "verdict": verdict, "against": name,
                        "ratio": claim / value}
        verdicts.append(best)
    accepted = sum(1 for item in verdicts if item["verdict"] in ("exact", "octave"))
    return {
        "status": "scored",
        "claims": len(verdicts),
        "exact": sum(1 for item in verdicts if item["verdict"] == "exact"),
        "octave": sum(1 for item in verdicts if item["verdict"] == "octave"),
        "wrong": sum(1 for item in verdicts if item["verdict"] == "wrong"),
        "accept_rate": accepted / len(verdicts),
        "matched_against": sorted({str(item["against"]) for item in verdicts
                                   if item["against"]}),
        "references": references,
        "verdicts": verdicts,
    }


def tempo_chance_accept_rate(
    truth: TrackTruth, *, low: float = 60.0, high: float = 200.0, step: float = 0.1,
) -> float:
    """What a uniformly random BPM guess would score. The scorer's null model.

    Octave equivalence over a multi-candidate reference set buys a lot of
    accept bands, and an accept rate is meaningless without the number a
    coin would get. The plan command prints this beside the matrix, and the
    decision gates are written against it rather than against zero.
    """
    references = [truth.measured_bpm] + list(truth.excerpt_bpm_candidates)
    usable = [float(value) for value in references
              if isinstance(value, (int, float)) and value]
    if not usable:
        return 0.0
    steps = int(round((high - low) / step)) + 1
    accepted = sum(
        1 for index in range(steps)
        if any(_tempo_verdict(low + index * step, reference) for reference in usable)
    )
    return accepted / steps


RELATIVE_SEMITONES = 3
PITCH_ORDER = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")


def _semitone(pitch: str | None) -> int | None:
    return PITCH_ORDER.index(pitch) if pitch in PITCH_ORDER else None


def score_key(claims: Claims, truth: TrackTruth) -> dict[str, Any]:
    if truth.key_status == "unadjudicated":
        return {"status": "abstain", "reason": "key ground truth not authored",
                "claims": len(claims.keys)}
    if truth.key_status == "none":
        return {"status": "scored", "expected": "no stable tonal centre",
                "claims": len(claims.keys),
                "false_claims": len(claims.keys),
                "verdict": "correct" if not claims.keys else "false-claim"}
    if not claims.keys:
        return {"status": "absent", "claims": 0}
    truth_tonic = normalize_pitch(truth.key_tonic)
    truth_mode = normalize_mode(truth.key_mode)
    verdicts: list[dict[str, Any]] = []
    for tonic, mode in claims.keys:
        if tonic == truth_tonic and mode == truth_mode:
            verdict = "exact"
        elif tonic == truth_tonic:
            verdict = "parallel"          # right tonic, wrong mode
        else:
            claimed, expected = _semitone(tonic), _semitone(truth_tonic)
            verdict = "wrong"
            if claimed is not None and expected is not None:
                distance = (claimed - expected) % 12
                if {truth_mode, mode} == {"major", "minor"} and distance in (
                        RELATIVE_SEMITONES, 12 - RELATIVE_SEMITONES):
                    verdict = "relative"   # the relative major/minor
        verdicts.append({"tonic": tonic, "mode": mode, "verdict": verdict})
    best = min(verdicts,
               key=lambda item: ("exact", "relative", "parallel", "wrong").index(
                   item["verdict"]))
    return {"status": "scored", "claims": len(verdicts), "verdict": best["verdict"],
            "expected": {"tonic": truth_tonic, "mode": truth_mode},
            "verdicts": verdicts}


def _normalize_meter(value: str) -> str:
    cleaned = normalize(value).replace(" ", "")
    if cleaned in ("commontime", "common"):
        return "4/4"
    if cleaned in ("cuttime", "alla breve"):
        return "2/2"
    match = re.match(r"^(\d{1,2})/(\d{1,2})$", cleaned)
    return f"{int(match.group(1))}/{int(match.group(2))}" if match else cleaned


def score_meter(claims: Claims, truth: TrackTruth) -> dict[str, Any]:
    if truth.meter_status == "unadjudicated" or not truth.meter:
        return {"status": "abstain", "reason": "meter ground truth not authored",
                "claims": len(claims.meters)}
    if not claims.meters:
        return {"status": "absent", "claims": 0}
    expected = _normalize_meter(str(truth.meter))
    claimed = [_normalize_meter(value) for value in claims.meters]
    return {
        "status": "scored",
        "claims": len(claimed),
        "expected": expected,
        "claimed": claimed,
        "verdict": "correct" if expected in claimed else "wrong",
    }


def score_instruments(claims: Claims, truth: TrackTruth) -> dict[str, Any]:
    """Precision, recall and F1 with a hallucination canary separated out.

    ``allowed_extra`` names neither help recall nor hurt precision: they are
    the plausible-but-unverified ones. ``absent`` names are the canaries — an
    instrument the operator says is definitely not there — and are reported on
    their own line, because one of those is a different failure from an
    over-eager but harmless extra.
    """
    if truth.instruments_status == "unadjudicated" or not truth.instruments_present:
        return {"status": "abstain", "reason": "instrument ground truth not authored",
                "claimed": len(claims.instruments)}
    expected = {canonical_instrument(name) or normalize(name)
                for name in truth.instruments_present}
    neutral = {canonical_instrument(name) or normalize(name)
               for name in truth.instruments_allowed_extra}
    canaries = {canonical_instrument(name) or normalize(name)
                for name in truth.instruments_absent}
    claimed = set(claims.instruments)
    hit = claimed & expected
    scored_claims = claimed - neutral
    false_positives = scored_claims - expected
    precision = len(hit) / len(scored_claims) if scored_claims else 0.0
    recall = len(hit) / len(expected) if expected else 0.0
    f1 = (2 * precision * recall / (precision + recall)) if (precision + recall) else 0.0
    return {
        "status": "scored",
        "expected": sorted(expected),
        "claimed": sorted(claimed),
        "matched": sorted(hit),
        "missed": sorted(expected - claimed),
        "false_positives": sorted(false_positives),
        "canaries_claimed": sorted(claimed & canaries),
        "neutral_claimed": sorted(claimed & neutral),
        "unknown_terms": list(claims.unknown_instrument_terms),
        "precision": precision,
        "recall": recall,
        "f1": f1,
    }


def _lcs_length(left: Sequence[str], right: Sequence[str]) -> int:
    previous = [0] * (len(right) + 1)
    for item in left:
        current = [0]
        for index, other in enumerate(right):
            current.append(previous[index] + 1 if item == other
                           else max(previous[index + 1], current[index]))
        previous = current
    return previous[-1]


def phrase_similarity(claim: str, reference: str) -> float:
    """Containment of the claimed phrase in a reference line, by token LCS.

    Containment rather than symmetric overlap, because a model is asked for
    *short* phrases and a truth line is a whole lyric line; penalizing the
    claim for being shorter would mark a correct quotation as a partial match.
    """
    claim_tokens, reference_tokens = tokens(claim), tokens(reference)
    if not claim_tokens or not reference_tokens:
        return 0.0
    return _lcs_length(claim_tokens, reference_tokens) / len(claim_tokens)


def score_lyric_moments(
    claims: Claims,
    truth: TrackTruth,
    *,
    time_offset: float = 0.0,
) -> dict[str, Any]:
    """Match each quoted phrase to an aligned line, then measure the seconds.

    ``time_offset`` exists for the chunk-frame check: a chunked run that
    answered on its own clock rather than the source clock is measured a
    second time with the chunk's start added, and whichever frame fits better
    is reported. That is a compliance finding, not a licence to shop for a
    better score, so both numbers are kept.
    """
    lyrics: list[LyricTruth] = truth.lyrics
    if not lyrics:
        return {"status": "abstain", "reason": "no aligned lyric truth in the excerpt"}
    if not claims.lyric_moments:
        return {"status": "absent", "claims": 0, "truth_lines": len(lyrics)}

    errors: list[float] = []
    matched_lines: set[int] = set()
    fabricated = untimed = too_short = 0
    details: list[dict[str, Any]] = []
    for seconds, phrase in claims.lyric_moments:
        if len(tokens(phrase)) < LYRIC_MIN_TOKENS:
            too_short += 1
            continue
        scored = [(phrase_similarity(phrase, line.text), index)
                  for index, line in enumerate(lyrics)]
        best_similarity, best_index = max(scored) if scored else (0.0, -1)
        if best_similarity < LYRIC_MATCH_THRESHOLD:
            fabricated += 1
            details.append({"phrase": phrase, "seconds": seconds,
                            "match": None, "similarity": best_similarity})
            continue
        matched_lines.add(best_index)
        if math.isnan(seconds):
            untimed += 1
            details.append({"phrase": phrase, "seconds": None,
                            "match": lyrics[best_index].text,
                            "similarity": best_similarity, "error_seconds": None})
            continue
        error = abs((seconds + time_offset) - lyrics[best_index].start_seconds)
        errors.append(error)
        details.append({"phrase": phrase, "seconds": seconds + time_offset,
                        "match": lyrics[best_index].text,
                        "truth_seconds": lyrics[best_index].start_seconds,
                        "similarity": best_similarity, "error_seconds": error})
    scoreable = len(claims.lyric_moments) - too_short
    return {
        "status": "scored",
        "claims": len(claims.lyric_moments),
        "too_short": too_short,
        "scoreable": scoreable,
        "matched": len(errors) + untimed,
        "untimed": untimed,
        "fabricated": fabricated,
        "fabrication_rate": fabricated / scoreable if scoreable else 0.0,
        "truth_lines": len(lyrics),
        "coverage": len(matched_lines) / len(lyrics),
        "median_error_seconds": statistics.median(errors) if errors else None,
        "mean_error_seconds": statistics.fmean(errors) if errors else None,
        "within_2s": sum(1 for error in errors if error <= 2.0) / len(errors)
        if errors else 0.0,
        "within_5s": sum(1 for error in errors if error <= 5.0) / len(errors)
        if errors else 0.0,
        "details": details,
    }


def score_time_frame(
    claims: Claims, truth: TrackTruth, *, chunk_start: float,
) -> dict[str, Any]:
    """Did a chunked answer use the source clock it was told to use?

    Only meaningful when the chunk does not start at the excerpt start; for
    the whole-excerpt condition the two frames coincide and the result is
    ``not-applicable``.
    """
    if abs(chunk_start - truth.excerpt_start) < 1e-6:
        return {"status": "not-applicable"}
    absolute = score_lyric_moments(claims, truth, time_offset=0.0)
    relative = score_lyric_moments(claims, truth, time_offset=chunk_start)
    if absolute.get("status") != "scored" or relative.get("status") != "scored":
        return {"status": "abstain"}
    absolute_error = absolute.get("median_error_seconds")
    relative_error = relative.get("median_error_seconds")
    if absolute_error is None and relative_error is None:
        return {"status": "abstain"}
    if relative_error is None or (absolute_error is not None
                                  and absolute_error <= relative_error):
        frame = "absolute"
    else:
        frame = "chunk-local"
    return {
        "status": "scored",
        "frame_used": frame,
        "obeyed": frame == "absolute",
        "median_error_absolute": absolute_error,
        "median_error_chunk_local": relative_error,
    }


def score_sections(claims: Claims, truth: TrackTruth) -> dict[str, Any]:
    """Agreement with the repository's own segmentation, never "correctness".

    ``summary.sections`` is an estimate from ``tools/analyze_audio.py``. A
    model that disagrees with it may be right; the number here is agreement,
    and the plan says so where it is read.
    """
    if not truth.sections:
        return {"status": "abstain", "reason": "no measured sections in the excerpt"}
    boundaries = sorted({start for start, _ in truth.sections}
                        | {end for _, end in truth.sections})
    if not claims.sections:
        return {"status": "absent", "claims": 0, "measured_boundaries": len(boundaries)}
    claimed = sorted(start for start, _ in claims.sections)
    hits = 0
    for value in claimed:
        if any(abs(value - boundary) <= SECTION_TOLERANCE_SECONDS
               for boundary in boundaries):
            hits += 1
    recalled = sum(
        1 for boundary in boundaries
        if any(abs(value - boundary) <= SECTION_TOLERANCE_SECONDS for value in claimed))
    precision = hits / len(claimed)
    recall = recalled / len(boundaries)
    return {
        "status": "scored",
        "claimed_boundaries": len(claimed),
        "measured_boundaries": len(boundaries),
        "tolerance_seconds": SECTION_TOLERANCE_SECONDS,
        "agreement_precision": precision,
        "agreement_recall": recall,
        "agreement_f1": (2 * precision * recall / (precision + recall))
        if (precision + recall) else 0.0,
    }


# ---------------------------------------------------------------------------
# The unscoreable dimensions: concreteness and consistency
# ---------------------------------------------------------------------------


def concreteness(text: str) -> dict[str, Any]:
    """Count falsifiable statements against generic ones, by a pinned rule.

    A sentence is **concrete** when it contains at least one of:

    1. a number with a musical unit (bpm, Hz, kHz, dB, bars, beats, seconds);
    2. a timestamp, either ``m:ss`` or ``NN s``;
    3. an instrument from ``INSTRUMENT_LEXICON``;
    4. a term from ``MUSIC_TERM_LEXICON``;
    5. a quoted phrase of two or more words.

    It is **generic** when it contains a term from ``GENERIC_TERMS`` and none
    of the five. Sentences that are neither are counted as ``neutral`` and
    excluded from the ratio, so padding does not improve the score by
    diluting it.
    """
    unit_number = re.compile(
        r"\d+(?:\.\d+)?\s*(?:bpm|hz|khz|db|bars?|beats?|semitones?|cents?)\b",
        re.IGNORECASE)
    concrete = generic = neutral = 0
    for sentence in sentences(text or ""):
        lowered = normalize(sentence)
        is_concrete = bool(
            unit_number.search(sentence)
            or _CLOCK.search(sentence)
            or _SECONDS.search(sentence)
            or instruments_in_text(sentence)[0]
            or any(re.search(rf"\b{re.escape(term)}\b", lowered)
                   for term in MUSIC_TERM_LEXICON)
            or any(len(tokens(match.group(1))) >= 2 for match in _QUOTED.finditer(sentence))
        )
        if is_concrete:
            concrete += 1
        elif any(re.search(rf"\b{re.escape(term)}\b", lowered) for term in GENERIC_TERMS):
            generic += 1
        else:
            neutral += 1
    words = len(tokens(text or ""))
    judged = concrete + generic
    return {
        "lexicon_sha256": LEXICON_SHA256,
        "words": words,
        "sentences": concrete + generic + neutral,
        "concrete": concrete,
        "generic": generic,
        "neutral": neutral,
        "concrete_per_100_words": (concrete / words * 100.0) if words else 0.0,
        "concrete_share": (concrete / judged) if judged else 0.0,
    }


def jaccard(left: Iterable[str], right: Iterable[str]) -> float:
    first, second = set(left), set(right)
    if not first and not second:
        return 1.0
    union = first | second
    return len(first & second) / len(union) if union else 0.0


def inter_run_agreement(runs: Sequence[Claims]) -> dict[str, Any]:
    """Mean pairwise vocabulary agreement across repeats of one cell.

    This is the substitute for scoring "feel". Two runs that describe the same
    hundred seconds with disjoint vocabulary are not both usable, whichever
    one a listener prefers.
    """
    if len(runs) < 2:
        return {"status": "abstain", "reason": "fewer than two repeats", "runs": len(runs)}
    descriptor_pairs: list[float] = []
    instrument_pairs: list[float] = []
    for first in range(len(runs)):
        for second in range(first + 1, len(runs)):
            descriptor_pairs.append(
                jaccard(runs[first].descriptors, runs[second].descriptors))
            instrument_pairs.append(
                jaccard(runs[first].instruments, runs[second].instruments))
    numeric: dict[str, Any] = {}
    for name in ("energy", "tension", "valence"):
        values = [run.numeric[name] for run in runs if name in run.numeric]
        if len(values) >= 2:
            numeric[name] = {
                "mean": statistics.fmean(values),
                "stdev": statistics.pstdev(values),
                "range": max(values) - min(values),
            }
    return {
        "status": "scored",
        "runs": len(runs),
        "descriptor_jaccard_mean": statistics.fmean(descriptor_pairs),
        "descriptor_jaccard_min": min(descriptor_pairs),
        "instrument_jaccard_mean": statistics.fmean(instrument_pairs),
        "numeric_stability": numeric,
    }


# ---------------------------------------------------------------------------
# Determinism of the reformatting arms
# ---------------------------------------------------------------------------


def _normalized_value(value: Any) -> Any:
    if isinstance(value, bool) or value is None:
        return value
    if isinstance(value, (int, float)):
        return round(float(value), 6)
    if isinstance(value, str):
        return normalize(value)
    if isinstance(value, list):
        return tuple(sorted((repr(_normalized_value(item)) for item in value)))
    if isinstance(value, dict):
        return tuple(sorted((key, repr(_normalized_value(item)))
                            for key, item in value.items()))
    return repr(value)


def field_disagreement(
    documents: Sequence[dict[str, Any]], fields: Sequence[str],
) -> dict[str, Any]:
    """Field-level disagreement across N runs on byte-identical input.

    The operator flagged the separate-reformatter strategy as a determinism
    risk. This is that risk measured rather than argued: for each field, the
    share of runs whose normalized value differs from the modal one, plus the
    share of runs whose whole record is identical to the modal record.
    """
    if len(documents) < 2:
        return {"status": "abstain", "reason": "fewer than two runs",
                "runs": len(documents)}
    per_field: dict[str, float] = {}
    for name in fields:
        values = [_normalized_value(document.get(name)) for document in documents]
        counts: dict[str, int] = {}
        for value in values:
            counts[repr(value)] = counts.get(repr(value), 0) + 1
        modal = max(counts.values())
        per_field[name] = 1.0 - modal / len(values)
    whole = [repr(tuple(_normalized_value(document.get(name)) for name in fields))
             for document in documents]
    counts = {}
    for value in whole:
        counts[value] = counts.get(value, 0) + 1
    identical = max(counts.values()) / len(whole)
    return {
        "status": "scored",
        "runs": len(documents),
        "fields": per_field,
        "field_disagreement_rate": statistics.fmean(per_field.values()) if per_field else 0.0,
        "worst_field": max(per_field, key=per_field.__getitem__) if per_field else None,
        "identical_output_rate": identical,
    }


__all__ = [
    "Claims",
    "GENERIC_TERMS",
    "INSTRUMENT_LEXICON",
    "LEXICON_SHA256",
    "LYRIC_MATCH_THRESHOLD",
    "LYRIC_MIN_TOKENS",
    "MUSIC_TERM_LEXICON",
    "OCTAVE_FACTORS",
    "SCORER_VERSION",
    "SECTION_LABELS",
    "SECTION_TOLERANCE_SECONDS",
    "STOPWORDS",
    "TEMPO_TOLERANCE",
    "canonical_instrument",
    "claims_from_structured",
    "claims_from_text",
    "concreteness",
    "content_tokens",
    "field_disagreement",
    "instruments_in_text",
    "inter_run_agreement",
    "jaccard",
    "normalize",
    "normalize_mode",
    "normalize_pitch",
    "parse_seconds",
    "phrase_similarity",
    "score_instruments",
    "score_key",
    "score_lyric_moments",
    "score_meter",
    "score_sections",
    "score_tempo",
    "score_time_frame",
    "sentences",
    "tempo_chance_accept_rate",
    "timed_positions",
    "tokens",
]
