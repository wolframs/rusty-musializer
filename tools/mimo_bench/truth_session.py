#!/usr/bin/env python3
"""Build the listening session that collects key, metre and instruments.

    build/lyrics-research-v2/.venv/bin/python tools/mimo_bench/truth_session.py

The benchmark's ground truth for key, metre and instruments can only come from
a listener. This writes one Listening Lab protocol per benchmark excerpt
(``tools/listening-lab``), three questions each, every one answerable by
listening and choosing rather than by naming music theory:

* **Key** — the top three candidates of a chroma key profile over the
  excerpt, each rendered as the excerpt with that key's tonic chord held
  quietly underneath. The right one sounds like home; a wrong one rubs.
* **Metre** — the excerpt with a counted click in 4/4, 3/4 and 6/8 (and 12/8
  when the beat grid hears triplet subdivisions), the high click on "one",
  on the beat grid librosa tracks in the excerpt.
* **Instruments** — a three-state checklist (hear it / maybe / not there) from
  the scorer's lexicon, plus chips for the rest of the lexicon and a free-text
  line.

Every audition file is rendered here, beforehand; **nothing is played**. The
sidecar ``session.json`` maps every option to the ground-truth value it stands
for, and ``truth_convert.py`` reads the answers back through it. Regenerating
over a session that already has answers is refused.

Needs numpy, librosa and soundfile (the ``lyrics-research-v2`` venv has them)
and ffmpeg on PATH.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    __package__ = "mimo_bench"

from . import audio, matrix, report  # noqa: E402
from .bench_io import TRACKS, Track, atomic_write_json, sha256_file, stored_calls  # noqa: E402
from .bench_io import repeats_present  # noqa: E402
from . import truth_spec as spec  # noqa: E402

SAMPLE_RATE = 44100
PITCHES = ("C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B")
DISPLAY = {"C#": "C♯", "Eb": "E♭", "F#": "F♯", "Ab": "A♭", "Bb": "B♭"}

# Krumhansl–Kessler probe-tone profiles (1982), the standard chroma key profile.
MAJOR_PROFILE = (6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88)
MINOR_PROFILE = (6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17)

KEY_CANDIDATES = 3
DRONE_TO_MUSIC_RMS = 0.30   # the chord sits under the music, audibly but quietly
CLICK_PEAK = 0.55           # relative to the music's own peak

KEY_NONE_OF_THESE = "None of these sits right"
KEY_NO_STABLE = "No stable home note: it wanders or never settles"
KEY_CANT_TELL = "Can't tell"
METER_FREE = "Free: no steady count fits"
METER_CANT_TELL = "Can't tell"

# meter -> (beats per bar, subdivisions per beat)
METERS: dict[str, tuple[int, int]] = {
    "4/4": (4, 1),
    "3/4": (3, 1),
    "6/8": (2, 3),
    "12/8": (4, 3),
}


class SessionError(RuntimeError):
    pass


# ---------------------------------------------------------------------------
# Analysis
# ---------------------------------------------------------------------------


def key_candidates(y, sr) -> list[dict[str, Any]]:
    """All 24 major/minor keys ranked by profile correlation over the excerpt."""
    import librosa
    import numpy as np

    harmonic = librosa.effects.harmonic(y)
    chroma = librosa.feature.chroma_cqt(y=harmonic, sr=sr).mean(axis=1)
    ranked = []
    for tonic in range(12):
        for mode, profile in (("major", MAJOR_PROFILE), ("minor", MINOR_PROFILE)):
            score = float(np.corrcoef(np.roll(np.array(profile), tonic), chroma)[0, 1])
            ranked.append({"tonic": PITCHES[tonic], "mode": mode, "correlation": score})
    ranked.sort(key=lambda item: item["correlation"], reverse=True)
    return ranked


def beat_grid(y, sr) -> dict[str, Any]:
    """Tracked beat times, plus how strongly the music subdivides in threes."""
    import librosa
    import numpy as np

    onset = librosa.onset.onset_strength(y=y, sr=sr)
    tempo, beats = librosa.beat.beat_track(onset_envelope=onset, sr=sr, units="time")
    times = librosa.times_like(onset, sr=sr)
    beats = np.asarray(beats, dtype=float)
    if len(beats) < 8:
        raise SessionError("the beat tracker found fewer than 8 beats")

    def strength_at(positions) -> float:
        frames = np.clip(np.searchsorted(times, positions), 0, len(onset) - 1)
        return float(np.mean(onset[frames]))

    gaps = np.diff(beats)
    starts = beats[:-1]
    halves = strength_at(starts + gaps / 2.0)
    thirds = strength_at(np.concatenate([starts + gaps / 3.0, starts + 2.0 * gaps / 3.0]))
    # Low-frequency onsets decide where "one" goes for each grouping.
    low = librosa.onset.onset_strength(y=y, sr=sr, fmax=200.0, n_mels=32)
    low_frames = np.clip(np.searchsorted(times, beats), 0, len(low) - 1)
    return {
        "tempo_bpm": float(np.atleast_1d(tempo)[0]),
        "beats": beats.tolist(),
        "beat_low_strength": low[low_frames].astype(float).tolist(),
        "subdivision_half_strength": halves,
        "subdivision_third_strength": thirds,
        "ternary_ratio": thirds / halves if halves > 0 else 0.0,
    }


def downbeat_phase(low_strength: list[float], beats_per_bar: int) -> int:
    best, best_value = 0, float("-inf")
    for phase in range(beats_per_bar):
        picked = low_strength[phase::beats_per_bar]
        value = sum(picked) / len(picked) if picked else float("-inf")
        if value > best_value:
            best, best_value = phase, value
    return best


def offered_meters(grid: dict[str, Any]) -> list[str]:
    meters = ["4/4", "3/4", "6/8"]
    # A grid whose off-beat energy sits on the thirds rather than on the half
    # suggests a compound (triplet) feel over four beats as well.
    if grid["ternary_ratio"] > 1.0:
        meters.append("12/8")
    return meters


# ---------------------------------------------------------------------------
# Audition audio
# ---------------------------------------------------------------------------


def chord_drone(tonic: str, mode: str, seconds: float, sr: int):
    """The tonic triad, held: root an octave below, triad around C3–B3."""
    import numpy as np

    root = PITCHES.index(tonic)
    base = 130.8128 * 2.0 ** (root / 12.0)          # C3 .. B3
    third = 4 if mode == "major" else 3
    voices = [(base / 2.0, 0.8), (base, 1.0), (base * 2 ** (third / 12.0), 0.7),
              (base * 2 ** (7 / 12.0), 0.7)]
    t = np.arange(int(seconds * sr)) / sr
    signal = np.zeros_like(t)
    for frequency, weight in voices:
        for partial, amplitude in ((1, 1.0), (2, 0.35), (3, 0.12)):
            signal += weight * amplitude * np.sin(2 * np.pi * frequency * partial * t)
    fade = np.minimum(1.0, np.minimum(t / 2.0, (t[-1] - t) / 2.0))
    return signal * fade


def click_track(grid: dict[str, Any], meter: str, seconds: float, sr: int):
    """A counted click: high on 'one', mid on each beat, soft subdivisions."""
    import numpy as np

    beats_per_bar, subdivisions = METERS[meter]
    beats = grid["beats"]
    phase = downbeat_phase(grid["beat_low_strength"], beats_per_bar)
    out = np.zeros(int(seconds * sr))
    blip_t = np.arange(int(0.035 * sr)) / sr

    def blip(at: float, frequency: float, amplitude: float) -> None:
        start = int(at * sr)
        if start < 0 or start >= len(out):
            return
        wave = amplitude * np.sin(2 * np.pi * frequency * blip_t) * np.exp(-blip_t / 0.008)
        end = min(len(out), start + len(wave))
        out[start:end] += wave[: end - start]

    for index, at in enumerate(beats):
        if (index - phase) % beats_per_bar == 0:
            blip(at, 1760.0, 1.0)
        else:
            blip(at, 1320.0, 0.6)
        if subdivisions > 1 and index + 1 < len(beats):
            gap = beats[index + 1] - at
            for step in range(1, subdivisions):
                blip(at + gap * step / subdivisions, 990.0, 0.3)
    return out


def write_mp3(samples, sr: int, destination: Path) -> None:
    import numpy as np
    import soundfile

    peak = float(np.max(np.abs(samples))) or 1.0
    if peak > 0.98:
        samples = samples * (0.98 / peak)
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise SessionError("ffmpeg is not on PATH")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as scratch:
        wav = Path(scratch) / "mix.wav"
        soundfile.write(str(wav), samples.astype("float32"), sr, subtype="PCM_16")
        subprocess.run(
            [ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "error", "-y",
             "-i", str(wav), "-c:a", "libmp3lame", "-b:a", "192k", str(destination)],
            check=True)


# ---------------------------------------------------------------------------
# What MiMo claimed, only to decide which lexicon rows a track needs
# ---------------------------------------------------------------------------


def claimed_instruments(track: Track) -> list[str]:
    found: set[str] = set()
    for cell in matrix.cells():
        if cell.track_slug != track.slug:
            continue
        for repeat in repeats_present(cell.id):
            for record in stored_calls(cell.id, repeat):
                claims, _ = report.claims_for_record(record)
                found.update(claims.instruments)
    return sorted(found)


# ---------------------------------------------------------------------------
# The protocol
# ---------------------------------------------------------------------------


def display_key(tonic: str, mode: str) -> str:
    return f"{DISPLAY.get(tonic, tonic)} {mode}"


def tab_letter(position: int) -> str:
    """The Lab names a non-blind protocol's tracks A, B, C… in file order.

    The track order is fixed: plain, the key chords, then the counts. Options
    name the tab they refer to, because the Lab numbers options 1-6 while the
    tabs carry letters, and a label like "Chord 2" would make the listener
    translate between the two.
    """
    return chr(ord("A") + position)


def instrument_rows(track: Track) -> tuple[list[str], list[str]]:
    rows = list(spec.INSTRUMENT_ROWS)
    promoted = [name for name in claimed_instruments(track) if name not in rows]
    rows.extend(promoted)
    return rows, promoted


def build_protocol(track: Track, plan: dict[str, Any]) -> dict[str, Any]:
    # Tab labels stay short: the Lab's tabs truncate after about ten characters,
    # which cut "E♭ major" vs "E♭ minor" off at first.
    tracks = [{"id": "plain", "label": "Plain",
               "path": plan["audio"]["plain"]}]
    for index, candidate in enumerate(plan["key"]["candidates"], start=1):
        tracks.append({"id": f"key-{index}",
                       "label": display_key(candidate["tonic"], candidate["mode"]),
                       "path": plan["audio"][f"key-{index}"]})
    for meter in plan["meter"]["offered"]:
        meter_id = "meter-" + meter.replace("/", "-")
        tracks.append({"id": meter_id, "label": f"Count {meter}",
                       "path": plan["audio"][meter_id]})

    key_options = list(plan["key"]["options"])
    meter_options = list(plan["meter"]["options"])
    rows = plan["instruments"]["rows"]
    others = plan["instruments"]["other_groups"]

    row_fields = []
    for canonical in rows:
        hint = spec.INSTRUMENT_HINTS.get(canonical)
        row_fields.append({
            "id": spec.row_field_id(canonical),
            "type": "single",
            "label": canonical + (f" — {hint}" if hint else ""),
            "required": True,
            "show_when": {"field": "answer", "any_of": [spec.INSTRUMENTS_DONE]},
            "options": [{"value": value, "label": label}
                        for value, label in spec.INSTRUMENT_STATES],
        })
    for index, group in enumerate(others, start=1):
        row_fields.append({
            "id": f"others-{index}",
            "type": "multi",
            "label": ("Anything else here you clearly hear?" if index == 1
                      else "…or one of these?"),
            "help": "Tick only what you hear. Leaving one unticked says nothing about it.",
            "show_when": {"field": "answer", "any_of": [spec.INSTRUMENTS_DONE]},
            "options": [{"value": name,
                         "label": name + (f" ({spec.INSTRUMENT_HINTS[name]})"
                                          if name in spec.INSTRUMENT_HINTS else "")}
                        for name in group],
        })

    confidence = {
        "id": "confidence",
        "type": "scale",
        "label": "How sure are you?",
        "options": [{"value": "guess", "label": "Close to a guess"},
                    {"value": "lean", "label": "Fairly"},
                    {"value": "clear", "label": "Clear"}],
    }
    key_tracks = ["plain"] + [f"key-{index}"
                              for index in range(1, len(plan["key"]["candidates"]) + 1)]
    meter_tracks = ["meter-" + meter.replace("/", "-") for meter in plan["meter"]["offered"]]
    title = plan["title"]
    return {
        "schema": "musializer.listening-test/v1",
        "id": spec.protocol_id(track.slug),
        "title": f"{title} — key, count and instruments (100 s excerpt)",
        "instructions": (
            "Three questions about the 100-second excerpt the benchmark sent to MiMo. "
            "No music theory needed: each question is answered by listening and picking. "
            "Answers save the moment you click, so you can stop and come back any time. "
            "\"Can't tell\" is a good answer: it keeps that question out of the score "
            "instead of guessing."),
        "blind": False,
        "tracks": tracks,
        "questions": [
            {
                "id": spec.QUESTION_KEY,
                "at_seconds": 0.0,
                "window": {"pre": 0.0, "post": 100.0},
                "question": "Which held chord sounds like home under this music?",
                "detail": (
                    "Play, then switch between the chords with [ and ] (or the tabs) while "
                    "the music keeps going. The right one blends: the melody keeps landing "
                    "on it and the song's endings settle onto it. A wrong one rubs or "
                    "sounds sour somewhere. If two feel equally right, pick the one that "
                    "fits the chorus. Tab A, Plain, is the music alone, to reset your ear."),
                "kind": "choice",
                "options": key_options,
                "tracks": key_tracks,
                "loop": False,
                "required": True,
                "feedback": {"fields": [confidence],
                             "note": {"collapsed": True,
                                      "label": "Something the choices missed"}},
            },
            {
                "id": spec.QUESTION_METER,
                "at_seconds": 0.0,
                "window": {"pre": 0.0, "post": 100.0},
                "question": "Which count can you keep going along with the music?",
                "detail": (
                    "Each version adds a click on the beat; the high click is \"one\". "
                    "Switch between them while it plays. Pick the one where counting "
                    "1-2-3-… along feels natural for the whole excerpt and \"one\" keeps "
                    "falling on strong moments. If the count fits but \"one\" sits on the "
                    "wrong beat, still pick that count."),
                "kind": "choice",
                "options": meter_options,
                "tracks": meter_tracks,
                "loop": False,
                "required": True,
                "feedback": {"fields": [confidence],
                             "note": {"collapsed": True,
                                      "label": "Something the choices missed"}},
            },
            {
                "id": spec.QUESTION_INSTRUMENTS,
                "at_seconds": 0.0,
                "window": {"pre": 0.0, "post": 100.0},
                "question": "Which instruments are in this excerpt?",
                "detail": (
                    "Go down the list: I hear it / Maybe / Not there. \"Not there\" means "
                    "you are sure it is absent; use Maybe when unsure. Skip around in the "
                    "waveform as much as you like."),
                "kind": "choice",
                "options": [spec.INSTRUMENTS_DONE, spec.INSTRUMENTS_CANT],
                "tracks": ["plain"],
                "loop": False,
                "required": True,
                "feedback": {"fields": row_fields,
                             "note": {"collapsed": False,
                                      "label": "An instrument the list lacks (free text)",
                                      "placeholder": "e.g. sitar, steel drum"}},
            },
        ],
    }


def plan_track(track: Track, title: str) -> dict[str, Any]:
    import librosa
    import numpy as np

    source = audio.excerpt_path(track)
    if not source.is_file():
        raise SessionError(f"{source} is missing; run `tools/mimo_bench/run.py prepare`")
    music, sr = librosa.load(str(source), sr=SAMPLE_RATE, mono=True)
    seconds = len(music) / sr

    ranked = key_candidates(music, sr)
    candidates = ranked[:KEY_CANDIDATES]
    grid = beat_grid(music, sr)
    meters = offered_meters(grid)

    out_dir = spec.AUDIO_DIR / track.slug
    files: dict[str, str] = {}
    music_rms = float(np.sqrt(np.mean(music ** 2))) or 1.0
    music_peak = float(np.max(np.abs(music))) or 1.0

    write_mp3(music, sr, out_dir / "plain.mp3")
    files["plain"] = str(out_dir / "plain.mp3")
    for index, candidate in enumerate(candidates, start=1):
        drone = chord_drone(candidate["tonic"], candidate["mode"], seconds, sr)
        drone *= DRONE_TO_MUSIC_RMS * music_rms / (float(np.sqrt(np.mean(drone ** 2))) or 1.0)
        path = out_dir / f"key-{index}.mp3"
        write_mp3(music + drone, sr, path)
        files[f"key-{index}"] = str(path)
    for meter in meters:
        clicks = click_track(grid, meter, seconds, sr)
        clicks *= CLICK_PEAK * music_peak / (float(np.max(np.abs(clicks))) or 1.0)
        meter_id = "meter-" + meter.replace("/", "-")
        path = out_dir / f"{meter_id}.mp3"
        write_mp3(music + clicks, sr, path)
        files[meter_id] = str(path)

    key_options: dict[str, dict[str, Any]] = {}
    for index, candidate in enumerate(candidates, start=1):
        label = (f"{tab_letter(index)} · "
                 f"{display_key(candidate['tonic'], candidate['mode'])} fits")
        key_options[label] = {"status": "adjudicated", "tonic": candidate["tonic"],
                              "mode": candidate["mode"]}
    key_options[KEY_NONE_OF_THESE] = {"status": "unadjudicated", "tonic": None,
                                      "mode": None, "why": "none of the candidates fit"}
    key_options[KEY_NO_STABLE] = {"status": "none", "tonic": None, "mode": None}
    key_options[KEY_CANT_TELL] = {"status": "unadjudicated", "tonic": None, "mode": None}

    meter_options: dict[str, dict[str, Any]] = {}
    for position, meter in enumerate(meters, start=1 + len(candidates)):
        meter_options[f"{tab_letter(position)} · {meter} fits"] = {"status": "adjudicated", "value": meter}
    meter_options[METER_FREE] = {"status": "adjudicated", "value": "free"}
    meter_options[METER_CANT_TELL] = {"status": "unadjudicated", "value": None}

    rows, promoted = instrument_rows(track)
    rest = [name for name in spec.lexicon_terms() if name not in rows]
    groups = spec.chip_groups(rest)

    return {
        "slug": track.slug,
        "title": title,
        "excerpt": {"path": str(source), "sha256": sha256_file(source),
                    "start_seconds": track.excerpt_start_seconds,
                    "seconds": round(seconds, 3)},
        "audio": files,
        "audio_sha256": {name: sha256_file(Path(path)) for name, path in files.items()},
        "key": {"method": "Krumhansl–Kessler profile over librosa chroma_cqt of the "
                          "harmonic part of the excerpt",
                "ranked": [dict(item, correlation=round(item["correlation"], 4))
                           for item in ranked[:6]],
                "candidates": candidates,
                "options": key_options},
        "meter": {"method": "librosa beat_track over the excerpt; 'one' placed on the "
                            "phase with the strongest low-frequency onsets",
                  "tempo_bpm": round(grid["tempo_bpm"], 2),
                  "beats": len(grid["beats"]),
                  "ternary_ratio": round(grid["ternary_ratio"], 3),
                  "offered": meters,
                  "options": meter_options},
        "instruments": {"rows": rows, "promoted_from_mimo_claims": promoted,
                        "other_groups": groups},
    }


TITLES = {"shut-up-cat": "Shut Up Cat", "constellation-whale": "Constellation Whale"}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--force", action="store_true",
                        help="replace a session that has no answers yet")
    options = parser.parse_args(argv)

    for track in TRACKS:
        answers = spec.ANSWERS_DIR / f"{spec.protocol_id(track.slug)}.answers.jsonl"
        if answers.exists():
            print(f"refusing: {answers} already holds answers to this session; "
                  f"they would no longer match regenerated options. Convert them "
                  f"first, or move them aside on purpose.", file=sys.stderr)
            return 2
    if spec.SIDECAR_PATH.exists() and not options.force:
        print(f"refusing: {spec.SIDECAR_PATH} exists; pass --force to rebuild a "
              f"session nobody has answered yet.", file=sys.stderr)
        return 2

    sidecar: dict[str, Any] = {"schema": spec.SIDECAR_SCHEMA,
                               "session_version": spec.SESSION_VERSION,
                               "protocols": {}}
    for track in TRACKS:
        plan = plan_track(track, TITLES.get(track.slug, track.slug))
        protocol = build_protocol(track, plan)
        path = spec.protocol_path(track.slug)
        atomic_write_json(path, protocol)
        plan["protocol_path"] = str(path)
        plan["protocol_sha256"] = sha256_file(path)
        sidecar["protocols"][protocol["id"]] = plan
        print(f"{track.slug}: key candidates "
              f"{[display_key(c['tonic'], c['mode']) for c in plan['key']['candidates']]}, "
              f"metres {plan['meter']['offered']} (beat {plan['meter']['tempo_bpm']} bpm, "
              f"ternary ratio {plan['meter']['ternary_ratio']}), "
              f"{len(plan['instruments']['rows'])} instrument rows "
              f"({len(plan['instruments']['promoted_from_mimo_claims'])} promoted)")
    atomic_write_json(spec.SIDECAR_PATH, sidecar)
    print(f"session written to {spec.SESSION_ROOT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
