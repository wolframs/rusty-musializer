# Rusty Musializer documentation

This directory is the front door for implementation and behavior documentation.
The root [`README`](../README.md) remains the short user-facing overview; the
documents here explain enough of the system that a maintainer can find the
authoritative code and evidence without reconstructing the project from scratch.

## Start here

- [`CODE_ARCHITECTURE.md`](CODE_ARCHITECTURE.md) maps crate boundaries, state
  ownership, the preview/export data flow, persistence, and verification layers.
- [`ASSIST_PIPELINE.md`](ASSIST_PIPELINE.md) follows an Assist request from the UI
  through the Python evidence tools, validation, staging, and project application.
  It also records the lyrics timing design and its trust boundaries.
- [`PHASE0_INVENTORY.md`](PHASE0_INVENTORY.md) records the observable contract
  inherited from the frozen C application: CLI grammar, settings, schemas, and
  environment variables. It is a contract inventory, not a code tour.

## Operations and evidence

- [`../tools/ANALYSIS_ADAPTERS.md`](../tools/ANALYSIS_ADAPTERS.md) is the operator
  guide for analysis dependencies, discovery, privacy boundaries, and commands.
- [`../tools/MEASURED_ANALYSIS.md`](../tools/MEASURED_ANALYSIS.md) describes the
  deterministic measured-audio artifacts consumed by higher-level analysis.
- [`LYRICS_TIMING_INVESTIGATION.md`](LYRICS_TIMING_INVESTIGATION.md) is the dated
  investigation record for the 2026-08-01 lyrics timing repair, including the
  acceptance tracks, negative controls, and independent checks.
- [`../FEATURE_PARITY_PLAN.md`](../FEATURE_PARITY_PLAN.md) is the sole live
  completion queue and the C-to-Rust feature ledger.
- [`../AGENTS.md`](../AGENTS.md) contains engineering constraints, the `unsafe`
  inventory, test-silence requirements, and the differential-testing method.

## Historical material

[`../REWRITE_PLAN.md`](../REWRITE_PLAN.md) preserves design reasoning and the
history of the rewrite. Its phase sketches are not the current task list. When a
historical note and current code disagree, current code plus its tests are the
implementation authority; the frozen C code is the behavioral authority for
features covered by parity.

## Documentation rules

To keep this directory useful rather than merely large:

1. Describe current ownership and data flow, and link to the owning source file.
2. Separate observable contracts, implementation mechanisms, investigation
   evidence, and historical decisions.
3. Record a negative control when a document makes a correctness claim based on
   a harness or probe.
4. Put live work only in `FEATURE_PARITY_PLAN.md`; do not grow a second backlog
   inside documentation.
5. Add every new document to this index.
