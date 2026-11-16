#!/usr/bin/env python3
"""Stage a feelings answer for the Assist review gate, without the network (J2).

    feelings_fixture.py PROJECT.musi AUDIO JOB_DIR [ANSWER.json] [--reused]

Seeds PROJECT.musi (a project the gate saved from a synthetic treatment) with
three earlier feeling cues and two of the user's own markers, then writes a job
folder the panel can stage from: JOB_DIR/result.bridge.tsv carrying AUDIO's
digest and length, one section and one SEMANTIC cue per answer segment. With
--reused, JOB_DIR/assist-manifest.json says the answer was a saved one.

ANSWER.json is a semantic cache (`musializer.analysis-cache/v1`, its
`normalized.segments`) or a bare `musializer.semantic-score/v1` document: a
real answer's section count, text lengths and number ranges. Its times are
scaled onto AUDIO's length. Without one, an invented eleven-section answer
over the same shape stands in, so the gate runs in any checkout; a real answer
about the operator's songs belongs in build/ only and is never committed.

Prints one line: `feelings fixture: sections=N earlier=3 manual=2 duration=S`.
"""

import base64
import hashlib
import json
import sys
import wave
from pathlib import Path

# Invented for the gate: the shape of a real 300-second answer (eleven
# sections, a quiet stretch, a climax, text of a hundred-odd characters),
# none of its words.
INVENTED = [
    (0, 24, 0.78, 0.55, 0.52, 0.92, "A stuttering intro of clipped synth stabs and a dry kick, setting a restless, playful tone before anything settles."),
    (24, 58, 0.94, 0.38, 0.85, 0.96, "The first drop: a wide, bright chord stack over a running bassline, every element pushing forward at once."),
    (58, 83, 0.62, 0.35, 0.48, 0.90, "A lighter passage where the drums thin out and a soft lead carries a simple, sweet melody."),
    (83, 100, 0.76, 0.60, 0.55, 0.88, "A short build: snare rolls and a rising filter sweep that leans into the next section."),
    (100, 135, 0.50, 0.28, 0.68, 0.93, "A floating interlude of warm pads and small bell tones, spacious and unhurried."),
    (135, 155, 0.72, 0.75, 0.60, 0.91, "Tension gathers in a glitchy, accelerating rhythm with pitched stutters climbing upward."),
    (155, 185, 0.86, 0.30, 0.88, 0.94, "A bouncing groove with punchy kicks and a cheerful hook that keeps returning."),
    (185, 199, 0.58, 0.65, 0.62, 0.89, "A held breath: sustained chords and one long riser, waiting for the release."),
    (199, 260, 0.98, 0.40, 0.82, 0.96, "The climax, relentless and loud, with a sawtooth bass and the hook stacked in octaves."),
    (260, 285, 0.55, 0.25, 0.58, 0.91, "A soft landing: the beat drops away and a gentle reprise of the opening motif plays."),
    (285, 300, 0.84, 0.48, 0.70, 0.93, "One last punchy return of the main motif, cut off abruptly at the end."),
]

# The project's earlier feeling cues (what Apply replaces) and the user's own
# markers (what it must not touch), as fractions of the track.
EARLIER = [(0.0, [0.40, 0.30, 0.20, 0.80]), (0.40, [0.85, 0.55, 0.60, 0.80]), (0.75, [0.50, 0.25, -0.30, 0.70])]
MANUAL = [(0.12, 1, "cue", [1.0]), (0.55, 2, "semantic", [1.0])]


def b64(text):
    return base64.b64encode(text.encode("utf-8")).decode("ascii")


def segments_from(path):
    document = json.loads(Path(path).read_text(encoding="utf-8"))
    normalized = document.get("normalized", document)
    segments = normalized["segments"]
    return [
        (s["start_seconds"], s["end_seconds"], s["energy"], s["tension"], s["valence"],
         s["confidence"], s.get("summary", ""))
        for s in segments
    ]


def main(argv):
    flags = [arg for arg in argv[1:] if arg.startswith("--")]
    args = [arg for arg in argv[1:] if not arg.startswith("--")]
    if len(args) not in (3, 4) or any(flag != "--reused" for flag in flags):
        print(__doc__, file=sys.stderr)
        return 2
    project_path, audio_path, job = Path(args[0]), Path(args[1]), Path(args[2])
    segments = segments_from(args[3]) if len(args) == 4 else INVENTED

    with wave.open(str(audio_path), "rb") as handle:
        duration_ms = round(handle.getnframes() * 1000 / handle.getframerate())
    digest = hashlib.sha256(audio_path.read_bytes()).hexdigest()
    answer_end = segments[-1][1]
    scale = duration_ms / 1000.0 / answer_end

    project = json.loads(project_path.read_text(encoding="utf-8"))
    seconds = duration_ms / 1000.0
    project["semantic_events"] = [
        {"timestamp_seconds": round(fraction * seconds, 3), "id": index + 1, "type": "semantic", "values": values}
        for index, (fraction, values) in enumerate(EARLIER)
    ]
    project["manual_events"] = [
        {"timestamp_seconds": round(fraction * seconds, 3), "id": ident, "type": kind, "values": values}
        for fraction, ident, kind, values in MANUAL
    ]
    project_path.write_text(json.dumps(project, indent=2) + "\n", encoding="utf-8")

    lines = ["MUSIALIZER_BRIDGE\t1", f"AUDIO\t{digest}\t{duration_ms}",
             f"SECTION\t1\t0\t{duration_ms}\tspectrum\t500\t{b64(json.dumps(['fixture']))}"]
    previous_end = 0
    for index, (start, end, energy, tension, valence, confidence, summary) in enumerate(segments):
        start_ms = previous_end
        end_ms = duration_ms if index == len(segments) - 1 else round(end * scale * 1000)
        lines.append("\t".join([
            "SEMANTIC", str(100 + index), str(start_ms), str(end_ms),
            str(round(energy * 1000)), str(round(tension * 1000)), str(round(valence * 1000)),
            str(round(confidence * 1000)), b64(summary),
        ]))
        previous_end = end_ms
    job.mkdir(parents=True, exist_ok=True)
    (job / "result.bridge.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    manifest = job / "assist-manifest.json"
    if "--reused" in flags:
        manifest.write_text(json.dumps({
            "schema_version": "musializer.assist-manifest/v1", "mode": "mimo",
            "semantic_reused": {"reused": True, "answered_utc": "2026-11-15 16:08 UTC"},
        }) + "\n", encoding="utf-8")
    elif manifest.exists():
        manifest.unlink()
    print(f"feelings fixture: sections={len(segments)} earlier={len(EARLIER)} "
          f"manual={len(MANUAL)} duration={seconds:g}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
