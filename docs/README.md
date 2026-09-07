# Rusty Musializer documentation

This directory is the front door for implementation and behavior documentation.
The root [`README`](../README.md) is the short user-facing overview; the
documents here explain enough of the system that a maintainer can find the
authoritative code and evidence without reconstructing the project from scratch.

**The code is the source of truth about what is built.** These documents exist
for the things code cannot say: a contract that spans files, a schema somebody
else's tool must satisfy, a trap that has already cost a session, and why a
decision went the way it did. When a document and the tree disagree about a
*feature*, the tree wins. When they disagree about a *bound, a default, a key or
a grammar*, that is a defect until somebody changes one of them on purpose.

## Start here

- [`CODE_ARCHITECTURE.md`](CODE_ARCHITECTURE.md) — crate boundaries, state
  ownership, the shared preview/export frame path, persistence, and what each
  correctness layer is for. Boundaries and rules, not a feature inventory.
- [`CODE_MAP.md`](CODE_MAP.md) — the **generated** inventory of Cargo targets,
  Rust modules, verification tools, tests and schemas. Regenerate with
  `python3 tools/code_map.py`; never hand-edit it. `tools/verify.sh` checks it
  first and fails on a stale map.
- [`PHASE0_INVENTORY.md`](PHASE0_INVENTORY.md) — **live contract tables**: the
  CLI grammar and its order effects, every scene settings descriptor with its
  bounds and defaults, the `.musi` and ancillary JSON schemas, and the
  environment overrides. `AGENTS.md` names these among the surfaces that may
  change only deliberately. The name is rewrite-era and the file is not; its
  section numbers are cited from source comments and must stay stable.
- [`ASSIST_PIPELINE.md`](ASSIST_PIPELINE.md) — an Assist request from the UI
  through the Python evidence tools, validation, staging and project
  application, plus the lyrics decision tree and its trust boundaries.
- [`ASSIST_PROVIDER_CONTRACTS.md`](ASSIST_PROVIDER_CONTRACTS.md) — the design
  authority for Assist provider configuration: task contracts and their boundary
  ladder, the credentials lifecycle, the secret-exposure inventory, fallback
  rules, and the execution-snapshot schema every job records.

## Operations and evidence

- [`LYRICS_GROUPING_REVIEW.md`](LYRICS_GROUPING_REVIEW.md) — two pending lyric
  phrase judgments, browser audition, portable protocol bundle and resume state.
- [`LYRICS_ASSIST_REPAIR_2026-09-06.md`](LYRICS_ASSIST_REPAIR_2026-09-06.md) —
  ten-track lyrics investigation, production repairs and unadjudicated evidence.

- [`../tools/listening-lab/README.md`](../tools/listening-lab/README.md) — the
  local browser A/B listening workspace: agent-authored audio protocols, precise
  waveform playback, blinding, and append-only feedback logs.
- [`../tools/ANALYSIS_ADAPTERS.md`](../tools/ANALYSIS_ADAPTERS.md) — the operator
  guide for analysis dependencies, discovery, privacy boundaries and commands.
- [`../tools/MEASURED_ANALYSIS.md`](../tools/MEASURED_ANALYSIS.md) — the
  deterministic measured-audio artifacts consumed by higher-level analysis.
- [`MIMO_BENCHMARK_PLAN.md`](MIMO_BENCHMARK_PLAN.md) — **designed, built, and
  not yet run**: the benchmark of `xiaomi/mimo-v2.5`'s musical description, with
  its chunking / register / output-shaping axes, scoring rubric, decision gates
  and cost projection. Running it spends credits and sends audio off the
  machine, and needs an explicit operator go-ahead.

### Lyrics timing

Four dated records and one design proposal, kept because the policy they
produced is deliberately conservative in ways nobody would guess from the code
alone. Read them in this order if you are changing localization:

- [`LYRICS_TIMING_INCIDENT_2026-08-17.md`](LYRICS_TIMING_INCIDENT_2026-08-17.md)
  — **read this one first.** The Groyper Idol wrong-source and wrong-occurrence
  failure, the replay measurements, the external-project review, and
  localization policy v2's evidence and abstention rules. It is also where the
  acceptance metric moved from coverage to accepted-cue precision, which
  supersedes the gate the benchmark below used.
- [`LYRICS_TIMING_INVESTIGATION.md`](LYRICS_TIMING_INVESTIGATION.md) — the
  2026-08-01 repair: five compounding failures, the acceptance tracks, the
  forced-alignment policy, and seven negative controls with what each caught.
- [`LYRICS_TIMING_BENCHMARK_RESULTS.md`](LYRICS_TIMING_BENCHMARK_RESULTS.md) —
  the 2026-08-04 four-track scoreboard that selected anchor→block MMS, the
  operator's listen-check adjudication, and the finding that the aligner's own
  score does not separate right from wrong. Carries a superseded-acceptance
  header pointing at the incident.
- [`LYRICS_TIMING_WEB_EVIDENCE.md`](LYRICS_TIMING_WEB_EVIDENCE.md) — the
  published and community evidence that answered the experiment matrix's cut
  questions, so nobody re-benches them on local GPU time.
- [`LYRICS_TIMING_RESEARCH_PLAN.md`](LYRICS_TIMING_RESEARCH_PLAN.md) — the
  2026-08-04 design proposal. Its status header says which parts shipped as LT1
  and which were superseded; it is not a queue.

## Live queues and repository rules

- [`../FEATURE_PARITY_PLAN.md`](../FEATURE_PARITY_PLAN.md) is the **only** live
  completion queue. Do not grow a second backlog inside documentation.
- [`../AGENTS.md`](../AGENTS.md) carries the engineering constraints: the
  `unsafe` inventory, the test-silence requirements, the GPU headless-gate path,
  the traps this rewrite has already paid for, and the negative-control rules.
  It is authoritative wherever it and a document here disagree.

## Historical material

`archive/` holds closed records. They are kept for their evidence and their
method, not as instructions, and each carries its own archived-on header.

- [`archive/C_PORT_HISTORY.md`](archive/C_PORT_HISTORY.md) — the boundary around
  the earlier C implementation: port-era annotations, `tests/differential/`,
  `tools/differential_*.sh` and `REWRITE_PLAN.md` are migration evidence, not a
  behavioral authority or a completion gate. Two clarifications it predates: the
  differential harnesses remain **live regression anchors** (diverging from one
  is a decision that updates the harness, not a free change), and
  `PHASE0_INVENTORY.md` is **not** archived material — it is the live contract
  inventory listed under "Start here" above.
- [`archive/FEATURE_PARITY_HISTORY.md`](archive/FEATURE_PARITY_HISTORY.md) — the
- [`archive/ASSIST_AUDIT_2026-08-29.md`](archive/ASSIST_AUDIT_2026-08-29.md) — the assist-area audit: 24 findings, a Rust↔Python protocol map, and the solid-areas record; the live queue's AX section is its shortlist.
  completed parity and operator waves.
- [`archive/UX_PERSPECTIVE_REVIEW_2026-08-03.md`](archive/UX_PERSPECTIVE_REVIEW_2026-08-03.md)
  — the point-in-time user-perspective review whose findings became UX0.
- `archive/LYRICS_TIMING_INCIDENT_2026-08-17.bridge.tsv` — the preserved failing
  bridge behind the incident record above. Not regenerable; every other artifact
  from that job lives under the gitignored `build/`.

## Documentation rules

To keep this directory useful rather than merely large:

1. **Write down what the code cannot say.** Contracts that span files, schemas
   another tool must satisfy, traps, and *why* a decision went one way. Do not
   restate the tree — a feature inventory here is stale the week it is written.
2. **Point at the live answer instead of copying it.** A test count, a flag
   list, a "currently implemented" table: name the file or command that answers
   it now. `cargo test`, `tools/verify.sh` and `cli.rs` do not go stale.
3. Separate observable contracts, implementation mechanisms, investigation
   evidence, and closed history — and say which one a document is in its first
   paragraph.
4. Record a negative control when a document makes a correctness claim based on
   a harness or probe. A check that has never failed proves nothing.
5. Put live work only in `FEATURE_PARITY_PLAN.md`.
6. Date what is dated. A record with a date can go stale honestly; the same
   sentence written as "currently" cannot.
7. Add every new document to this index, and move a closed record into
   `archive/` with a header saying when and why — rather than leaving it to read
   as current.
8. Keep topology in `CODE_MAP.md` generated from source; repair it with
   `python3 tools/code_map.py` instead of hand-editing it.
