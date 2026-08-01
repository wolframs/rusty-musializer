# Lyrics timing investigation

Date: 2026-08-01

## Outcome

Automatic lyrics assistance now has an independent acoustic refinement stage.
Whisper still supplies transcription and rough ordering, and embedded metadata
still supplies authoritative display text, but neither one's segment boundaries
are treated as final cue timing. `tools/force_align_lyrics.py` teacher-forces the
decided cue text through the CUDA-backed TorchAudio MMS forced-alignment model and
records line- and word-level evidence in `lyrics.aligned.json`.

The acceptance set is:

| Track | Reference path | No-reference path |
| --- | --- | --- |
| `Fuck, We Are.mp3` (106.52 s) | 23 embedded-metadata cues | 28 reviewed cues; one false duplicate removed |
| `Floor Mechanics I_ Load Bearing.mp3` (189.92 s) | 65 embedded-metadata cues | 55 reviewed cues; one false duplicate removed |

The no-reference fixtures are audio-packet copies with all metadata removed:
`build/lyrics-investigation/fixtures/fuck-we-are-stripped.mp3` and
`build/lyrics-investigation/fixtures/floor-mechanics-stripped.mp3`. Their decoded
PCM hashes are byte-identical to their respective originals:

- Fuck: `aab7ee4e...d3364f`
- Floor: `fcaab6d0...7627631`

The four v13 Assist runs all produced an ordered native bridge and parse through
the Rust application boundary. Full evidence remains under the gitignored
`build/lyrics-investigation/runs/` tree.

## Why timing failed

This was not one bug. Five independent failures compounded:

1. `import_whisper.py` did not recognize whisper.cpp timestamp controls such as
   `[_TT_700]`. The control was glued to the preceding BPE word, making that word
   inherit the coarse segment end. A real opening word consequently spanned more
   than 13 seconds.
2. The helper requested DTW while whisper.cpp's default flash attention was on.
   whisper.cpp accepts that combination but disables DTW, so provenance claimed a
   timing backend that had not run. With `--no-flash-attn`, real DTW did run, but
   repeated sung material produced much worse hallucination loops and timestamp
   collapse. DTW is therefore explicit diagnostic behavior, not the default.
3. Ordinary Whisper token onsets were useful, but token ends could stretch to the
   containing segment boundary. Valid onsets are now retained and heuristic tails
   are capped at 0.50 seconds.
4. Global reference matching used the minimum and maximum matching token. One
   remote repeated word could therefore create a 20-second cue. Matching now
   selects a coherent local cluster and can recover unused nearby boundary words.
5. Missing reference lines were interpolated into gaps too small to contain them,
   then minimum-duration repair made the cues overlap. Interpolation now refuses a
   gap that cannot hold the whole run.

Even after those repairs, Whisper's first token was often early by 0.5-1.1 seconds
and occasionally by 3-5 seconds. That is why parser repair alone was not enough.

## Forced-alignment policy

MMS_FA runs one acoustic request per cue. A batching negative control showed that
long, repeated lines could select a different occurrence depending on neighboring
cues in the same CTC request. Each request searches only 0.75 seconds backward but
6 seconds forward, reflecting the measured direction of Whisper's singing error
while preventing a preceding repeated phrase from stealing the alignment.

The output policy is intentionally conservative:

- Strong acoustic evidence replaces provisional boundaries.
- A broad input cue may be narrowed to an acoustic span inside it even when the
  first sung word is weak.
- Implausibly large boundary moves need support from the relevant boundary word.
- Weak decisions retain the input timing and are marked uncertain; a low CTC
  score alone never deletes sung text.
- A no-reference multi-word cue is removed only when Whisper confidence and MMS
  evidence are both weak, or when two identical candidates claim the same
  acoustic span. Single-word exclamations are retained.
- Metadata display text and ordering never come from the acoustic model.

`external_analysis.py assist --mode lyrics|all` makes this stage mandatory. The
runtime is discovered through `MUSIALIZER_ALIGN_PYTHON`, `--align-python`, or
`~/.local/share/musializer/lyrics-align/.venv/bin/python`. The doctor and support
bundle check treat the helper, CUDA runtime, and MMS model as required local-lyrics
assets. Cache provenance includes model, policy version, settings hash, source
hash, and audio hash.

## Acceptance evidence

### Text and path agreement

After removal of the two false duplicates, global exact-token alignment gives:

| Track | Metadata tokens | Stripped tokens | Exact matches | Metadata recall | Stripped precision |
| --- | ---: | ---: | ---: | ---: | ---: |
| Fuck | 128 | 128 | 125 | 97.66% | 97.66% |
| Floor | 351 | 349 | 346 | 98.58% | 99.14% |

The differences are transcription spellings or segmentation, not missing song
regions. The metadata and stripped Floor paths contain 329 identically aligned
MMS words. For all 319 matches whose score is at least 0.15 on both paths, every
start and end agrees within 80 ms; median is 0 ms and P95 is under 5 ms. This
compares the same word independent of cue segmentation.

Fuck contains deliberately repeated `be/come`, `I'm here`, and `we are` phrases,
so a word-only sequence comparison has ambiguous occurrences. The rendered cue
envelopes resolve them: the metadata compound cue and stripped component cues
agree on the beginning/end of each passage, and remote blind transcription
confirms their order. The broad fallback cues are explicitly uncertain rather
than being replaced by a neighboring repeated phrase.

### Independent full-mix versus vocal-stem control

Demucs `htdemucs` vocal stems were analyzed with the same v13 MMS policy; they
are verification artifacts, not a production dependency. For non-repeated
material the full mix and vocal stem normally agree to 0-20 ms. On Floor, 628 of
636 scored metadata word boundaries and 618 of 630 scored stripped word
boundaries are within 0.5 seconds; almost all are within 0.1 seconds. The outliers
are two explicitly repeated passages:

- `and the body is fine` versus the following `the body has never been more
  fine`; both production paths independently choose the earlier authored line.
- three consecutive `For now` phrases; the metadata path places `kick` between
  the second and the `part that doesn't have a name`, confirming the full-mix
  stripped selection for the latter phrase.

For Fuck, the outliers are likewise the repeated `be/come`, `I'm not yielding`,
`I'm here`, and `we are` phrases. Metadata/stripped passage envelopes and the
remote order/count checks adjudicate them. Weak per-cue CTC attempts that chose a
neighbor are rejected and do not become output timing.

### Remote audio checks

OpenRouter audio models were used only for phrase presence, order, and count.
Their blind absolute timestamps compressed musical time severely and are not a
timing oracle. Candidate-timestamp prompts were also prone to anchoring.

Two providers independently found:

- Fuck has exactly one `we are` before `coupled`; the stripped provisional
  `But we are` cue was false.
- Floor says `forgetting themselves` once; the two stripped candidates both
  snapped to the same acoustic span and the lower-confidence one was false.

Three blind GPT Audio section transcripts cover the complete authored Fuck vocal
sequence. Three analogous Floor sections cover its body. Earlier adversarial
tracks additionally established that missing authored lines and hallucinated
outros must be omitted rather than forced into a metadata lane.

Reported OpenRouter spend for all investigation calls: **USD 0.3281545**, well
below the authorized USD 3.50 ceiling. The API key never appears in an artifact,
command output, or tracked file.

## Negative controls

| Perturbation/control | What failed | Meaning |
| --- | --- | --- |
| Restore old special-token regex | Parser test returns `me[_TT_700]` instead of `me` | The regression pins the actual timestamp-token bug. |
| Increase coherent-cluster gap 5 s to 50 s | Remote-word cluster test fails | The test detects the cue-stretch failure. |
| Request real DTW with flash attention disabled | Singing transcription explodes into repeated one-second hallucinations | DTW's apparent benefit was false provenance, not a usable backend. |
| Batch multiple cues in one CTC request | Repeated words change occurrence with batching context | Per-cue acoustic windows are required. |
| Drop every low-scoring review cue (v7) | Seven real Fuck lines disappear along with one false line | CTC score alone is not text-presence evidence. |
| Symmetric 3 s CTC window (v8) | Earlier repeated words steal cue starts | Search windows must reflect predominantly early Whisper onsets. |
| Keep both high-scoring identical Floor candidates (v10) | Both map to one span 2 ms apart and the bridge becomes out of order | Acoustic duplicate collapse detects a confident Whisper hallucination. |

## Research conclusions

- WhisperX-style ASR plus a separate phoneme/CTC aligner remains the practical
  architecture for word timestamps; the local MMS_FA stage follows that split.
- Whisper attention contains alignment information, but the current singing
  evidence shows it is not a sufficient final oracle for this product.
- Demucs is useful as an independent accompaniment negative control, not as a
  mandatory user-facing stage.
- General audio-language models are useful semantic auditors. Their timing and
  lyric transcription must be checked rather than trusted, especially for dense
  production and repetition.
- Recent music-oriented systems such as SAM Audio, STARS, and VocalParse are
  relevant future comparisons, but none currently replaces a deterministic,
  locally auditable forced-alignment lane here.

## Reproduction and safety

- Source MP3s under `/home/wolfram/Music` were never modified.
- Analysis commands decode audio but never initialize playback.
- Any future application run must use `--mute`; UI automation must use the
  private-Xvfb headless path.
- Private fixtures, stems, raw model responses, and acceptance artifacts remain
  under gitignored `build/lyrics-investigation/`.
- The tracked regression suite is `tests/test_lyrics_timing.py`; the normal gate
  is `tools/support_bundle_check.sh`, followed by `tools/verify.sh`.
