# Assist-area audit, 2026-08-29

Operator request: *"run a check of the whole AI assistance area — I feel like we
still got quite a few holes and consistency issues in there."* Three agents: the
Rust stack, the Python helper side plus end-to-end seams, and a protocol map of
every format crossing the Rust↔Python boundary. Already-queued items (UX0-B10–
B17, AP5-b/c/d, AP6-e, C2, E2/E3/E4, F3) were excluded by brief and confirmed
still-open; every finding below is new. The live queue's **AX** section carries
the ranked shortlist; this file is the evidence.

**Status, 2026-08-31: closed except A7, A8, A12 and B12's `find_tool` rung.**
AX-1 through AX-6 landed in seven commits (`46d1426`, `16b5ecb`, `d05a552`,
`1333258`, `694cce3`, `eae5376`, `d2e4995`, `b7558e8`), fixing A1-A6, A9-A11,
B1-B5 and B7-B11 plus protocol-map rows 1/2b/6/7/9 and B12's `cache_dir` half.
The wave's evidence — what each stage changed, its negative controls and what
they caught — is the **AX** section at the end of
`docs/archive/FEATURE_PARITY_HISTORY.md`; what is still open is **AX-7** and
**AX-8** in `FEATURE_PARITY_PLAN.md`, with A8 and A12 recorded there as
decisions rather than defects.

The verdict, reached by both halves independently: **the pure core — contracts,
snapshot freezing, credential storage — is among the most carefully built code
in the repository. The holes are seams where the same question is answered
twice by different code and one copy drifted.** About eight of the Rust-side
findings collapse if the dialog's readiness badge and the pre-spawn preflight
become one function and the snapshot becomes the only authority on what is
sent.

---

## A. Rust stack (12 findings, ranked)

**A1 (M) — `readiness()` and `preflight()` are two independent answers to one
question, and the pre-spawn one is the weaker.** `execution::preflight` has one
call site (`runtime/assist/plan.rs:313`); the dialog badge is a separate
implementation at `app/ui/assist_settings.rs:2416`. Preflight has **no
Codex-discovery arm and no doctor arm** (`core/assist/execution.rs:1034` bails
for every non-OpenRouter route). Scenario: codex not installed, TC-WORDING on
its recommended codex route — the Routing tab shows `Blocked — codex not
found`, Start is accepted anyway, whisper runs for tens of minutes, the wording
lane dies inside the helper. Preflight's own docstring (`execution.rs:996-1000`)
says its purpose is preventing exactly this.

**A2 (S) — `ExecutionBlock::NoModel` is unreachable by construction.** Pushed
only when `entry.model_id.is_empty()` (`execution.rs:1030`), but
`ResolvedRoute::model_label()` (`:181-191`) never returns empty — OpenRouter
without a model yields the literal `"not chosen"`, which either starts a job
asking for a model named "not chosen" (catalog never fetched) or blocks with a
misleading `NoEndpoint`. The dialog's `NO_MODEL` branch
(`assist_settings.rs:2448-2455`) guards a different set than preflight.

**A3 (M) — a loose-permission or corrupt credentials file is reported as "no
key is configured".** `plan::openrouter_secret` (`plan.rs:225-230`) collapses
every `AssistFileError` into `None` via `.ok()??` — the BeatUpdate shape.
`CredentialSource` has no `Refused` variant, so `NoCredential`'s sentence tells
the user to add a key they already have. The dialog models this correctly
(`CredentialState::Refused` with chmod remediation,
`assist_settings.rs:5470-5500`): one fact, one correct sentence, one dead-end.

**A4 (S) — the ZDR toggle is a pressable control that cannot turn ZDR off, and
the snapshot then contradicts the request.** `process/assist.rs:317-319` passes
`--zdr` unconditionally for model modes; `external_analysis.py:1843` ORs it
with the route's requirement. Pressing off changes `assist.json` and the frozen
snapshot but not the request — §6 defines `provider_constraints` as "as sent",
so provenance understates the constraint actually applied. Direction is safe;
the control is dishonest. `prefer_gpu`/`stem_separation` two panes over show
the honest pattern (drawn disabled, "not wired").

**A5 (S) — `audio_scope` is taken from the suitability overlay while ignoring
`boundary_applied`** (`execution.rs:702-709`). mms-ctc/TC-ALIGN and
whisper.cpp/TC-COARSE carry `AudioScope::WholeTrack`, so every default Timed
lyrics job writes a snapshot with `boundary_applied: local-only` and
`audio_scope: whole-track` on lanes that open no socket. The test at
`execution.rs:1176` asserts `boundary_applied` on every row and never looks at
`audio_scope`.

**A6 (S) — the confirmation offers an enabled "Start analysis" beside the red
sentence saying the job cannot start.** `panels/assist.rs:3551-3568` gates the
button only on `helper_available`; pressing it fails at `:371` and drops the
user from Confirmation to Empty. `disabled_button` with the block sentence as
hint is the repo's own pattern.

**A7 (S) — `is_credential_named`'s doc comment forbids the blanket strip two
call sites perform on it.** `runtime/assist/env.rs:43-45` says the marker list
reports, never strips ("would break SSH_AUTH_SOCK, XAUTHORITY");
`discover.rs:338-345` and `assist_settings.rs:2886-2893` strip anyway, and
`discover.rs:283` spawns `$SHELL -lc` — a login shell — without them. An rc
file touching ssh-add/keychain stalls into the discovery timeout →
`CODEX_NOT_FOUND` on a machine where codex is installed. Two copies of
`strip_credential_variables`, one per crate.

**A8 (S/M) — E1's strip is applied at three of nine spawn sites.** Filtered:
the helper, discovery, the dialog's doctor/catalog children. Inheriting
wholesale: `process/ffmpeg.rs:478`, `process/dialogs.rs:292,387`
(kdialog/zenity), `process/reveal.rs:172` (`sh -c xdg-open`),
`process/font_import.rs:724` (network-contacting Python),
`assist_settings.rs:1155` (`curl`). E1 as written still holds (the key leaves
the app's own env at `main.rs:193`, verified first statement of `main`), but
the contract names ffmpeg and the dialog binaries explicitly.

**A9 (S) — the implemented-route table is hand-maintained twice with no test
pinning the pair**: `ContractId::route_is_implemented` (`contracts.rs:263-303`)
vs the dict at `external_analysis.py:1277-1287`. The widget-id-namespace trap
at protocol scale.

**A10 (S) — `AssistMode::data_boundary()` is a static per-mode consent sentence
drawn above a resolved graph that could contradict it**
(`assist_ui_state.rs:106-121` vs `panels/assist.rs:2235-2250`). Latent today
only because `implemented_route_types` blocks the contradicting route.

**A11 (S) — an unparseable workflow token widens to the audio-leaving
workflow**: `plan.rs:280` `WorkflowKind::parse(..).unwrap_or(All)` — the
opposite default from `Boundary::parse`, whose doc calls a defaulted boundary
"a silent widening". Unreachable today; wrong shape to leave loaded.

**A12 (S) — `CredentialMode::EnvImport` and `Session` are schema values nothing
writes**; an env-imported key displays as `Session`, so §3's two lifecycles are
one UI state.

Noted, not queued: `discover::resolve_cached` caches a thorough-probe
`NotFound` for the process lifetime — installing codex mid-session needs a
restart (adjacent to AP6-e).

**Solid:** the boundary ladder / eligible-implemented split / fallback algebra
(complete, exhaustively tested); the credentials file lifecycle (0600/0700
before any secret byte, temp+`sync_all`+rename, refuse-not-repair, `Forget`
proven byte-identical for other entries); snapshot freezing (one resolve at
Start, deterministic bytes, negative-controlled); no credential reaches argv
anywhere; `Secret` has no `Clone`/`Display` and a redacting `Debug`.

---

## B. Python helpers and end-to-end seams (12 findings, ranked)

**B1 (M) — the doctor's verdict is about a different installation than the one
a job uses — four channels.** The doctor calls the discovery defaults with no
arguments (`musializer_doctor.py:362,380`), while jobs pass
`assist.json`-configured `whisper_bin`/`whisper_model`/`align_python` as
flags that beat everything; `start_doctor` forwards only `--codex-bin`. Both
directions wrong: a working dialog-set path reads `not ready`; a typo'd one
gets a green doctor and a job that dies at `external_analysis.py:507`. Same
shape for the OpenRouter key (doctor reads the repo `.env`, desktop always
passes `--no-dotenv` and strips the imported key) and `models_dir`.
`docs/ASSIST_PIPELINE.md:302` names the doctor as the authority for exactly
this.

**B2 (S) — the one diagnosis that exists is written to disk and discarded.**
`external_analysis.py:2424-2426` prints precise causes ("{name} exited with
code {n}", "exceeded its {t}s timeout", "could not start {name}") to stderr;
Rust logs it and then reports **one fixed sentence for every cause**
(`panels/assist.rs:521-533`). "whisper.cpp is missing" and "the model is
corrupt" are the same toast. Overlaps queued B14, but the mechanism is
one-line, not a feature.

**B3 (S) — `tools/verify.sh` makes an outbound HTTPS call to openrouter.ai on
every run.** `test_provider_discovery.py:905` live-fetches the public catalog
with no opt-in gate (skips only if the network is already down), and it runs
via `support_bundle_check.sh` inside `verify.sh`. The dialog's own tooltip
warns the same fetch "still discloses your IP"; the gate does it silently.

**B4 (S) — the two catalog cache schemas are pinned on each side separately and
by neither across the boundary.** Negative control: bumping both Python
`SCHEMA_VERSION` constants to `/v2` leaves **241 Python tests green** (they all
reference the constant by name) and no Rust test can notice. User sees the
model picker silently revert to "never fetched".

**B5 (S) — the doctor report's `schema_version` is never checked by either
Rust reader**, every field is `#[serde(default)]`, no `deny_unknown_fields` —
any JSON object parses, so a renamed `runtimes` key yields a green "Doctor
finished; runtime identities updated." over an empty list and a snapshot with
no runtime provenance. `parse_catalog_facts` in the same file checks its
schema: one file, two policies.

**B6 (M) — a set-but-missing `codex_bin` blocks the dialog and silently runs a
different codex in the job.** `codex_discovery.path()` returns `None` for
`OverrideMissing`, `--codex-bin` is omitted, the helper defaults to hunting
bare `codex` on an augmented PATH. `settings.rs:196-201` states the rule
("set-but-missing is a loud failure, never a silent fallback"); only the
dialog implements it.

**B7 (M) — the doctor's asset checklist cannot detect the failure that kills
the doctor.** It omits five of `DISTRIBUTION_SUPPORT_FILES` — including
`lyric_align.py`, imported at `external_analysis.py:39` top-level. Proved in a
scratch tree: delete it → the doctor itself dies at import with empty stdout,
the dialog shows "The doctor report could not be read: EOF while parsing…"
(stderr captured and thrown away at `assist_settings.rs:6818`), and every job
dies at import behind B2's generic toast. `missing_support_files()` exists and
is called only by tests.

**B8 (M) — the Assist orchestrator has no Python tests.** `run_assist`,
`build_bridge`, `parse_bridge`, `_cache_matches`, `run_whisper`,
`discover_reference_lyrics`: zero test call sites. Two gutting perturbations
each left 241 green: `_cache_matches` ignoring `accept=` (an artifact from a
different model/route is reused — §5 rule 7 gone) and dropping the
`audio.sha256` comparison (track B reuses track A's transcript). 108 of the
241 tests are the MiMo bench harness; the shipping path has ~133 covering pure
predicates.

**B9 (S) — the gate's end-to-end bridge check never verifies the bridge's
identity fields**: `analysis_bridge_check.rs:14` passes `(None, None)`, so the
helper writing the wrong digest/duration is invisible to both suites (a third
perturbation confirmed it).

**B10 (S) — the dialog's doctor and catalog-refresh children have no timeout**,
and a wedge permanently disables every button in the dialog ("Disabled while
another background job is running") with no cancel short of restarting the app.

**B11 (S) — a 40-minute deadline hit, and a user cancellation, emit no notice
at all** (`AssistPoll::Stopped` → `Vec::new()`); every other terminal outcome
toasts.

**B12 (S) — three cross-language helpers duplicated, one already drifted**:
`cache_dir()` twice in Rust and once in Python — Python applies `.strip()` and
`.expanduser()` where Rust applies neither, so `XDG_CACHE_HOME="~/c"` resolves
to two different directories; `strip_credential_variables` twice; `find_tool`
drops the `MUSIALIZER_*_HELPER` override rung its own comment claims to honor.

**Solid:** the execution snapshot (pinned both sides, refused loudly, size-
capped, driven end-to-end by `secret_canary_check.sh`); the bridge TSV (header
and arities pinned in both languages, writer self-validates, real-writer-to-
real-reader shell gate with a negative control); setsid/EPERM contract matches
AGENTS.md and is tested; E1 holds at every helper (`_safe_local_env`,
`_safe_environ`, canary-asserted from outside); atomic writes + cache identity
(mkstemp→fsync→replace, refused refresh never touches the last good file);
staleness honest where shown (`NeverFetched`/`Unreadable`/`Loaded` distinct —
though `parse_catalog_facts` collapses unreadable into "never looked", which
silently disengages the modality guard; documented as a decision).

---

## C. The protocol map (every format crossing the boundary)

| # | Format | Writer→Reader | Version checked | One-sided gaps |
| --- | --- | --- | --- | --- |
| 1 | `.bridge.tsv` | Py→Rust | both, exact | **no Python unit test** (self-validate + shell gate only) |
| 2 | `assist-manifest.json` | Py→Rust | Rust (mismatch → silently no review panel) | fixtures hand-written on each side, none shared |
| 2b | embedded `execution_snapshot` (observed) | Py→Rust | structural only (`deny_unknown_fields`) | **no Rust test**; one added Python key silently drops the whole provenance display |
| 3 | `assist-execution.json` | Rust→Py | both, loud | Python ignores extra/missing per-contract fields untested; Rust's `skip_serializing_if` shape never exercised by the Python fixture |
| 4 | `assist.json` | Rust→Rust+Py | both | default-shaped file (no `local_runtimes` key) untested on the Python side |
| 5 | OpenRouter catalog | Py→Rust ×2 | all three readers | Rust `execution.rs` mismatch → `None` = modality guard silently off |
| 6 | Codex catalog | Py→Rust | both | **no Rust test for a wrong version** (the OpenRouter twin has one) |
| 7 | doctor report | Py→Rust ×2 | **never in Rust** | see B5 |
| 8 | `lyric-sync/v1` | Py→Rust | **never in Rust** (manifest gates instead) | Rust tests vs hand fixture, Python vs JSON Schema — different authorities; `schemas/*.json` unread by Rust |
| 9 | helper argv + env | Rust→Py | no version field | Rust pins argv against a **fake helper**; no Python test accepts the real flag set |
| 10 | protocol/answers JSONL | Rust→bash | n/a | not a Python format (correct) |
| 11 | `build/analysis/*` internals | Py→Py | Python only | Rust reads none (correct) |

Every cross-language constant — `MUSIALIZER_BRIDGE\t1`, seven `TC-*` tokens,
four `RouteType` tokens, `"Codex default"`, `"anchor-block-mms"`, six schema
strings — is a hand-duplicated literal. No shared fixtures (`fixtures/` holds
only a README), no codegen: each side is pinned to its own copy.

---

## Method notes

- One audit disclosure: running the permitted offline Python suite made a live
  HTTPS request to openrouter.ai — that became finding B3 rather than a
  methodological slip; there was no way to know before running it.
- The perturbation results (241-green through three orchestrator guttings, the
  `/v2` schema bump, the +1000 ms bridge duration) were all run in scratch
  trees and reverted; the working tree was never modified.
