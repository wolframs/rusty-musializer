# Optional analysis adapters

These Python 3 helpers run outside Musializer's Rust renderer. Remote analysis
reads `OPENROUTER_API_KEY` from the process environment. The explicit
`external_analysis.py assist --mode mimo|all` desktop workflow may instead read
that one named value from the repository's ignored `.env`; it parses the file
as data and never sources it as shell code. No other dotenv values are copied
to child processes.

## Whisper timing import

```console
python3 tools/import_whisper.py whisper.json track.mp3 lyrics.json \
  --duration 213.7 --model medium.en \
  --corrected-lines corrected-lines.json
```

The importer accepts common Whisper `segments[].words[]`, root-level `words`,
Remotion whisper.cpp `transcription[].offsets` with nested BPE tokens, and
Remotion caption arrays using `startMs`/`endMs`. Nested tokens are aggregated
into words using whisper.cpp's leading-space boundaries. Supplied corrected
caption arrays replace guessed segment text but do not erase independently
timed word evidence. Timestamps and confidences are clamped; empty intervals
outside the audio duration are discarded.

## One-shot external analysis orchestration

`external_analysis.py` is the noninteractive entry point intended for both the
desktop UI and terminal use. One `assist` invocation corresponds to one UI
action and always writes `scene-plan.json`, `assist-manifest.json`, and the
validated `analysis.bridge.tsv` when it succeeds:

The current UI integration is available from a source checkout, including the
per-user Linux launcher. Portable archive packaging is tracked separately in
`FEATURE_PARITY_PLAN.md`; any distribution that omits `tools/`, `prompts/`, or
`schemas/` reports the helpers as unavailable instead of showing controls that
cannot run. The Python interpreter and optional analysis programs/models remain
external dependencies.

Run `python3 tools/musializer_doctor.py` for a human-readable preflight, or add
`--json` for automation. `--require local_lyrics` gates FFmpeg, Python/NumPy,
Whisper, Codex, the writable cache directory, and the relevant assets;
`--require remote_mimo` gates the local measured-analysis prerequisites plus
OpenRouter credential presence. The doctor performs discovery only: it does not
run a model or network request and never emits credential values.

```console
python3 tools/external_analysis.py assist track.mp3 analysis/ \
  --duration 213.7 --mode lyrics
python3 tools/external_analysis.py assist track.mp3 analysis/ \
  --duration 213.7 --mode sections
python3 tools/external_analysis.py assist track.mp3 analysis/ \
  --duration 213.7 --mode mimo --zdr
python3 tools/external_analysis.py assist track.mp3 analysis/ \
  --duration 213.7 --mode all --bridge analysis/track.bridge.tsv
```

Modes have deliberately narrow authority:

- `lyrics` runs/reuses measured analysis and configured whisper.cpp, then
  chooses one of two lyric paths. If authored reference lyrics are found —
  an explicit `--lyrics-file`, a sibling `<stem>.lyrics.txt`, or an
  unsynchronized lyric tag embedded in the audio container (read locally via
  ffprobe) — the manifest records the exact chosen source, alignable-line count
  and reference hash so an embedded fallback cannot masquerade as user-supplied
  text. The deterministic `tools/lyric_align.py` stage synchronizes the
  authored lines against Whisper word timing and writes the `lyric_sync` lane
  to `lyrics.sync.json`; no model request is involved. Display text comes
  verbatim from the reference; section headings, sound events, and delivery
  instructions are classified out; repetition-loop hallucinations in the
  evidence are excluded by a repeats-plus-duration detector; unmatched lines
  are interpolated only across short trusted gaps (flagged `estimated` and
  `uncertain`) and otherwise reported in `unmatched`. Without a reference,
  the evidence-preserving Codex review runs as before. The manifest records
  `lyric_source`, reference identity and unmatched counts; the job log names
  the sync source.
- `sections` is entirely local and uses measured audio analysis plus any
  independently valid cached local lyric sync or lyric review. It never
  consumes a cached MiMo semantic lane.
- `mimo` runs/reuses measured analysis and the existing MiMo/OpenRouter helper.
  This explicit command is the authorization boundary for the remote request.
- `all` performs both lyric and MiMo work, then plans sections.

All stages are hash-checked and cache-aware at their own provenance boundary.
Measured caches include the analyzer version and analysis configuration;
Whisper caches include the adapter version, model file hash, language, timing
model, and measured duration; lyric sync caches include the Whisper evidence
hash, the reference text hash, and the aligner version; Codex reviews include
the Whisper source hash, selected model, and repository prompt version/hash;
forced-alignment caches include the selected sync/review lane hash, MMS model
identity, alignment algorithm version, and timing policy;
MiMo caches include its model, prompt, output schema, audio metadata, routing,
fallback, and ZDR request settings. A mismatch regenerates that stage and its
downstream products while leaving still-valid upstream evidence reusable.
Whisper is configured with `MUSIALIZER_WHISPER_BIN` and
`MUSIALIZER_WHISPER_MODEL` or the corresponding flags. Discovery otherwise
checks `~/.local/share/musializer/whisper.cpp` (the durable per-user install;
CUDA-enabled on this workstation) and then the legacy tmpfs setup at
`/tmp/music-visualizations-whisper-1.8.6`, taking `build/bin/whisper-cli`
from the first install that has one. Models are ranked
`ggml-large-v3.bin`, then `ggml-large-v3-turbo.bin`,
`ggml-large-v3-q5_0.bin`, and `ggml-medium.en.bin`, with the best model in
any install outranking a lesser model in a preferred install.
The full model now leads because the 2026-08-17 Groyper regression recovered
17/18 authored lines in its coarse pass and 16/18 after conservative acoustic
gating, versus turbo's 14/18 and 10/18. Turbo remains the fallback for an
installation without the full model and is substantially cheaper on CPU.
Whisper receives a temporary FFmpeg-decoded 16 kHz mono WAV, requests full
JSON, runs with one worker thread per host CPU, keeps GPU/flash attention
enabled, and defaults to a one-hour timeout. Its token onsets are retained as
provisional text/order evidence, with obviously stretched interval ends capped.
Short Whisper spans outside every authored placement are retained separately
as performed-vocal candidates. They enter the editor as non-rendering
`Potential` cues for human promotion; they never rewrite authored text or
silently become accepted captions. Long tail segments and known repetition
loops remain excluded as likely hallucination evidence.
They are **not published as final cue boundaries**: real produced-song audits
found first-word onsets commonly 0.5-1.1 seconds early and occasionally 3-5
seconds early. After authored sync, the anchor/block MMS_FA CTC lane compares a
trusted section-bounded path, a coarse-local boundary refinement, and — for a
section without its own rare exact anchor — an unconditioned global challenger.
Estimated and low-confidence coarse rows never narrow a search window, and two
coarse-conditioned paths cannot validate one another. The lane writes
`lyrics.aligned.json`, including the competing acoustic evidence. It never
changes authored wording or order; an occurrence-scale or global-localization
dispute, or a path whose authored order remains backwards, is `unresolved` and
cannot become a displayable bridge cue. Smaller boundary disagreements remain
uncertain review flags. The no-authored-text review lane continues to use
per-cue MMS refinement. A no-reference phrase is omitted only when weak Whisper
and MMS evidence agree, or
when two identical candidates claim the same acoustic span. Full-mix and
Demucs-vocal-stem controls normally agreed within 0.02 seconds; the recorded
outliers were repeated phrases and were adjudicated against the independent
metadata/no-metadata path rather than averaged away. Configure its Python with
`MUSIALIZER_ALIGN_PYTHON` or install it at
`~/.local/share/musializer/lyrics-align/.venv/bin/python`; the model uses the
normal Torch cache. The lower-level `whisper --dtw-model` option remains
diagnostic-only: it disables flash attention as whisper.cpp requires and
imports its centisecond `t_dtw` moments, but real repeated-song tests made that
decoding path hallucinate loops and collapse many tokens onto the same moment.
The umbrella timeout is at least ten minutes and defaults to 40 minutes so the
local alignment and MiMo's bounded retries can finish.

`--dry-run` performs no child process or network call and emits a credential-
free action description. Child processes are argv arrays without a shell.
Private lyric/audio content is passed by file or stdin, not argv; captured child
output is never copied into error logs. Local FFmpeg, Whisper, measured-analysis,
and Codex children receive an environment with credential-like variables
removed. The OpenRouter helper receives only `OPENROUTER_API_KEY`, inherited
from the environment or parsed as the one permitted key from the ignored
repository `.env`; it is never written to an argument, request dump, cache key,
or manifest.

The lower-level commands remain available for diagnosis and custom workflows:

```console
python3 tools/external_analysis.py whisper track.mp3 lyrics.json \
  --duration 213.7 --whisper-bin /path/to/whisper-cli \
  --model /path/to/ggml-large-v3.bin
python3 tools/external_analysis.py sync-lyrics lyrics.json reference.txt \
  lyrics.sync.json
python3 tools/external_analysis.py clean-lyrics lyrics.json lyrics.review.json
python3 tools/external_analysis.py plan measured.json scene-plan.json \
  --lyrics lyrics.sync.json --semantic semantic.cache.json \
  --bridge analysis.bridge.tsv
```

Codex runs ephemerally in a read-only sandbox with a ten-minute default timeout,
structured output, and the repository-owned
`prompts/lyrics_cleanup_system.md` (contract v2). Every reviewed line must cite
Whisper line indices chronologically and stay within their timing envelope; a
long source segment may be split across several short display cues, each
bounded at 200 characters and 15 seconds by the local validator with far
tighter targets in the prompt. The request annotates detected repetition-loop
hallucination intervals so the model omits them deliberately, the model must
account for every source line to the end of the track, and the persisted
review records a coverage block (cited/uncited-reliable counts and flagged
intervals). After validation a deterministic splitter
(`lyric_align.split_long_cues`) reduces any remaining oversized cue at
sentence/clause boundaries, snapping piece timing to evidence word gaps. The
review is a separate `lyric_review` lane; it never overwrites Whisper evidence
and is rejected if it adds uncited lines. An evidence-preserving review may
legitimately retain zero lines; the desktop reports that as a completed result
with no editor changes and does not offer Apply.

The output schema forwarded to `codex exec --output-schema` must stay inside
the structured-output keyword subset; `uniqueItems` in particular is rejected
by the endpoint with `invalid_json_schema` (this silently failed every lyric
review until 2026-07-16). Uniqueness and all other stricter bounds are
enforced locally by the review validator. When the Codex child exits
abnormally, a bounded tail of its output is preserved as
`lyrics.review.diagnostic.log` beside the other per-job artifacts (the same
directory that already holds the private Whisper evidence); the summary job
log stays free of child output. The diagnostic is removed again by the next
successful review. Real child stdout and stderr are continuously drained into
bounded in-memory tails while the process runs, so the bound applies before
the diagnostic is written rather than only afterward. POSIX child commands run
in private process groups that are cleaned after timeout or direct-child exit;
the Windows desktop worker retains its Job Object containment. Every successful Assist
job writes privacy-safe lyric/section/semantic counts to its UI-accessible job
log, and `assist-manifest.json` records the same `result_counts`.

The deterministic section planner combines measured section boundaries,
measured feature changes, lyric gaps, and (when explicitly supplied) subjective
semantic changes. The `assist sections` mode may supply measured evidence and
an independently valid cached local lyric review, but never semantic evidence;
`assist mimo`, `assist all`, or a lower-level `plan --semantic` invocation can
supply semantic evidence. Each recommendation records lane-specific reasons.
MiMo remains a creative signal and never becomes measured timing or
authoritative lyrics.

### Importing an existing MiMo chat export

The user's earlier OpenRouter Chat export can be reduced to a safe subjective
notes lane without copying its embedded input audio or reasoning trace:

```console
python3 tools/external_analysis.py import-mimo openrouter-chat.json track.mp3 \
  semantic-notes.json --duration 213.7
```

Only assistant `output_text` is retained. Because free-form legacy output has no
validated timestamps or numeric scores, it stays `semantic_interpretation_notes`;
the planner may use its words as subjective scene hints but does not fabricate
energy, confidence, or segment timing.

### Bounded application bridge format

Canonical and provenance-rich artifacts remain JSON. The bridge is a derived,
ASCII-only TSV so the application can parse it with fixed bounds at its Rust
boundary. It is regenerated from canonical inputs and is never an evidence
source.

The exact v1 grammar is one record per LF-terminated line, with no quoting:

```text
MUSIALIZER_BRIDGE<TAB>1
AUDIO<TAB>audio_sha256<TAB>duration_ms
LYRIC<TAB>uint64_id<TAB>start_ms<TAB>end_ms<TAB>confidence_milli_or_-1<TAB>none|uncertain<TAB>text_utf8_base64
SECTION<TAB>uint64_id<TAB>start_ms<TAB>end_ms<TAB>scene_name<TAB>strength_milli<TAB>reasons_json_utf8_base64
SEMANTIC<TAB>uint64_id<TAB>start_ms<TAB>end_ms<TAB>energy_milli<TAB>tension_milli<TAB>valence_milli<TAB>confidence_milli<TAB>summary_utf8_base64
SEMANTIC_NOTE<TAB>uint64_id<TAB>text_utf8_base64
```

Times are rounded integer milliseconds. Unit values are integer thousandths;
valence retains its signed `[-1000,1000]` range. IDs are stable nonzero 64-bit
values derived from record identity. Text and JSON use RFC 4648 base64, so tabs,
newlines, and arbitrary UTF-8 never alter the record shape. Consumers must
reject an unknown header/version, wrong field count, invalid integer/base64,
out-of-order or out-of-range timing, unknown scenes, oversized decoded fields,
and duplicate IDs before replacing the last valid bridge.

## MiMo semantic interpretation

Inspect the exact request shape without a key or network access:

```console
python3 tools/mimo_openrouter.py track.mp3 --duration 213.7 --dry-run \
  --request-dump request.redacted.json --zdr
```

Submit an explicitly authorized analysis and atomically write its cache:

```console
OPENROUTER_API_KEY=... python3 tools/mimo_openrouter.py \
  track.mp3 track.semantic-cache.json --duration 213.7 --zdr
```

`--provider NAME` may be repeated to set provider order. `--no-fallbacks`
pins routing to that eligible set. `--zdr` asks OpenRouter to restrict routing
to Zero Data Retention endpoints, which can reduce provider availability.

The MiMo cache is one atomic envelope containing:

- the deterministic cache key and credential-free request settings;
- the raw OpenRouter response for audit/re-normalization;
- the validated `musializer.semantic-score/v1` document consumed by projects.

No cache replace occurs until the HTTP response, completion JSON, timing, and
creative fields have all validated. The cache key covers audio SHA-256, exact
model, prompt version, response schema, audio format, and routing/privacy
settings. Prompt/schema hashes and the measured duration ensure a version bump
cannot be accidentally skipped without changing the key. It intentionally does
not contain the credential or base64 audio.

Semantic segments must form a contiguous, ordered, non-overlapping partition
of the complete audio duration. The prompt and request schema ask for this,
the normalizer enforces it with a one-millisecond boundary tolerance, and the
persisted schema records the cross-item rule as
`x-musializer-coverage: contiguous-full-duration`. Incomplete model timelines
are rejected rather than silently promoted to a complete creative score.

## Google Fonts caption faces

`tools/google_fonts.py` is not an analysis adapter -- it produces no lane and
touches no project -- but it is the other optional network capability, so it
follows the same rules and is documented beside them.

```console
python3 tools/google_fonts.py --dry-run fetch "Space Mono" /tmp/out
python3 tools/google_fonts.py catalogue build/fonts/catalogue.json \
    --index build/fonts/catalogue.tsv
python3 tools/google_fonts.py fetch "Space Mono" build/fonts/job
```

`catalogue` fetches the family list, reduces it to what a picker shows, caches
the JSON, and optionally writes a bounded TSV index for the application.
Families with no Latin subset are dropped: the caption atlas could not draw
them, so offering one would download a face that renders empty boxes.

`fetch` resolves a family to its regular-weight TrueType file, downloads it,
retrieves the licence it is distributed under, and writes both plus a manifest.
The stylesheet endpoint is requested without advertising woff2 support, which
is what makes it answer with a `.ttf`: raylib has no woff2 decompressor, so a
woff2 URL would download perfectly and then fail to load.

Four hosts are permitted -- `fonts.google.com`, `fonts.googleapis.com`,
`fonts.gstatic.com`, `raw.githubusercontent.com` -- and the list is enforced
before each request and again against the response URL. Payloads are bounded,
and the downloaded file must carry an sfnt magic number before it is written,
so a captive portal's login page cannot land on disk named `.ttf`.

A face whose licence cannot be retrieved is refused rather than downloaded
without it: the application copies the face into a project bundle that gets
shared, which is redistribution.

The manifest is described by `schemas/font-import-v1.schema.json` and mirrored
as one TSV row in `import.tsv`, which is what the application reads. Its
digests are claims: the application re-hashes both files itself before either
is used. `--dry-run` prints the hosts a real run would contact and opens no
connection.

## Schemas and tests

JSON Schemas live in `schemas/`. Run the dependency-free offline suite with:

```console
python3 -m unittest discover -s tests/adapters -v
```

All HTTP behavior in the suite uses injected mock transports; tests never call
OpenRouter or Google Fonts.
