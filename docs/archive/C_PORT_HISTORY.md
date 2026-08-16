# C port history (archived)

The earlier C implementation and the differential harnesses built around it are
historical migration evidence. They are not a behavioral authority, completion
gate, or dependency of Rusty Musializer.

Port-era source annotations, `tests/differential/*_oracle.c`,
`tools/differential_*.sh`, `REWRITE_PLAN.md`, and the C-oriented portions of
`PHASE0_INVENTORY.md` are retained only to explain old decisions and bugs. New
work must be specified, tested, and judged against the Rust application and its
current product goals. Reproducing a known C defect is not a correctness
requirement.

As of 2026-08-16, `tools/verify.sh` no longer reads the sibling C checkout, pins
its commit, checks its worktree, or offers a differential mode. The historical
harness scripts are deliberately outside the verification pipeline.
