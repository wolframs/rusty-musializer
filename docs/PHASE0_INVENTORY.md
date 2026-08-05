# Phase 0 inventory of the frozen C oracle

Every citation is `path:line` **relative to `../musializer`**, the read-only C
repository frozen at commit `9300af942bd00d8c85fc4e3c8c02cf2b6356764f`
(`9300af9`) on branch `master`. Nothing in that tree was modified, built, or
executed to produce this document; it was read and grepped only.

Sections 3, 5, and 6 are contract tables. Other agents will code against them.
Where the code and a document in the C repository disagree, the code wins and
this file records the code.

---

## 1. Build profile and version

`build/config.h` is the configured feature set of the binary that produced the
test baseline:

| Macro | State | `build/config.h` line |
| --- | --- | --- |
| `MUSIALIZER_TARGET_LINUX` | **defined** | `build/config.h:2` |
| `MUSIALIZER_TARGET_WIN64_MINGW` | commented out | `build/config.h:3` |
| `MUSIALIZER_TARGET_WIN64_MSVC` | commented out | `build/config.h:4` |
| `MUSIALIZER_TARGET_MACOS` | commented out | `build/config.h:5` |
| `MUSIALIZER_TARGET_OPENBSD` | commented out | `build/config.h:6` |
| `MUSIALIZER_HOTRELOAD` | **off** | `build/config.h:9` |
| `MUSIALIZER_UNBUNDLE` | **off** (resources are bundled into the executable) | `build/config.h:12` |
| `MUSIALIZER_MICROPHONE` | **off** | `build/config.h:15` |

So the Rust vertical slice targets: Linux, no hot-reload DLL split, resources
bundled, no microphone capture.

`./build/musializer --version` prints exactly `musializer 2026.07`.

Version string sites — there are **three separate literals, not one constant**,
which is a parity trap worth fixing on the Rust side by making it one constant:

- `src/musializer.c:323` — `puts("musializer 2026.07");` — the `--version`
  output. Lowercase `m`, no `v` prefix, single space.
- `src/musializer.c:255` — `"Musializer 2026.07 - deterministic music
  visualization workspace\n"` — the first line of `--help`. Capital `M`.
- `src/plug.c:4293` — writes `"musializer-2026.07"` into
  `project->metadata.application_version` (hyphen, not space). This is the
  string that lands in every saved `.musi`.

There is no `MUSIALIZER_VERSION` macro anywhere in `src/`.

---

## 2. Test baseline

Recorded from the run already performed; the binaries were not re-executed here.

**C: 327 of 327 assertions pass.**

- `tests/` holds 42 `.c` files: **41 named `test_*.c`** plus the shared
  `tests/audio_fixtures.c`. Note that `tests/test_main.c` is the harness entry
  point *and* itself a test file (it self-tests the audio fixture generators,
  `tests/test_main.c:6-37`), and `tests/test_support.c` /
  `tests/test_support.h` is the assertion framework.
- One test binary. `src_build/nob_stage2.c:319` compiles
  `./tests/test_support.c` and `./tests/audio_fixtures.c` alongside the test
  translation units with `-std=c11 -Wall -Wextra -Wpedantic`, linking `-lm`.

**Python: 137 tests + 15 subtests pass**, across **11 files** in
`tests/adapters/`:

```
test_analysis_adapters.py     test_lyric_align.py        test_productization.py
test_command_line_session.py  test_measured_analysis.py  test_render_product_smoke.py
test_external_analysis.py     test_musializer_doctor.py  test_scene_quality.py
test_google_fonts.py          test_nob_windows_job.py
```

**`tests/e2e/` is manual-only and must never be automated.** It contains
exactly `tests/e2e/test_lyrics_assist_e2e.py` and `tests/e2e/README.md`. The
suite gates itself on `MUSIALIZER_WHISPER_BIN` / `MUSIALIZER_WHISPER_MODEL`
being set (`tests/e2e/test_lyrics_assist_e2e.py:68-69`, skip logic at
`:100-105`) and drives a real Whisper model. The Rust rewrite must not wire
this into CI or into any default test target.

---

## 3. CLI surface

Source of truth: `src/musializer.c`. The whole parser is `main()` at
`src/musializer.c:315-662` plus four value parsers at `:19-250`. No other file
consumes `argv`.

### 3.1 Pre-pass: help and version short-circuit

`src/musializer.c:317-326` scans **all** of `argv` **before** anything else —
before `reload_libplug()`, before `InitWindow`, before `plug_init`.

- `-h`, `--help` → print help to **stdout**, `return 0`.
- `--version` → `puts("musializer 2026.07")`, `return 0`.

Consequences the Rust port must reproduce: these win from **any** position,
they win **even when other arguments are invalid**, they open no window, and
they exit `0`. `--help` before `--version` in the same scan iteration means for
`musializer --version --help` the loop hits `-h/--help` check first at index 1
only if index 1 *is* the help flag — the scan is per-index, both checks per
index, so whichever flag comes **first in argv** wins.

### 3.2 Complete flag table

Order below is the order of the `if` chain in the main loop
(`src/musializer.c:398-551`), which is also the order a Rust parser should
match arms in to be behaviourally identical.

| Flag | Values | Applied | Failure | Line |
| --- | --- | --- | --- | --- |
| `--mute` | none | **immediately**, `SetMasterVolume(0.0f)` | — | `:399-405` |
| `--scene NAME` | 1 | immediately, `plug_select_scene` | warn + error | `:406-412` |
| `--ascii-image FILE` | 1 | immediately: `plug_load_ascii_image` then `plug_select_scene("ascii")` | warn + error | `:413-422` |
| `--event SPEC` | 1 | immediately, `plug_record_event` | warn + error | `:423-432` |
| `--route SPEC` | 1 | **deferred** to after the loop | warn + error | `:433-452` |
| `--render FILE` | 1 | stored; render starts after all setup | warn + error if missing value | `:453-461` |
| `--render-window S D` | **2** | stored, applied after routes | warn + error | `:462-475` |
| `--resolution WxH` | 1 | stored, applied after routes | warn + error | `:476-483` |
| `--fps N` | 1 | stored, applied after routes | warn + error | `:484-490` |
| `--quality NAME` | 1 | stored, applied after routes | warn + error if missing value | `:491-499` |
| `--project FILE` | 1 | immediately, `plug_load_project` | warn + error | `:500-506` |
| `--save-project FILE` | 1 | stored, saved near the end | warn + error if missing value | `:507-515` |
| `--analysis-bridge FILE` | 1 | stored, loaded after render config | warn + error if missing value | `:516-524` |
| `--auto-scenes` | none | flag; applied after bridge | — | `:525-528` |
| `--reload-once` | none | flag; applied after ui-probe | — | `:529-532` |
| `--ui-probe SPEC` | 1 | parsed immediately, **applied last** | warn + error | `:533-545` |
| *positional* | — | immediately | warn + error | `:546-550` |

The three pre-pass flags (`-h`, `--help`, `--version`) never reach this loop.

### 3.3 Verdict on the plan's claims

| Plan claim | Verdict |
| --- | --- |
| `--project` | correct, `:500` |
| `--render` | correct, `:453` |
| `--render-window` | correct **but takes two values**, `:462` |
| `--scene` | correct, `:406` |
| `--ascii-image` | correct, `:413` |
| `--event` | correct, `:423` |
| `--route` | correct, `:433` |
| `--mute` | correct, `:399` |
| `--version` | correct, `:322` |
| `-h` / `--help` | correct, `:318` |
| positional audio path | correct, and it also accepts `.musi`, `:546` |
| routes applied after every positional and `--project` | **correct**, `:553-561`, rationale comment at `:446-448` |

**Eight flags the plan missed**, all real and all reachable:

1. `--save-project FILE` (`:507`) — headless save; sets
   `exit_after_save`, which **skips the main loop entirely** unless `--render`
   is also present (`:617`).
2. `--analysis-bridge FILE` (`:516`) — imports a verified analysis bridge TSV.
3. `--auto-scenes` (`:525`) — enables imported scene suggestions.
4. `--resolution WIDTHxHEIGHT` (`:476`).
5. `--fps N` (`:484`).
6. `--quality NAME` (`:491`) — `balanced` | `high` | `master`.
7. `--reload-once` (`:529`) — exercises exactly one hot-reload handoff.
8. `--ui-probe SPEC` (`:533`) — the headless UI capture hook. This is the
   largest missed surface; see 3.6.

### 3.4 Value grammars

**`--event TYPE:SECONDS:ID:VALUE`** — `parse_command_line_event`,
`src/musializer.c:19-61`.

- `TYPE`: text up to the first `:`, must be non-empty and **strictly under 16
  bytes** (`:23`). Accepted: `lyric`, `semantic`, `cue`, `custom` (`:47-51`)
  mapping to `EVENT_TYPE_LYRIC` / `_SEMANTIC` / `_CUE` / `_CUSTOM`.
- `SECONDS`: `strtod`, must consume up to and stop exactly at a `:` (`:29-30`).
  **The host parser does not reject a negative timestamp** — the comment at
  `:60` says "The plug owns canonical validation and insertion", so
  `plug_record_event` is where the real bound lives.
- `ID`: **digits only**, `[0-9]+`, non-empty, terminated by `:` (`:33-40`). No
  sign, no whitespace, no hex. Parsed with `strtoull`.
- `VALUE`: `strtof`, must consume to `'\0'` (`:44-45`). Exactly one value;
  `value_count = 1`.
- `ERANGE` on any numeric conversion is rejected.
- Result: `Event_Record{ timestamp_seconds, id, type, value_count = 1,
  values = {value} }`.
- Example: `lyric:12.5:1:0.75`.

**`--route PARAM:SOURCE:BAND:IN_MIN:IN_MAX:OUT_MIN:OUT_MAX[:CURVE][:noclamp]`**
— parsed by `scene_route_parse_spec`, `src/scene_routes.c:109-189`, reached via
`plug_add_scene_route` (`src/plug.c:1072-1083`).

- Whole spec must be `< 256` bytes (`SCENE_ROUTE_SPEC_CAPACITY`,
  `src/scene_routes.c:97`, checked `:113`).
- Split on `:` into at most **9** fields (`SCENE_ROUTE_SPEC_MAX_FIELDS`, `:97`).
  Fewer than **7** fields is an error; a 10th `:` is an error (`:127`).
- `PARAM`: a scene setting key from section 5. The `settings.` prefix is
  **optional** — if absent it is prepended (`:130-138`), so `loom.weight` and
  `settings.loom.weight` are the same route. Must resolve via
  `scene_settings_descriptor_by_key`; the scene is **derived from the key**, not
  from the currently selected scene (`:184-188`).
- `SOURCE`: `rms` | `peak` | `spectral_flux` | `beat_phase` | `band`
  (`:140-149`, matched against `musi_analysis_source_name`). *Post-legacy
  (UX0-C15, 2026-08-04): the Rust rewrite additionally accepts `time`, an
  eight-second triangle clock. This inventory documents the frozen C only;
  the C cannot parse a `time` route.*
- `BAND`: `strtoul`, must consume the whole field, `<= 0xFFFF` (`:152-153`).
  Then `scene_route_valid` (`src/scene_routes.c:45-49`) requires: if
  `SOURCE == band` then `BAND < 256` (`AUDIO_ANALYZER_MAX_BANDS`,
  `src/audio_analyzer.h:9`); **if `SOURCE != band` then `BAND` must be exactly
  `0`**.
- `IN_MIN`, `IN_MAX`, `OUT_MIN`, `OUT_MAX`: `strtod`, must consume the whole
  field, must be finite (`:99-107`). Validation (`:52-61`) requires
  `IN_MAX > IN_MIN` **strictly**, both output endpoints finite, and
  **`OUT_MIN != OUT_MAX`** — a flat mapping is rejected outright, with the
  rationale at `src/scene_routes.c:57-60` (it would be byte-identical to a
  plain slider and would lose its authoring identity on reopen).
- Fields 8 and 9 are **order-free optional tokens** (`:163-182`). Each must be
  one of `clamp`, `noclamp`, `step`, `linear`, `smoothstep`, `ease_in`,
  `ease_out`. Anything else is an error. Repeats are allowed and last-wins.
- Defaults when the tokens are absent: `interpolation = linear`,
  `clamp = true` (`:161-162`).
- Duplicate `PARAM` within one scene is rejected
  (`scene_route_table_add`, `src/scene_routes.c:71-75`); at most **12** routes
  per scene (`SCENE_ROUTES_PER_SCENE = SCENE_SETTINGS_MAX_CONTROLS`,
  `src/scene_routes.h:16`).
- Host-side cap: at most **256** `--route` occurrences
  (`COMMAND_LINE_ROUTE_CAPACITY`, `src/musializer.c:11`); the 257th warns and
  errors (`:440-444`).
- If no track is loaded, routes land in `p->pending_scene_routes` rather than
  the track (`src/plug.c:1078-1079`).
- Example from the help text: `loom.weight:band:2:0:1:0.4:2.2:smoothstep`.

**`--render-window START DURATION`** — `src/musializer.c:462-475`, two separate
argv words.

- Both parsed by `parse_seconds` (`:75-85`): `strtod`, must consume the whole
  string, must be finite and `>= 0`, must be non-empty.
- `DURATION` must additionally be `> 0.0` (`:466`).
- Applied after routes via `plug_configure_render_window`
  (`src/musializer.c:571-577`); exact frame bounds are re-validated against the
  decoded transport when the render actually starts
  (`src/plug.c:7174-7175`).
- **Index-advance quirk to reproduce or deliberately fix**:
  `i += i + 2 < argc ? 2 : (argc - 1 - i);` at `src/musializer.c:473`. When
  fewer than two values remain, it advances to the last index instead of
  consuming two, so `musializer --render-window 5` errors and stops cleanly
  rather than looping.
- Docs say the frames must match the same span of a full render
  (`src/musializer.c:279-280`).

**`--resolution WIDTHxHEIGHT`** — `parse_resolution`, `src/musializer.c:87-100`.
Separator is the literal lowercase `x`; a second `x` anywhere after it is an
error; the width text must be `< 16` bytes. Both halves go through
`parse_positive_u32` (`:63-73`): `strtoul`, whole-string, **non-zero**,
`<= UINT32_MAX`. `plug_configure_render` (`src/plug.c:7145-7169`) then rejects
a half-specified pair (`(width == 0) != (height == 0)`) and runs
`render_export_config_validate`.

**`--fps N`** — `parse_positive_u32`, so a positive decimal integer only. `0`
is rejected. Fractional and rational FPS are not expressible on the CLI even
though the project schema stores an exact rational (see 6.3).

**`--quality NAME`** — string compared in `plug_configure_render`,
`src/plug.c:7157-7160`: `balanced`, `high`, `master`. Anything else fails
validation and produces the message at `src/musializer.c:566-568`.

**`--scene NAME`** — `scene_id_from_name`, `src/plug.c:933-964`. Accepts the ten
short names **and six long aliases**:

| Accepted spellings | Scene |
| --- | --- |
| `spectrum` | Spectrum |
| `pulse`, `pulse-field` | Pulse Field |
| `orbital`, `orbital-lattice` | Orbital Lattice |
| `ascii`, `ascii-field` | ASCII Field |
| `atlas`, `song-atlas` | Song Atlas |
| `terrarium`, `spectral-terrarium` | Spectral Terrarium |
| `constellation` | Constellation |
| `cadence` | Cadence |
| `loom` | Loom |
| `pentagram`, `pentagram-orbits` | Pentagram Orbits |

Comparison is exact and case-sensitive `strcmp`. Selecting a scene can also
fail for a reason unrelated to the name: `plug_select_scene`
(`src/plug.c:984-986`) refuses a change while the route editor holds the active
context. Selecting a scene resets `scene_switches.enabled` to false and clears
the switch plan (`src/plug.c:966-977`).

**`--ui-probe`** — see 3.6.

### 3.5 Positional arguments, unknown flags, and errors

`src/musializer.c:546-550` is the fall-through arm:

```c
if (IsFileExtension(argv[i], ".musi") ?
    !plug_load_project(argv[i]) : !plug_load_track(argv[i])) {
```

- Extension `.musi` (raylib `IsFileExtension`, case-insensitive) → load as a
  project. Anything else → load as an audio track.
- There is **no arity limit**; every unmatched argv word is treated as an input
  and loaded in argv order, so a later `--project` or positional can overwrite
  earlier state. This is exactly why routes are deferred.
- **Any unrecognized flag falls into this arm.** `musializer --typo` warns
  `Could not load command-line track: --typo` and exits `1`. There is no
  "unknown option" diagnostic and no `--` end-of-options marker.

Error model: every failure sets one shared `bool command_line_error`
(`src/musializer.c:384`) and logs a `LOG_WARNING`; the loop keeps going.
`exit_status = command_line_error ? 1 : 0` (`:618`). Once set, it poisons the
later stages by short-circuit: analysis-bridge (`:579-584`), auto-scenes
(`:585-588`), save-project (`:589-593`), and ui-probe (`:598`) all refuse to
run if `command_line_error` is already true. Render start and `--reload-once`
are likewise gated on `exit_status == 0` (`:619`, `:630`). A render that is
still active or has failed at loop exit forces `1` (`:653-655`).

### 3.6 `--ui-probe` grammar

`parse_ui_probe`, `src/musializer.c:131-250`. One argv word:
`key=value[,key=value...]`.

- Total spec length must be non-zero and `< 256` bytes (`:136-138`).
- Split on `,` then on the **first** `=`. An empty key, an empty value, or a
  missing `=` is an error (`:155-156`).
- **Every key may appear at most once**; a repeat is an error, not last-wins.
  An unknown key is an error (`:244-246`). The rationale is at `:128-130`: a
  typo in a capture script must not quietly photograph the wrong UI state.

| Key | Value grammar | Effect | Line |
| --- | --- | --- | --- |
| `panel` | `none` \| `tune` \| `export` \| `lyrics` \| `assist` | `Plug_Ui_Panel` | `:111-119`, `:161-165` |
| `fullscreen` | `0` \| `1` exactly | `probe.fullscreen` | `:121-126`, `:166-172` |
| `play` | `0` \| `1` exactly | `probe.playing`; transport is parked unless `play=1` | `:172-176` |
| `lyric` | `strtoul`, whole-string, `1..4096` inclusive; `0` rejected | `probe.lyric_selection`, selects the nth lyric cue | `:177-187` |
| `zoom` | `strtod`, whole-string, finite, `1.0..100000.0` inclusive | `probe.timeline_zoom`; `1` = whole track | `:188-197` |
| `style` | literally `caption`, nothing else | `probe.caption_style_pane = true`; needs `panel=lyrics` | `:198-201` |
| `fonts` | `consent`, **or** any other string treated as a filesystem path | `probe.font_browser = true`; `consent` shows the network-consent panel, a path loads a family list from disk so a capture never contacts Google. Path must fit `PLUG_UI_PROBE_PATH_CAPACITY` | `:202-217` |
| `assist` | literally `confirm`, nothing else | `probe.assist_confirmation = true` | `:218-221` |
| `lyrics-file` | any path that fits the capacity | `probe.lyrics_reference_path`; selects an authored lyric sheet for the next Assist lyrics run | `:222-231` |
| `time` | `parse_seconds`: finite, `>= 0`, whole-string | `probe.seek_seconds` **and sets `seek_requested = true`** | `:232-237` |
| `size` | `parse_resolution`, i.e. `WIDTHxHEIGHT`, both positive | host-side window geometry, not plug state | `:238-243` |

Application is deliberately last, `src/musializer.c:598-614`, and only if
`command_line_error` is false:

1. If `size` was given, `SetWindowSize(width, height)`. **GLFW clamps to the
   `SetWindowMinSize(960, 640)` floor set at `:354`**, so a deliberately tiny
   probe photographs the smallest layout the app actually permits (`:599-601`).
2. `SetWindowPosition(0, 0)` unconditionally, so a capture of a display sized
   to the window needs no guesswork about compositor placement (`:605-607`).
3. `plug_apply_ui_probe(ui_probe.probe)`. Failure warns "a panel or seek probe
   needs a loaded, seekable track" and sets `command_line_error` (`:608-613`).

Documented semantic constraints from the help text (`src/musializer.c:288-309`):
every panel except `none` needs a loaded track; `style=caption` and `fonts=`
need `panel=lyrics`; `lyric=N` needs `panel=assist`; audio-reactive scenes need
`play=1` but then capture a frame that is not reproducible.

### 3.7 Full application order

1. Pre-pass `-h`/`--help`/`--version` over all argv → may exit `0`.
2. `SIGPIPE` → `SIG_IGN` on non-Windows (`:327-335`; rationale at `:328-331`).
3. `reload_libplug()` (`:337`).
4. `SetConfigFlags(FLAG_WINDOW_RESIZABLE | FLAG_WINDOW_ALWAYS_RUN |
   FLAG_WINDOW_HIGHDPI | FLAG_MSAA_4X_HINT)`, `InitWindow(1280, 720)` (factor
   80 × 16:9, `:346-349`), `SetWindowMinSize(960, 640)`, window icon from
   `./resources/logo/logo-256.png`, `SetExitKey(KEY_NULL)` (`:354-364`).
5. `InitAudioDevice()`, then `SetAudioStreamBufferSizeDefault(8192)`
   (`PREVIEW_AUDIO_BUFFER_FRAMES`, `:10`; rationale at `:371-375` — decode-ahead
   headroom, not output latency, and it only affects streams created after the
   call).
6. `plug_init()` (`:380`).
7. The argv loop, left to right (3.2).
8. Deferred routes, in argv order (`:553-561`).
9. Render config, only if any of width / fps / quality was set (`:563-569`).
10. Render window (`:571-577`).
11. Analysis bridge (`:579-584`).
12. Auto-scenes (`:585-588`).
13. Save project (`:589-593`).
14. **`plug_mark_command_line_state_clean()`** (`:597`). Rationale at
    `:594-596`: startup configuration is not an edit, so autosave must not write
    a one-off `--resolution 2560x1440` permanently into a project opened with
    `--project`.
15. UI probe (`:598-614`).
16. `--reload-once` handoff: `plug_pre_reload` → `reload_libplug` →
    `plug_post_reload`; a `NULL` state is a veto and an error (`:619-629`).
17. `plug_start_render` if `--render` (`:630-636`).
18. Main loop, unless `exit_after_save` (`:637-651`). `KEY_H` triggers an
    interactive hot-reload handoff (`:639-648`).
19. `plug_shutdown`, `CloseAudioDevice`, `CloseWindow`, return `exit_status`.

---

## 4. Scene registry

Registry array: `src/scene.c:17-28`, `scene_registry[COUNT_SCENES]` with
designated initializers, so the array index **is** the `Scene_Id` and the order
below is normative. Enum: `src/scene.h:17-29`. Stable CLI/persistence names:
`scene_stable_name`, `src/scene.c:47-63`. Display names: the `.name` field of
each descriptor.

| # | Enum (`src/scene.h`) | Stable name (`src/scene.c:50-59`) | UI display name | `state_version` | Implementing file |
| --- | --- | --- | --- | --- | --- |
| 0 | `SCENE_SPECTRUM` | `spectrum` | `Spectrum` | 1 | `src/scene_spectrum.c:149-155` |
| 1 | `SCENE_PULSE_FIELD` | `pulse` | `Pulse Field` | 1 | `src/scene_pulse_field.c:124-131` |
| 2 | `SCENE_ORBITAL_LATTICE` | `orbital` | `Orbital Lattice` | 2 | `src/scene_orbital_lattice.c:314-321` (+ `src/scene_orbital_lattice_motion.c`) |
| 3 | `SCENE_ASCII_FIELD` | `ascii` | `ASCII Field` | 4 | `src/scene_ascii_field.c:445-452` |
| 4 | `SCENE_SONG_ATLAS` | `atlas` | `Song Atlas` | 4 | `src/scene_song_atlas.c:723-730` (+ `src/song_atlas_map.c`) |
| 5 | `SCENE_SPECTRAL_TERRARIUM` | `terrarium` | `Spectral Terrarium` | 2 | `src/scene_spectral_terrarium.c:608-615` |
| 6 | `SCENE_CONSTELLATION` | `constellation` | `Constellation` | 2 | `src/scene_constellation.c:322-329` (+ `src/scene_constellation_motion.c`) |
| 7 | `SCENE_CADENCE` | `cadence` | `Cadence` | 1 | `src/scene_cadence.c:473-480` (+ `src/scene_cadence_timing.c`) |
| 8 | `SCENE_LOOM` | `loom` | `Loom` | 2 | `src/scene_loom.c:391-398` (+ `src/scene_loom_weave.c`) |
| 9 | `SCENE_PENTAGRAM` | `pentagram` | `Pentagram Orbits` | 1 | `src/scene_pentagram.c:436-443` |

`COUNT_SCENES == 10` (`src/scene.h:28`) and
`SCENE_SETTINGS_SCENE_COUNT == 10` (`src/scene_settings_values.h:8`). These two
constants are independent in the C and must be kept in lockstep; the Rust side
should derive one from the other.

Notes for the port:

- `scene_stable_name` falls back to `"spectrum"` for an out-of-range id
  (`src/scene.c:62`), and `scene_name` falls back to the literal `"Unknown"`
  (`src/scene.c:44`). Neither ever returns NULL.
- Only `SCENE_SPECTRUM` has `state_size == 0` and no `init`/`update`/`unload` —
  it is a pure draw function (`src/scene_spectrum.c:149-155`).
- `state_version` is the hot-reload / rebind compatibility key.
  `scene_instance_rebind` (`src/scene.c:105-124`) reallocates state whenever
  version or size changed, and deliberately does **not** call the old `unload`
  because the old descriptor may have vanished during reload
  (`src/scene.c:118-120`). Scene state may therefore own only plain memory or
  resources released by the plug's pre-reload hook. That invariant survives the
  rewrite.

---

## 5. Scene settings contract

Source: the descriptor tables at `src/scene_settings.c:13-122`, built with two
macros at `src/scene_settings.c:8-11`:

```c
#define SETTING(key_, label_, min_, max_, default_, precision_) \
    { key_, label_, min_, max_, default_, precision_, SCENE_SETTING_SLIDER }
#define TOGGLE(key_, label_, default_) \
    { key_, label_, 0.0f, 1.0f, default_, 0, SCENE_SETTING_TOGGLE }
```

Descriptor struct: `src/scene_settings.h:55-63` —
`{ const char *key; const char *label; float minimum; float maximum;
float default_value; unsigned precision; Scene_Setting_Kind kind; }`.
Kinds: `SCENE_SETTING_SLIDER = 0`, `SCENE_SETTING_TOGGLE = 1`
(`src/scene_settings.h:14-17`).

**`precision` is decimal places for display, not a step.** There is no step
field. A slider is continuous within `[minimum, maximum]`; `precision` is `2`
for fractional controls and `0` for integer-presenting controls (counts, degrees).
`TOGGLE` forces `min = 0.0`, `max = 1.0`, `precision = 0`, and the validator
additionally requires the value be **exactly** `0.0f` or `1.0f`
(`value_in_range`, `src/scene_settings.c:143-149`).

Storage is `float values[10][12]` (`src/scene_settings.h:65-67`) with
`SCENE_SETTINGS_MAX_CONTROLS = 12` (`src/scene_settings_values.h:9`). Only the
atlas scene actually uses all 12.

Per-scene index enums are in `src/scene_settings.h:19-53` and are 0-based in
table order.

### 5.0 Scene index → table

`tables[]`, `src/scene_settings.c:129-141`, in `Scene_Id` order: spectrum(8),
pulse(8), orbital(9), ascii(6), atlas(12), terrarium(8), constellation(8),
cadence(7), loom(7), pentagram(8). **75 controls total.**

### 5.1 Spectrum — 8 controls, `src/scene_settings.c:13-22`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.spectrum.amplitude` | Amplitude | 0.40 | 2.00 | **1.00** | 2 | slider |
| 1 | `settings.spectrum.trail` | Trail size | 0.25 | 2.50 | **1.00** | 2 | slider |
| 2 | `settings.spectrum.saturation` | Saturation | 0.25 | 1.25 | **1.00** | 2 | slider |
| 3 | `settings.spectrum.glow_softness` | Glow softness | 1.00 | 8.00 | **3.00** | 2 | slider |
| 4 | `settings.spectrum.hue_swing` | Semantic hue swing | 0.00 | 120.00 | **55.00** | 0 | slider |
| 5 | `settings.spectrum.core_glow` | Core glow size | 0.00 | 3.00 | **1.00** | 2 | slider |
| 6 | `settings.spectrum.bar_taper` | Bar taper | 0.30 | 1.50 | **0.50** | 2 | slider |
| 7 | `settings.spectrum.reflection` | Floor reflection | 0.00 | 1.00 | **0.30** | 2 | slider |

### 5.2 Pulse Field — 8 controls, `src/scene_settings.c:24-33`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.pulse.scale` | Field scale | 0.55 | 1.15 | **1.00** | 2 | slider |
| 1 | `settings.pulse.rings` | Ring count | 6.00 | 48.00 | **24.00** | 0 | slider |
| 2 | `settings.pulse.motion` | Rotation speed | 0.00 | 2.00 | **1.00** | 2 | slider |
| 3 | `settings.pulse.arc` | Arc length | 0.50 | 1.50 | **1.00** | 2 | slider |
| 4 | `settings.pulse.weight` | Line weight | 0.30 | 2.50 | **1.00** | 2 | slider |
| 5 | `settings.pulse.petals` | Petal fold (0 = auto) | 0.00 | 12.00 | **0.00** | 0 | slider |
| 6 | `settings.pulse.hue` | Hue shift (deg) | -180.0 | 180.0 | **0.0** | 0 | slider |
| 7 | `settings.pulse.glow` | Center bloom | 0.00 | 2.00 | **1.00** | 2 | slider |

### 5.3 Orbital Lattice — 9 controls, `src/scene_settings.c:35-45`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.orbital.motion` | Motion speed | 0.15 | 2.00 | **1.00** | 2 | slider |
| 1 | `settings.orbital.radius` | Lattice radius | 0.55 | 1.45 | **1.00** | 2 | slider |
| 2 | `settings.orbital.depth` | Depth spacing | 0.55 | 1.55 | **1.00** | 2 | slider |
| 3 | `settings.orbital.nodes` | Node size | 0.35 | 2.20 | **1.00** | 2 | slider |
| 4 | `settings.orbital.links` | Link weight | 0.00 | 2.20 | **1.00** | 2 | slider |
| 5 | `settings.orbital.tilt` | Camera tilt | 0.00 | 2.00 | **1.00** | 2 | slider |
| 6 | `settings.orbital.hue` | Hue shift (deg) | -180.0 | 180.0 | **0.0** | 0 | slider |
| 7 | `settings.orbital.reactivity` | Beat reactivity | 0.00 | 2.00 | **1.00** | 2 | slider |
| 8 | `settings.orbital.sway` | Drift & sway | 0.00 | 2.00 | **1.00** | 2 | slider |

### 5.4 ASCII Field — 6 controls, `src/scene_settings.c:47-54`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.ascii.motion` | Wave motion | 0.00 | 2.00 | **1.00** | 2 | slider |
| 1 | `settings.ascii.cycling` | Glyph cycling | 0.00 | 2.00 | **1.00** | 2 | slider |
| 2 | `settings.ascii.scanlines` | Scanlines | 0.00 | 2.00 | **1.00** | 2 | slider |
| 3 | `settings.ascii.split` | Color split | 0.00 | 2.00 | **1.00** | 2 | slider |
| 4 | `settings.ascii.gain` | Brightness gain | 0.50 | 2.50 | **1.30** | 2 | slider |
| 5 | `settings.ascii.tint` | Band hue spread | 0.00 | 1.00 | **0.45** | 2 | slider |

### 5.5 Song Atlas — 12 controls (the only scene at capacity), `src/scene_settings.c:56-69`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.atlas.height` | Terrain height | 0.35 | 2.75 | **1.00** | 2 | slider |
| 1 | `settings.atlas.width` | Terrain width | 0.55 | 3.20 | **1.00** | 2 | slider |
| 2 | `settings.atlas.depth` | Depth spacing | 0.50 | 1.65 | **1.00** | 2 | slider |
| 3 | `settings.atlas.camera` | Camera height | 0.25 | 1.75 | **1.00** | 2 | slider |
| 4 | `settings.atlas.contours` | Contour weight | 0.00 | 2.50 | **1.00** | 2 | slider |
| 5 | `settings.atlas.color` | Hue shift (deg) | -180.0 | 180.0 | **0.0** | 0 | slider |
| 6 | `settings.atlas.speed` | Camera drift | 0.00 | 2.50 | **1.00** | 2 | slider |
| 7 | `settings.atlas.wireframe` | Surface style | 0.0 | 1.0 | **0.0** | 0 | **toggle** |
| 8 | `settings.atlas.detail` | Sampling detail | 1.00 | 3.00 | **1.00** | 0 | slider |
| 9 | `settings.atlas.hue_motion` | Hue motion | 0.0 | 1.0 | **0.0** | 0 | **toggle** |
| 10 | `settings.atlas.orbit` | Camera orbit | -180.0 | 180.0 | **0.0** | 0 | slider |
| 11 | `settings.atlas.zoom` | Camera distance | 0.60 | 1.80 | **1.00** | 2 | slider |

### 5.6 Spectral Terrarium — 8 controls, `src/scene_settings.c:71-80`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.terrarium.motion` | Camera motion | 0.00 | 2.00 | **1.00** | 2 | slider |
| 1 | `settings.terrarium.growth` | Plant growth | 0.40 | 1.80 | **1.00** | 2 | slider |
| 2 | `settings.terrarium.particles` | Particle size | 0.00 | 2.20 | **1.00** | 2 | slider |
| 3 | `settings.terrarium.sim_speed` | Ecosystem motion | 0.20 | 2.50 | **1.00** | 2 | slider |
| 4 | `settings.terrarium.creature_speed` | Creature speed | 0.20 | 2.00 | **1.00** | 2 | slider |
| 5 | `settings.terrarium.glass_opacity` | Habitat glass | 0.00 | 0.40 | **0.13** | 2 | slider |
| 6 | `settings.terrarium.density` | Population density | 0.30 | 1.00 | **1.00** | 2 | slider |
| 7 | `settings.terrarium.creature_glow` | Creature glow | 0.00 | 2.00 | **1.00** | 2 | slider |

Note `density` defaults to its **maximum**, not its midpoint.

### 5.7 Constellation — 8 controls, `src/scene_settings.c:82-91`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.constellation.motion` | Camera motion | 0.00 | 2.00 | **1.00** | 2 | slider |
| 1 | `settings.constellation.scale` | Field scale | 0.50 | 1.60 | **1.00** | 2 | slider |
| 2 | `settings.constellation.glow` | Node glow | 0.00 | 2.20 | **1.00** | 2 | slider |
| 3 | `settings.constellation.event_duration` | Event glow duration | 0.50 | 5.00 | **2.40** | 2 | slider |
| 4 | `settings.constellation.event_reach` | Event spread | 1.00 | 6.00 | **2.00** | 0 | slider |
| 5 | `settings.constellation.hue_swing` | Semantic hue swing | 0.00 | 140.00 | **70.00** | 0 | slider |
| 6 | `settings.constellation.density` | Star density | 1.00 | 3.00 | **3.00** | 0 | slider |
| 7 | `settings.constellation.web` | Web brightness | 0.00 | 2.00 | **1.00** | 2 | slider |

Note `density` defaults to its **maximum**.

### 5.8 Cadence — 7 controls, `src/scene_settings.c:93-101`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.cadence.scale` | Type scale | 0.55 | 1.35 | **1.00** | 2 | slider |
| 1 | `settings.cadence.swarm` | Swarm spread | 0.00 | 2.00 | **1.00** | 2 | slider |
| 2 | `settings.cadence.focus` | Focus speed | 0.50 | 3.00 | **1.00** | 2 | slider |
| 3 | `settings.cadence.beat` | Beat breathing | 0.00 | 2.00 | **1.00** | 2 | slider |
| 4 | `settings.cadence.glow` | Particle glow | 0.00 | 2.00 | **1.00** | 2 | slider |
| 5 | `settings.cadence.spacing` | Letter spacing | 0.50 | 2.00 | **1.00** | 2 | slider |
| 6 | `settings.cadence.hue_swing` | Semantic hue swing | 0.00 | 160.00 | **80.00** | 0 | slider |

### 5.9 Loom — 7 controls, `src/scene_settings.c:103-111`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.loom.density` | Thread density | 0.50 | 2.00 | **1.00** | 2 | slider |
| 1 | `settings.loom.weight` | Thread weight | 0.40 | 2.50 | **1.00** | 2 | slider |
| 2 | `settings.loom.complexity` | Weave complexity | 0.50 | 2.00 | **1.00** | 2 | slider |
| 3 | `settings.loom.edge` | Growth edge | 0.50 | 2.00 | **1.00** | 2 | slider |
| 4 | `settings.loom.saturation` | Saturation | 0.30 | 1.30 | **1.00** | 2 | slider |
| 5 | `settings.loom.motion` | Beat lift | 0.00 | 2.00 | **1.00** | 2 | slider |
| 6 | `settings.loom.glints` | Onset glints | 0.00 | 2.00 | **1.00** | 2 | slider |

### 5.10 Pentagram Orbits — 8 controls, `src/scene_settings.c:113-122`

| # | Key | Label | Min | Max | **Default** | Prec | Kind |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `settings.pentagram.motion` | Orbit speed | 0.00 | 2.50 | **1.00** | 2 | slider |
| 1 | `settings.pentagram.nest` | Nest curves | 2.00 | 12.00 | **9.00** | 0 | slider |
| 2 | `settings.pentagram.orbits` | Orbit count | 4.00 | 24.00 | **14.00** | 0 | slider |
| 3 | `settings.pentagram.glow` | Spark glow | 0.00 | 2.20 | **1.00** | 2 | slider |
| 4 | `settings.pentagram.chords` | Pentagram lines | 0.00 | 2.00 | **1.00** | 2 | slider |
| 5 | `settings.pentagram.hue` | Hue shift (deg) | -180.0 | 180.0 | **-91.0** | 0 | slider |
| 6 | `settings.pentagram.pulse` | Music coupling | 0.00 | 2.00 | **1.00** | 2 | slider |
| 7 | `settings.pentagram.zoom` | Field scale | 0.60 | 1.50 | **1.00** | 2 | slider |

**`settings.pentagram.hue` defaults to `-91.0`, not `0.0`.** It is the only
hue control that does not default to zero and the single most likely default to
get wrong.

### 5.11 Behaviour the settings API guarantees

- `scene_settings_init` (`src/scene_settings.c:164-173`): zeroes the whole
  struct, then writes each descriptor default. Indices beyond a scene's control
  count stay `0.0f` and are never validated.
- `value_in_range` (`:143-149`): rejects non-finite, out-of-`[min,max]`, and —
  for toggles — anything that is not exactly `0.0f` or `1.0f`.
- `scene_settings_get` (`:187-198`): **self-healing**. If the stored value is
  out of range it silently returns the descriptor default. If the *descriptor*
  does not exist it returns `1.0f`, not `0.0f`.
- `scene_settings_set` (`:200-208`): rejects an out-of-range value rather than
  clamping.
- `scene_settings_reset_scene` (`:210-217`): restores that scene's defaults.
- **Legacy snapshot back-fill.** `scene_settings_count_is_legacy`
  (`:239-251`) enumerates every historical per-scene control count that must
  remain loadable: spectrum `3` or `7`; pulse `5`; orbital `5` or `7`; ascii
  `4`; atlas `8` or `10`; terrarium `3` or `7`; constellation `3` or `7`. Cadence,
  loom, and pentagram have no legacy counts. `scene_settings_apply_snapshot`
  (`:268-282`) copies the present values and **back-fills the rest from
  descriptor defaults**. This is a load-bearing compatibility contract: an old
  `.musi` with a 3-value spectrum snapshot must still open, and the missing five
  values must come from the defaults above.
- Presets: `SCENE_SETTINGS_PRESETS_PER_SCENE = 8`
  (`src/scene_settings_values.h:10`), name capacity `129` bytes including the
  terminator (`src/scene_settings.h:11`), ids nonzero and below `next_id` which
  starts at `1` (`src/scene_settings.c:284-289`).
- `Scene_Settings_Snapshot` (`src/scene_settings_values.h:13-17`) is
  `{ bool captured; size_t count; float values[12]; }`; an uncaptured snapshot
  must have `count == 0` (`:257`).

---

## 6. Schemas

### 6.1 All twelve files in `schemas/`

| File | Governs |
| --- | --- |
| `analysis-cache-v1.schema.json` | Envelope for cached remote analysis responses (`$id` `.../analysis-cache-v1`, title "Musializer remote analysis cache envelope v1"). |
| `analysis-provenance-v1.schema.json` | Which adapter, version, model, provider, and prompt produced an analysis artifact. Mirrors the `provenance` block embedded in `.musi`. |
| `codex-lyric-review-output-v1.schema.json` | Output contract for the Codex-driven lyric review tool. The only schema in the directory with **no `$id` and no `title`**. |
| `font-import-v1.schema.json` | What `tools/google_fonts.py` writes after retrieving one caption face. **Rewrite must satisfy this** — see 6.4. |
| `lyric-review-v1.schema.json` | Evidence-preserving lyric review records. |
| `lyric-sync-v1.schema.json` | Deterministic localization of authored reference lyrics; display text is authored truth, timing is acoustic evidence, and an unlocatable line is flagged, never absent. Governs both `lyrics.sync.json` and `lyrics.aligned.json` — see 6.5 for the tranche-LT1 additions. |
| `lyric-timing-v1.schema.json` | Imported lyric timing (the `lyric_timing` analysis lane artifact). |
| `measured-analysis-v1.schema.json` | Offline measured-audio analysis output (the `measured_signal` lane artifact). Largest of the ancillary schemas at 7.1 KB. |
| `project-v1.schema.json` | **Canonical `.musi` project contract.** Rewrite must satisfy this — see 6.2/6.3. 24 KB. |
| `scene-plan-v1.schema.json` | Deterministic scene-switch plan (the imported suggestion sequence behind `--auto-scenes`). |
| `semantic-notes-v1.schema.json` | Imported free-form MiMo interpretation notes. |
| `semantic-score-v1.schema.json` | MiMo SemanticScore (the `semantic_score` lane artifact: energy / tension / valence / confidence). |

All except `codex-lyric-review-output-v1` use `$id`
`https://musializer.local/schemas/...` — **except** `font-import-v1`, which
uses `https://musializer.invalid/schemas/...`
(`schemas/font-import-v1.schema.json:3`). That inconsistency is in the frozen
tree; the port should not "fix" it without a decision, because tooling may
match on the string.

### 6.2 `project-v1.schema.json` — top level

`schemas/project-v1.schema.json:1-108`. `"additionalProperties": false` at
`:8`, so **unknown fields are rejected, not ignored**. The C codec agrees and
returns `MUSI_PROJECT_IO_ERROR_UNKNOWN_FIELD`
(`tests/test_project_io.c:200`).

Portability rule that JSON Schema alone cannot express, stated in `$comment` at
`:6`: **every `maxLength` is also the maximum UTF-8 byte count, and `U+0000`
is forbidden.** JSON Schema `maxLength` counts code points; the bounded C codec
counts bytes. Producers must honour the stricter byte rule.

Required (`:9-18`): `schema_version`, `metadata`, `audio`, `output`,
`deterministic_seed`, `scenes`, `cues`, `analysis_lanes`.

Optional: `ascii_image`, `caption_style`, `lyrics`, `scene_switches`,
`scene_presets`, `semantic_events`, `manual_events`.

| Field | Type / bounds | Default | Line |
| --- | --- | --- | --- |
| `schema_version` | `const "musializer.project/v1"` | — | `:20-22` |
| `metadata` | object, see below | — | `:23-25` |
| `audio` | object, see below | — | `:26-28` |
| `ascii_image` | `null` or `ascii_image_asset` | `null` | `:29-40` |
| `caption_style` | object, see 6.3 | shipped defaults when absent | `:41-44` |
| `output` | object, see below | — | `:45-47` |
| `deterministic_seed` | `uint64`, `0 .. 18446744073709551615` | — | `:48-50`, `:923-927` |
| `scenes` | array, **`minItems 1`**, `maxItems 32`; layer order is back to front; `instance_id` unique | — | `:51-59` |
| `cues` | array, `maxItems 256` | — | `:60-67` |
| `analysis_lanes` | array, `maxItems 8`; at most one of each `kind`; every `audio_sha256` must equal `audio.sha256` | — | `:68-75` |
| `lyrics` | object | — | `:76-78` |
| `scene_switches` | object | — | `:79-81` |
| `scene_presets` | array, `maxItems 80` | `[]` | `:82-90` |
| `semantic_events` | array, `maxItems 1024` | `[]` | `:91-99` |
| `manual_events` | array, `maxItems 1024`; **user-authored only** — model-derived semantic events must never be promoted into this lane | — | `:100-107` |

Ordering and overlap rules the C validator enforces beyond the schema:

- `cues` must be sorted by `(start_seconds, cue_id)` with unique `cue_id`.
  Ranges are half-open `[start, end)`. Cues for the same scene and parameter
  must not overlap. At a cue end the `to_value` **persists** until the next
  matching cue (`:66`).
- `lyrics.cues` are sorted by `(start_seconds, end_seconds, id)` and
  **overlaps are allowed** here (`:394`) — the opposite of parameter cues.
- `scene_switches`: nonempty cues form **contiguous full-duration coverage**;
  `enabled` is the durable user opt-in (`:857`).

Shared `$defs` primitives:

| `$def` | Definition | Line |
| --- | --- | --- |
| `sha256` | string, `^[0-9a-f]{64}$` — lowercase hex only | `:913-916` |
| `stable_name` | string, 1..64, `^[A-Za-z0-9][A-Za-z0-9._:-]*$` | `:917-922` |
| `uint64` | integer, `0 .. 18446744073709551615` | `:923-927` |
| `positive_uint64` | integer, `1 .. 18446744073709551615` | `:668-672` |
| `caption_rgba` | string, `^[0-9a-f]{8}$` — RGBA, exactly eight **lowercase** hex digits. Uppercase, a leading `#`, and shorthand are all rejected so one colour has one spelling | `:260-264` |

Object definitions:

**`metadata`** (`:512-550`) — required `project_id` (`stable_name`), `title`
(string 1..128), `application_version` (string 1..64). Optional `author`
(≤128, default `""`), `created_utc` (≤32, default `""`), `modified_utc` (≤32,
default `""`).

**`audio_asset`** (`:174-215`) — all six required: `mode` (`imported` |
`referenced`), `path` (1..1024), `sha256`, `duration_seconds`
(`exclusiveMinimum 0`), `sample_rate` (integer 1..768000), `channels`
(integer 1..64).

**`ascii_image_asset`** (`:144-173`) — required `path` (1..1024), `sha256`,
`columns` (1..**96**), `rows` (1..**54**).

**`output_settings`** (`:551-618`) — required `width`, `height`,
`fps_numerator`, `fps_denominator`, `start_seconds`, `end_seconds`, `format`;
optional `quality`.

| Field | Bounds | **Default** |
| --- | --- | --- |
| `width` | integer 16..16384 | **1920** |
| `height` | integer 16..16384 | **1080** |
| `fps_numerator` | integer 1..240240 | **30** |
| `fps_denominator` | integer 1..1001 | **1** |
| `start_seconds` | number, `minimum 0` | **0** |
| `end_seconds` | number, `exclusiveMinimum 0` | — (required) |
| `format` | `mp4_h264` \| `mkv_h264` \| `webm_vp9` \| `mov_prores` \| `png_sequence` | **`mp4_h264`** |
| `quality` | `balanced` \| `high` \| `master` | **`high`** |

Extra rules at `:617`: `end_seconds > start_seconds` and
`end_seconds <= audio.duration_seconds`; FPS is the exact rational
`fps_numerator / fps_denominator` and **must not exceed 240**; **the current
editor supports integer FPS only and rejects other valid v1 rationals without
rewriting them.** `quality` is durable intent — supersampling and encoder
details are derived, never serialized separately (`:614`).

**`scene`** (`:712-765`) — required `instance_id` (`positive_uint64`),
`scene_type` (`stable_name`; the stable scene names from section 4), `enabled`
(bool), `start_seconds` (≥0), `end_seconds` (>0), `opacity` (number 0..1),
`blend_mode` (`normal` | `add` | `multiply` | `screen`), `mappings` (array,
`maxItems 120`). `end_seconds > start_seconds` and
`<= audio.duration_seconds` (`:764`).

**`mapping`** (`:431-511`) — all nine required: `parameter` (`stable_name`),
`source` (`rms` | `peak` | `spectral_flux` | `beat_phase` | `band`; the Rust
rewrite also writes `time` post-legacy, which the C cannot read),
`band_index` (integer 0..65535, default **0**), `input_min`, `input_max`,
`output_min`, `output_max` (numbers), `interpolation` (`step` | `linear` |
`smoothstep` | `ease_in` | `ease_out`), `clamp` (bool, default **true**).
Conditional at `:490-509`: **if `source` is not `band`, `band_index` must be
`0`.** Extra rules at `:510`: `input_max > input_min`; parameter names unique
within their scene; `clamp: false` permits deterministic curve extrapolation.
The C additionally rejects `output_min == output_max`
(`src/scene_routes.c:61`) — a constraint the schema does **not** express.

**`parameter_cue`** (`:619-667`) — all eight required: `cue_id` and
`target_scene_id` (`positive_uint64`), `parameter` (`stable_name`),
`start_seconds` (≥0), `end_seconds` (>0), `from_value`, `to_value` (numbers),
`interpolation` (same five values). `target_scene_id` must identify a scene.

**`analysis_lane`** (`:110-143`) — all five required: `kind`
(`measured_signal` | `lyric_timing` | `semantic_score`), `path` (1..1024),
`sha256`, `audio_sha256`, `provenance`. Referenced artifacts are **not**
required in order to reopen evaluated project data (`:74`).

**`provenance`** (`:673-711`) — required `adapter` (`stable_name`),
`adapter_version` (1..64), `schema_version` (1..64). Optional `model` (≤128,
default `""`), `provider` (≤128, default `""`), `prompt_version` (≤64, default
`""`).

**`lyrics`** (`:375-395`) — required `next_id` (`uint64`, so `0` is legal here)
and `cues` (array, `maxItems 1024`). **`lyric_cue`** (`:347-374`): required
`id` (`positive_uint64`), `start_seconds` (≥0), `end_seconds` (>0), `text`
(string 1..**511**).

**`manual_event`** (`:396-430`) — required `timestamp_seconds` (≥0), `id`
(`positive_uint64`), `type` (`lyric` | `semantic` | `cue` | `custom`),
`values` (array of numbers, `minItems 1`, `maxItems 4`).

**`semantic_event`** (`:859-912`) — same shape, but `type` is
`const "semantic"` and `values` is **exactly 4** items with per-position
bounds via `prefixItems` and `"items": false`:
`[0] energy 0..1`, `[1] tension 0..1`, `[2] valence −1..1`,
`[3] confidence 0..1`.

**`scene_preset`** (`:766-796`) — required `id` (`positive_uint64`),
`scene_name` (`stable_name`), `name` (string 1..128), `settings` (array of
numbers, `minItems 1`, `maxItems 12`).

**`scene_switches`** (`:838-858`) — required `enabled` (bool) and `cues`
(array, `maxItems 256`). **`scene_switch_cue`** (`:797-837`): required `id`
(`positive_uint64`), `start_seconds` (≥0), `end_seconds` (>0), `scene_name`
(`stable_name`), `strength` (number 0..1); optional `settings` (array of
numbers, `maxItems 12`, default `[]`, "Empty only for early v1 projects").

### 6.3 `caption_style` and `caption_font_asset` in detail

`caption_style`, `schemas/project-v1.schema.json:265-346`.

The whole block is optional at the top level, but **once present every one of
its nine members is required** (`:268-279`): "a partially specified style would
silently mix the author's intent with shipped defaults." The C agrees —
`tests/test_project_io.c:834`,
`project_io_rejects_a_half_specified_or_misspelled_caption_style`.

**Post-legacy extension (2026-08-03):** a tenth, optional member `effects`
(glow, soft shadow, plate roundness — `caption_effects` in the schema) that the
frozen C does not know. A default block is never written, so every pre-effects
project keeps its exact C-era serialization; the authoritative description
lives in the schema and `crates/musializer-core/src/project/caption_effects.rs`,
not in this inventory, which documents the C's behaviour only.

Every measurement is a fraction of the frame, never a pixel count, so a project
typeset against a preview window exports identically at any resolution
(`:43`).

| Member | Type / values | Bounds | **Shipped default** | Schema line | C default line |
| --- | --- | --- | --- | --- | --- |
| `face` | enum `alegreya` \| `space_grotesk` \| `imported` | — | **`alegreya`** | `:281-287` | `src/project.c:78`, enum values `src/project.h:91-96` (`ALEGREYA = 0`, `SPACE_GROTESK = 1`, `IMPORTED = 2`) |
| `box` | enum `none` \| `shadow` \| `plate` | — | **`plate`** | `:288-295` | `src/project.c:79`, enum `src/project.h:101-107` (`NONE = 0`, `SHADOW = 1`, `PLATE = 2`) |
| `anchor` | enum, nine values (see below) | — | **`bottom_center`** | `:296-308` | `src/project.c:80`, enum `src/project.h:111-120` |
| `size_scale` | number, fraction of frame **height** | 0.012 .. 0.300 | **0.047** | `:309-314` | `src/project.h:127-129` |
| `margin_scale` | number, inset from anchored edges, fraction of frame height | 0.0 .. 0.400 | **0.065** | `:315-320` | `src/project.h:130-132` |
| `width_scale` | number, widest the box may become, fraction of frame **width** | 0.20 .. 1.00 | **0.82** | `:321-326` | `src/project.h:134-136` |
| `text_rgba` | `caption_rgba`, eight lowercase hex digits | — | **`ffffffff`** (`0xFFFFFFFFu`) | `:327-329` | `src/project.h:137` |
| `box_rgba` | `caption_rgba`; plate fill, or shadow colour when `box == "shadow"`; **ignored when `box == "none"`** | — | **`000000b7`** (`0x000000B7u`, i.e. black at alpha 183) | `:330-333` | `src/project.h:141` |
| `font` | `null` **or** `caption_font_asset` | — | **`null`** | `:334-344` | — |

`box` semantics (`:294`): `none` draws text alone, `shadow` offsets a copy
behind it, `plate` is the rounded panel the product shipped with.

`anchor` enum, in schema order, which matches the C enum values 0..8
(`src/project.h:111-119`): `bottom_left`(0), `bottom_center`(1),
`bottom_right`(2), `middle_left`(3), `middle_center`(4), `middle_right`(5),
`top_left`(6), `top_center`(7), `top_right`(8).

**The `font` / `face` biconditional.** Schema `:343`: `font` is "present
exactly when `face` is `imported`. Any other arrangement is a project whose
captions cannot be reproduced from the file." The C enforces it as a single
expression, `src/project.c:164`:

```c
(caption->face == MUSI_CAPTION_FACE_IMPORTED) != caption->font.present
```

i.e. a mismatch in either direction invalidates the project. Range checks for
the three scales and the three enums are at `src/project.c:152-163`.

`caption_font_asset`, `schemas/project-v1.schema.json:216-259`. Six required
fields (`:219-226`), **and sha256 appears on both the face and its licence**:

| Field | Type / pattern | Notes |
| --- | --- | --- |
| `path` | string 1..1024 | Project-relative path **inside the sibling `<stem>.assets/` bundle**. Verified against `sha256` before the face is used (`:232`). |
| `sha256` | `$defs/sha256`, `^[0-9a-f]{64}$` | Digest of the face. Required, non-empty. |
| `family` | string 1..128 | Display name for the control. **Never used to locate the file** (`:241`). |
| `licence_path` | string, `maxLength 1024`, **no `minLength`** | May be the empty string, for a face imported from the user's own disk whose terms this application cannot assert. Non-empty only together with `licence_sha256` and `licence_name` (`:246`). |
| `licence_sha256` | `^([0-9a-f]{64})?$` — **optional-empty pattern, not `$defs/sha256`** | Digest of the bundled licence text, verified before the text is shown. **Empty exactly when `licence_path` is empty** (`:251`). |
| `licence_name` | string, `maxLength 128`, no `minLength` | Which terms these are, e.g. `OFL-1.1`. Required whenever a licence file is bundled: "an unnamed licence tells a recipient that terms exist without telling them which" (`:256`). |

The three-licence-fields rule is therefore: all empty, or all populated. The
`^([0-9a-f]{64})?$` pattern is the mechanism, and it is easy to get wrong by
reusing `$defs/sha256` — do not.

### 6.4 `font-import-v1.schema.json` in full

`schemas/font-import-v1.schema.json`, 68 lines, `additionalProperties: false`
(`:7`). This is what `tools/google_fonts.py` writes after retrieving one caption
face. Framing from `:5`: "The application re-computes both digests against the
files on disk before either is trusted; this manifest is a description of a
download, never a warrant for it." **Nine required fields, no optional
fields** (`:8-18`).

| Field | Type / pattern | Bounds | Notes | Line |
| --- | --- | --- | --- | --- |
| `schema_version` | `const "musializer.font-import/v1"` | — | — | `:20-22` |
| `family` | string, `^[A-Za-z0-9][A-Za-z0-9 '+.-]*$` | 1..128 | The family as Google Fonts names it. A display label only — the bytes are identified by their digest. Note the character class permits **space, apostrophe, plus, dot, hyphen** and requires an alphanumeric first character | `:23-29` |
| `source` | string, `^https://fonts\.gstatic\.com/` | — | Constrained to the declared host. No `minLength`/`maxLength` | `:30-34` |
| `font_path` | string | `minLength 1` | The downloaded TrueType file, in the job's own output directory | `:35-39` |
| `font_sha256` | string, `^[0-9a-f]{64}$` | — | — | `:40-43` |
| `font_bytes` | integer | **1 .. 33554432** (32 MiB) | Bounded so a redirect to something enormous fails before it fills a disk. Matches `CAPTION_IMPORTED_FONT_BYTE_LIMIT`, `src/plug.c:369` | `:44-49` |
| `licence_path` | string | `minLength 1` | **Always present here** — unlike `caption_font_asset.licence_path`, this may not be empty. A face whose licence could not be retrieved is refused rather than bundled without it, because copying it into a shareable project is redistribution | `:50-54` |
| `licence_sha256` | string, `^[0-9a-f]{64}$` | — | Required and non-empty, again unlike the project-side asset | `:55-58` |
| `licence_name` | **enum**: `OFL-1.1` \| `Apache-2.0` \| `UFL-1.0` | — | Taken from the directory `google/fonts` sorts the family into, **not guessed from the text** | `:59-66` |

The asymmetry between the two licence blocks is deliberate and load-bearing:
an *imported download* must carry a licence (closed enum, both digests
mandatory), whereas a *project-bundled* face may have come from the user's own
disk and therefore may legitimately carry no licence at all (open string, empty
allowed, all-or-nothing).

### 6.5 Asset bundle layout

`musi_project_asset_category_directory`, `src/project_io.c:1272-1279`, and the
enum at `src/project_io.h:54-58`:

| Enum | Value | Directory under `<stem>.assets/` |
| --- | --- | --- |
| `MUSI_PROJECT_ASSET_AUDIO` | 0 | `audio` |
| `MUSI_PROJECT_ASSET_IMAGE` | 1 | `images` |
| `MUSI_PROJECT_ASSET_FONT` | 2 | `fonts` |

`musi_project_asset_category_valid` (`src/project_io.c:1267-1270`) is defined
as "the directory lookup returned non-NULL", with the comment at
`src/project_io.h:60-61`: adding a category without adding it here is a
compile-time-silent, runtime-rejected mistake. Bundled audio is addressed as
`<stem>.assets/audio/<sha256>.<ext>` (`src/track_identity.h:7`); bundled fonts
land in `<stem>.assets/fonts/` (`src/project.h:150`).

Bundle failure modes are a named enum, `Musi_Project_Bundle_Result`
(`src/project_io.h:67-78`): `OK`, `ERROR_ARGUMENT`, `ERROR_PATH`,
`ERROR_DIRECTORY`, `ERROR_SOURCE`, `ERROR_COPY`, `ERROR_SYNC`,
`ERROR_IDENTITY`, `ERROR_COLLISION`, `ERROR_PUBLISH`. Durable-write failures
are a separate enum, `Musi_Project_File_Result` (`src/project_io.h:41-52`),
which distinguishes `ERROR_SYNC`, `ERROR_PUBLISH`, and `ERROR_DURABILITY` —
the rewrite must keep that granularity or lose the atomic-save guarantees the
tests assert.

### 6.6 `lyric-sync-v1` after tranche LT1

`schemas/lyric-sync-v1.schema.json` governs both `lyrics.sync.json` (the coarse
Whisper-derived proposal) and `lyrics.aligned.json` (the acoustic result). The
LT1 fields below are **additive and optional**: the Rust bridge reads none of
them, and `tools/external_analysis.py build_bridge` reads only `lines`, so an
unresolved line cannot become a cue. `tests/test_lyric_anchor_block.py` pins the
lane against this file, because nothing validates it at runtime.

| Field | Type | Notes |
| --- | --- | --- |
| `localization_policy` | string | `anchor-block-mms`. Absent on the coarse lane and on the no-reference per-cue lane |
| `localization_policy_version` | string | Part of the cache identity: an artifact from another policy version is regenerated, never reused |
| `unresolved[]` | array | `reference_line_index`, `text`, `reason`, `abstained`, and the coarse window if one existed. Cues + unresolved always account for every alignable authored line, and the helper refuses to write a lane where they do not |
| `review_flags[]` | array | `flag` is `coarse_disagreement` (coarse proposal versus block placement, > 3 s) or `unresolved`. Never derived from the aligner's score |
| `anchors[]`, `blocks[]` | array | Audit trail for the localization: rare n-grams unique on both sides, and the ordered search partitions they cut |
| `order_violations[]` | array | Timed lines whose start goes backwards against authored order. Cues are emitted sorted by start, so the bridge still parses |
| `generation` | object | `whisper_sha256`, `coarse_sha256`, `reference_sha256` |
| `timing_refinement` | object | Adapter, alignment version, policy versions and request identity. Pre-existing for the per-cue lane; the schema only declared it now |
| `lines[].status` | string | `aligned`, `weak`, or a refusal reason. Audit only |
| `lines[].score`, `first_word_score`, `last_word_score`, `word_alignments` | number / array | The aligner's own CTC evidence. Audit only: `confidence` stays `null`, because the 2026-08-04 adjudication measured the score at 0.139 median on correct lines versus 0.142 on wrong ones |
| `lines[].block_index`, `line_position` | integer | Which block placed the line, and its position among alignable lines |
| `lines[].coarse_start_seconds`, `coarse_end_seconds`, `review_flagged` | number / boolean | The other view and whether the two disagreed |
| `statistics.unresolved_lines`, `abstained_lines`, `review_flagged_lines`, `coarse_disagreement_lines`, `anchor_count`, `block_count`, `evidence_tokens`, `order_violations` | integer | LT1 counters; `matched_tokens` became optional because the anchor lane has no token-match stage |

`assist-manifest.json` gains `result_counts.lyrics_unresolved`,
`result_counts.lyrics_review_flags`, and a `lyric_localization`
`{policy, policy_version}` block (null on the per-cue lane). The Whisper lane's
`provenance.request_settings` gains `text_conditioning` and `vad_model_sha256`;
see 9.3 for `MUSIALIZER_WHISPER_VAD_MODEL`.

---

## 7. Resources

### 7.1 The tree

19 files, 512,570 bytes total.

```
resources/
  fonts/
    Alegreya-Regular.ttf        258860   caption default face
    OFL.txt                       4488   Alegreya licence (SIL OFL 1.1)
    SpaceGrotesk-Regular.otf     79556   UI chrome face, also selectable for captions
    SpaceGrotesk-OFL.txt          4401   Space Grotesk licence (SIL OFL 1.1)
  icons/
    fullscreen.png                4148   loaded
    fullscreen.svg               24648   build input only
    microphone.png                8878   loaded
    microphone.svg                2242   build input only
    play.png                      5188   loaded
    play.svg                      2096   build input only
    render.png                   11610   loaded
    render.svg                    2977   build input only
    volume.png                   23544   loaded
    volume.svg                    6450   build input only
  logo/
    logo-256.ico                 34537   Windows resource compiler only
    logo-256.png                 34768   loaded as the window icon
    logo.svg                      4458   build input only
  shaders/
    glsl120/circle.fs              573   bundled but never selected in this build
    glsl330/circle.fs              608   loaded
```

### 7.2 What the application loads at runtime

Every runtime asset read goes through one indirection, `plug_load_resource`
(declared `src/plug.h:133`).

| Asset | Site |
| --- | --- |
| `./resources/logo/logo-256.png` → window icon | `src/musializer.c:356-362`; `LoadImageFromMemory` at `:359` |
| Five UI icon PNGs (`fullscreen`, `volume`, `play`, `render`, `microphone`) | table at `src/plug.c:124-128`, load loop at `src/plug.c:8152-8157` (→ `LoadImageFromMemory` → `LoadTextureFromImage` → mipmaps + bilinear) |
| `./resources/fonts/SpaceGrotesk-Regular.otf` → UI chrome font | `src/plug.c:8065-8080`, at `FONT_SIZE` 64 (`src/plug.c:103`), restricted to the interface codepoint subset by `ui_font_codepoint()` (`:8074`) |
| The **same** Space Grotesk bytes, loaded a second time with the **full** curated caption codepoint set → `p->caption_alt_font` | `src/plug.c:8092-8102`; the rationale at `:8092-8095` is that the chrome subset would silently drop Greek and Cyrillic in captions |
| `./resources/fonts/Alegreya-Regular.ttf` → caption default face | `src/plug.c:8118-8127`, with a basic-Latin fallback at `:8133` |
| `./resources/shaders/glsl330/circle.fs` | `src/plug.c:8146-8147`, path built as `TextFormat("./resources/shaders/glsl%d/circle.fs", GLSL_VERSION)`; `GLSL_VERSION` is hard-coded **330** at `src/plug.c:100`, so the `glsl120` variant is bundled and never selected in this configuration |

An imported (user-downloaded) caption face is the one exception: it is read with
raylib `LoadFileData` directly, **not** through the bundle
(`src/plug.c:405`, `LoadFontFromMemory` at `:414`), under a 32 MiB cap
(`CAPTION_IMPORTED_FONT_BYTE_LIMIT`, `src/plug.c:369`).

Never loaded at runtime: all six `.svg` files (inputs to `./nob svg`,
`src_build/nob_stage2.c:513-519`, and the macOS iconset,
`src_build/nob_macos.c:238-246`), `logo/logo-256.ico` (Windows resource
compiler only, `src/musializer.rc:3`), and the two `OFL.txt` licence texts
(copied into distributions only).

### 7.3 Bundling vs `MUSIALIZER_UNBUNDLE`

Flag declared in the configurer, `src_build/configurer.c:67-70`, with **no**
`.enabled_by_default`, so bundled is the default.

Two compile-time implementations of the loader, `src/plug.c:63-95`:

- **Bundled** (`#ifndef MUSIALIZER_UNBUNDLE`, `:63`): includes the generated
  `build/bundle.h` (`:64`). `plug_load_resource` (`:71-80`) does a linear
  `strcmp` over the generated `resources[]` table and returns
  `&bundle[resources[i].offset]` — a pointer into a static array, `NULL` if the
  path is absent. `plug_free_resource` (`:66-69`) is a **no-op**.
- **Unbundled** (`#else`, `:81`): `plug_load_resource` (`:89-95`) is
  `LoadFileData(file_path, &dataSize)`, so **the process CWD must contain
  `resources/`**, and `plug_free_resource` (`:83-86`) is `UnloadFileData`.

Generator: `generate_resource_bundle()`, `src_build/nob_stage2.c:365-424`. The
manifest is the `Resource resources[]` array at `src_build/nob_stage2.c:352-363`
— exactly **10** entries, in this order:

```
./resources/logo/logo-256.png
./resources/shaders/glsl330/circle.fs
./resources/shaders/glsl120/circle.fs
./resources/icons/volume.png
./resources/icons/play.png
./resources/icons/render.png
./resources/icons/fullscreen.png
./resources/icons/microphone.png
./resources/fonts/SpaceGrotesk-Regular.otf
./resources/fonts/Alegreya-Regular.ttf
```

It concatenates each file and appends a NUL after each
(`nob_da_append(&bundle, 0)`, `:383`) — that terminator is what makes the `.fs`
shader source usable directly as a C string. Offsets and sizes are recorded at
`:380-382`, and `./build/bundle.h` is written at `:386-420` with the `Resource`
struct, `resources_count`, the offset table, and `unsigned char bundle[]` as
20 bytes per row of hex (`:410-419`). Payload: 427,733 bytes of assets + 10 NUL
bytes = 427,743 bytes.

Invocation gates:

- `./nob build`: `src_build/nob_stage2.c:458-460` — regeneration is wrapped in
  `#ifndef MUSIALIZER_UNBUNDLE`, so it is skipped entirely when unbundled.
- `./nob dist`: `src_build/nob_stage2.c:486-489` **refuses** unbundled builds
  ("Distribution builds require bundled resources"), then regenerates
  unconditionally at `:495`.
- macOS `.app`: `src_build/nob_macos.c:199-201` refuses too ("We do not ship
  with unbundled resources").

Distribution trees do **not** ship the font binaries — they are inside the
executable. They ship only `resources/logo/logo-256.png`,
`resources/fonts/OFL.txt`, `resources/fonts/SpaceGrotesk-OFL.txt`
(`src_build/nob_stage2.c:212-214`, directories created at `:244-246`).

### 7.4 The caption face and its licence

`caption_face()`, `src/plug.c:351-364`, is the selector:

- `MUSI_CAPTION_FACE_SPACE_GROTESK` → `p->caption_alt_font` (the second,
  full-coverage Space Grotesk load), `:352-355`
- `MUSI_CAPTION_FACE_IMPORTED` → `p->caption_imported_font`, `:356-359`
- default / unloadable → `return p->font;` (`:363`), documented at `:360-362`
  as "Alegreya, which is the caption default and the fallback for a face this
  build cannot load. Deliberately not `GetFontDefault`."

So **captions default to `resources/fonts/Alegreya-Regular.ttf`**, and its
licence file is present: `resources/fonts/OFL.txt:1` reads "Copyright 2011 The
Alegreya Project Authors (https://github.com/huertatipografica/Alegreya)" —
SIL OFL 1.1. The UI chrome face is Space Grotesk (`ui_font()`,
`src/plug.c:340-344`, falling back to `GetFontDefault()`) with
`resources/fonts/SpaceGrotesk-OFL.txt:1` "Copyright 2020 The Space Grotesk
Project Authors (https://github.com/floriankarsten/space-grotesk)".

Both licence texts must be carried into the Rust repository alongside the font
binaries if the fonts are carried; they are not synthetic fixtures and are not
copied by this Phase 0 pass.

---

## 8. `.musi` compatibility fixtures

**There are zero `.musi` files anywhere in the frozen repository outside
`build/`, and zero checked into git.**

- `find /home/wolfram/Projects/musializer -name '*.musi' -not -path '*/build/*'`
  → no results.
- `git ls-files | grep -i musi` returns only source and packaging files whose
  *names* contain "musi": `musializer-logged.bat`,
  `packaging/linux/io.github.tsoding.musializer.desktop.in`,
  `packaging/linux/io.github.tsoding.musializer.xml`, `src/musializer.c`,
  `src/musializer.rc`, `tools/musializer-launcher`,
  `tools/musializer_doctor.py`, `tests/adapters/test_musializer_doctor.py`.
- `.gitignore:1-2`, `:10-11`, `:25` ignore `music/`, `*.wav`, `*.mp4`, and
  `build/`. The repository is deliberately structured so no audio and no
  generated project is ever committed.

Consequence: **nothing to copy, and no risk of leaking private audio via a
checked-in fixture.** `fixtures/musi/` in this repository is intentionally
empty; regenerate with `tools/ui_fixture.sh` in the C tree if a real `.musi` is
ever needed.

### 8.1 The only `.musi` "fixture" is generated at runtime under `build/`

`tools/ui_fixture.sh` is the builder, and its header comment at `:7-8` states
the policy: "Everything lands under `build/`, which is ignored by Git: no
fixture audio, bridge, or project is ever committed."

Its audio is 100% synthetic ffmpeg `aevalsrc` — see 10.1. Crucially,
`tools/ui_fixture.sh:63-72` builds the `.musi` **by running the application
itself** with `--save-project` rather than hand-writing JSON, "so the fixture
exercises the real import/save path rather than a hand-written .musi the codec
never saw." `tools/ui_capture.sh:37` then consumes `$FIXTURE_DIR/demo.musi`.
Lyrics and sections in the fixture are original authored placeholder text
(`tools/ui_fixture.sh:40-54`).

### 8.2 Compatibility fixtures are built inline in C

`tests/test_project_io.c` is the compatibility suite, and its technique is the
thing to port, not any file. It builds a maximal in-memory project, serializes
it with the **real** serializer, then **textually deletes** blocks from the
resulting JSON to synthesize an older document, then deserializes.

- `tests/test_project_io.c:25-76` — `static Musi_Project fixture(void)`: one
  struct literal with **every field non-default**, with the comment at `:34-35`
  explaining why ("so the round-trip memcmp below actually checks the caption
  style rather than confirming that zeroed defaults survive"). Sets
  `deterministic_seed = UINT64_MAX`, `instance_id = UINT64_MAX - 1`,
  `band_index = 65535`, title `Kitty "Atlas"\n世界`.
- `tests/test_project_io.c:77-84` — `encode()`: two-pass measure-then-serialize
  via `musi_project_json_serialize`.

| Test | Line | Compatibility property |
| --- | --- | --- |
| `project_io_round_trip_preserves_every_field_and_uint64` | `:86` | Full-fidelity round trip; `u64` max as `18446744073709551615`; string escaping |
| `project_io_early_v1_without_semantic_events_opens_with_empty_lane` | `:95` | Missing optional block: splices out `,"semantic_events":[…]`, expects count 0 and revision 1 |
| `project_io_original_v1_defaults_new_workspace_fields` | `:110` | Missing newer fields: deletes `,"quality":"master"` and truncates at `,"lyrics":`; expects `QUALITY_HIGH`, `lyrics.next_id == 1`, `lyrics.duration_seconds` inherited from audio |
| `project_io_early_v1_defaults_optional_scene_authoring_fields` | `:131` | Deletes per-cue `settings` arrays, `scene_presets`, `ascii_image`; expects defaults with `scene_name` preserved |
| `project_io_embedded_semantics_survive_without_provenance_artifact` | `:162` | Embedded events survive a dangling lane artifact path |
| `project_io_round_trips_all_enum_spellings` | `:176` | Every enum value of format, quality, source, asset mode, blend, interpolation, lane kind |
| `project_io_rejects_unknown_duplicate_missing_and_trailing_data_atomically` | `:200` | **Strict, NOT forward-compatible**: unknown field → `ERROR_UNKNOWN_FIELD`, duplicate → `ERROR_DUPLICATE_FIELD`, `{}` → `ERROR_MISSING_FIELD`, trailing byte → `ERROR_SYNTAX`; and the output struct is **bytewise untouched** on failure |
| `project_io_rejects_malformed_numbers_strings_and_oversize_input` | `:214` | `u64` overflow, lone surrogate `\uD800`, `MUSI_PROJECT_JSON_MAX_INPUT + 1` |
| `project_io_decodes_unicode_surrogate_pairs` | `:224` | `\uD83D\uDE3A` → 😺 |
| `project_io_round_trips_finite_double_extremes_and_non_c_locale` | `:236` | ±`DBL_MAX`/`DBL_MIN` under `de_DE.UTF-8`; asserts `"0,25"` never appears |
| `project_io_rejects_decoded_nul_oversize_strings_and_invalid_utf8_output` | `:249` | `\u0000`, 129-byte string overflow, invalid UTF-8 on serialize |
| `project_io_rejects_arrays_beyond_schema_capacity` | `:264` | `MUSI_PROJECT_MAX_SCENES + 1` → `ERROR_CAPACITY` |
| `project_io_v1_without_caption_style_gets_the_shipped_defaults` | `:804` | Missing optional caption block → the 6.3 defaults |
| `project_io_writes_colours_as_eight_lowercase_hex_digits` | `:824` | RGBA wire encoding |
| `project_io_rejects_a_half_specified_or_misspelled_caption_style` | `:834` | Partial caption block rejection |

Filesystem tests at `:353`, `:432`, `:559`, `:653`, `:737` cover asset
resolution, bundling, and atomic temp-file writes; they do write real `.musi`
files, but only into `mkdtemp` directories prefixed `.musializer-project-`
(`:341`).

`tests/test_preset_store.c:51`, `:217-225` uses the same inline technique for
`"schema_version":"musializer.presets/v1"` documents, including a truncated
one.

### 8.3 Audio path strings in the test suite — all synthetic, all safe

No test reads from a real `music/` directory. The path strings that appear are
literals used to exercise separator normalization, extension handling, and
asset-bundle escape checks:

| File:line | Strings | Verdict |
| --- | --- | --- |
| `tests/test_project_io.c:31` | `audio\kitty.mp3` (backslash intentional) | synthetic |
| `tests/test_project_io.c:374-416`, `452-541`, `568-616` | `assets/song.mp3`, `assets\song.mp3`, `song.wav`, `assets/deep/song.wav`, `assets/../../outside/song.wav`, `literal\name.wav`, `source.MP3`, `show.assets/audio/<sha>.mp3`, `/definitely/not/a/musializer-asset.wav` | synthetic |
| `tests/test_project.c:27` | `assets/autoregressive-kitty.mp3` | synthetic; references the upstream public demo track *name* only |
| `tests/test_render_export.c:258-261` | `/music/Autoregressive Kitty.mp3` → `…-musializer-constellation-1080p30.mp4` | **string literal only, no file**; a pure filename-derivation assertion. Worth flagging because it hardcodes `/music/` |
| `tests/test_track_identity.c:17-43` | `ec3646f6…abfeb.wav`, `/music/song.mp3`, `C:\music\song.mp3`, `a/b\c/song.mp3` | synthetic |
| `tests/test_assist_ui_state.c:190-220` | `kitty.mp3`, `/music/kitty.mp3`, `/music/a.b.mp3`, `.mp3`, `/my.music/kitty` | synthetic |
| `tests/test_track_timeline.c:47-48` | `song.wav`, `album/live.FLAC` | synthetic |
| Python: `tests/adapters/test_render_product_smoke.py:29`, `test_command_line_session.py:44`, `test_measured_analysis.py:125,139,179`, `tests/e2e/test_lyrics_assist_e2e.py:215` | `awkward duration.mp3`, `session.wav`, `generated.wav`, `click.wav`, `tone.wav`, `spoken script.mp3` — all created in `tempfile` directories | synthetic |

### 8.4 Synthetic audio generators — regeneration spec

`tests/audio_fixtures.h` (35 lines) and `tests/audio_fixtures.c` (150 lines).
No C code was copied into this repository; this section is the spec so the Rust
side can regenerate the same waveforms.

Struct, `tests/audio_fixtures.h`:

```c
typedef struct {
    float *samples;            // interleaved, frame-major
    size_t frame_count;
    unsigned int sample_rate;
    unsigned int channel_count;
} Audio_Fixture;
```

Sample format is **`f32`, interleaved**, index `frame * channel_count +
channel`. There is **no `i16` anywhere** in the C fixtures and **no WAV header
writing at all**. There is **no RNG, no seed, no noise generator, and no
windowing** — every generator is closed-form deterministic. The only envelope
is the linear beat decay. Constant: `AUDIO_FIXTURE_PI
3.14159265358979323846` used as a `double` (`tests/audio_fixtures.c:7`).

Signatures, `tests/audio_fixtures.h:14-32`, all returning `bool`
(`false` = invalid arguments or OOM):

```c
bool audio_fixture_silence(Audio_Fixture*, unsigned sample_rate, unsigned channel_count, size_t frame_count);
bool audio_fixture_sine   (Audio_Fixture*, unsigned sample_rate, unsigned channel_count, size_t frame_count, float frequency_hz, float amplitude);
bool audio_fixture_sweep  (Audio_Fixture*, unsigned sample_rate, unsigned channel_count, size_t frame_count, float start_frequency_hz, float end_frequency_hz, float amplitude);
bool audio_fixture_impulse(Audio_Fixture*, unsigned sample_rate, unsigned channel_count, size_t frame_count, size_t impulse_frame, float amplitude);
bool audio_fixture_stereo_imbalance(Audio_Fixture*, unsigned sample_rate, size_t frame_count, float frequency_hz, float left_amplitude, float right_amplitude);
bool audio_fixture_beat   (Audio_Fixture*, unsigned sample_rate, unsigned channel_count, size_t frame_count, float beats_per_minute, float amplitude);
void audio_fixture_destroy(Audio_Fixture*);
```

**Allocation contract**, `audio_fixture_allocate`,
`tests/audio_fixtures.c:9-28`. Rejects: `fixture == NULL`; **`fixture->samples
!= NULL`** (reusing a live fixture fails — callers must `destroy` first);
`sample_rate == 0`; `channel_count == 0`; `frame_count == 0`; and the two
overflow guards `frame_count > SIZE_MAX / channel_count` and
`frame_count * channel_count > SIZE_MAX / sizeof(float)`. Then `calloc`, so the
buffer starts **zero-filled**.

**Shared signal precondition**, `valid_signal`, `tests/audio_fixtures.c:30-34`:
`isfinite(frequency_hz) && frequency_hz >= 0.0f && isfinite(amplitude) &&
fabsf(amplitude) <= 1.0f`. A **negative** amplitude is legal as long as
`|a| <= 1`.

`audio_fixture_silence` (`:36-40`) — pure `calloc`, all zeros.

`audio_fixture_sine` (`:42-58`) — extra guard `frequency_hz > sample_rate *
0.5f` (Nyquist, float comparison). Phase is computed in **f64**, cast to
**f32**, then `sinf`:

```c
float sample = amplitude * sinf((float) (2.0 * AUDIO_FIXTURE_PI * frequency_hz *
                                          (double) frame / sample_rate));
```

All channels receive the identical sample; no per-channel phase offset. In
Rust: `amplitude * ((2.0f64 * PI * freq as f64 * frame as f64 / rate as f64)
as f32).sin()`.

`audio_fixture_sweep` (`:60-84`) — linear-frequency chirp by **phase
accumulation**, not by an instantaneous-phase formula. Both endpoints are
Nyquist-checked.

```c
double phase = 0.0;
for (size_t frame = 0; frame < frame_count; ++frame) {
    double progress = frame_count > 1 ? (double) frame / (double) (frame_count - 1) : 0.0;
    double frequency = start_frequency_hz + (end_frequency_hz - start_frequency_hz) * progress;
    float sample = amplitude * sinf((float) phase);
    for (unsigned int channel = 0; channel < channel_count; ++channel) {
        fixture->samples[frame * channel_count + channel] = sample;
    }
    phase += 2.0 * AUDIO_FIXTURE_PI * frequency / sample_rate;
    phase = fmod(phase, 2.0 * AUDIO_FIXTURE_PI);
}
```

Order matters for bit-identity: the sample is emitted **before** the phase
advances, so frame 0 is always exactly `0.0`; and `phase` is wrapped with
`fmod` into `[0, 2π)` **every frame**, so the accumulated rounding of that
per-frame wrap is observable and must be replicated in f64. `progress` divides
by `frame_count - 1` so the end frequency is hit exactly at the last frame;
`frame_count == 1` yields `progress = 0`.

`audio_fixture_impulse` (`:86-98`) — validated with `valid_signal(0.0f,
amplitude)`; `impulse_frame >= frame_count` is rejected. Writes `amplitude`
into every channel of the single frame `impulse_frame`; everything else remains
the `calloc` zeros.

`audio_fixture_stereo_imbalance` (`:100-116`) — **channel count is hardcoded
2** (the allocate call passes the literal `2`); there is no `channel_count`
parameter. The unit sine is computed first and *then* scaled per side, unlike
`sine` which folds the amplitude into the same expression — mathematically
equal, but keep the multiply order for exactness:

```c
float wave = sinf((float) (2.0 * AUDIO_FIXTURE_PI * frequency_hz *
                            (double) frame / sample_rate));
fixture->samples[frame * 2]     = left_amplitude * wave;
fixture->samples[frame * 2 + 1] = right_amplitude * wave;
```

`audio_fixture_beat` (`:118-142`) — a click track. Guards: `isfinite(bpm) &&
bpm > 0.0f` plus `valid_signal(0.0f, amplitude)`.

```c
size_t beat_period = (size_t) llround(60.0 * sample_rate / beats_per_minute);
if (beat_period == 0) beat_period = 1;
size_t pulse_frames = sample_rate / 200; // A short, click-like 5 ms envelope.
if (pulse_frames == 0) pulse_frames = 1;

for (size_t beat = 0; beat < frame_count; beat += beat_period) {
    for (size_t offset = 0; offset < pulse_frames && beat + offset < frame_count; ++offset) {
        float envelope = amplitude * (1.0f - (float) offset / pulse_frames);
        for (unsigned int channel = 0; channel < channel_count; ++channel) {
            fixture->samples[(beat + offset) * channel_count + channel] = envelope;
        }
    }
}
```

Bit-identity notes: `beat_period` is `llround(60.0 * sample_rate / bpm)` — f64
division with round-half-away-from-zero, **not** truncation. `pulse_frames` is
**integer** `sample_rate / 200` (truncating), i.e. exactly 5 ms; at 8000 Hz
that is 40 frames. The envelope is a linearly decaying ramp
`amplitude * (1 - offset / pulse_frames)` computed as an **f32** division, and
it never reaches 0 because `offset` maxes at `pulse_frames - 1`. The pulse is
**unipolar positive**, not a DC-free oscillation. Everything between pulses
stays zero.

`audio_fixture_destroy` (`:144-149`) — `free(samples)`, then zero the struct;
NULL-safe.

**Concrete parameter values the suite actually uses**, which double as the
expected-value oracle (`tests/test_main.c:6-37`): `silence(8000, 2, 80)`;
`sine(8000, 1, 80, 1000, 0.5)` expects `samples[2] ≈ 0.5`;
`sweep(8000, 1, 80, 100 → 1000, 0.5)` checked for finiteness only;
`impulse(8000, 2, 80, frame 17, 0.75)`;
`stereo_imbalance(8000, 80, 500, L = 0.8, R = 0.2)` expects
`0.8 * sinf(2π * 500 / 8000)` and the `0.2` counterpart at frame 1;
`beat(8000, 2, 8000, 120 bpm, 0.9)` expects positive at frames 0 and 4000.
Also `tests/test_song_atlas_map.c:27` `silence(8000, 2, 16000)` (2 s) and
`:78-79` `sine(8000, 1, 24000, 220, 0.7)` / `sine(8000, 1, 24000, 1800, 0.7)`
(3 s).

**Two further generators live outside `audio_fixtures.c`** and are worth
porting deliberately because they differ:

1. `tests/test_audio_analyzer.c:9-15` uses an **f32** tau constant and a 44100
   sample rate, a different numeric pipeline from the f64 phase above:

   ```c
   #define TEST_SAMPLE_RATE 44100u
   const float tau = 6.28318530717958647692f;
   samples[i] = amplitude*sinf(tau*frequency*(float)i/(float)TEST_SAMPLE_RATE);
   ```

   and `tests/test_audio_analyzer.c:86-91` builds an anti-phase stereo pair
   (`stereo[i*2] = value; stereo[i*2+1] = -value;`) at 880 Hz to test channel
   mixing.

2. `tests/adapters/test_measured_analysis.py:24-44` is the **only** WAV writer
   and the **only** windowing / `i16` code in the whole test tree. `click_track`
   uses the rising half of a `2*width`-point **Hann** window
   (`np.hanning(2n)[:n]`) as an attack, `+=`-accumulated so overlapping clicks
   sum, with `width = max(8, round(sample_rate * 0.0125))` (12.5 ms).
   `write_stereo_wav` writes standard 44-byte RIFF/WAVE via Python's `wave`
   module: 2 channels, 16-bit, `struct.pack("<h", round(value * 32767))`, right
   channel `= -0.5 * left`. **Python's `round()` is banker's rounding
   (half-to-even), which differs from C `llround`** — relevant if a Rust port
   ever needs to match these bytes. Callers use 16000 Hz / 440 Hz @ 0.25
   (`:122-126`), 8000 Hz `click_track` (`:139`), and 8000 Hz / 220 Hz @ 0.2 for
   half a second (`:179-181`).

---

## 9. Environment overrides

### 9.1 Read by the C application

| Variable | Site | Purpose | Default when unset | Parsing |
| --- | --- | --- | --- | --- |
| `MUSIALIZER_PRESET_STORE` | `src/preset_store.c:31` | Absolute override for the per-user preset JSON path | Platform default (rows below) | Non-empty string used verbatim (`override[0] != '\0'`, `:32`), copied with `snprintf`; **fails if it would truncate** |
| `APPDATA` | `src/preset_store.c:37` | Windows preset root → `%APPDATA%\Musializer\presets.json` (`:37-38`) | none; `path_join` returns false if NULL or empty (`:22`) | raw prefix concatenation |
| `HOME` | `src/preset_store.c:40`, `:48` | macOS `$HOME/Library/Application Support/Musializer/presets.json` (`:39-40`); Linux fallback `$HOME/.local/share/musializer/presets.json` (`:47-48`) | none; false if unset | raw prefix concatenation |
| `XDG_DATA_HOME` | `src/preset_store.c:43` | Linux data root → `$XDG_DATA_HOME/musializer/presets.json` (`:43-46`) | `$HOME/.local/share` path | non-empty check only |
| `MUSIALIZER_RENDER_SUPERSAMPLE` | `src/plug.c:456` | Kill switch for offline-render supersampling | `config->supersample_factor` from the export config is honoured | **Exact string compare to `"0"`** (`strcmp(supersampling, "0") == 0`, `:458`) forces factor 1. Every other value — including `"1"`, `"false"`, `"2"` — is **ignored**. Logs "Offline render supersampling disabled by environment" at `:470` |
| `MUSIALIZER_FONT_HELPER` | `src/plug.c:1554` | Path override for the `tools/google_fonts.py` helper | Probes `<appdir>/tools/google_fonts.py`, `<appdir>/../tools/google_fonts.py` (`:1560-1564`), then `./tools/google_fonts.py` (`:1568`) | Non-empty **and** must pass `FileExists(path)` (`:1558`); a set-but-missing override fails outright with **no** fallback to probing |
| `MUSIALIZER_ASSIST_HELPER` | `src/plug.c:2051` | Path override for the `tools/external_analysis.py` Assist helper | Probes `<appdir>/tools/external_analysis.py`, `<appdir>/../tools/…` (`:2057-2061`), then `./tools/external_analysis.py` (`:2065`) | Same shape as above: non-empty + `FileExists` (`:2055`), no fallback |
| `PATH` | `src/ffmpeg_posix.c:32` | `ffmpeg_available()` walks it manually looking for an `X_OK` `ffmpeg` | NULL or empty → returns `false` (`:33`) | split on `':'` (`:36`); **an empty element is treated as `"."`** (`:39-40`) |

Test-only: `tests/test_preset_store.c:86-104` sets and unsets
`MUSIALIZER_PRESET_STORE`, `XDG_DATA_HOME`, and `HOME` to exercise the
resolution order.

**`MUSIALIZER_CAPTURE_DISPLAY` and `MUSIALIZER_CAPTURE_SETTLE` are shell-only.**
No C code reads either one; they are consumed by `tools/ui_capture.sh` and
`tools/ui_fixture.sh`.

### 9.2 Read by the shell tooling

| Variable | Site | Purpose | Default | Parsing |
| --- | --- | --- | --- | --- |
| `MUSIALIZER_UI_FIXTURE_DIR` | `tools/ui_capture.sh:24` | Where `demo.musi` / `demo.assets` live | `build/ui-review` | `${VAR:-default}` |
| `MUSIALIZER_CAPTURE_DISPLAY` | `tools/ui_capture.sh:25`, `tools/ui_fixture.sh:15` | Xvfb display **number**, used as `":$DISPLAY_NUM"` | **`77`** | `${VAR:-77}`, **no validation**; interpolated straight into the Xvfb and `DISPLAY` strings |
| `MUSIALIZER_CAPTURE_SETTLE` | `tools/ui_capture.sh:29` | Seconds between launch and grab | **`6`** | `${VAR:-6}` passed verbatim to `sleep "$SETTLE"` (`:90`); **no numeric validation or bounds** — a non-numeric value makes `sleep` error and the grab happen immediately |
| `WAYLAND_DISPLAY` | `tools/ui_capture.sh:59` | **unset**, so the app cannot attach to the operator's compositor | n/a | `unset` |
| `PULSE_SERVER` | `tools/ui_capture.sh:60-61` (exported), `tools/ui_fixture.sh:69` (per-command) | Forced to `/nonexistent/musializer-capture` so no audio client stream reaches the desktop | n/a | literal assignment |
| `DISPLAY` | `tools/ui_capture.sh:88`, `tools/ui_fixture.sh:69` | Set per-command to `":$DISPLAY_NUM"`; the ffmpeg x11grab input is `":$DISPLAY_NUM.0"` (`ui_capture.sh:94`) | n/a | literal |
| `XDG_DATA_HOME` | `tools/install-linux-launcher.sh:7` | Desktop-entry install root | `$HOME/.local/share` | `${VAR:-…}` |
| `XDG_BIN_HOME` | `tools/install-linux-launcher.sh:8` | Launcher binary install root | `$HOME/.local/bin` | `${VAR:-…}` |
| `XDG_STATE_HOME` | `tools/musializer-launcher:7`, `tools/install-linux-launcher.sh:156` | Launcher log root, `…/musializer/launcher.log` | `$HOME/.local/state` | `${VAR:-…}` |

### 9.3 Read by the Python tooling

| Variable | Site | Purpose | Unset behaviour | Parsing |
| --- | --- | --- | --- | --- |
| `OPENROUTER_API_KEY` | `tools/mimo_openrouter.py:319` | Bearer credential | `raise RuntimeError("OPENROUTER_API_KEY is not set in the process environment")` (`:320-321`) | truthiness only |
| `OPENROUTER_API_KEY` | `tools/external_analysis.py:287` | Same credential for the MiMo helper | Falls back to parsing the repo `.env` **as data, not shell** (`:288-300`): skips blanks, `#`, and lines without `=`; matches the exact name; strips one layer of matching `"` or `'` | `.strip()`ed string, truthiness |
| `MUSIALIZER_WHISPER_BIN` | `tools/external_analysis.py:1220`, `--whisper-bin` default `:1450` | whisper.cpp CLI path | Discovers `<install>/build/bin/whisper-cli` (`:1223-1227`); if still unresolved → `AnalysisValidationError` (`:1493`) | raw string → `Path`; **env always wins over discovery**, comment `:1197-1198` |
| `MUSIALIZER_WHISPER_MODEL` | `tools/external_analysis.py:1221`, `:1451` | ggml model file | Preference-ordered discovery `_WHISPER_MODEL_PREFERENCE` (`:1204-1209`); a better model anywhere beats a lesser model in a preferred install (`:1229-1235`) | raw string → `Path` |
| `MUSIALIZER_WHISPER_VAD_MODEL` | `tools/external_analysis.py`, `whisper_vad_model()` | Opt-in whisper.cpp VAD model (`--vad`/`--vad-model`). **Added by tranche LT1**, and off by default: Silero v6.2.0 rejects sung vocals over accompaniment (canary kept 0.4 s of 114.84 s) | no VAD; a named path that is not a readable file logs the reason and also runs without VAD, rather than failing the job | `.strip()`, `expanduser()` → `Path`; its SHA-256 enters the Whisper cache identity |
| whole `os.environ` (filter) | `tools/external_analysis.py:272-276`, `_safe_local_env()` | Strips any variable whose upper-cased name contains `KEY`, `TOKEN`, `SECRET`, `PASSWORD`, `CREDENTIAL`, or `AUTH` before spawning local children | n/a | substring blacklist |
| `CUDA_VISIBLE_DEVICES` | `tools/musializer_doctor.py:114` | GPU-presence heuristic | `""` → falls through to a `/dev/dri/renderD128` probe (`:118`), then `{"kind": "none"}` (`:121`) | `.strip()`; treated as "GPU present" if non-empty and not literally `"-1"` |
| whole `os.environ` (filter) | `tools/musializer_doctor.py:126-132` | Same credential stripping before invoking `nvidia-smi` | n/a | same substring blacklist |
| `MUSIALIZER_WHISPER_BIN` / `_MODEL` | `tests/e2e/test_lyrics_assist_e2e.py:68-69` | e2e skip gating (`:100-105`) | test skipped | truthiness |

### 9.4 Read by the Rust rewrite

Not in the frozen C: the AI settings dialog (tranche AP3) has no oracle at all,
and the surfaces below have no other way to be driven from a headless run.

Two are real configuration a user may set; the rest are **probe seams**, each
inert when unset, each read once in `AssistSettingsDialog::open`. They are
environment variables rather than `--ui-probe` keys for the reason
`MUSIALIZER_ASSIST_PROBE_DIR` already is: the probe grammar lives in `cli.rs`,
which that tranche does not own.

| Variable | Site | Purpose | Unset behaviour | Parsing |
| --- | --- | --- | --- | --- |
| `MUSIALIZER_ASSIST_SETTINGS` | `runtime::assist::files::settings_path` | Absolute override for `assist.json` | `$XDG_CONFIG_HOME/musializer/assist.json`, else `$HOME/.config/musializer/assist.json` (§2) | non-empty check only; used verbatim as a path |
| `MUSIALIZER_ASSIST_CREDENTIALS` | `runtime::assist::files::credentials_path` | Absolute override for `credentials.json` | the same ladder with `credentials.json` (§3) | non-empty check only |
| `MUSIALIZER_ASSIST_SETTINGS_OPEN` | `ui/assist_settings.rs`, `main.rs` | Opens the dialog on a section | dialog closed | `Section::parse`: `1`/`routing`, `local`/`local-models`, `codex`, `openrouter`, `privacy`/`diagnostics`, case-insensitive and trimmed; an unknown token opens nothing |
| `MUSIALIZER_ASSIST_SETTINGS_TAB` | `apply_probe_state` | How many Tab steps to apply at open, so a focus ring is photographable | no steps | `u32`, **capped at 256**; a non-number is ignored |
| `MUSIALIZER_ASSIST_SETTINGS_SCROLL` | `apply_probe_state`, applied in `draw_body` | Scroll offset in logical pixels, for photographing a section bottom | no scroll | `f32`, floored at 0; **applied on the first drawn frame**, not at open, then clamped against the measured content height |
| `MUSIALIZER_ASSIST_SETTINGS_HOVER` | `apply_probe_state` | `1` lets the dialog's own tooltips fire immediately | with `_OPEN` set, the dwell is **infinite** so no stray pointer can pop a tip into a capture; otherwise the normal dwell | exact `1` after trimming |
| `MUSIALIZER_ASSIST_SETTINGS_KEY` | `apply_probe_state` | A **fixture** credential to seed the masked Replace field with, so the mask can be photographed | field empty | non-empty string, used verbatim. Never a real key: the gate plants a sentinel here and greps every artifact for it |
| `MUSIALIZER_ASSIST_SETTINGS_NOW` | `now_utc` | Pins "now" so a cache age badge is the same number in every run | the wall clock | `parse_rfc3339_utc`: exactly `YYYY-MM-DDTHH:MM:SSZ`; anything else yields no time and every age reads "age unknown" |
| `MUSIALIZER_ASSIST_SETTINGS_KEY_TEST` | `start_key_test` | Stubs the `Test` outcome, so no capture opens a socket | the real `curl --config -` request | `KeyTest::parse`: `ok`, `invalid`, `revoked`, `rate-limited`, `no-network`, `no-key`. An unrecognized value is reported as a no-network failure **naming the variable**, never silently ignored |
| `MUSIALIZER_ASSIST_SETTINGS_REFRESH` | `start_refresh` | Stubs the catalog/Codex `Refresh` outcome, for the same reason | the real `python3 tools/…` child | `ok` is a success status line; every other value is a stubbed failure echoing the value |
| `MUSIALIZER_ASSIST_SETTINGS_DOCTOR` | `reload` | Reads a doctor report from a file instead of running `tools/musializer_doctor.py` | "Not probed" | a path; read and parsed as `musializer.doctor/v1`. A read or parse failure is shown as a doctor error, not as an absent report |
| `MUSIALIZER_ASSIST_SETTINGS_ESCAPE` | `apply_probe_state` | Presses Escape once at open | no press | exact `1` after trimming. **Deferred by one frame when `_ACTIVATE` is also set**, because an Enter is only consumed while a control is being drawn |
| `MUSIALIZER_ASSIST_SETTINGS_DIRTY` | `apply_probe_state` | Marks a real route override as edited, so the unsaved-changes confirm step is reachable | draft clean | exact `1`. Falls back to flipping `catalog.show_experimental` if the override left the draft clean, so the seam cannot silently test nothing |
| `MUSIALIZER_ASSIST_SETTINGS_ACTIVATE` | `apply_probe_state` | Presses Enter once on the focused control | no press | exact `1`. This is what makes `Save`, `Forget` and `Test` reachable from a headless run at all |

`tools/google_fonts.py`, `tools/analyze_audio.py`, `tools/lyric_align.py`,
`tools/lyric_anchor_block.py`, `tools/anchor_block_align.py`,
`tools/import_whisper.py`, and `tools/analysis_io.py` read **no** environment
variables.

---

## 10. Headless capture harness

Two scripts. `tools/ui_fixture.sh` builds a reusable fixture once;
`tools/ui_capture.sh` photographs a catalogue of UI states against it.
`tools/UI_REVIEW.md:52-53` documents the conventional invocation:
`tools/ui_fixture.sh` then `tools/ui_capture.sh build/ui-review/shots`, with a
blessed reference set kept in `build/ui-review/reference/`
(`tools/UI_REVIEW.md:56`).

### 10.1 `tools/ui_fixture.sh` — build the fixture

`set -eu` at `:11`. `OUT=${1:-build/ui-review}` at `:13`.

Preflight (`:17-19`): `[ -x ./build/musializer ]` else "missing $APP - run
./nob build debug"; `command -v ffmpeg`; `command -v Xvfb`.

1. **Synthesizes a 40 s stereo WAV, only if absent** (`:23-29`):

   ```
   ffmpeg -loglevel error -y -f lavfi \
     -i "aevalsrc='0.45*sin(2*PI*100*t)*exp(-9*mod(t,0.5))
                 + 0.22*sin(2*PI*(300+180*sin(2*PI*0.08*t))*t)
                 + 0.12*sin(2*PI*1500*t)*exp(-30*mod(t,0.25))':d=40:s=44100:c=stereo" \
     -c:a pcm_s16le "$OUT/demo.wav"
   ```

   A 100 Hz decaying pulse, a vibrato'd ~300 Hz partial, and a 1500 Hz
   hi-hat-like transient.
2. An inline `python3 - "$OUT" <<'PY'` heredoc (`:31-61`) writes
   `demo.bridge.tsv`: header `MUSIALIZER_BRIDGE\t1`, then
   `AUDIO\t<sha256 of demo.wav>\t40000`, **8 base64-encoded `LYRIC` rows**
   (1500–36500 ms, deliberately mixed lengths, `:40-49`) and **3 `SECTION`
   rows** (spectrum 0–13000, loom 13000–26000, cadence 26000–40000, `:50-54`).
3. Builds the project **with the real application** rather than hand-writing
   `.musi` (`:63-72`):

   ```
   Xvfb ":$DISPLAY_NUM" -screen 0 1280x720x24 -nolisten tcp >"$OUT/.xvfb.log" 2>&1 &
   xvfb_pid=$!
   sleep 2
   DISPLAY=":$DISPLAY_NUM" PULSE_SERVER=/nonexistent/musializer-capture \
       "$APP" --mute "$OUT/demo.wav" \
       --analysis-bridge "$OUT/demo.bridge.tsv" \
       --save-project "$OUT/demo.musi" >"$OUT/fixture.log" 2>&1 || true
   ```

   then kills and waits on Xvfb (`:73-74`).
4. Postconditions (`:76-78`): `demo.musi` must exist, and
   `grep -c 'applied 8 lyrics, 3 scene sections' "$OUT/fixture.log"` must
   succeed.
5. Writes `reference.lyrics.txt` (3 authored lines, `:81-85`) and `fonts.tsv`
   (a `musializer.font-catalogue/v1` header plus 8 real families with script
   coverage, `:91-101`) so **no capture run ever contacts Google Fonts**.
6. Artifacts: `build/ui-review/{demo.wav, demo.bridge.tsv, demo.musi,
   reference.lyrics.txt, fonts.tsv, fixture.log, .xvfb.log}`.

### 10.2 `tools/ui_capture.sh` — photograph the states

`set -u` at `:19` — note **not** `-e`, deliberately, so one failing state does
not abort the run. Usage: `tools/ui_capture.sh OUTDIR [CATALOGUE]`, catalogue
defaulting to `tools/ui_states.txt` (`:23`).

Preflight (`:31-41`): OUTDIR required (exit **2**); `[ -x ./build/musializer ]`;
catalogue file exists; `command -v Xvfb`; `command -v ffmpeg`; and the fixture
master `$FIXTURE_DIR/demo.musi` must exist else "run tools/ui_fixture.sh".

Session isolation (`:56-61`): `unset WAYLAND_DISPLAY`; then
`PULSE_SERVER=/nonexistent/musializer-capture; export PULSE_SERVER`.

Per state — the loop is `while IFS='|' read -r name size args` (`:64`), with
comments and blanks skipped (`:65`) and `width=${size%x*}` /
`height=${size#*x}` (`:66-67`):

1. **A throwaway fixture copy.** `rm -rf "$WORKDIR"; mkdir -p "$WORKDIR"; cp
   "$MASTER" "$PROJECT"`, plus `cp -r "$FIXTURE_DIR/demo.assets"
   "$WORKDIR/demo.assets"` when present (`:75-80`). `WORKDIR="$OUTDIR/.fixture"`
   and `PROJECT="$WORKDIR/demo.musi"` (`:52-53`); the filename is preserved
   because the asset bundle is addressed relative to the project stem
   (`:72-74`). The literal token `PROJECT` in the catalogue args is substituted
   by `sed "s#PROJECT#$PROJECT#g"` (`:70`).
2. **One Xvfb per state**, geometry taken from the catalogue line:

   ```
   Xvfb ":$DISPLAY_NUM" -screen 0 "${width}x${height}x24" -nolisten tcp \
       >"$OUTDIR/.xvfb.log" 2>&1 &
   xvfb_pid=$!
   sleep 2
   ```

   Display number defaults to **77** (`MUSIALIZER_CAPTURE_DISPLAY`), depth 24.
3. Launch: `DISPLAY=":$DISPLAY_NUM" $APP --mute $resolved >"$log" 2>&1 &`
   (`:88`, with a deliberate `# shellcheck disable=SC2086` for word splitting).
4. **The wait is a blind `sleep "$SETTLE"`** (`:90`). There is **no polling of
   the window** and **no `xdotool`, `xwininfo`, or `wmctrl` anywhere in the
   repository**. `SETTLE=${MUSIALIZER_CAPTURE_SETTLE:-6}` (`:29`); the comment
   at `:26-28` notes that with `play=1` the captured playhead is roughly
   `time + SETTLE`.
5. **The frame grab is ffmpeg x11grab** — not ImageMagick `import`, not `xwd` —
   guarded by a liveness check (`:92-94`):

   ```
   if kill -0 "$app_pid" 2>/dev/null; then
       ffmpeg -loglevel error -y -f x11grab -video_size "${width}x${height}" \
           -i ":$DISPLAY_NUM.0" -frames:v 1 "$out" >>"$log" 2>&1
   ```

   If the application already exited it `wait`s, prints
   `FAIL $name (application exited early; see $log)`, and sets `status=1`
   (`:95-101`).
6. Teardown: `kill "$app_pid"` → `sleep 1` → `kill` / `wait` Xvfb (`:103-106`).
7. Verification: the PNG must exist else `FAIL $name (no capture)`
   (`:116-119`). The script also sha256-compares the per-state copy against the
   master digest captured once at `:54`
   (`master_before=$(sha256sum "$MASTER" | cut -d' ' -f1)`) and, if a state
   wrote the project, prints
   `ok   $name -> $out (WROTE the project; captures may drift)` (`:108-115`).

After the loop (`:122-126`): `rm -rf "$WORKDIR"`, and if the **master** digest
changed, `FAIL the master fixture changed during the run; captures are not
comparable` with `status=1`. Exits `status` (`:128`).

Artifacts: `$OUTDIR/<name>.png` (`:68`), `$OUTDIR/<name>.log` (application
stdout/stderr plus ffmpeg errors, `:69`), a shared `$OUTDIR/.xvfb.log`
overwritten each state (`:83`), and the transient `$OUTDIR/.fixture/`.
`<name>` is the first catalogue field verbatim.

### 10.3 `tools/ui_states.txt`

Format documented at `:1-4`: `name|WIDTHxHEIGHT|args`, `#` comments and blank
lines ignored, the token `PROJECT` substituted with the throwaway copy's path.
**41 states.** Sizes: 20 × `1280x720`, 11 × `960x640` (the
`SetWindowMinSize` floor), 5 × `1920x1080`, 5 × `1600x900`. Groups: entry and
landing (`:13-15`), default workspace (`:18-20`), expanded panels including
lyrics / tune / export / assist plus the cue-editing form and the armed Assist
prompt (`:23-46`), fullscreen stage (`:49-50`), a ten-scene framing sweep with
`play=1` (`:53-63`), timeline zoom 8× and 40× (`:67-68`), caption-style states
(`:69-70`), and the font browser using `fonts=consent` and
`fonts=build/ui-review/fonts.tsv` so nothing reaches the network (`:75-77`).
The application-side surface is `--ui-probe` (keys used: `size`, `panel`,
`time`, `play`, `fullscreen`, `lyric`, `assist`, `lyrics-file`, `zoom`,
`style`, `fonts`) plus `--project`, `--scene`, and `--mute`.

### 10.4 Reimplementation notes for Rust, and this machine's missing tools

- **`import` (ImageMagick) and `xdotool` are not installed on this machine, and
  neither is used by these scripts.** The only external binaries the harness
  needs are **`Xvfb`** and **`ffmpeg`**. No substitute is required for the
  frame grab: `ffmpeg -f x11grab -video_size WxH -i :NN.0 -frames:v 1 out.png`
  is the actual mechanism.
- The one genuinely fragile part is the blind `sleep`. A Rust reimplementation
  can improve on it without changing observable output by having the
  application signal readiness — for example a line on stdout after
  `plug_apply_ui_probe` succeeds — and having the harness wait for that line
  with a timeout instead of a fixed 6 seconds. That would remove the
  `MUSIALIZER_CAPTURE_SETTLE` guesswork and the `time + SETTLE` playhead drift
  documented at `tools/ui_capture.sh:26-28`. Keep the env var as an override so
  existing scripts still work.
- Reproduce these invariants exactly or the captures stop being comparable:
  one Xvfb per state at depth 24 sized to the state; `WAYLAND_DISPLAY` unset;
  `PULSE_SERVER` pointed at a nonexistent path; the project copied per state
  and the master's digest verified before and after the whole run; the window
  parked at `(0, 0)`; and `--mute` on every launch.

---

## Appendix: things that surprised me, for the integration owner

1. **The version string exists three times in three spellings** —
   `musializer 2026.07`, `Musializer 2026.07`, `musializer-2026.07`
   (`src/musializer.c:323`, `:255`, `src/plug.c:4293`). Make it one constant
   with three formatters.
2. **There is no unknown-flag diagnostic.** Every unrecognized `--flag` is
   treated as a file path to load (`src/musializer.c:546-550`). A Rust
   `clap`-style parser will *diverge* here by default, and the Python adapter
   tests may depend on the current behaviour.
3. **`--render-window` takes two argv words**, and its index-advance expression
   (`src/musializer.c:473`) is unusual enough to deserve a comment in the port.
4. **`settings.pentagram.hue` defaults to `-91.0`.** Every other hue control
   defaults to `0.0`.
5. **`settings.terrarium.density` and `settings.constellation.density` default
   to their maximum**, not their midpoint.
6. **`scene_settings_get` silently heals** an out-of-range stored value to the
   descriptor default, and returns `1.0f` — not `0.0f` — for a nonexistent
   descriptor (`src/scene_settings.c:187-198`).
7. **`caption_font_asset.licence_sha256` uses `^([0-9a-f]{64})?$`**, not
   `$defs/sha256`. Reusing the shared definition would wrongly reject a
   legitimately unlicensed user-disk import.
8. **`font-import-v1.schema.json` uses `musializer.invalid`** while every other
   schema uses `musializer.local` (`schemas/font-import-v1.schema.json:3`).
9. **`glsl120/circle.fs` is bundled but unreachable** because `GLSL_VERSION` is
   hard-coded to 330 (`src/plug.c:100`).
10. **Space Grotesk is loaded twice** with two different codepoint sets
    (`src/plug.c:8065`, `:8102`), because the UI subset would silently drop
    Greek and Cyrillic in captions.
11. **`MUSIALIZER_RENDER_SUPERSAMPLE` only responds to the exact string `"0"`**
    (`src/plug.c:458`). `MUSIALIZER_RENDER_SUPERSAMPLE=false` does nothing.
12. **`MUSIALIZER_FONT_HELPER` and `MUSIALIZER_ASSIST_HELPER` do not fall back**
    when set to a missing path — they fail hard (`src/plug.c:1558`, `:2055`).
13. **`ffmpeg_available()` treats an empty `PATH` element as `"."`**
    (`src/ffmpeg_posix.c:39-40`).
14. **The project codec is strict, not forward-compatible**: unknown fields are
    a hard error (`schemas/project-v1.schema.json:8`,
    `tests/test_project_io.c:200`). Compatibility comes from *optional* fields
    with documented defaults, not from ignoring extras. The Rust port must not
    reach for a permissive `serde` default here.
15. **Every `maxLength` in `project-v1` is a UTF-8 byte count, not a code point
    count** (`schemas/project-v1.schema.json:6`). A naive
    `String::chars().count()` check will accept documents the C rejects.
16. **`plug_mark_command_line_state_clean()`** (`src/musializer.c:597`) exists
    because a one-off `--resolution` once got autosaved permanently into a
    project. Any Rust dirty-tracking design must have an equivalent.
