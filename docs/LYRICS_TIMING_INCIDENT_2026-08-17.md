# Lyrics timing incident: Groyper Idol (2026-08-17)

This is the durable forensic record for the failure reported against
`/home/wolfram/Music/Groyper Idol.mp3`. The source audio stays outside the
repository. Reproducible local artifacts live under gitignored
`build/groyper-idol/`.

## What actually failed

Two independent failures landed on top of each other.

First, `18 / 1024` was not truncation. `1024` is the editor's cue capacity.
The MP3 embeds a `lyrics-eng` tag containing a different lyric sheet from the
Suno prompt supplied with the report. With no explicit sheet and no sibling
`Groyper Idol.lyrics.txt`, the documented discovery order selected that tag.
All 18 alignable embedded lines reached the TSV; ten bracketed headings and
production directions were intentionally classified as non-lyric structure.
Whisper also heard the embedded variant's opening wording, so the prompt text
cannot be reconstructed from this run's inputs. Authored-source identity is a
product trust boundary, not an ASR problem.

Second, the 18 timings were not safe. The rare anchors ended in the opening
chorus. The remaining 212-second terminal span was divided by lyric token
count, and the first five-line CTC request searched 35.36–112.40 seconds.
Although the independent coarse lane found the first chorus at 37.3–73.7
seconds, the block path forced it to 79.7–90.8 seconds. Ten of 18 cues were
flagged, with disagreement up to 44.6 seconds, but a flag still produced a
normal displayable cue and did not prevent Apply. Coverage had become a proxy
for "CTC emitted some span", not accepted-cue precision.

## The production repair: localization policy v2

Policy v2 keeps authored display text immutable and changes localization in
four ways:

1. Section headings are retained as grouping evidence. When at least two
   sections have coarse proposals, their proposal centres bound overlapping
   section-sized CTC windows. A sparse intro anchor can no longer make a chorus
   search most of the remaining song.
2. A high-confidence coarse occurrence gets a second, tightly bounded one-line
   CTC refinement only when the independently searched section/block result
   agrees on the occurrence. The local result cannot validate its own
   coarse-selected window, even when the lexical match is exact. This recovers
   useful word boundaries without turning a forced local path into circular
   confirmation.
3. A coarse-derived section path is still only a candidate generator. A
   section with no rare exact anchor must also agree with the original
   unconditioned anchor/global search; disagreement parks the line with both
   proposals. Estimated and sub-0.8 coarse rows never narrow a section window.
4. Fine/coarse onset disagreement above eight seconds is an occurrence dispute.
   It becomes `unresolved` with both proposals retained for audit; it is never
   serialized as a displayable cue. Backwards authored order is handled by the
   same abstention rule, not hidden by sorting the bridge.
5. `assist-manifest.json` carries the exact reference source, SHA-256 and
   alignable-line count. The staged-result surface prints that identity, and an
   embedded source requires a second button labelled `Confirm embedded`.

The eight-second hard boundary is intentionally distinct from the existing
three-second review flag. A few seconds can be a boundary disagreement around a
long sung phrase; tens of seconds identify a different occurrence. It is
versioned in cache provenance and has a negative control: a +40-second unique
line must become unresolved even with a high CTC score.

## Measured incident replay

Both passes below reused the failed job's Whisper evidence and opened no audio
output device.

| result | accepted cues | unresolved | review flags | order violations |
| --- | ---: | ---: | ---: | ---: |
| shipped v1, embedded sheet | 18/18 | 0 | 10 | 0 |
| v2, embedded sheet | 10/18 | 8 | 13 | 0 |
| v2, supplied Suno prompt | 7/19 | 12 | 16 | 0 |

The embedded-sheet first chorus now starts at 39.43, 46.74, 53.26 and 70.40
seconds instead of 79.73, 83.83, 86.53 and 88.92. Its independent coarse starts
are 37.30, 45.88, 51.28 and 66.34, and the independently searched block starts
now agree at 39.55, 46.84, 53.38 and 71.19. Eight embedded-sheet lines are
parked where the coarse-conditioned path and unconditioned global path do not
independently agree. The four accepted chorus cues remain review-flagged because
their rare section anchor overruled the known-bad unconditioned path; that
conflict is preserved rather than hidden. The estimated breakdown proposal is
review evidence but is forbidden from narrowing its own section window.

The supplied prompt is a deliberately valuable adversarial run: the audio and
embedded sheet acoustically agree on different wording, so v2 abstains on
twelve of its 19 lyric lines rather than manufacture confident-looking
placements.
In particular, its apparently exact coarse matches for two mismatched lines at
146.08 and 165.94 seconds are rejected because their independent block paths
land at 99.10 and 107.13 seconds; exact lexical confidence is not occurrence
truth.
This is safer evidence, not proof that the remaining seven prompt lines were
performed verbatim. Exact boundary claims still require operator listening.

## External-project and literature review

None of the five reported projects safely solves omission plus repetition:

| project | useful clue | unsafe edge for this incident |
| --- | --- | --- |
| [FIZX](https://github.com/AmMoPy/FIZX) | optional Spleeter vocal stem plus wav2vec2 | whole-sequence forcing, no skip/abstention; its aeneas option also brings AGPL constraints |
| [mikezzb/lyrics-sync](https://github.com/mikezzb/lyrics-sync) | Demucs plus a singing-adaptable CTC trellis | every reference token must be placed; no unresolved state |
| [oneclick-subtitles-generator](https://github.com/nganlinh4/oneclick-subtitles-generator) | pluggable Faster-Whisper and Qwen ASR lanes | transcribes; it does not globally align authored repeated lyrics |
| [sparksthedragon0101/Lyricsync](https://github.com/sparksthedragon0101/Lyricsync) | WhisperX, optional Demucs and VAD retry | greedy projection and interpolation manufacture timing stubs; no usable licence grant |
| [LyricVision](https://github.com/KiwiSingh/LyricVision) | simple WhisperX integration | replaces the transcript with a whole-song authored segment, the same long-force shape that drifts here |

The design direction supported by the
[low-resource two-pass paper](https://arxiv.org/abs/2102.09202) and
[CTC Segmentation](https://github.com/lumaku/ctc-segmentation) is rare-anchor
spotting, globally ordered partial alignment, then bounded refinement. The
longer-term ceiling is a skip-capable candidate DAG over independent raw-mix and
vocal-stem ASR lanes, with runner-up margins and calibrated abstention. Demucs
and Qwen3-ASR are worthwhile experimental evidence lanes, not production truth
until the local adjudication set shows that they reduce false-certain cues.
Whole-song Qwen3-ForcedAligner remains retired: its official scope is speech and
the repository's four-track benchmark measured 47–76% coverage on the longer
songs with large drift.

## Acceptance metric corrected

Future lyric-timing work reports all of these, not coverage alone:

- authored-source identity and conflict state;
- accepted-cue precision and false-certain count;
- wrong-occurrence count (target: zero);
- onset/offset median, p90 and maximum error;
- abstention recall on deleted, duplicated and paraphrased sections;
- authored accounting: accepted plus unresolved equals every alignable line.

The desired failure is explicit uncertainty. A line that never became a cue is
repairable; a polished caption attached to the wrong chorus is not.
