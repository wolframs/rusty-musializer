# Repository guide for coding agents

The Rust rewrite of Musializer. The C repository is feature frozen and the
rewrite has started; no Cargo workspace exists yet.

`CLAUDE.md` is a symlink to this file. Edit `AGENTS.md`.

## Before implementation work

Read `REWRITE_PLAN.md`. Start with its "Handoff: start here" section, which
gives the incoming session its first moves in order; then the source ownership
map, which assigns every C file in the frozen tree to a workstream. The plan
also carries the frozen commit, the crate boundaries, the invariants that
survive the rewrite, and a NOTE ENTRIES section at the bottom recording what
has already been done, decided, or gone wrong.

Read the notes before assuming any section describes reality — the prose
describes the plan, the notes describe what happened. Add a note when you learn
something a later session would otherwise rediscover.

## The behavioral oracle

`../musializer` is read-only. It is frozen at commit
`9300af942bd00d8c85fc4e3c8c02cf2b6356764f` (`9300af9`) on branch `master` —
note `master`, not `main`.

- Never modify the C repository as a side effect of work here, including to
  "fix parity". If the oracle looks wrong, say so; do not edit it.
- Read `../musializer/CURRENT_FILE_POINTERS.md` before trusting any other
  document there. It marks which documents describe behaviour and which
  describe intent, and the difference has been expensive in that repository
  before.
- `EXTENSION_PLAN.md` is part roadmap with open decision gates, and
  `cadence-overhauls-2026-07-26.md` is an unimplemented scratchpad. Neither
  describes the frozen binary.
- `../musializer/AGENTS.md` is gitignored there and absent from a fresh clone,
  though it is present in the local working tree.
- The code and its tests are authoritative about behaviour. Documents are not.

## Rules for this repository

- Preserve unrelated work in both repositories.
- Keep first-party application code in Rust. Retaining raylib as an external C
  dependency through Rust bindings is intentional and in scope, as is FFmpeg as
  an external executable and the existing Python helpers as independent tools.
- Do not copy credentials, `.env`, user audio, generated video, analysis
  caches, or build artifacts into this repository. Synthetic fixtures only.
- Prefer small compiling checkpoints, but optimize for a fast Linux-first hobby
  rewrite rather than release-engineering ceremony.
- Only the integration owner edits the root manifest or broad application
  state. Leaf agents request dependencies rather than adding them.

## Still to be filled in

These land as the work reaches them, and this file is where they go:

- the actual Cargo commands and crate map, once the workspace exists;
- the raylib binding decision — `raylib-sys` building its own copy versus
  no-build mode linking the vendored raylib 5.5 from the frozen tree — recorded
  with the reason, once the vertical slice proves one;
- build, test, and style guidance;
- the `unsafe` inventory and where its invariants are documented.
