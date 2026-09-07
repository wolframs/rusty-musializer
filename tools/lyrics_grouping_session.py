#!/usr/bin/env python3
"""Prepare the two pending lyric-grouping judgments; never plays or uploads audio.

The Rust protocol runner and browser Listening Lab share one lossless audition
and the same question ids. Source music, generated audio and answers stay local.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import wave
import zipfile

CASES = [
    ('groyper', 'Groyper Idol.mp3', 25.0,
     'Groyper Idol — 26.5–29.7s', 'If shit goes south / take out your gun'),
    ('floor', 'Floor Mechanics I_ Load Bearing.mp3', 100.0,
     'Floor Mechanics — about 101.6–103.5s', 'I am / present'),
]


# Saved model proposals, not adjudicated vocal boundaries. Use identical outer
# edges for both interpretations so this audition compares grouping alone.
TIMING_PROPOSALS = {
    'groyper': {
        'parts': [('If shit goes south', 26.5, 28.0), ('take out your gun', 28.22, 29.7)],
        'evidence': '03/candidate-performed-runtime/lyrics.performance.json',
        'note': 'Model proposal: the two-line version clears the first line at 28.00 s and starts the second at 28.22 s. Judge these edges too; they are not confirmed.',
    },
    'floor': {
        'parts': [('I am', 101.45, 102.2), ('present', 102.85, 103.55)],
        'evidence': '07/performed-reference-audio-v3/reference.json',
        'note': 'Model proposal: the two-line version clears “I am” at 102.20 s and starts “present” at 102.85 s. Judge these edges too; they are not confirmed.',
    },
}


def lyric_comparison(identifier, title, source_start, audition_start):
    proposal = TIMING_PROPOSALS[identifier]
    def cue(text, start, end):
        return dict(text=text, start_seconds=round(start-source_start+audition_start, 6),
                    end_seconds=round(end-source_start+audition_start, 6))
    parts = proposal['parts']
    split = [cue(*part) for part in parts]
    return dict(source_title=title.split(' — ')[0], source_offset_seconds=source_start-audition_start,
        timing_note=proposal['note'],
        gap=dict(start_seconds=split[0]['end_seconds'], end_seconds=split[1]['start_seconds']),
        variants=[dict(label='One phrase', cues=[cue(' '.join(part[0] for part in parts), parts[0][1], parts[-1][2])]),
                  dict(label='Two phrases', cues=split)])


def prepare(audio_dir: Path, output: Path):
    if output.exists():
        raise ValueError('Session already exists; choose a new output directory to preserve answers')
    sources = [audio_dir / case[1] for case in CASES]
    if any(not p.is_file() for p in sources):
        raise ValueError('Both source tracks must exist in --audio-dir')
    pcm, evidence, questions, browser_questions = bytearray(), [], [], []
    for number, ((identifier, _, start, title, words), audio) in enumerate(zip(CASES, sources)):
        offset = len(pcm) / (16000 * 2)
        chunk = subprocess.check_output([
            'ffmpeg', '-v', 'error', '-i', str(audio), '-ss', str(start), '-t', '6',
            '-vn', '-ac', '1', '-ar', '16000', '-map_metadata', '-1', '-f', 's16le', 'pipe:1'], timeout=60)
        if len(chunk) != 6 * 16000 * 2:
            raise ValueError(f'{audio.name}: expected an entire six-second excerpt')
        pcm.extend(chunk)
        question = f'{title}: “{words}”. Does the pause separate two performed phrases?'
        options = ['one phrase', 'two phrases', 'not sure']
        questions.append(dict(id=identifier, at_seconds=offset, window=dict(pre=0, post=6),
                              question=question, kind='choice', options=options))
        browser_questions.append(dict(**questions[-1],
            detail='Compare the synchronized one-line and two-line previews above. Choose the grouping that fits, or not sure if the proposed timing prevents a judgment.',
            lyric_comparison=lyric_comparison(identifier, title, start, offset),
            tracks=['audition'], loop=True, required=True, feedback='grouping'))
        evidence.append(dict(question_id=identifier, source_audio=str(audio.resolve()),
            source_sha256=hashlib.sha256(audio.read_bytes()).hexdigest(),
            source_start_seconds=start, source_end_seconds=start + 6,
            audition_start_seconds=offset, audition_end_seconds=offset + 6,
            pcm_sha256=hashlib.sha256(chunk).hexdigest(),
            timing_proposal=TIMING_PROPOSALS[identifier]))
        if number + 1 < len(CASES):
            pcm.extend(bytes(2 * 16000 * 2))  # Separate auditions; source samples stay intact.
    output.mkdir(parents=True)
    (output / 'browser').mkdir()
    with wave.open(str(output / 'grouping.wav'), 'wb') as wav:
        wav.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
        wav.writeframes(pcm)
    digest = hashlib.sha256((output / 'grouping.wav').read_bytes()).hexdigest()
    protocol = dict(schema='musializer.protocol/v1', title='Lyrics — two grouping decisions',
                    audio=dict(path='grouping.wav', sha256=digest), items=questions)
    sheet = dict(schema='musializer.listening-test/v1', id='lyrics-grouping-20260907',
        title=protocol['title'], blind=False,
        instructions='Two short excerpts, three choices each. Replay as often as needed. Answers save immediately; uncertain is a useful answer. Separate sung echoes count separately, and a continuous chop run counts once.',
        tracks=[dict(id='audition', label='Original performances', path='../grouping.wav')],
        questions=browser_questions,
        feedback_templates=dict(grouping=dict(fields=[dict(id='confidence', type='scale',
            label='How clear is the phrasing?', required=True,
            options=[dict(value='clear', label='Clear'), dict(value='lean', label='Leaning'),
                     dict(value='uncertain', label='Uncertain')]),
            dict(id='second_onset', type='timestamps', label='Optional: mark where the second phrase begins',
                 max_selections=1, show_when=dict(field='answer', any_of=['two phrases']))],
            note=dict(collapsed=True, label='Add context the choices missed'))))
    def write(name, value):
        (output / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    write('grouping.protocol.json', protocol)
    write('browser/lyrics-grouping-20260907.listen.json', sheet)
    write('evidence.json', dict(status='awaiting_operator_judgment', audio_sha256=digest,
                               sample_rate=16000, cases=evidence))
    (output / 'run.sh').write_text('#!/bin/sh\nset -eu\ncd "$(dirname "$0")"\nexec "${MUSIALIZER_BINARY:-musializer}" --protocol "$PWD/grouping.protocol.json"\n')
    (output / 'run.sh').chmod(0o755)
    (output / 'README.md').write_text('''# Two lyric-grouping decisions

Open `grouping.protocol.json` in Musializer, or run:

    MUSIALIZER_BINARY=/absolute/path/to/musializer ./run.sh

This plays the original excerpts. Space pauses; R replays. Press 1 for one
phrase, 2 for two phrases, or 3 for not sure. An answer saves and advances;
N advances without answering. Keep the answers file beside the protocol.

For browser playback, point Listening Lab at this bundle's `browser/` folder.
Choose “Lyrics — two grouping decisions”. It includes waveform seeking,
looping, playback speed, synchronized one-line/two-line previews, proposed gap
markers, confidence, and optional second-phrase timestamps. The timed lyric
comparison is available in the browser; the Rust runner presents the audio and
question only.
Browser answers live in the host's Listening Lab answer directory. You may
use either surface; they share question ids. No answer is preselected.

The WAV joins two unchanged six-second PCM excerpts with a two-second gap.
`evidence.json` maps audition timestamps back to the original tracks. The
first excerpt is Groyper Idol at 25–31s; the second is Floor Mechanics at
100–106s. The grouping choices have not been adjudicated or applied.
''')
    with zipfile.ZipFile(output.with_suffix('.zip'), 'w', zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(output.rglob('*')):
            if path.is_file():
                archive.write(path, arcname=f'{output.name}/{path.relative_to(output)}')
    return output / 'grouping.protocol.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audio-dir', type=Path, default=Path.home() / 'Music')
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args()
    print(prepare(args.audio_dir, args.output_dir.resolve()))


if __name__ == '__main__':
    main()
