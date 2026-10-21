#!/usr/bin/env python3
"""Time a track's known lyrics with one whole-track CTC pass (MV9-g).

    tools/align_lyrics.py song.mp3 song.lyrics.json            # embedded tags
    tools/align_lyrics.py song.mp3 song.lyrics.json --lyrics words.txt

Writes ``musializer.aligned-lyrics/v1`` JSON that a treatment can name by path
(``"lyrics": "song.lyrics.json"``), and prints a report a director should read
before trusting it: the lines the acoustic model was unsure of, and how the
song's repeated lines line up with each other.

Why this exists. Making the "Door, No Room" video, this exact procedure — the
known words, forced-aligned with torchaudio's MMS_FA over the *whole* track at
once, with a wildcard token between lines to absorb instrumental stretches and
wordless vocals — placed all 88 lines of a 5:22 track from its embedded
``lyrics-eng`` tag. No Whisper, no hand-chosen windows. Two lines came back
weak; the chorus-repetition check (every repeat of a chorus line sits at the
same offset from the first) found which. The normal Assist route (LT2) is a
different, Whisper-led pipeline; this is the short path for a director who
already has the words.

What it is not: a transcription. It forces the words you give it onto the
audio. Wrong words get confidently wrong times; the scores and the repetition
table are how you notice.

Runs under the MMS alignment runtime (``musializer_doctor.py`` reports it). If
the interpreter running this file has no torch, it re-executes itself under
``$MUSIALIZER_ALIGN_PYTHON`` or ``~/.local/share/musializer/lyrics-align``'s
venv. Opens no audio device; ffmpeg decodes to a pipe.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SCHEMA = "musializer.aligned-lyrics/v1"
SAMPLE_RATE = 16_000
CHUNK_SECONDS = 30
# A cue opens a little before its first word is sung and holds a little after
# its last, so a reveal is under way when the word lands.
LEAD_SECONDS = 0.10
HOLD_SECONDS = 1.6
GAP_SECONDS = 0.05
WEAK_SCORE = 0.4


def alignment_python() -> Path | None:
    configured = os.environ.get("MUSIALIZER_ALIGN_PYTHON")
    for candidate in (
        Path(configured).expanduser() if configured else None,
        Path.home() / ".local/share/musializer/lyrics-align/.venv/bin/python",
    ):
        if candidate is not None and candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate
    return None


def ensure_runtime() -> None:
    try:
        import torch  # noqa: F401
        import torchaudio  # noqa: F401
    except ImportError:
        python = alignment_python()
        if python is None or Path(sys.executable).resolve() == python.resolve():
            sys.exit("align_lyrics: torch/torchaudio are not importable and no MMS "
                     "alignment runtime was found (see tools/musializer_doctor.py)")
        os.execv(str(python), [str(python), "-I", str(Path(__file__).resolve()), *sys.argv[1:]])


def embedded_lyrics(audio: Path) -> tuple[str, str]:
    """The track's lyric tag, preferring English, as ffprobe reports it."""
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format_tags:stream_tags", "-of", "json", str(audio)],
        capture_output=True, text=True, check=True,
    )
    payload = json.loads(probe.stdout or "{}")
    tag_sets = [payload.get("format", {}).get("tags", {})]
    tag_sets += [stream.get("tags", {}) for stream in payload.get("streams") or [] if isinstance(stream, dict)]
    candidates = [
        (key, value) for tags in tag_sets if isinstance(tags, dict)
        for key, value in tags.items()
        if "lyric" in key.lower() and isinstance(value, str) and value.strip()
    ]
    candidates.sort(key=lambda item: (0 if item[0].lower().endswith("eng") else 1, item[0].lower()))
    if not candidates:
        sys.exit(f"align_lyrics: {audio} has no lyric tag; pass --lyrics FILE")
    key, value = candidates[0]
    return value, f"embedded:{key}"


def sung_lines(text: str) -> list[str]:
    """Lines that are sung: no blank lines, no ``[Section: notes]`` headers."""
    lines = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or (line.startswith("[") and line.endswith("]")):
            continue
        lines.append(line)
    return lines


def words_of(line: str) -> list[str]:
    tokens = re.split(r"[\s—–-]+", line.lower().replace("’", "'"))
    return [word for word in (re.sub(r"[^a-z']", "", token) for token in tokens) if word]


def decode(audio: Path):
    import numpy as np

    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(audio), "-ac", "1", "-ar", str(SAMPLE_RATE), "-f", "f32le", "-"],
        capture_output=True, check=True,
    ).stdout
    return np.frombuffer(raw, dtype=np.float32).copy()


def align(samples, lines: list[str]) -> list[dict]:
    import torch
    import torchaudio
    from torchaudio.pipelines import MMS_FA as bundle

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = bundle.get_model(with_star=True).to(device)
    dictionary = bundle.get_dict(star="*")
    words, owner = ["*"], [-1]
    for index, line in enumerate(lines):
        for word in words_of(line):
            words.append(word)
            owner.append(index)
        words.append("*")
        owner.append(-1)
    wave = torch.from_numpy(samples)[None]
    with torch.inference_mode():
        emissions = []
        step = SAMPLE_RATE * CHUNK_SECONDS
        for start in range(0, wave.shape[1], step):
            emission, _ = model(wave[:, start:start + step].to(device))
            emissions.append(emission)
        emission = torch.cat(emissions, 1)
    tokens = [dictionary[character] for word in words for character in word]
    aligned, scores = torchaudio.functional.forced_align(
        emission, torch.tensor([tokens], device=device), blank=0)
    spans = torchaudio.functional.merge_tokens(aligned[0].cpu(), scores[0].exp().cpu())
    seconds_per_frame = wave.shape[1] / emission.shape[1] / SAMPLE_RATE
    timed: list[dict] = [{"words": []} for _ in lines]
    cursor = 0
    for word, index in zip(words, owner):
        span = spans[cursor:cursor + len(word)]
        cursor += len(word)
        if index < 0:
            continue
        timed[index]["words"].append({
            "start": span[0].start * seconds_per_frame,
            "end": span[-1].end * seconds_per_frame,
            "score": float(sum(token.score for token in span) / len(span)),
        })
    return timed


def build(lines: list[str], timed: list[dict], duration: float) -> list[dict]:
    out = []
    for index, (text, record) in enumerate(zip(lines, timed)):
        words = record["words"]
        start, end = words[0]["start"], words[-1]["end"]
        out.append({
            "at": round(max(start - LEAD_SECONDS, 0.0), 3),
            "end": round(end, 3),
            "text": text,
            "score": round(sum(word["score"] for word in words) / len(words), 3),
        })
    for index, line in enumerate(out):
        next_at = out[index + 1]["at"] if index + 1 < len(out) else duration
        until = min(next_at - GAP_SECONDS, line["end"] + HOLD_SECONDS, duration)
        line["until"] = round(max(until, line["at"] + 0.2), 3)
    return out


def report(lines: list[dict]) -> None:
    weak = [line for line in lines if line["score"] < WEAK_SCORE]
    print(f"{len(lines)} lines placed; {len(weak)} under score {WEAK_SCORE}:")
    for line in weak:
        print(f"  {line['at']:8.2f}  {line['score']:.2f}  {line['text']}")
    # Repetition is free evidence: the same words sung again should sit at a
    # whole number of bars from their first time, and every line of a
    # repeated block at the same offset. One that disagrees is the one to fix.
    seen: dict[str, list[float]] = {}
    for line in lines:
        seen.setdefault(line["text"].lower(), []).append(line["at"])
    repeats = {text: times for text, times in seen.items() if len(times) > 1}
    if repeats:
        print("repeated lines (offsets from the first):")
        for text, times in repeats.items():
            offsets = ", ".join(f"+{time - times[0]:.2f}" for time in times[1:])
            print(f"  {times[0]:8.2f}  {offsets:<28}  {text}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("audio", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--lyrics", type=Path, help="plain text, one line per sung line; "
                        "default: the audio file's own lyric tag")
    args = parser.parse_args()
    ensure_runtime()
    if args.lyrics:
        text, source = args.lyrics.read_text(encoding="utf-8"), f"file:{args.lyrics.name}"
    else:
        text, source = embedded_lyrics(args.audio)
    lines = sung_lines(text)
    if not lines:
        sys.exit("align_lyrics: no sung lines in the lyrics")
    samples = decode(args.audio)
    duration = len(samples) / SAMPLE_RATE
    timed = align(samples, lines)
    placed = build(lines, timed, duration)
    digest = hashlib.sha256(args.audio.read_bytes()).hexdigest()
    document = {
        "schema": SCHEMA,
        "audio_sha256": digest,
        "source": {"lyrics": source, "aligner": "torchaudio.pipelines.MMS_FA, whole track, wildcard between lines"},
        "lines": placed,
    }
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(json.dumps(document, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    temporary.replace(args.output)
    report(placed)
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
