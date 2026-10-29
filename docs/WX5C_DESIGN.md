# WX-5c design note: the rail, the big preview, the top bar

Written by the 5a/5b designer for the lead, 2026-10-24, and accepted by the
lead the same day (with the dock keeping its full height). `BRIEF.md` is
[`WX5_BRIEF.md`](WX5_BRIEF.md), the run's working brief. 5c follows BRIEF.md
"Rail and preview (5c)" and builds on the 5b tree (runs, posters merged by the
critic). It can be split between two Opus agents working at the same time:

- **Rail**: the frame, the rail and its flyouts, the transport, the lane head.
  That is me.
- **Top bar**: title and status, the mode tabs, Still / Contact sheet / Render.

A seam commit comes first, as in 5b. This note is the contract.

## 0. The frame, in one picture

```
+--------------------------------------------------------------------------+ dock
| TOP BAR 46: mark | title · duration · format · saved | tabs | Still Sheet Render
+------+-------------------------------------------------------------------+
| RAIL | PREVIEW (everything the sidebar gave back)                        |
|  52  |   flyout (tracks | scenes) floats over the preview's left edge     |
|      +-------------------------------------------------------------------+
|      | TRANSPORT 50: (play) 3:12.00 / 5:22.52  [scrub]  chips  vol  full  |
+------+-------------------------------------------------------------------+
| TIMELINE BAND (full workspace width, as today)                           |
|  lane head 26: PLAN · 27 runs | Auto | (context: Split ◀ ▶ Capture Delete) | + Feel + Scene + Custom Clear
|  plan lane (runs) / waveform / cue lane / zoom row                       |
+--------------------------------------------------------------------------+
```

- **The rail stops at the band.** It sits beside the preview and the transport
  only; the band keeps the full workspace width. This differs from the mockup,
  where the rail runs the full height, and that is deliberate:
  - lanes are a time axis, and width is resolution;
  - every band probe in the gate (export, lyrics, assist, D4, lane alignment)
    keeps its x. Only y changes, and only where the band's composition changes.
- **The top bar spans the workspace, not the dock.** The dock keeps its full
  height (WX-1: the shell lays out in a window narrowed by the dock).
  Matching the mockup's top bar over the dock would make the shell own pixels
  `main.rs` hands to egui. Not worth it in 5c; record it as an open choice.
- **The preview gains** the old sidebar's 320 − 52 = 268 px of width and the
  event row's 66 px of height, and gives 46 px of height to the top bar.
  At 1280x720 that is 960x344 → 1228x~360. At 960x640, 640x264 → 908x~280.

## 1. The rail and the flyouts (Rail)

- **Rail, 52 logical px:**
  - icons top to bottom: Tracks, Scenes (with ASCII imagery);
  - at the bottom: Editor (F8) and Dock (`Ctrl+\``), moved here from the
    transport;
  - icons from `ui/icons.rs`, each with a tooltip naming its shortcut (the
    UX0 rule that a tooltip is where a binding is written down);
  - the open flyout's icon wears the selection look (outline + tint, never
    the accent).
- **A flyout** floats over the preview's left edge at x = rail.right. It is
  336 logical wide and as tall as the window below the top bar; it may cover
  the transport and band, because it is transient.
  - It closes on Escape, on a press outside it, or on its icon.
  - It is modal for presses (they don't fall through to the preview or the
    band), not for keys.
  - It draws over the scene with an opaque `ui_surface` fill and a
    `ui_control_edge` border.
  - It is a `TextEntrySurface`-free surface: no keyboard capture.
- **Tracks flyout:** what the tracks panel holds today, on its existing widget
  ids, so claims don't move:
  - Open project / Add audio / Save / Save As;
  - the track list (selected = outline + tint);
  - each track's save state.

  The collapsed tracks strip (`collapsed_tracks_strip`) is deleted; the rail
  replaces it.
- **Scenes flyout:** the picker, plus the ASCII Import / Clear footer, which
  moves here from under the tiles. It is scene imagery.

  **The picker's shape changes, and with it the Pixel Cats decisions**
  (WORKSTATION.md: "six because six fit unscrolled", "two rows of
  label-width tabs"):
  - families become a left column (7 rows, one per family, no wrapping);
  - tiles are a 2-column grid on the right.

  At the 960x640 minimum the flyout is ~580 tall, so a family shows ≥ 20
  tiles unscrolled. **Pixel Cats can grow to ten stagings without a
  scrollbar.** Building the four unbuilt stagings stays out of scope.
  Scrolling remains for safety. `core::ui::scene_browser::PickerLayout`
  (TAB_ROWS, tab_slot, tiles) changes on purpose, with its exact-number unit
  tests.

  Tile posters (the scene at the playhead, CX-5's original ask) would come
  from the poster service with a forced scene. That's a follow-up; tiles keep
  names in 5c.
- **Probes and report lines:**
  - new `flyout=tracks|scenes|none` (by name).
  - `scene-family=F` now also opens the scenes flyout, so a capture shows what
    it browsed.
  - `scene-pick=NAME` is unchanged; it retargets through the shell method,
    not a tile press.
  - new `rail:` line: `rect=X,Y,W,H icons=tracks@…,scenes@…,editor@…,dock@… open=scenes flyout=X,Y,W,H`.
  - `scene picker:` gains `tiles=name@x,y,w,h;…` and `footer=import@…,clear@…`
    while the flyout is open. Every probe that presses the picker or the
    footer computes its point from these, never from a constant (the knot
    check's 82x252→231 and the footer's 403→383 have each moved twice this
    week).
- **Fullscreen** hides the top bar, rail, flyouts and band, as today. The
  **Editor** frame hides them too ("inactive workspace panels are hidden").
  The **welcome screen** is unchanged: no track means no rail or top bar.

## 2. The transport (Rail)

The mockup's row under the preview:
- a round play/pause button (38 px circle, ink fill, the icon in `ui_surface`);
- the timecode, position in ink and length muted (5a), in the tabular face
  at `UI_FONT_VALUE + 2`;
- the existing scrub bar, kept because review 1.9 made it the seek control
  and its gate checks stand;
- the fine-seek trio;
- chips on the right: the readout toggle, mute plus a volume chip ("70 %"),
  and fullscreen.

Tune / Export / Lyrics / Assist leave the transport for the top bar's tabs;
Editor and Dock go to the rail. `core::ui::transport_bar`'s ladder loses the
panel-button rung, and its 48 exact asserts move on purpose. **Monotonicity in
width stays the property.**

New `transport:` line: `play=X,Y,W,H mute=… volume=… fullscreen=… readout=…`.
The mute-tooltip probes read the button from it instead of `preview frame:`
plus 25.

## 3. The lane head (Rail)

The event row (66 px) and the segment controls row (26 px) become **one 26 px
lane-head row** above the plan lane:
- left: `PLAN · 27 RUNS · 70 SECTIONS` (micro, tracked) and the **Auto** toggle;
- middle, **in context**:
  - with a section selected: `Section 45 · Stage · 3:10.470` with ◀ ▶,
    *Split here*, *Capture tuning*, *Delete*;
  - with none selected: *Split here* and the hint "click a section to edit it";
- right: the event markers as chips — *+ Feel*, *+ Scene*, *+ Custom*,
  *Clear*.

The event row's caption ("6 manual markers – a marker carries one value…")
becomes those chips' tooltip.

Widget ids stay in their namespaces (`EVENT_ROW`, `TIMELINE` 20+). Every
gesture stays reachable, and the gate presses them by rects from a new
`lane head:` line.

`EVENT_ROW_HEIGHT` leaves the band budget. The plan section becomes
26 + 5 + 44 + 5 = 80 (unchanged). The band shrinks by 66 px in every state,
which is where the preview's height comes from. The Export and Assist panels
are bottom-anchored (their content ends `UI_PANEL_PADDING` above the window),
so their rows and the gate's EX1/PX3 click points keep their y. **Verify that
on the first capture; don't assume it.**

## 4. The top bar (Top bar agent)

- **Left:** the mark (the mockup's conic swatch), then the project title: the
  `.musi` stem, or the audio file's stem with no project.
- **Then:** `5:22 · 1080p 16:9 · 30 fps` from `render_config`, and the save
  state as a dot plus word. Saved, Unsaved, Save failed (reason) and No project
  file are the same four states the tracks header shows today. The current
  pixel check of that badge (`headless_check.sh`, the TRACKS header crop at
  180,4) moves here.
- **Artist:** nothing on disk carries one (no tag reader; `.musi` has no
  field). The slot is omitted, not faked. Whether a `.musi` gains
  `metadata.artist` is the operator's call; note it in the report.
- **Mode tabs: views onto existing state, never a second copy of it.**

  | Tab | Means |
  | --- | --- |
  | Direct | `UiPanel::None`, inspector closed |
  | Lyrics | `UiPanel::Lyrics` |
  | Look | the Tune inspector open |
  | Export | `UiPanel::Export` |
  | Assist | `UiPanel::Assist` |

  The keys (T, etc.) keep working. A tab is selected when its state holds:
  outline + tint, not the accent.
- **Right:**
  - **Still** (`ShellCommand::ExportStill`, exists);
  - **Contact sheet** (new `ShellCommand::ExportContactSheet`: a destination
    picker, then `export_contact_sheet` with `--sheet-at segments`, through
    the batch path the CLI already uses);
  - **Render…** (`StartRender`), `ButtonStyle::Primary`, **the one accent
    fill on the workspace**.

  If the contact-sheet wiring proves heavy, its button is drawn disabled
  with the reason, per "Missing beats pretending".
- **Consequences for the gate:** the accent census's `--primary` becomes 1 on
  *every* workspace capture, because Render is always there. That's one
  argument per call; the slab control is unchanged.
- **New `top bar:` line:**
  `title="…" info="5:22 · 1080p 16:9 · 30 fps" save=saved tabs=direct*,lyrics,look,export,assist actions=still,sheet,render rects=…`.

## 5. `core::ui::workspace_layout` and the differential harness

- `WorkspaceSidebar::layout` (the oracle's `workspace_sidebar_layout`) is
  what 5c stops drawing. **It is retired, not left pinned and unused:** a
  harness that pins code the window no longer calls proves nothing.
  - `WorkspaceSidebar`, `TracksPanelMode` and their constants go.
  - The two constants `lyrics_editor_layout` reads
    (`WORKSPACE_SCENES_MINIMUM`, `WORKSPACE_TRACKS_SINGLE_MINIMUM`) move into
    it as named legacy floors with unchanged values, so the lyrics band
    budget does not move in 5c. Relaxing it now that no sidebar competes for
    height belongs to the secondary-panel wave.
- **New `WorkspaceRail::layout`** in the same module (pure): the rail rect, the
  icon rects, the flyout rect for a window, scale and top-bar height. Exact
  unit tests, plus the shell's `WorkspaceFrame` tests (`shell_layout.rs`),
  whose pins move. `ShellWidths` keeps the inspector rules (the 620 px preview
  floor and the 240–520 clamp); `tracks_width` becomes the rail width.
- **`tools/differential_layout.sh`:**
  - the oracle section drops the sidebar rows and keeps `UiRect` primitives
    and `TimelineBand`, both still used and still compared with the C;
  - a `--- post-legacy (no oracle) ---` section prints the rail layout over
    the same sweep, diffed against a checked-in
    `tests/differential/layout_post_legacy.txt`, as `settings_dump` does.
  - **Negative controls, run and recorded:** perturb the rail width by
    1/16 px and the flyout's top by 1 px; both must fail the post-legacy
    diff while the unit tests stay green (the layout harness's own history).
  - It stays out of `verify.sh` (it reads the oracle checkout).
- **`docs/DIVERGENCES.md` rows:**
  1. sidebar → rail + flyouts, with the harness retirement and why;
  2. event row + segment controls → lane head;
  3. the transport row;
  4. the top bar and its tabs as views onto `UiPanel`/inspector state;
  5. the rail stopping at the band;
  6. the Pixel Cats picker constraint lifted.

## 6. Everything that addressed the sidebar

| Thing | 5c |
| --- | --- |
| Track list, actions, save badge | tracks flyout (same ids); the badge moves to the top bar |
| Collapsed tracks strip | deleted; the rail replaces it |
| Scene tiles, family tabs | scenes flyout; tile/tab rects on `scene picker:` |
| ASCII Import / Clear footer | scenes flyout footer; rects on `scene picker:`; D2 probes from them |
| `chrome:` line `tracks Stacked save TracksPanel` | `rail=closed/tracks/scenes save=TopBar` |
| Splitter between sidebar and preview | deleted (no sidebar to drag). `ui.json`'s `sidebar_width` stays readable and is ignored, with a comment: old files must load |
| Recent projects | welcome screen only, unchanged |
| `scene-pick=`, `scene-family=` | by name; `scene-family=` also opens the flyout |
| knot check's tile press | from `tiles=` |
| Gate band probes (export, lyrics, assist, D4, lanes) | x unchanged; y re-derived from `scene runs: lane=` / `timeline:` / `lane head:` / `transport:` where it moves |
| Dark section, accent census | `--primary 1` on workspace frames; the lane window unchanged |
| `dark_theme_check.sh`, `ui_ux_check.sh` (not in verify) | their hover points updated, or noted as not re-run |

## 7. Scale, minimum, dock

- **Logical units throughout:** rail 52, top bar 46, flyout 336.
  `effective_scale` already caps the scale so the logical window is ≥ 960x640,
  so every number above holds at 100–200 %.
- **Gate captures** at 1280x720, 960x640, 1600x900@125 and 2560x1440@150, in
  both appearances.
- **At 960x640 with the Tune inspector open:**
  - the preview floor rule (`ShellWidths`: 620 px or 30 %) still decides;
  - with the rail at 52 instead of ≥ 168 the preview gets ~360 px more width
    than today;
  - the 168 px "compact rail" branch in `ShellWidths` goes.
- **With the dock open:** the rail sits at the workspace's left edge and the
  dock at the window's right. The dock never squeezes the workspace below
  640, so the rail, the 620 px preview floor and the transport fit at 1280 +
  dock.

## 8. The split and the seam

**Seam commit (Rail, first, gate green, nothing drawn differently):**
- `WorkspaceFrame` gains `top_bar: UiRect` and `rail: UiRect`, both 0 height
  or width in the seam, so nothing moves yet;
- `ui/top_bar.rs` holds `impl Shell { fn top_bar(&mut self, d, input, rect, commands) }`,
  drawing nothing, and `fn describe_top_bar(&self) -> String`;
- `Shell::draw` calls it after the preview chrome;
- `ShellCommand::ExportContactSheet` and its `main.rs` arm (a "not built yet"
  notice);
- the `top bar:` and `rail:` printlns;
- probe key `flyout=` in `cli.rs`.

**Rail owns:**
- core: `workspace_layout.rs`, `scene_browser.rs`, `transport_bar.rs`,
  `lyrics_editor_layout.rs` (the constants only);
- `examples/layout_dump.rs`, `tests/differential/layout_post_legacy.txt`;
- app: `ui/shell_layout.rs`, `ui/shell.rs`, `ui/panels/events.rs`,
  `ui/panels/scene_timeline.rs`, `ui/icons.rs`, `ui/theme.rs`, `ui/widgets.rs`;
- tools: `headless_check.sh`, `differential_layout.sh`, `knot_check.*`,
  `accent_census.py`, `run_lane_check.py`, `dark_theme_check.sh`,
  `ui_ux_check.sh`;
- docs: `WORKSTATION.md`, `UI_TOOLKIT.md`, `docs/DIVERGENCES.md`.

**Top bar owns:**
- `ui/top_bar.rs`;
- `main.rs` (the contact-sheet command path, the `top bar:` line, anything
  the dock needs);
- `ui/panels/export.rs`, only what the contact-sheet command needs;
- `cli.rs` beyond the seam;
- `tools/top_bar_check.sh` (new, its own isolation, like `dock_check.sh`)
  plus its one `verify.sh` step.

It hands Rail the exact replacement for the save-badge crop in
`headless_check.sh`, and drafts its AGENTS rows in
`build/redesign/wx5c-topbar-rows.md`. It reads `UiPanel` and the inspector
through existing `Shell` methods; if it needs a new one, it asks Rail.

**Separate worktrees,** as Service had: the Top bar agent in its own branch from
the seam commit, gate on `:79`; Rail on `wx5-redesign`, `:78`. The lead merges by
cherry-pick. `CODE_MAP.md` is regenerated once at merge.

**Top bar's gate checks** (`top_bar_check.sh`), each with a control:
- a tab press opens its panel (`panel:` line), and a press in the gap
  between tabs changes nothing;
- Still and Contact sheet write their files (`--ui-probe save-to=`
  extended), and a refused destination writes nothing;
- the saved-state word carries ink (crop from `top bar:`), and a frame with
  the bar hidden in fullscreen has none.

**Rail's gate checks:**
- a rail icon press opens its flyout, and a press in the rail's gap leaves it
  closed;
- Escape and a press outside close it;
- a tile press through the flyout selects the scene (the knot check's
  `claimed=`);
- every family's tiles fit unscrolled at 960x640 (`scene picker:` reports
  `offset=0/0` for each family);
- the lane-head context controls are present only with a selection, and a
  press on *Split here* adds a section (`scene runs:` sections +1);
- lane alignment, runs, census and the dark section re-run at the new frame.

## 9. Order inside Rail

1. Seam.
2. `WorkspaceRail` plus the frame, with the rail drawing empty icons. Gate.
3. Flyouts (tracks, then scenes and the footer), the picker shape, the
   report lines. Gate.
4. Lane head (event row and segment controls). Gate.
5. Transport. Gate.
6. Differential harness and docs.
7. Merge Top bar, census `--primary 1`, final verify, release build,
   captures (both appearances, 1920/1280/960, 125 %/150 %, dock open, a
   flyout open).
