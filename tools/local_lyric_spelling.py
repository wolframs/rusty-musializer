#!/usr/bin/env python3
"""Performed inventory with exact local authored spelling.

The input remains the audio-derived inventory. No line or time is added,
removed or inferred from the sheet, and no reference text leaves the process.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from analysis_io import atomic_write_json, canonical_sha256
import lyric_align

VERSION = '1'


def project(document, reference):
    if document.get('schema_version') != 'musializer.lyric-review/v1':
        raise ValueError('Expected an audio-derived lyric review inventory')
    text = reference['text']
    if hashlib.sha256(text.encode('utf-8', errors='replace')).hexdigest() != reference['sha256']:
        raise ValueError('Written reference changed after discovery')
    targets = {tuple(lyric_align.normalize_tokens(row['text'])) for row in document['lines']}
    choices = {tokens: {} for tokens in targets if tokens}
    for row in lyric_align.classify_reference_lines(text):
        if row['kind'] not in ('lyric', 'backing'):
            continue
        display = row['display']
        spans = list(re.finditer(r'\S+', display))
        for begin in range(len(spans)):
            for end in range(begin + 1, len(spans) + 1):
                left, right = spans[begin].start(), spans[end - 1].end()
                snippet = display[left:right].strip('"“”() ')
                if not snippet or len(snippet) > 512:
                    continue
                tokens = tuple(lyric_align.normalize_tokens(snippet))
                if tokens not in choices:
                    continue
                origins = choices[tokens].setdefault(snippet, [])
                origins.append(dict(reference_line_index=row['index'], classified_text=display,
                                    start_character=left, end_character=right))
    result = copy.deepcopy(document)
    matches = []
    for index, row in enumerate(result['lines']):
        options = choices.get(tuple(lyric_align.normalize_tokens(row['text'])), {})
        if len(options) != 1:
            continue
        spelling, origins = next(iter(options.items()))
        original = row['text']
        row['text'] = spelling
        matches.append(dict(cue_index=index, original_audio_text=original,
                            authored_text=spelling, reference_spans=origins))
    for flag in result.get('review_flags', []):
        index = flag.get('cue_index')
        if type(index) is int and 0 <= index < len(result['lines']):
            flag['text'] = result['lines'][index]['text']
    result['source']['local_wording'] = dict(version=VERSION, input_sha256=canonical_sha256(document),
        policy='unique exact normalized-token match; no fuzzy substitution',
        reference=copy.deepcopy(reference), matches=matches,
        inventory_authority='corroborated_audio_observations', reference_scope='local-only',
        unmatched_cue_indices=sorted(set(range(len(result['lines']))) - {m['cue_index'] for m in matches}))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('reference', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    text = args.reference.read_text()
    reference = dict(text=text, source='file:' + args.reference.name,
                     sha256=hashlib.sha256(text.encode('utf-8', errors='replace')).hexdigest())
    atomic_write_json(args.output, project(json.loads(args.candidate.read_text()), reference))


if __name__ == '__main__':
    main()
