# Differential testing against the C oracle

Detail moved out of `AGENTS.md` on 2026-10-29. `AGENTS.md` keeps the rules; this file keeps the harness table, the lessons and the negative controls. Add a row to the table when you add a harness.

Every script compiles the relevant `../musializer/src/*.c` with output going into
our own `build/`. The oracle is read, never written, and its own build directory
is never touched.

| Harness | Result |
| --- | --- |
| `differential_analyzer.sh` | 104 bands agree to 4e-10 (float print precision) |
| `differential_settings.sh` | all 81 descriptors match exactly |
| `differential_routes.sh` | 380 rows, including the clamp asymmetry |
| `differential_route_persistence.sh` | 381 lines: parse grammar and the both-ways pairing |
| `differential_event_merge.sh` | 12 cases, 15 merged events, ids and ordering exact |
| `differential_assist_ui.sh` | 341 policy decisions exact |
| `differential_preset_store.sh` | 11 scene tokens, 44 presets, 3660 bytes of store JSON |
| `differential_song_atlas_map.sh` | 912 slices, 29374 values, largest delta **0** |
| `differential_ascii_art.sh` | 53847 records, 329738 values, largest delta **0** |
| `differential_project_io.sh` | 3061 values across 6 comparisons, **both `.musi` round trips** plus frame-lane boundaries, and the C's refusal of a v2 file pinned as an expected pair, largest delta **0** |
| `differential_timeline_view.sh` | 30865 records, 204953 values, largest delta **0** |
| `differential_layout.sh` | 13783 records, 348285 values, largest delta **0**; plus 1388 post-legacy rail records (WX-5c) and 2477 column-width records (WX-5d) against a pinned file |
| `differential_beat_tracker.sh` | 12352 records, 123522 values, largest delta **0** — **found a parity bug** |

Two of those harnesses now compare in **two sections**. The tree has a scene the
frozen C cannot have (Phosphor Dream, id 10, 2026-08-08), so `settings_dump` and
`route_persistence_dump` print the C-era rows, a `--- post-legacy (no oracle) ---`
marker, then the rest; the driver diffs the first half against the oracle exactly
and the second against `tests/differential/*_post_legacy.txt`. Both controls were
run: perturbing `settings.spectrum.trail`'s bound fails against the C, and
perturbing `settings.phosphor.dwell`'s fails against the pinned file. Do not
"simplify" this back to one diff — the point is that a post-legacy value is still
a `.musi` compatibility surface and still has to fail loudly when it moves.

**This is the pattern to copy for every pure module.** A number to compare beats
a paragraph of reasoning about whether a port is faithful, and it catches the
class of error review misses — the settings harness exists because ~81
descriptors were transcribed by hand and a single mistyped bound would surface
much later as a scene quietly ignoring a saved setting.

Compare integers exactly; give floats a tolerance, because libm and Rust's
intrinsics may differ in the last bits. Duplicate the fixture generator between
the C and Rust sides rather than sharing it — a shared generator can hide the
difference you are looking for.

**Build a negative control, and record what it caught.** A harness that has never
failed proves nothing, and the two most recent ones both earned this rule: the
Song Atlas harness was checked by moving the onset threshold 0.62 → 0.60, which
made 20 `onset` flags disagree, and by perturbing a loudness coefficient by 1e-7,
which made 17389 floats disagree with a smallest delta of 1.9e-9. Writing that
control also exposed a hole in the comparator itself — `nan` and `inf` *parse* as
floats in Python, and `abs(nan - nan) > tolerance` is `False`, so those columns had
been passing unconditionally. Perturb, watch it fail, revert byte-for-byte, and
prove the tree is clean.

**A unit test suite ported from the oracle is not a substitute, and there is now a
measurement rather than an opinion.** The `layout` harness was written because
`ui/timeline_view.rs` had **1** exact assertion where `test_timeline_view.c` pins 32,
and `workspace_layout`/`timeline_layout` were similarly range-heavy. Its two negative
controls both left the ports' own unit tests **fully green**:

| perturbation | the harness | the module's unit tests |
| --- | --- | --- |
| `>=` → `>` in `workspace_layout`'s mode ladder | 431 values fail | **9 passed** |
| `margin * 2.0` → `* 2.0625` in `timeline_layout` (1/16 px) | 14673 values fail | **7 passed** |

The second is the instructive one: the tightest unit expectation gives `scale` a 1e-3
tolerance, and 0.375 px spread over a 628 px row lands inside it. So the rule is not
"prefer harnesses"; it is that a **property assertion cannot pin a value**. `assert!(x
>= MIN)` and `assert!(rect.contains(inner))` are satisfied by wrong formulas, and a
layout error is invisible in a capture because everything moves together and still
looks self-coherent. Where the oracle pinned a number, pin the number — and prefer a
differential comparison, because a hand-transcribed expectation can be mistyped or
(worse) copied from our own output, and would then pass forever.

Both harnesses found **no disagreement**: the ports were right. That is the other half
of the point — a harness on a suspected module converts "we cannot tell" into "it is
correct", which is worth as much as finding a bug.

**And then the next one found an actual bug, in the place the argument predicted.**
`beat_tracker` was chosen for a harness because its port's strongest evidence was a
*hand-transcribed* eight-value table — `%.9g` phases pasted from a scratch C build
that no longer exists, unre-derivable without redoing the work. Replacing 8 pinned
values with 123522 compared ones surfaced this:

`beat_tracker_update` returns false for two unrelated reasons. It refuses bad input
*before* writing its out-parameter; or it computes a position inside `[0, 1)`,
narrows it to a float that lands on exactly `1.0`, **writes that**, and refuses it
afterwards (`beat_tracker.c:76-78`). `plug.c:1139-1144` keeps a
`float beat_phase = 0.0f;`, passes its address, and uses whatever is in it either
way — so the scene frame gets `0.0` in the first case and `1.0` in the second. The
port returned `Option<f32>`, had nowhere to put the `1.0`, and collapsed both to
`0.0`. `beat_phase` is a documented route source, so that is visible in a rendered
MP4.

Three things about how it was found are worth copying:

- **It surfaced from writing the excuse down.** The first version of the harness
  pinned this as an expected divergence, with a comment arguing it was an oracle
  quirk the port deliberately did not reproduce. Reading the C's *caller* to justify
  that sentence is what showed the sentence was false. Pin asymmetries as named
  pairs — and then go check the claim you wrote in the pair.
- **The `Option` was the disguise.** `None` reads as "no value", so two refusals
  with different observable results looked identical at the type level. The fix is a
  three-variant `BeatUpdate`, because the C's `(bool, out float)` pair carries
  information an `Option` cannot.
- **It is not a corner case.** The grid hits it **187 times in 12250 steps**: any
  tick that nearly divides the beat interval drifts the position a hair under a
  whole multiple, and 2⁻²⁵ below 1.0 is enough. A 0.3 s tick against the neutral
  0.5 s clock does it by the tenth step.

Its two other negative controls behaved like `layout`'s — **10 unit tests green**
against 3 and 732 failing harness values, for `>` → `>=` on the discontinuity
threshold and for an exclusive upper bound on the credible-gap range. Neither test
suite has a case for a gap of exactly 0.75 s or exactly 1.5 s, which is why.

Two smaller lessons from the same build, both from checks failing *before* the port
was suspected:

- **Echo the inputs, so a drift between the duplicated generators fails in a key
  column.** `3.4028235e38f` in C is a *float* literal widened to a `double`
  parameter — a different number from the same decimal parsed as a double. The two
  harnesses disagreed by 3.4e30 on the first run, in the echoed `time` column, and
  it was mine, not the port's.
- **Clippy's `excessive_precision` can be reporting a dead test.** A row feeding
  `0.0399999991` as "the float just below the 0.04 floor" rounds back *to* `0.04f`,
  so it duplicated the row above it and the boundary looked tested while going
  untested. The real value has to be computed (`nextafterf` / `from_bits`), not
  written as a decimal.

**Where the two sides cannot express the same rejection, pin the asymmetry as an
expected pair.** The C takes bare pointers and Rust takes slices, so each can refuse
an input the other cannot even represent: the C rejects a `NULL` buffer, and Rust
rejects a 15-byte buffer for a 2x2 image that the C has no length parameter to
notice. The `ascii_art` harness asserts those as four named pairs — including
*deliberately not* calling the C with the truncated buffer, since that would read
past the end — rather than quietly excluding the cases. An untested rejection path
is where a difference hides.

**These run on demand, not in `tools/verify.sh`.** Since 2026-08-16 the gate no
longer reads the sibling C checkout, pins its commit or offers a differential mode
(`docs/archive/C_PORT_HISTORY.md`), because a green tree must not depend on a
read-only clone somebody else's machine may not have. Run
`tools/differential_*.sh` directly when you touch a module one of them pins, or
when the row above is the evidence you are relying on. A harness failing is still
a contract broken; it is just no longer a build break.

Adding one: put the C harness in `tests/differential/`, the Rust side in
`crates/musializer-core/examples/` (examples need no manifest entry, so they
never collide with a parallel agent), and the driver in `tools/`. Add a row above.
