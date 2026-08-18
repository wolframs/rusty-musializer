# Assist provider configuration: contracts and threat model (P0)

Date: 2026-08-04. Status: design authority for the provider-configuration
workstream. Not a task queue — P1–P5 items belong in `FEATURE_PARITY_PLAN.md`.

Design intent: `docs/LYRICS_TIMING_RESEARCH_PLAN.md`, "Assistance provider
configuration workstream". Where that section and this document disagree, this
one wins: it carries the operator's 2026-08-04 storage decisions recorded in
`AGENTS.md`, "Persistent file storage" — **no OS/vendor wallet**, a
0600 credentials file instead.

Current mechanisms this replaces or wraps: `tools/external_analysis.py`
(`_safe_local_env`, `_openrouter_env`, `--codex-model`),
`tools/mimo_openrouter.py` (hard-coded `MODEL = "xiaomi/mimo-v2.5"`),
`crates/musializer-runtime/src/process/assist.rs` (`AssistSpec`, argv, `--zdr`),
`crates/musializer-app/src/ui/panels/assist.rs` (staging boundary).

## 1. Task contracts

Boundary ladder, lowest to highest. A route may never be resolved for a contract
whose declared maximum boundary is lower than the route's own.

| rank | boundary | meaning |
| --- | --- | --- |
| 0 | `local-only` | nothing leaves the machine; no socket is opened |
| 1 | `text-leaves-machine` | derived text/JSON leaves; no audio, no raw PCM |
| 2 | `audio-leaves-machine` | audio bytes leave; requires per-job confirmation |

Route types: `builtin` (in-process deterministic Rust), `local-proc` (child
process on this machine), `codex` (installed `codex exec`), `openrouter`.

| id | task | inputs | max boundary | eligible route types | allowed fallback policies |
| --- | --- | --- | --- | --- | --- |
| `TC-MEASURED` | measured audio features | decoded PCM, duration | `local-only` | `builtin` | `none` (locked, not user-routable) |
| `TC-COARSE` | coarse lyric evidence / localization | full audio, optional vocal stem, language hint | `audio-leaves-machine` | `local-proc`, `openrouter` | `none`, `ask`, `local-only`, `same-boundary` |
| `TC-ALIGN` | known-text forced alignment | full audio or block slices + authored lyric text | `local-only` | `local-proc` | `none`, `local-only` |
| `TC-WORDING` | lyric wording review when no authored text exists | bounded Whisper JSON (`musializer.lyric-timing/v1`) | `text-leaves-machine` | `codex`, `openrouter` | `none`, `ask`, `same-boundary` |
| `TC-SEMANTIC` | semantic / feeling analysis | complete audio or explicitly shown excerpts | `audio-leaves-machine` | `openrouter` | `none`, `ask` |
| `TC-PLAN` | scene-plan reasoning / review | measured-analysis JSON, section artifacts | `text-leaves-machine` | `builtin`, `codex`, `openrouter` | `none`, `ask`, `local-only`, `same-boundary` |
| `TC-VERIFY` | independent timing verification | named review excerpts only, never the whole song by default | `audio-leaves-machine` | `local-proc`, `openrouter` | `none`, `ask`, `local-only`, `same-boundary` |

Rules that follow from the table and are not negotiable per-route:

- `TC-ALIGN` is `local-only` by contract even though remote aligners exist. A
  remote aligner would need the contract's maximum raised deliberately, with a
  benchmark and a schema bump — not a new route entry.
- `TC-VERIFY` sends excerpts. Sending the whole track under `TC-VERIFY` is a
  distinct confirmation, and the excerpt list is shown before Start.
- Modality eligibility (`audio -> text`) is necessary and not sufficient. The
  suitability overlay (`recommended` / `experimental` / `unsupported`) gates what
  a picker offers by default; see the plan's OpenRouter section.
- A workflow button (`Timed lyrics`, `Scene changes`, `MiMo feelings`,
  `Full assist`) resolves to a set of these ids. Confirmation shows the resolved
  set, per-contract, before any process starts.

### Implemented adapters in this build

The contract table above is the durable settings/schema capability envelope; it
is not a claim that every listed adapter exists today. The executable currently
dispatches exactly these identities:

| Contract | Implemented route |
| --- | --- |
| `TC-MEASURED` | `builtin/builtin-analyzer` |
| `TC-COARSE` | `local-proc/whisper.cpp` |
| `TC-ALIGN` | `local-proc/mms-ctc` |
| `TC-WORDING` | `codex/codex` |
| `TC-SEMANTIC` | `openrouter/openrouter` |
| `TC-PLAN` | `builtin/builtin-planner` |
| `TC-VERIFY` | not implemented or composed by any workflow |

A schema-legal future route is retained in a saved profile, but the UI marks it
unimplemented and preflight refuses it. The helper repeats the same check before
reading or writing caches. It is never acceptable to execute one adapter while
recording another in provenance.

The four workflow buttons map to output lanes and stages as follows:

| Workflow | Staged editor output | Stages that actually run |
| --- | --- | --- |
| Timed lyrics | lyrics only | measured analysis, local Whisper, conditional Codex wording review only when no authored source is found, local MMS alignment, deterministic planner support |
| Scene changes | scene switches only | measured analysis and deterministic planner |
| MiMo feelings | semantic cues only | measured analysis, OpenRouter MiMo over the track audio, deterministic planner |
| Full assist | all three lanes | the union of the preceding stages |

## 2. Non-secret preferences record

One versioned, atomically replaced file in the per-user config directory.
Path resolution follows the same shape as `ui/preferences.rs` and is implemented
in `runtime::assist::files::settings_path`: `$MUSIALIZER_ASSIST_SETTINGS`, else
`$XDG_CONFIG_HOME/musializer/assist.json`, else `$HOME/.config/musializer/assist.json`.
Size cap and `deny_unknown_fields` as in `ui/preferences.rs`; a corrupt file is
an error, never a silent reset.

```text
schema              string   "musializer.assist-settings/v1"
active_profile      string   profile id; "recommended" is built in and unwritable

profiles[]
  id                string
  label             string
  routes            map<contract_id, Route>   sparse; absent = inherit recommended

Route
  contract          string   TC-MEASURED | TC-COARSE | ... (§1)
  route_type        enum     builtin | local-proc | codex | openrouter
  runtime_id        string   "whisper.cpp" | "mms-ctc" | "qwen3-fa" | "codex" | "openrouter"
  model_id          string?  provider model slug, or a local model identity
  model_path        string?  local weights, resolved against models_dir
  reasoning_effort  enum?    codex only: minimal | low | medium | high
  fallback          enum     none | ask | local-only | same-boundary   (§5)
  provider          Provider?  openrouter only

Provider (OpenRouter provider-selection constraints; never a secret)
  order[]           string[]  preferred provider slugs, in order
  only[]            string[]
  ignore[]          string[]
  allow_fallbacks   bool      default false for boundary>=1 contracts
  zdr_required      bool      default true for any audio-leaving contract
  max_price_prompt      number?   USD per 1M tokens
  max_price_completion  number?
  max_price_audio       number?

local_runtimes
  models_dir        string    operator decision: default <install dir>/models/,
                              fallback <home>/musializer/models; always shown
  codex_bin         string?   an explicit path to the codex executable; overrides
                              runtime::assist::discover's ladder entirely (AP6);
                              set-but-missing refuses loudly rather than silently
                              falling back to discovery
  whisper_bin       string?   overrides MUSIALIZER_WHISPER_BIN
  whisper_model     string?   overrides MUSIALIZER_WHISPER_MODEL
  align_python      string?   overrides MUSIALIZER_ALIGN_PYTHON
  prefer_gpu        bool
  stem_separation   enum      never | on-demand | always

catalog
  network_allowed   bool      false = manual Refresh only, never on open
  refresh_on_open   bool
  last_filters      map<string,string>   e.g. {input_modalities: "audio", output_modalities: "text"}
  last_refresh_utc  string?   RFC 3339; display only
  show_experimental bool      unlocks non-recommended models in pickers

credentials
  openrouter
    mode            enum      none | file | session | env-import
    lookup_id       string    opaque account label, e.g. "default"; NEVER the secret
    fingerprint     string?   first 8 hex of sha256(secret); NEVER any key characters
    label           string?   provider-returned label from the last successful Test
```

Absent fields inherit the built-in `recommended` profile. A default-valued
profile is not serialized, so a settings file written before a field existed
stays readable and byte-comparable.

## 3. Credentials lifecycle

| aspect | rule |
| --- | --- |
| file | `$MUSIALIZER_ASSIST_CREDENTIALS`, else `$XDG_CONFIG_HOME/musializer/credentials.json`, else `$HOME/.config/musializer/credentials.json`; schema `musializer.assist-credentials/v1`, one object keyed by `provider/lookup_id` |
| permissions | file `0600`, containing directory `0700`, both set **before** any secret bytes are written |
| write | write to a sibling temp file created with mode `0600`, `fsync`, then `rename` — never truncate-in-place |
| read refusal | refuse to read and report an actionable error if the file is group- or world-readable, the way `ssh` refuses a loose private key. Do not silently repair it |
| session-only | secret held in process memory for this run; nothing written. Survives no restart, and the dialog says so |
| env import | at startup, read `OPENROUTER_API_KEY` **once** into the session store, then remove it from this process's environment (§4 E1) |
| repo `.env` | legacy CLI path only. The desktop flow disables the helper's `.env` fallback so a key the dialog never saw cannot silently authorize a job and falsify provenance |
| display | provider `label` plus `sha256(secret)[0..8]`. Never any character of the key, never a length, never a masked-but-selectable field |
| in memory | best effort: one owner, zeroized on drop, no `Clone`, no `Debug`/`Display` (a hand-written `Debug` prints `<redacted>`). The defensible claim is minimized scope and no durable plaintext copy — not that a managed runtime erased every copy |

Dialog flows:

| flow | behaviour |
| --- | --- |
| Replace | masked input, immediate `Test`, and the new secret is committed only after `Test` succeeds or the user explicitly saves untested |
| Forget | removes this provider's entry, rewrites the file atomically, leaves other providers' entries byte-identical, and clears the session copy. If the file becomes empty, the file is removed |
| Test | `GET https://openrouter.ai/api/v1/key` — read-only, non-inference, spends no credits. Records `label`, rate/usage state, and free-tier flag. Never a chat completion |
| Test outcomes | `ok` / `invalid` / `revoked` / `rate-limited` / `no-network` / `no-key` — each a distinct actionable state, never a generic failure |

## 4. Secret-exposure inventory

| id | exposure | rule that prevents it |
| --- | --- | --- |
| E1 | env of unrelated children (`ffmpeg`, `kdialog`/`zenity`, `codex`, local analysis) | `external_analysis.py::_safe_local_env` already strips every variable whose name contains `KEY`, `TOKEN`, `SECRET`, `PASSWORD`, `CREDENTIAL`, `AUTH`, and only `_openrouter_env` re-adds the one authorized key. Extend the same rule to the Rust side: the app removes `OPENROUTER_API_KEY` from its own environment after the startup import, so no child can inherit it by accident. Note this needs `std::env::remove_var`, which is `unsafe` in edition 2024 — it must run before any thread starts and get a `SAFETY:` comment and an `AGENTS.md` unsafe-inventory row |
| E2 | `argv` (visible in `/proc/*/cmdline` to the same user, and in `ps` output pasted into a bug report) | secrets are passed only through the child's environment. No flag ever takes a key. `AssistSpec` builds argv from paths, mode, duration and `--zdr`, and that stays true |
| E3 | logs and the job log file | `mimo_openrouter.py` already logs the request with `"Authorization": "Bearer <redacted>"`. Every new request-echo path does the same. No log line prints an environment map |
| E4 | analysis manifests / dry-run output | `assist --dry-run` reports `"credentials": "environment only; omitted"`, asserted by `tools/support_bundle_check.sh`. The execution snapshot (§6) records `credential_present: bool` and `credential_fingerprint`, never `lookup_id` if it would expose an account label unnecessarily |
| E5 | `.musi` project files | credentials and `lookup_id` are user configuration, never project content. Projects carry the execution snapshot only |
| E6 | model/catalog cache under `$XDG_CACHE_HOME` | catalog responses are normalized to a bounded allowlist of fields before writing. Response headers are never persisted. Catalog strings are untrusted display data and never become paths or shell fragments |
| E7 | support bundle / doctor | `musializer_doctor.py` checks credential **membership** only (`"OPENROUTER_API_KEY" in environment`), never the value. A diagnostics/crash bundle collector **does not exist yet** (2026-08-05 evidence sweep; open as AP5-c in `FEATURE_PARITY_PLAN.md`) — when one is built it must apply the same name-marker strip before capturing any environment, with a canary test asserting it |
| E8 | crash output, panic messages, backtraces | the secret type has no `Debug`/`Display` that prints it; a struct holding it derives nothing. Panic payloads carry contract ids, not payloads |
| E9 | clipboard | no control copies a key, and the masked input field is not copyable. "Copy diagnostics" copies the redacted snapshot |
| E10 | HTTP request echo in error paths | on transport failure, report status, provider and contract id. Never the request headers or body |
| E11 | preference file | `assist.json` carries `lookup_id` and `fingerprint` only. A schema-level test asserts no field can hold a secret |

Acceptance for all of the above is one canary: a syntactically valid key with a
unique sentinel substring, planted through every entry route, then a recursive
scan of the config dir, cache dir, `build/analysis/`, `.musi` output, logs,
support bundle, and `/proc/<pid>/cmdline` finds zero occurrences.

## 5. Fallback boundary rules

Four policy values, per route:

| value | on route failure |
| --- | --- |
| `none` | fail the contract and report it. No substitution |
| `ask` | pause the job, show the proposed substitute route and its boundary, wait for a decision. A running job that reaches `ask` with no user present stays paused, and times out into `none` |
| `local-only` | substitute only routes with boundary rank 0 |
| `same-boundary` | substitute only routes whose boundary rank is **equal** to the failed route's. Not "≤", because a quiet downgrade also changes which model produced the result |

Invariants:

1. **A local failure never becomes a remote request without a new decision.**
   Rank may increase only through `ask` plus an explicit answer. `none`,
   `local-only` and `same-boundary` can never raise rank.
2. **Confirmation precedes any boundary-raising request, per job.** A prior
   confirmation for one contract does not authorize another.
3. **A job snapshots its resolved route graph at Start.** Settings edited mid-run
   affect the next job only. The snapshot is immutable and is what provenance
   records — the resolver is never re-run against current settings after the fact.
4. **A missing eligible endpoint blocks.** If `zdr_required` or the provider
   `only`/`order`/price constraints leave no endpoint, the request is refused and
   the reason names the constraint. Constraints are never weakened silently.
5. **A route that loses its required modality is invalid.** The last valid
   catalog is preserved; the route is marked unresolvable rather than silently
   re-pointed at another model.
6. **Codex discovery failure preserves `Codex default`.** Never a guessed model id.
   Discovery itself (override, `PATH`, well-known install directories, then the
   login shell's `PATH`) is `runtime::assist::discover` (AP6); this document
   states the policy consequence, not the ladder.
7. **Cache acceptance compares route identity per contract.** Changing the
   `TC-ALIGN` model must not reuse an artifact produced by another one; this
   extends the existing `_provenance_matches` model/prompt comparison to the
   contract-keyed route identity.

## 6. Execution snapshot and provenance

Recorded once per analysis run, immutable, embedded in the manifest, in each
produced artifact's `provenance`, and in the staged `AnalysisCandidate`.

```text
snapshot_schema        string   "musializer.assist-execution/v1"
settings_schema        string   the assist.json schema that resolved it
profile_id             string
resolved_at_utc        string   RFC 3339
contracts[]
  contract             string   TC-*
  route_type           enum     builtin | local-proc | codex | openrouter
  runtime_id           string
  runtime_version      string?  e.g. whisper.cpp build, aligner package version
  model_id             string   the model that ACTUALLY ran, not the setting
  model_sha256         string?  local weights, where hashing is practical
  reasoning_effort     enum?
  boundary_applied     enum     local-only | text-leaves-machine | audio-leaves-machine
  boundary_confirmed   bool     true only where rank >= 1 and a user confirmed
  audio_scope          enum?    none | excerpts | whole-track
  excerpt_spans[]      [start,end] seconds, when audio_scope = excerpts
  provider_constraints Provider  as sent, including zdr
  provider_served      string?   provider slug the response reported
  prompt_version       string?
  prompt_sha256        string?
  schema_version       string?   output schema the response was validated against
  fallback_policy      enum
  fallback_taken       bool
  fallback_from        string?   the route id that failed, when fallback_taken
catalog_revision       string?   cache schema version + fetch timestamp
suitability_revision   string?   suitability overlay version
credential_present     bool
credential_fingerprint string?   sha256(secret)[0..8]; never the lookup label
```

`model_id` is observed, not inferred: for Codex it is what `codex exec --model`
was invoked with (and, when the response reports one, what the response says);
for OpenRouter it is the `model` field the response returned, which may differ
from the requested slug when a variant is served. A run whose snapshot lacks
exact local model identity or exact remote provider/model/policy is not
reproducible and must not be accepted as benchmark evidence.

## 7. Decisions taken here

1. Contract ids are stable tokens (`TC-*`) and are the key in preferences, in the
   snapshot, and in cache acceptance — so a route change invalidates by id.
2. `TC-MEASURED` appears in the routing matrix but is locked, so the user can see
   the whole pipeline rather than only its configurable parts.
3. `same-boundary` means equal rank, not "≤" — a silent downgrade still changes
   the answer's provenance.
4. The app removes `OPENROUTER_API_KEY` from its own environment after import,
   rather than relying only on the Python helper's strip (E1).
5. The credentials file is refused, not repaired, when its permissions are loose.
6. The desktop flow disables the helper's repository-`.env` fallback, because a
   key the dialog never saw would produce a truthful-looking but wrong snapshot.
