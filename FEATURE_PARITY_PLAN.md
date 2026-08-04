# Rusty Musializer Feature-Parity Plan

This is the **single authoritative task list** for reaching feature parity with
the frozen C Musializer. If another document, source comment, old agent handoff or
ignored worktree names unfinished work, reconcile it here before acting on it.

`REWRITE_PLAN.md` is the historical design and session record. Its NOTE ENTRIES
remain valuable evidence, but its phase plans and agent handoffs are no longer a
live queue. `docs/PHASE0_INVENTORY.md` is a behavioral inventory of the C oracle,
not a task list. `AGENTS.md` contains repository rules and deliberate exclusions.

`UX_PERSPECTIVE_REVIEW.md` is the evidence document for a committed user-facing
review, not a second task list. Its complete set of confirmed defects, workflow
findings, opportunities and verification blind spots is normalized as **UX0**, the
next task below. The review's numbered sections retain the detailed evidence;
UX0 is authoritative about whether each item is still open.

## Target and meaning of parity

- **Oracle:** `../musializer`, branch `master`, frozen commit
  `9300af942bd00d8c85fc4e3c8c02cf2b6356764f` (`9300af9`). It is read-only.
- **Audited Rust baseline:** `a664b7d`, 2026-08-01.
- **Baseline evidence:** `tools/verify.sh` is 19 passed / 0 failed: thirteen
  differential harnesses plus the headless capture gate.
- **Feature parity means observable capability parity.** A C workflow must be
  available in Rust and its persistent or rendered result must retain the same
  meaning. Mechanisms, ownership, UI composition and implementation structure may
  improve when the result is equivalent or better.
- **Code-detail equality is not a goal.** Do not reproduce global state, hot
  reload architecture, tinyfiledialogs, unsafe pointer shapes or C source layout
  merely because the oracle uses them.
- **A difference is a parity gap** when it changes a `.musi` file, rendered video,
  analysis number, documented command, user workflow or durable edit.
- **The C is legacy (operator decision, 2026-08-03).** See `AGENTS.md` — the
  rewrite supersedes the C; formats may evolve past it with a `schema_version`
  bump, harnesses pin kept behaviour rather than forbid change, and this plan
  remains the queue only for capability parity a user would otherwise lose.

### Deliberate exclusions

These do not block parity unless the operator changes the target:

- Microphone capture (`MUSIALIZER_MICROPHONE`)
- Hot reload, including functional `--reload-once`
- Non-Linux platforms, including Windows, macOS and OpenBSD; this is a
  Linux-first rewrite

Exact pixel identity, C source organization and reproducing known C defects are
also not goals. Any new deliberate product-level exclusion must be approved and
added both here and to `AGENTS.md`.

## Current fit

The following are already delivered and must stay green:

- All ten scene drawing implementations and deterministic scene state
- Audio playback, analyzer and beat tracker
- Per-scene settings, routes, route persistence and preset model/UI
- Multi-track workspace and `.musi` open/save with asset verification
- Bidirectional C/Rust `.musi` differential round trips and frame-lane boundary
  selection: 2,550 values, delta 0
- FFmpeg export, deterministic full/windowed rendering and publication
- Lyrics editor, caption-style editor and local/imported font runtime support
- Assist UI/state/controller and analysis-bridge import
- Manual event capture, clear/undo and scene-cue model
- Whole-track waveform, Song Atlas map and image-backed ASCII derivations
- All bottom-panel layouts and the headless scene/panel capture sweep
- Native-size Space Grotesk shell typography with compiler-enforced draw and
  measurement boundaries

The remaining work is concentrated at integration boundaries and missing product
entry points. A green pure-module harness does not prove those boundaries.

## Operator-requested UI scale and workspace resizing

Added 2026-08-01. This is a product extension rather than a parity gap: scale
1.0 and every automatic split must retain the frozen layout, while larger UI
scales and user-sized splits are Rust-side capabilities.

- [x] Introduce an explicit logical UI coordinate space, stepped UI scale and
      deterministic command-line/probe override. Keep scene/export pixels
      independent from shell scale.
- [x] Rasterize Space Grotesk at the physical sizes each scale needs while
      measuring and laying out in logical units; do not magnify the 1x atlases.
- [x] Map pointer and scissor coordinates through the same scale boundary as
      drawing, and report the effective scale in headless evidence.
- [x] Add draggable sidebar, Tune-inspector and bottom-panel splitters. Preserve
      the existing content-aware layout as Auto, clamp every user split against
      the preview and panel floors, and reset a split by double-clicking it.
- [x] Persist scale and split preferences per user, outside `.musi`; a corrupt
      preference file must survive unread and must not be overwritten.
- [x] Keep all existing 1x gates green and add silent headless captures at 125%,
      150% and a 2560x1440 window, including font, hit-target, clipping and
      splitter evidence.

Completed 2026-08-01. `tools/verify.sh` is 20 passed / 0 failed. The private-Xvfb
gate covers 100%, 125%, explicit 150%, 1440p Auto selecting 150%, custom logical
splits, and a scaled tooltip hit target; the three new layout frames were also
inspected at original resolution. Every UI font report used a scale-matched
native atlas with zero non-native requests. The scene and export paths remain in
framebuffer pixels outside the shell camera.

## Ordered completion map

**UX0 is the next task.** Its work may be divided across agents where ownership
does not overlap, but the complete review is one closure tranche and no named
item may be silently dropped. When UX0 closes, resume the remaining A-G work in
dependency order. P0 establishes the shared render path on which later assertions
depend. P1 makes authored project behavior real. P2 closes missing C workflows.
P3 packages the external product. P4 removes false status and proves the result.

```text
NEXT user-perspective closure
  UX0-A confirmed defects -> UX0-B workflow friction -> UX0-C opportunities
                                                   -> UX0-D verification gates

P0 project-aware frame path
  A1 frame lanes -> A2 lyric overlay -> A3 integration evidence
                  -> B1 automatic switching -> B2 cue semantics -> B3 evidence
                                               -> B4 interactive control

P1 durable editing
  C1 dirty marking -> C2 draft/context guards -> C3 reset workflow
                   -> C4 all-track autosave -> C5 project-preset adoption

P2 missing entry points
  D1 drop dispatch -> D2 ASCII image UI
  D3 lyrics TSV import/export
  D4 timeline information, pan and transactional seek
  D5 CLI execution semantics
  D6 Tune reachability/toggles -> D7 keyboard workflows -> D8 actionable notices

P3 complete external bundle
  E1 support-file manifest -> E2 Assist -> E3 Google Fonts -> E4 dist/doctor

P4 honesty and gate
  F0 robust shell typography -> F1 stale UI/status removal -> F2 stale handoff cleanup
  G1 integration gates -> G2 final feature audit
```

## NEXT — UX0: close the complete user-perspective review

**Source and scope:** `UX_PERSPECTIVE_REVIEW.md`, reviewed at `d22f9de` while
the D4 scene-timeline work was deliberately excluded. Revalidate every code
pointer against current `HEAD` before editing. This task includes the **entire**
review: all 14 confirmed defects, every named workflow recommendation, all ten
product opportunities and all four verification blind spots.

UX0 deliberately includes operator-requested product work beyond strict C parity.
Keeping it here is intentional: this repository has one live queue, and the
review must not become a competing roadmap merely because some recommendations
extend rather than reproduce the oracle.

Confirmed defects, workflow failures and verification blind spots require an
implemented fix plus proportionate evidence. Product opportunities are accepted
into this tranche as real work, not a parking-lot list. An item can leave UX0
only as implemented, merged into a more specific task below with its UX0
acceptance criteria preserved, or explicitly deferred/excluded by the operator
with the reason recorded here. Existing C2/D6/D7/D8/E2/E4/G1 tags are dependency
links rather than duplicate queues; closing the older task does not implicitly
close the UX0 item.

### UX0-A — confirmed defects

- [x] **UX0-A01 — sub-millisecond lyric cue panic:** make timing-row arithmetic
      total for every loadable cue, align the model/editor minimum-gap policy and
      add a regression fixture that previously panicked (`review` 1.1).
      Done in `e650c76`: total `clamp_form_start`/`clamp_form_end` in core,
      form and lane unified on `LYRIC_MIN_CUE_SECONDS = 0.02`, loadability
      unchanged, panic shape pinned by regression tests.
- [x] **UX0-A02 — stranded pointer ownership:** release a widget claim when the
      physical button is up even if its original panel disappeared; clear or
      reconcile splitter drags across fullscreen and test both paths (1.2).
      Done in `a1aafa5`: `release_stranded_claim` in `begin_frame` (previous-
      frame pointer only, so no click can be eaten), splitter drags abandoned
      and scrubs completed on fullscreen/inspector, 4 negative controls.
- [x] **UX0-A03 — cross-track lyric draft corruption [C2]:** bind drafts to their
      owning track and guard every context switch so a stale cue id cannot update
      another document; cover Apply, Discard and Cancel (1.3).
      Done in `2cb8385`: edits stamped with `owner_slot` at push and refused at
      drain on mismatch; SelectTrack/panel/open/add-audio guarded, scene change
      deliberately not (same document), quit folded into confirm_close. C2
      should build on `lyric_draft_allows_context_change`.
- [x] **UX0-A04 — blank Export panel:** derive its minimum height from panel
      geometry, draw an explanation when content cannot fit and require visible
      ink in the headless capture rather than trusting a report line (1.4).
      Done in `7e980a9` + the wave-3 commit: `EXPORT_MIN_BAND_HEIGHT` derived
      from the panel's own rows (including the 27 px TIMELINE header the first
      derivation missed — caught by the ink gate, not by the replay test, which
      now models every rect on the real path), both the automatic budget and
      the persisted-split floor consume it, a too-small box draws a named
      notice, and the gate measures ink (darkest pixel 20 vs blank ~247). The
      full control rows are in a default-size capture for the first time.
- [x] **UX0-A05 — non-Latin authored text:** render lyric rows, editable lyric
      text and track names through an atlas that contains the project's glyphs;
      ellipsize deliberately and assert the seeded editor does not substitute
      question marks (1.5).
      Done in the wave-3 commit: `Faces::authored()` serves the caption atlas
      (fallback ladder caption → caption-alt → ui → default, reported), cue
      rows/field/track names all draw through it, rows ellipsize with U+2026,
      and the gate asserts `face=caption missing=0` on the seeded fixture.
      `missing=0` means the bundled repertoire, not every script — Hebrew,
      Arabic and CJK need the imported-face path, pinned by a test.
- [x] **UX0-A06 — font-search shortcut leakage [D7]:** make every text-entry
      surface participate in one focus policy and prove that typing suppresses
      every global shortcut (1.6).
      Done in `a1aafa5`: `TextEntrySurface::ALL` swept by
      `text_entry_has_focus`, focus gated on the surface being drawn, and the
      suppression test enumerates the surfaces so a new one fails until wired.
      D7's keymap work should reuse the enum.
- [x] **UX0-A07 — hidden Tune edit scope:** state whether Tune is editing base
      scene settings or a particular cue snapshot, including cue/time identity
      where applicable (1.7).
      Done in `b73383d`: header badge from `Track::active_cue`, pure
      `tune_scope_label` under test, `tune scope:` probe-report line as
      capture evidence.
- [x] **UX0-A08 — misleading scene reset [C3]:** make routed values explicit in
      reset behavior/notices and give Reset the Confirm/Undo workflow already
      required by C3 (1.8).
      Done in `b73383d`: arm/confirm keyed per (track, scene), routed-count
      notice, routes untouched. Undo itself remains C3's obligation.
- [x] **UX0-A09 — RMS/progress ambiguity:** either make the transport bar a real
      seek control with secondary level indication or restyle it unmistakably as
      a meter; place analyzer telemetry under the HUD flag (1.9).
      Done in the wave-3 commit: the bar is a real scrub control (pause on
      press, one Seek on release, resume; transactional like the timeline
      scrubber; inert grey when `transport_seekable` is false) with RMS as a
      4 px secondary strip, and the bands/peak/rms caption is HUD-gated with
      probe runs keeping it on.
- [x] **UX0-A10 — raw error variants:** replace both user-facing `Debug` formats
      with the existing `Display` messages and test the visible text (1.10).
      Done in `787010a`: both sites use `Display`, `LyricsValidation` gained a
      cue-naming `Display`, and `errors_display_as_sentences_not_variant_names`
      pins the visible strings.
- [x] **UX0-A11 — ineffective notice tray [D8]:** provide dark-surface contrast,
      wrapping/clipping and severity-derived persistence so consequential errors
      remain legible and present (1.11).
      Done in the wave-3 commit: opaque dark card, all severities ≥6.5:1 (the
      old failing pairs kept as a negative control), detail wraps to ≤3 lines
      with honest ellipsis, `Severity::dwell` at the notify seam (Error
      persists until dismissed, Warning 14 s, Info 6 s), per-notice close box.
      D8's rebuild starts from this.
- [x] **UX0-A12 — Tracks/Save disappearance:** retain current-track identity and
      a save route when a bottom panel consumes the sidebar; never draw an
      illegible fractional row (1.12).
      Done in the wave-3 commit: a 26 px collapsed strip (track name through
      the authored face + Save with an explanatory tooltip) where the panel
      vacated, Ctrl+S/Ctrl+Shift+S bound, rows below 60% visibility not drawn,
      a 30-configuration sweep asserts a save route always exists, and the
      `chrome:` report line is the capture evidence. `workspace_layout` is
      untouched — the layout harness still binds.
- [x] **UX0-A13 — obscured Assist refusal [C2]:** keep the Apply-blocking reason
      visible beside artifact actions and add Candidate, Running and Failed probe
      states so consequential Assist bodies are reviewable (1.13).
      Done in `22292e0`: reserved reason slot (beside/above), buttons never
      move, non-overlap swept over 1361 widths, core untouched so the assist-ui
      harness still binds. Probe states landed with it; captures follow in the
      UX0-D02 gate run.
- [x] **UX0-A14 — silently shadowed lyric overlaps:** show overlap/shadow state in
      the lane and form, warn which cue loses display time and clamp a new cue's
      default end to the next cue where possible (1.14).
      Done in `e650c76`: `LyricsDocument::cue_shadow`/`shadowed_cues` in core
      (cross-checked against `at_time`), lane hatching, form warning naming the
      shadowing cue, `shadowed N` in `describe()`, Add-cue default clamped.
      `at_time` unchanged.

### UX0-B — workflow friction and trust

- [ ] **UX0-B01 — continuous save state [C1/D8]:** show durable dirty state on
      the track and Save affordance; distinguish saved, unsaved and save-failed
      state before quit is attempted (`review` 2, Saving and trust).
- [ ] **UX0-B02 — lyric seek/stamp loop:** make cue selection seek-capable, add
      playhead stamping for start/end, auto-advance after Apply and preserve a
      usable play/tap loop while lyric text has focus (2, lyric timing loop).
- [ ] **UX0-B03 — lyric edit recovery:** add undo/inverse edits and safe deletion
      for single and multi-cue timing changes, including accidental lane drags.
- [ ] **UX0-B04 — precise lyric timing controls:** support an established
      fine/normal/coarse nudge ladder, hold-repeat or equivalent efficient input,
      typed times and one consistent minimum cue gap in form and lane.
- [ ] **UX0-B05 — expose lyric split/merge:** connect the already-ported model
      operations to the editor with playhead-aware behavior and tests.
- [ ] **UX0-B06 — large lyric-document navigation:** add visible/interactive
      scrolling, keyboard navigation, jump-to-playhead and readable ellipsis for
      long rows at 100+ cues.
- [ ] **UX0-B07 — visible lyric draft state [C2/D8]:** mark an edited draft and
      use consistent Apply/Cancel/Discard language in controls and refusals.
- [ ] **UX0-B08 — understandable route affordance [D6]:** replace or explain the
      ambiguous `~` control, restore useful static/dynamic tooltips and make the
      reason for a disabled route action visible.
- [ ] **UX0-B09 — precision Tune controls [D6]:** support typed values,
      wheel/keyboard nudging, per-setting reset and efficient navigation across
      all 104 bands.
- [ ] **UX0-B10 — reviewable Assist reasoning:** carry section reasons, semantic
      summaries and semantic notes through the candidate boundary and render them
      before Apply (2, Assist).
- [ ] **UX0-B11 — actionable timing uncertainty:** preserve cue confidence and
      uncertainty identity and visually identify the exact cues needing review.
- [ ] **UX0-B12 — selective/reversible Assist Apply [C2]:** allow per-lane
      inclusion and retain a pre-apply snapshot with a visible Undo action.
- [ ] **UX0-B13 — meaningful Assist progress [E2]:** surface phase/stage progress
      from helpers so long-running work is distinguishable from a stalled job.
- [ ] **UX0-B14 — actionable Assist failure [D8]:** show the useful tail of the
      failure log and provide Retry/Open actions rather than only a clipboard
      path.
- [ ] **UX0-B15 — remote-mode prerequisites [E2]:** check credentials before
      starting, disclose relevant cost/privacy prerequisites and explain the MiMo
      provider/model route in visible language.
- [ ] **UX0-B16 — recoverable missing-support state [E4]:** name missing files,
      provide the support/doctor remedy and make the state actionable.
- [ ] **UX0-B17 — consistent Assist visual semantics:** reserve amber for actual
      staged/warning/missing states and use the normal selected-state language
      elsewhere.
- [ ] **UX0-B18 — discoverable keymap [D7]:** expose Tab/Shift+Tab, UI scale,
      End and splitter reset (plus the full supported set) in a visible keymap or
      equivalent help surface.
- [ ] **UX0-B19 — useful tall-window layout:** use surplus vertical space for
      content such as larger rows or preset slots rather than simultaneous dead
      regions, without regressing minimum-window behavior.

### UX0-C — product opportunities

**Landed 2026-08-04 (operator-requested lyrics/typography batch, first
post-legacy work):** free INK/PLATE colour picking (hand-built SV/hue/alpha
picker beside the swatch rows), the always-visible "Import a face..." route
(`CaptionFormLayout`, form minimum 152 → 160), at-size caption atlases and an
SDF-typeset Cadence replacing the magnified 64 px atlas, the rounded plate
outline, and `caption_style.effects` — audio-drivable glow (RMS/bass/beat/
flux/time pulse and hue drives), soft shadow and authorable plate roundness,
resolved per frame in `core::project::caption_effects` and serialized only when
authored so pre-effects `.musi` files stay byte-identical. Divergence rows in
`AGENTS.md`; captures and luma/chroma gates in `tools/headless_check.sh`
(`lyrics-effects-*`, `lyrics-picker-*`, `fx-glow`, `fx-soft-shadow`).

**Review round, 2026-08-04 (operator played with the landed batch):** all six
landed the same day. The glow is a real two-pass Gaussian over an offscreen
buffer (`runtime::halo`); the backing tuners live beside BACKING on the Style
pane; a hue drive saturates achromatic bases with the drive value; PULSE/HUE
carry a full mapping editor (quiet/loud in→out, curve, clamp, live meter,
transfer graph) built from `ui/mapping_editor.rs` — the same componentry the
Tune route editor now draws with, over the same `evaluate_mapping` semantics —
persisted as optional `pulse_tuning`/`hue_tuning` blocks that leave earlier
files byte-identical; scene routes gained the `time` source; and tooltips
cover the caption panes and the Tune editor. Probe `tune=pulse|hue`, capture
`lyrics-tune-pulse-960x640`; probe runs without `hover=` can no longer
photograph a stray tooltip (the dwell is infinite unless a tip was asked for).

**Items:**

- [x] **UX0-C11 — real glow:** the 17-tap additive glow reads as discrete copies
      ("gravitational lensing") once RADIUS grows, and 100 % GLOW is thin.
      Replace with an offscreen render-texture blur so radius widens a single
      halo; re-measure the fx gate deltas and re-run the zero-strength control.
      *Done 2026-08-04: two-pass separable Gaussian in `runtime::halo` +
      `halo_blur.fs`, composited additively twice; gate glow delta 123 (taps
      measured 125), zero-strength control exactly 0, radius sweep 0.08/0.3/0.6
      → 169/135/95 with one widening halo and no copies
      (`build/glow-evidence/`); glow export deterministic across runs and
      intact inside the supersampled target. Follow-up, same day: the soft
      shadow rides the same blur through `halo_mask.fs` (luminance-as-alpha,
      normal blending); the seeder's soft-shadow fixture is now bright enough
      to measure (soft-vs-hard delta 152, was 3-4), the zero-blur variant
      degenerates to the legacy composition at exactly delta 0 (a standing
      in-gate control). `GLOW_TAPS`, `SHADOW_TAPS` and `GlowTap` are removed
      from `core::project::caption_effects` — both effects now draw from one
      blur, with no tap table left. Evidence under `build/shadow-evidence/`.*
- [x] **UX0-C12 — effects pane grouping:** BLUR/SHADE/ROUND are backing tuners,
      not effects; move them to the Style pane beside BACKING (contextual to
      Shadow/Plate) so the Effects pane is glow-only.
- [x] **UX0-C13 — achromatic hue drives:** a hue drive on a white/grey/black glow
      colour does nothing; blend saturation in with the drive so an achromatic
      base still sweeps colour.
- [x] **UX0-C14 — drive tuning layer:** give PULSE/HUE the Tune popover's
      quiet/loud in→out ranges, curve and live readout, by extracting the
      mapping editor from `panels/tune.rs` into shared componentry.
- [x] **UX0-C15 — Time route source:** add the caption pane's Time drive (8 s
      triangle) as an `AnalysisSource` for scene routes; keep the route
      differential harnesses green (additive token).
- [x] **UX0-C16 — styling tooltips:** tooltips across the caption Style/Effects
      panes and the Tune mapping editor, using the existing widget tooltip path.
- [ ] **UX0-C17 — give the caption tune editor room:** the mapping editor fits
      the caption pane's 160 px only by cutting Swap, exiling the transfer
      graph to the other column and packing the curve row 6 px from the floor.
      Growing the pane when a disclosure opens means changing
      `lyrics_editor_layout`'s harness-pinned panel heights — a deliberate
      layout change, so it queues rather than sneaks in.

- [ ] **UX0-C01 — clip export:** expose render in/out or start/duration controls
      and drive the existing windowed `RenderPlan` path (`review` 3.1).
- [ ] **UX0-C02 — vertical and square output:** add explicit width/height output
      formats without changing the C-ordered persisted resolution enum, then
      capture-audit every scene at tall and square aspect ratios (3.2).
- [ ] **UX0-C03 — lyric tap timing:** deliver the play-and-tap stamping workflow,
      row seek and advancement described by B02 as a polished primary flow (3.3).
- [ ] **UX0-C04 — preset audition/A-B:** make preset exploration reversible with
      hold-to-audition or an explicit settings snapshot comparison (3.4).
- [ ] **UX0-C05 — scene thumbnails:** generate/cache deterministic preview stills
      so scene choice is visual rather than ten text-only names (3.5).
- [ ] **UX0-C06 — recent projects:** use the welcome screen's spare region for a
      durable, failure-tolerant recent-file list and direct reopen flow (3.6).
- [ ] **UX0-C07 — Tune randomize/mutate:** add bounded Surprise/Nudge operations
      with the same reversible audition semantics as C04 (3.7).
- [ ] **UX0-C08 — project-level palette/look:** define a coherent track-level
      color treatment mapped across scene descriptors and automatic changes
      without breaking routes or saved settings (3.8).
- [ ] **UX0-C09 — cover-art/logo layer:** generalize project image assets into a
      track-level visual layer and define its scene-host/render/route semantics
      (3.9).
- [ ] **UX0-C10 — still-frame export:** publish a user-selected supersampled
      frame through the offline render path and reuse it where appropriate for
      thumbnails/cover output (3.10).

### UX0-D — verification blind spots

- [x] **UX0-D01 — pixel-backed panel gates:** measure expected ink/content inside
      every panel rect, beginning with Export; a textual state report alone is
      insufficient (`review` 4.1, feeds G1).
      Export done (`7e980a9` + wave-3): ffprobe ink assertion in the panel
      rect, and it immediately earned its keep — it caught the 27 px header
      the derivation and its unit test both missed. Extending the technique
      to the remaining panels stays a G1 item.
- [x] **UX0-D02 — Assist state captures:** make Candidate, Running and Failed
      deterministic probe states and visually gate their consequential content
      (4.2, feeds G1).
      Done in `22292e0` + `93972ca`: `--ui-probe assist=` synthesizes all
      three states in-process, seven captures per sweep (including the narrow
      reason-row branch), report-line assertions pin state/staged. The
      blocked-Apply reason is confirmed visible in the frame.
- [x] **UX0-D03 — non-Latin editor gate:** assert that the seeded authored text
      remains distinguishable and is not replaced by U+003F in editor surfaces
      (4.3, feeds G1).
      Done in the wave-3 commit: `missing=` in the `lyrics:` report line is a
      direct glyph-coverage count for the strings on screen (inference-free,
      unlike a `?`-pixel heuristic); the gate asserts `missing=0` on every
      lyric capture plus positive `face=caption` pins on the cue-pane frames,
      including a new capture that binds the Greek/Cyrillic line into the
      editable field.
- [x] **UX0-D04 — dark-overlay contrast coverage:** extend the palette/contrast
      sweep to severity text and every other semantic color used on dark overlay
      surfaces (4.4, feeds G1).
      Done in the wave-3 commit: every `theme::rgba` entry declares its
      `Surface`, the sweep measures each against its declared backgrounds
      (20 entries, 16 pairs, all AA), and the old failing chrome-on-dark pairs
      are pinned as a permanent negative control.

UX0 completes only when every checkbox above has implementation evidence and the
review document has a final disposition annotation or companion table for all
items. Record commits, captures, negative controls and any operator-approved
deferrals here as work lands; do not create a separate UX completion plan.

## P0 — make saved project content reach the renderer

### A1 — one project-aware `SceneFrame` path

- [x] Replace the preview and export uses of `..SceneFrame::idle(effective)` with
      one shared frame-building path.
- [x] At `time_seconds`, sample the current track's semantic lane with
      `project::semantic_lane::sample`.
- [x] Select the active lyric with `LyricsDocument::at_time`.
- [x] Merge manual and semantic events through `SceneEventMerge`; preserve the
      C ordering and semantic-id qualification already pinned by the differential
      harness.
- [x] Feed the same semantic, lyric and merged-event view to preview and export.
- [x] Preserve the no-track behavior: default semantics, no lyric, empty events.
- [x] Define failure behavior visibly. An invalid merge must be reported and use
      an empty frame view; it must not retain a prior track's events.

Observable closure:

- Semantic valence/tension/confidence modulates every scene that reads it.
- Cadence receives active lyrics instead of always drawing its ambient fallback.
- Loom receives its semantic lane instead of always relying on fallback data.
- Constellation receives both manual and model-derived event markers.
- Preview and export see identical project data at the same scene time.

### A2 — shared lyric captions

- [x] Port the application-side `draw_scene_lyric_overlay` composition using the
      existing `caption_layout` and `runtime::font` machinery.
- [x] Draw the shared overlay after every scene except Cadence, which owns its
      lyric composition just as the C app does.
- [x] Honor the project's caption anchor, width, size, margin, text/box colors,
      box mode and imported caption face in both preview and export.
- [x] Keep the overlay resolution-independent through the export pixel scale.
- [x] Add evidence for long UTF-8 text, the three-line/ellipsis contract, all box
      modes, a non-default anchor and an imported face.

### A3 — frame-boundary evidence

- [x] Generate a synthetic `.musi` containing lyrics, semantic events and manual
      events; do not commit user media.
- [x] Add a headless report line naming the active lyric id, semantic availability
      and merged event count.
- [x] Capture at least one shared-caption scene, Cadence, Loom and Constellation.
- [x] Export selected frames and assert that seeded project lanes change the
      output. Include a negative control that drops one lane and demonstrably
      changes the evidence.
- [x] Confirm the same fixture written by C and Rust produces equivalent lane
      selection at boundary timestamps.

Completion evidence (2026-08-01, `84ad8bb`): `ProjectFrameLanes` is the shared
preview/export constructor and owns a
fresh merge so an invalid lane logs its error and exposes zero events. The pure
suite pins no-track state, exact lyric/semantic boundaries, canonical ids/order,
UTF-8 three-line ellipsis, all box modes, a non-default anchor and 2x export
scaling. `tools/headless_check.sh` generates the `.musi` and bundled imported face,
captures Spectrum, Cadence, Loom and Constellation, reports
`lyric=1 semantic=available source=11 merged-events=4` in both preview and export
at `t=1.000`, and repeats both full outputs deterministically. Removing lyrics,
semantics or manual events changes both preview and export hashes and changes only
the named report fields. The expanded bidirectional project harness compares 510
values per path / 2,550 total at delta 0; its 1 ms-early lyric negative control
produced four boundary failures in each direct comparison (12 discrepancies).

## P0 — automatic scene plans and cue semantics

### B1 — drive automatic switching in preview and export

- [x] Apply `SceneSwitchTimeline::update` before frame construction on both paths.
- [x] Make `--auto-scenes` actually enable the imported/saved plan; reject an
      empty plan with the oracle's observable behavior.
- [x] Replay accepted Assist section plans and enabled `.musi` scene plans.
- [x] Apply a cue's captured settings snapshot before audio routes, matching the
      C precedence: base/cue settings first, per-frame routing second.
- [x] Reset the switch cursor and cue-settings state on seek, track change,
      project open, export start/end and plan enable/disable.
- [x] Keep preview and export scene selection identical for the same time.

### B2 — base-scene selection and manual `+ Scene` capture

- [x] When a user selects a different base scene, set the pending-selection state,
      remember the previous base scene and mark the project dirty.
- [x] Selecting a base scene while Auto scenes is enabled must disable playback of
      the plan, retain its cues and explain that result in the notice tray.
- [x] The first manual scene cue recorded after time zero must backfill the former
      base scene from zero before inserting the new cue.
- [x] Recording a cue must capture the selected scene's effective tuning snapshot,
      clear the pending flag and apply the cue immediately at the playhead.
- [x] Tune edits made while a cue snapshot is active must update that cue rather
      than silently changing only the base scene settings.

### B3 — scene-plan evidence

- [ ] Add preview and export tests at every cue start/end boundary, including seek
      backward, fast-forward and a windowed export beginning after cue zero.
- [ ] Assert selected scene id and a cue-specific setting, not merely the enabled
      flag or the words "automatic scene plan".
- [x] Prove a manual first cue at `t > 0` retains the prior base scene on
      `[0, t)`.
- [x] Add a negative control that disables the call to `update` and is caught.

Completion evidence (2026-08-01, `a664b7d`, operator-reported lyric deletion and
PCM scene cue repair): the lyrics form now assigns disjoint widget ids to Apply, Discard,
Delete and both timing rows. Reusing Delete's id for START -0.1 reproduced the
failure and the new collision test failed 8/9 before restoration; the isolated
Xvfb click now leaves 5 cues from the 6-cue fixture and clears the selection.
`Track::advance_scene_plan` is the shared preview/export driver and installs a
cue snapshot before routes. A two-cue 8-second project reports Loom after its
4.0-second boundary in preview and in a 0.2-second windowed MP4 beginning at
4.0; the same disabled project reaches Loom through `--auto-scenes`. Unit tests
pin base-scene pending/backfill state, exact boundary transitions, seek-style
reset and active-cue tuning. Returning before `SceneSwitchTimeline::update` made
the boundary test fail (`None` versus `Some(Spectrum)`) and was reverted.

### B4 — interactive automatic-scene control

- [x] Add the C workflow's visible Auto-scenes On/Off control for a non-empty
      saved or Assist-created plan; `--auto-scenes` must not be the only way to
      enable it.
- [x] Reset the switch cursor and cue-settings state on either transition, and
      restore the base scene when disabling the plan.
- [x] Retain the cues, mark the project dirty and report the cue count/state.
- [x] Capture enabled and disabled states and prove that the next preview/export
      frame follows the same plan state.

Audit evidence: the C control and transition are at `plug.c:2207-2226`; the Rust
Assist composition at `ui/panels/assist.rs:1089-1119` has no equivalent. The only
production enable path is startup `--auto-scenes`.

Completion evidence (2026-08-01, working tree): the Assist header now shows the
C workflow's `Current auto scenes: On/Off (N)` button whenever the current track
has a plan and the panel is at least 560 px wide. Its `ShellCommand` runs through
one track transition that refuses an empty plan, retains every cue, rewinds the
cursor, clears cue-specific settings, marks dirty and restores the base
`SceneInstance` on disable. A unit test pins empty-plan refusal, cue retention and
both-direction rewind. The silent Xvfb gate opens a generated two-cue project at
5.0 seconds and captures both states: disabled reports `disabled (2 cues)` and
Constellation, while `--auto-scenes` reports `enabled (2 cues)` and Loom; their
frames differ. The gate first failed by refusing a copied project whose bundled
audio directory was absent, proving the fixture still enforces asset identity;
copying the content-addressed bundle produced the green run.

## P1 — durable edits, guards and autosave

### C1 — complete dirty marking

- [ ] Audit every command that mutates data serialized into `.musi` and route it
      through one dirty-marking policy.
- [ ] At minimum, mark dirty after setting changes, scene reset, base-scene
      selection, cue-snapshot tuning, caption-style edits, lyric import, ASCII
      clear/import and scene-plan enable/disable.
- [ ] Keep purely transient playback, selection, hover and panel state clean.
- [ ] Test that each durable mutation starts/restarts the 1.5-second autosave
      settle and participates in the quit guard.

### C2 — lyric and route draft context guards

- [ ] Expose one truthful `lyric_editor_has_unsaved_draft` query.
- [ ] Block or resolve dirty lyric/route drafts before track change, scene change,
      panel change, project open, save, export start, Assist apply and quit where
      the C workflow protects them.
- [ ] Include the active lyric draft in `confirm_close` and in autosave's
      `editor_dirty` argument.
- [ ] Never let Assist replace lyrics while a conflicting lyric draft is open.
- [ ] Give every refusal an actionable notice; do not silently discard or switch.
- [ ] Cover Apply, Discard and Cancel paths, not just the blocked path.

### C3 — reversible scene reset

- [ ] Replace immediate "Reset scene" with the C workflow: Reset, Confirm, then
      Undo reset.
- [ ] Preserve the pre-reset settings until another reset/context invalidates the
      undo snapshot.
- [ ] Mark reset and undo as durable edits and commit to an active cue snapshot
      when applicable.
- [ ] Capture all three button states.

### C4 — autosave every track

- [ ] Cache decoded sample rate and channel count on each `Track` so project
      serialization does not depend on the currently bound music stream.
- [ ] Autosave every due track, not only the current one.
- [ ] Refuse autosave while that track owns a dirty editor draft.
- [ ] Test two dirty tracks, a background track loaded from a project, failure of
      one save without suppressing another, and recovery after a failed save.

### C5 — adopt project-carried presets

- [ ] When a project opens, merge its embedded scene presets into the shared
      preset library by scene and values, matching the C's deduplication rule.
- [ ] Persist newly adopted presets and make them immediately visible in Tune.
- [ ] Notify only when the shared library actually changes; a persistence failure
      must not make the in-memory and on-disk libraries silently disagree.
- [ ] Test reopening a project whose preset is absent, already present and equal
      in name but different in values.

Audit evidence: C calls `shared_presets_adopt` after hydration at
`plug.c:4992-5005`; Rust hydrates `track.scene_presets` but `open_project` never
merges them into `app.shared_presets`, which is the library Tune reads.

## P2 — missing C workflows and timeline affordances

### D1 — typed file-drop dispatch

- [ ] Dispatch dropped `.musi` files through project open.
- [ ] Dispatch PNG/JPEG/BMP through ASCII import and select ASCII Field.
- [ ] Continue dispatching supported audio formats through track load.
- [ ] Preserve the C behavior that an image dropped before audio is staged for the
      next track.
- [ ] Report unsupported/corrupt input by its attempted type.
- [ ] Add non-interactive probes or direct command tests for all three branches.

### D2 — ASCII image import and clear UI

- [ ] Add "Import image -> ASCII" to the scene browser with PNG/JPEG/BMP filters.
- [ ] Show "Clear image" only when the active track owns an image-backed grid.
- [ ] Import transactionally, select ASCII Field on success and mark the project
      dirty; a failed decode must preserve the previous grid.
- [ ] Clear path, digest, cells and dimensions together and mark dirty.
- [ ] Capture empty, populated, cleared and staged-with-no-track states.

### D3 — timed-lyrics TSV import/export

- [ ] Replace the disabled Lyrics Export/Import buttons with native save/open
      dialog commands.
- [ ] Use `LyricsDocument::bridge_export` and `bridge_import`; do not invent a
      second codec.
- [ ] Import transactionally, validate against track duration, mark dirty and
      preserve the old document on failure.
- [ ] Export must not dirty the project.
- [ ] Test cancel, overwrite confirmation/backend failure, invalid TSV, valid
      round trip and UTF-8 text.

### D4 — timeline content and navigation

- [ ] Draw the merged manual/semantic event markers over the waveform lane.
- [ ] Draw the lyric cue lane even when the Lyrics editor is closed.
- [ ] Add Shift-wheel pan and middle-drag pan with robust pointer-claim release.
- [x] Make waveform scrubbing transactional: pause on press, track the target
      while dragging, seek once on release and restore the prior play state.
- [x] On every transport discontinuity, clear queued pre-seek PCM and analyzer
      history as well as beat, scene-clock, scene-plan and cue-settings state.
- [ ] Keep wheel zoom anchored at the pointer and every lane aligned through the
      shared `TimelineView` conversion.
- [ ] Give tick labels an opaque backing so waveform amplitude cannot erase their
      contrast.
- [ ] Capture event colors, lyric spans, zoom, pan and an off-screen boundary.

Operator follow-up (2026-08-04), scoped to the existing D4 ownership:

- [x] Keep proportional Space Grotesk for time readouts, but lay out its digits in
      tabular cells so `00:11.111` and `00:55.555` reserve exactly the same width.
      The toolbar timecode, progress/level bars, zoom/view readout and adjacent
      arrow-key hint must not breathe while playback advances.
- [x] Apply playback-follow before any timeline lane draws and pixel-snap the
      shared playhead, so the scene, waveform and lyric lanes consume one view
      state per frame without marker flicker during a zoomed follow-scroll.
- [x] Bound the waveform playhead handle at both strip edges; at the track end no
      part of the marker may draw outside the PCM element.
- [x] Left-drag on the waveform and scene-plan body must preview one playhead and
      commit one transactional seek on release. Preserve lyric-cue left-drag for
      cue editing and preserve scene-boundary drag for retiming.
- [x] Middle-drag any timed lane to pan the shared view. A manual pan suspends
      playback-follow until the visible Follow affordance is used, and release or
      a disappearing surface must never strand the pointer claim.
- [x] Retain a higher-detail whole-track waveform envelope than the C oracle's
      2,048-bin hot-reload array; document the memory/detail tradeoff and pin the
      Rust-side cap with a test.
- [x] Scope lyric-cue hover to both axes of the actual cue lane. A pointer over the
      scene preview, controls or waveform must not highlight a cue that merely
      shares its X coordinate.

Completion evidence (2026-08-04): live time strings draw through proportional
Space Grotesk with tabular digit cells; rendered `00:01.111`/`00:05.555`
snapshots keep both transport bars and the lower hint at identical X bounds.
Playback-follow and deferred wheel zoom now mutate the shared view before any
lane draws, and one bounded pixel-snapped renderer serves the scene, PCM and lyric
playheads. Its edge test and a `00:07.999 / 00:08.000` capture keep the right-hand
line and triangle inside the PCM box. One shell transaction now owns scene-body
and PCM scrubs (pause, preview, one release-time Seek, conditional resume), while
scene boundary and lyric-cue drags retain their domain behavior. Middle-pan uses
a fixed origin, releases on physical button-up, persists as `free view`, and the
visible Follow action restores automatic tracking. The waveform cap is 32,768
min/max pairs (16x the C hot-reload array, 256 KiB per track); the full headless
report observes all 32,768. Lyric hover has a two-axis lane gate and a negative
control for the same X over preview/PCM. App/core tests are 231/694 passed, clippy
is clean, `tools/verify.sh --quick` is 19/0, and the private-Xvfb sweep passes with
0 ordinary output underruns plus its forced-starvation negative control.

Completion evidence (2026-08-03): C's press/drag/release contract at
`plug.c:3174-3199` now maps to one pause command on press, an in-memory target
through the drag and one transactional seek plus conditional resume on release.
`seek_preview` mirrors the full reset at `plug.c:2661-2678`: resume a paused
raylib buffer so Stop can reset it, stop, clamp/seek, refill both halves, clear
the callback ring plus analyzer/beat history, reset the scene clock/plan/cue
state, then play and restore the prior pause state. The headless gate observes
zero output underruns on the ordinary and seek/reopen paths; its main-thread
stall negative control observes 45-46 output-starved mixer reads, proving the new
counter measures the silence-producing queue rather than analyzer-ring drops.

The same audit found the Rust composition root had omitted the C build's 8,192
frame preview half-buffer and started a stream before its first refill. Rust now
sets that size before opening any `Music`, attaches the analyzer, fills both
halves and only then plays; it services the decoder before and after frame-boundary
maintenance and pauses/refills around whole-track decode, font rasterization and
probe PNG encoding. The ordinary Xvfb run changed from 17 analyzer-ring drops / 30
output underruns to 0 / 0. Spectrum's onset-only full-width white floor gradient
was also removed: the frame-eight floor crop remains at luma 21, while restoring
the old pass raises it to 55 and fails the gate.

Operator acceptance (2026-08-03, after `7099741`): real-device playback and the
Spectrum fix were tested interactively and reported to work well. Treat the audio
consistency and white-floor-flash defects as closed; retain the zero-underrun and
onset-floor negative controls as regression gates.

The next timeline gap is separate: `+ Scene` can capture a scene-plan cue at the
playhead, and `SceneSwitchTimeline` already supports split/insert, remove, retime
and retarget operations, but no scene-plan lane is drawn and there is no
comfortable editor for those operations.

- [x] Add an editable scene-plan lane to the timeline; creation, selection,
      retargeting, boundary movement and deletion need a discoverable workflow.
- [x] Extract the shared timed-lane mechanics from the lyric editor:
      `TimelineView` geometry, clipping/minimum visible width and drag threshold.
      Keep stable-id selection, hit zones and preview state in the lane policies,
      while one shell-level owner arbitrates pointer gestures. Reuse those
      primitives for lyrics, scenes and later lanes instead of cloning
      `lyric_lane_edit` or coupling the scene editor to the Lyrics panel.
- [x] Keep lane policy and durable commands domain-specific. Lyrics may overlap,
      multi-select, move as a group and resize either edge; scene segments form a
      contiguous whole-track partition, select by stable cue id and move only a
      shared internal boundary. Apply scene split, retime, retarget, tuning
      capture, remove and enable/disable commands outside the draw pass.
- [x] Make scene boundaries independently hit-testable from scene segments and
      keep transition presentation separate from segment identity. A later
      transition can then carry its own kind, duration and curve and render as a
      boundary overlay without changing segment coverage, selection semantics or
      the cut-only `.musi` representation in this task.
- [x] Move timeline-lane gesture arbitration into the shell so a lane drag cannot
      also scrub, pan or seek. Preserve preview == committed-result tests, add
      short/off-screen boundary cases, and capture the lane both enabled and
      disabled at whole-track and zoomed views.

Completion evidence (2026-08-03): the always-visible lane draws contiguous,
scene-colored blocks above the waveform and exposes Auto, Split here, previous/
next scene, Capture tuning and Delete controls. Splitting selects the resulting
segment on the next frame without predicting a model id; a boundary drag previews
the exact value `SceneSwitchTimeline::retime` accepts. Stable-id workspace methods
apply every durable edit and reset live cue state before the composition root
refreshes the current automatic scene.

`ui::timed_lane` now owns the geometry the lyric and scene painters share, while
`ui::scene_lane_edit` keeps the contiguous-partition policy separate. Tests pin
boundary priority over both adjacent bodies, a boundary exactly on the zoom edge,
an off-screen boundary that must not be recreated at the lane edge, retime
preview == commit, whole-track coverage after retarget/remove, and AA text
contrast for all ten scene colors in enabled and disabled states. The silent
headless gate captures the same two-cue plan disabled, enabled and at 4x zoom;
its lane-only crop measures saturation 55.1 enabled and 20.0 disabled, where a
missing/empty lane is zero.

Review follow-up: while a scene boundary owns the pointer, wheel zoom and
playback follow-scrolling are suspended so the time under a stationary hand
cannot move with the view. Playback itself is not paused, and ordinary follow
resumes after release.

### D5 — CLI execution semantics

- [x] Execute positional audio, `--project`, `--scene`, `--event` and
      `--ascii-image` actions left-to-right instead of reducing them to one input.
- [x] Preserve every successfully loaded audio track; apply deferred routes only
      after all immediate input actions so they target the resulting project.
- [x] Select ASCII Field only after a successful image import.
- [x] Once any CLI error occurs, keep parsing for diagnostics but suppress the
      later bridge, auto-scenes, save-project, UI-probe and render side effects
      that the C suppresses.
- [x] Add side-effect assertions for multiple inputs, interleaved actions, routes
      accompanying a project and an early error followed by `--save-project`.

Audit evidence: the C applies input actions immediately and defers only routes
(`musializer.c:500-561`). Rust collects them into one `Option<Input>` and applies
routes before opening it (`main.rs:307-382`, `:464-499`); later side effects also
run despite `options.error` (`:510-632`).

Completion evidence (2026-08-01, working tree): argv replay now owns the stream
and opens every input at its position; only routes remain deferred. Render config
precedes save, all later stages honor the shared error, startup mutations are
marked clean across every loaded track, and unknown flags again follow the C's
positional-audio arm. The headless negative control first reproduced all three
principal failures — `1 open` with only the last input, failed ASCII selecting
ASCII Field, and an error-returning command still publishing its project — then
reported `2 open, current 0`, retained Loom, and left the destination absent.
Additional gates pin a route applied after project hydration and a saved
854x480/24 fps/master render config. Workspace/app tests are 138 passed and
Clippy is warning-free.

### D6 — Tune reachability and descriptor controls

- [ ] Make the Tune inspector vertically scrollable with clipping, robust pointer
      release and a visible position indicator so every descriptor remains
      reachable at every supported window size and route-editor expansion.
- [ ] Render `SettingKind::Toggle` as a labelled binary control rather than a
      numeric slider; preserve the Song Atlas manual-count detail readout.
- [ ] Capture the last descriptor, both toggle states and an expanded route row at
      the minimum supported window size.

Audit evidence: C scrolls and clips the inspector and branches for toggles
(`plug.c:6116-6255`). Rust stops drawing when the next row does not fit and treats
every unrouted descriptor as a slider (`ui/panels/tune.rs:281-415`).

### D7 — keyboard workflows and discoverability

- [ ] Restore Ctrl+S, Ctrl+Shift+S, R for Export and direct 1-0 scene selection,
      using the same draft/context guards as the corresponding buttons.
- [ ] Expose 1-0 in scene-tile tooltips; retain Tab cycling as a Rust addition.
- [ ] Add input tests proving text entry suppresses every global shortcut.

Audit evidence: C handles these at `plug.c:1382-1396` and `:7585-7610`. Rust's
global keyboard path (`ui/shell.rs:571-637`) has transport, Tab and Tune bindings
but none of those C workflows.

### D8 — actionable notice tray

- [ ] Allow application call sites to provide the full notice specification
      instead of forcing every notice to be transient and pathless.
- [ ] Restore wrapping, persistence, file paths/tooltips, Dismiss, Copy path,
      Assist Retry and a `+N more` indicator for hidden notices.
- [ ] Test clipboard/backend failure, overflow, long UTF-8 text and the retry
      action; capture transient, persistent and path-bearing cards.

Audit evidence: C's tray implements these actions at `plug.c:6663-6738`.
`Shell::notify` discards persistence/path data and the Rust tray is text-only
(`ui/shell.rs:257-268`, `:1842-1902`).

## P3 — package the complete external product

### E1 — define and copy the support bundle

Completed 2026-08-01 (`a664b7d`): source-bundle restoration depends only on
the frozen oracle's reviewed first-party support files. E2/E3 consume the same
manifest, while E4 will reuse it for archive staging.

- [x] Treat the C distribution's support-file list as the starting manifest, not
      `external_analysis.py` alone.
- [x] Bring over and review the required Python modules, prompt, schemas and docs:
      `analysis_io.py`, `analyze_audio.py`, `external_analysis.py`,
      `google_fonts.py`, `import_whisper.py`, `lyric_align.py`,
      `mimo_openrouter.py`, `musializer_doctor.py`, the lyrics cleanup prompt and
      every schema those tools read or write.
- [x] Preserve the helpers as independent tools; first-party application code
      remains Rust.
- [x] Remove C-build assumptions and resolve resources relative to the installed
      bundle without requiring the C checkout.
- [x] Add one authoritative Rust distribution-support manifest so install and
      archive paths cannot drift.

Evidence: all copied helper/assets matched the frozen files by SHA-256 before
the doctor and user documentation were adapted to Cargo/Rust paths.
`runtime::support::DISTRIBUTION_SUPPORT_FILES` names the bundle and tests every
entry. `tools/support_bundle_check.sh` compiles all helpers, runs a real local
Sections analysis over synthetic PCM, parses its bridge through Rust, dry-runs
the model-authorized modes without credentials, and proves a headerless bridge
fails as its negative control.

### E2 — make Assist runnable

Lyrics-timing evidence added 2026-08-01: the local `lyrics` path now follows
Whisper/reference review with CUDA MMS forced alignment and publishes only the
refined lane. Two embedded-metadata tracks and audio-identical tag-stripped
copies complete end to end, including native bridge parsing, independent
Demucs-vocal controls, remote phrase-count audits, and targeted negative
controls. The full methodology and metrics are in
`docs/LYRICS_TIMING_INVESTIGATION.md`. This closes the timing investigation but
does not close E2's remaining discovery, lifecycle, staging, or UI work.

- [ ] Verify all local/offline Assist modes without network credentials.
- [ ] Verify explicit Codex/OpenRouter paths only when opted in; never copy
      credentials into fixtures, logs or the repository.
- [ ] Exercise real helper discovery from source, installed launcher and extracted
      distribution layouts.
- [ ] Test cancellation, timeout, process-group cleanup, invalid bridge output,
      staging, Apply and Discard.
- [ ] Re-probe helper availability at the user decision boundary so installing or
      removing support files updates the controls/status without restarting;
      preserve hard failure for an explicitly set but missing helper override.
- [ ] Keep Assist progress/review state visible in fullscreen with a non-exported
      badge for running, cleanup, candidate-ready and failure/artifact states.
- [x] Keep a fake helper for deterministic lifecycle coverage, but add at least one
      real support-bundle smoke test.

### E3 — make Google Fonts import runnable

- [ ] Package `google_fonts.py`, `analysis_io.py` and its schema dependencies.
- [ ] Keep network consent once-per-run and never persist it.
- [ ] Verify catalogue caching, host allow-list/redirect refusal, digest checks,
      licence publication, import into a project and project reopen.
- [ ] Poll the importer once per application frame, including while the browser
      pane is closed, and consume each verified import exactly once.
- [ ] Apply an import transactionally: rasterize it, set the imported caption
      face plus asset/runtime paths, mark dirty and notify success; rasterization
      failure must preserve the prior face and project fields.
- [ ] Re-probe helper availability when Browse/Fetch is requested rather than
      caching startup state for the entire session.
- [ ] Keep a fully offline fake/helper fixture for the default verification gate.

### E4 — installation, doctor and distribution

- [ ] Provide the C app's equivalent self-contained distribution/archive workflow,
      including executable, launcher, MIME data, resources and support bundle.
- [ ] Make `musializer_doctor.py` or a Rust equivalent report FFmpeg, helper,
      Whisper/model and writable-data-path readiness.
- [ ] Test the per-user launcher from a directory other than the repository root.
- [ ] Test an extracted distribution on a clean path with no Cargo invocation and
      no dependency on `../musializer`.
- [ ] Document optional external dependencies and which features degrade without
      them.

## AP — assistance provider configuration (accepted 2026-08-04)

Design authority: `docs/ASSIST_PROVIDER_CONTRACTS.md` (P0, done), which carries
the operator's storage decisions from `AGENTS.md` — no OS/vendor wallet, a 0600
credentials file, user-chosen models directory. Later phases (dialog, execution
wiring) get their tranches only after these land. The lyrics-timing benchmark
(`docs/LYRICS_TIMING_RESEARCH_PLAN.md`, trimmed per
`docs/LYRICS_TIMING_WEB_EVIDENCE.md`) runs in parallel and is not gated on this.

### LT1 — anchor→block lyric localization in production

Benchmark verdict 2026-08-04 (`docs/LYRICS_TIMING_BENCHMARK_RESULTS.md`):
anchor→block MMS reached 100% authored-line coverage on all four tracks and
recovered the canary's Whisper-looped outro lines; whole-song Qwen failed its
decision gate. Both gates cleared 2026-08-04: AP1 landed (`d132a3e`) and the
operator adjudicated all spot-check lines (anchor lane correct 14/21, its only
failures the predicted repeated-phrase and weak-exclamation classes; verdicts
in `build/lyrics-research-v2/ground_truth_adjudication.json`). Key design
consequence: the aligner's own score does not separate right from wrong —
review flags come from cross-lane disagreement and unresolved lines
(Invariant 4), never from raw model score.

- [x] (2026-08-04) Replace the per-cue Whisper-window MMS path with anchor→block
      alignment. Policy in `tools/lyric_anchor_block.py` (pure, no model),
      acoustics in `tools/anchor_block_align.py` (alignment venv), orchestration
      in `external_analysis.run_assist`. `lyrics.sync.json` is demoted to the
      coarse proposal; `lyrics.aligned.json` is the anchor/block result. The
      no-reference lane keeps `force_align_lyrics.py` unchanged. Invariant 1 is
      enforced, not assumed: `validate_full_coverage` refuses a lane where cues
      plus `unresolved` do not account for every alignable authored line.
- [x] (2026-08-04) Whisper evidence pass: `--max-context 0` on every run.
      whisper-cli 1.8.6 has **no** `--no-context` (`params.no_context` is never
      wired to an argument), but zero max context skips history conditioning at
      `whisper.cpp:7097`. Measured: the canary's 90 s loop is gone and its first
      outro line transcribes at 90.6 s; duplicate segments across the four
      tracks fall 31→13, 27→11, 22→0, and the choir track gains evidence 32→55.
      **VAD is measured harmful and stays off**: Silero v6.2.0 keeps 0.4 s of
      the canary at threshold 0.50 and 3 segments at 0.10. It is opt-in through
      `MUSIALIZER_WHISPER_VAD_MODEL`, and an absent model degrades to no VAD.
      Both are recorded in the lane's `request_settings`.
- [ ] Surface unresolved lines and review flags in the Assist review UI by
      name and time range, so the user is pointed at what to fix instead of
      hunting. A flag is cross-view disagreement (coarse proposal vs
      anchor-block placement, >3 s) or an unresolved/abstained line — not the
      aligner's own score, which the adjudication proved uninformative. The
      additive JSON fields are in place (`unresolved`, `review_flags`; see
      `docs/PHASE0_INVENTORY.md` 6.6) and the current Rust ignores them.
- [x] (2026-08-04) Repeated-phrase abstention. A repeated authored line abstains
      when the coarse view puts it nearer a *sibling* occurrence's block
      placement than its own by more than the 3 s review tolerance, or when two
      identical lines collapse onto one acoustic phrase. **The pinned example no
      longer reproduces**: with the repetition loop contained, both views place
      `shut-up-cat` line 26 on its own occurrence (110.0 s / 108.3 s), so it
      keeps a cue. The criterion is pinned instead against the recorded pre-LT1
      numbers in `tests/test_lyric_anchor_block.py`, where it fires on line 26
      and on nothing else in that six-occurrence group.
- [x] (2026-08-04) Localization policy versioned in cache provenance:
      `localization_policy` / `localization_policy_version` on the document and
      in `timing_refinement`, checked by `_anchor_block_cache_accepts` alongside
      the whisper/coarse/reference digests. The coarse lane records
      `role: coarse_proposal`, and the Whisper lane's `text_conditioning` and
      `vad_model_sha256` invalidate any lane decoded under the old flags.
- [x] (2026-08-04) Evidence: all four benchmark tracks rerun through the
      *production* path reach 100% coverage with 0 omitted and 0 unresolved
      (15/15, 33/33, 51/51, 42/42); the canary's outro lines are real cues at
      90.72 s and 93.94 s against the adjudicated 90.7/93.9; every
      adjudicated-correct line matches the anchor-lane time to ≤0.21 s; the
      choir track's order violation is **fixed** (0, not pinned) — it came from
      the pre-`--max-context 0` evidence. `tools/support_bundle_check.sh`,
      `cargo test` and `tools/verify.sh --quick` are green, and the four real
      bridges parse through `analysis_bridge_check` unchanged. Known regression:
      `shipped-the-disposition` line 41 (`rawr.`, the weak one-word-exclamation
      class the adjudication predicted) moved from 150.6 s to 134.6 s against a
      truth of 156.2 s, and is **not** flagged, because both views agree and
      both are wrong — cross-view disagreement cannot catch a shared-evidence
      error.

### AP1 — persistence foundation

- [x] AP1-a (2026-08-04) `musializer.assist-settings/v1` in `musializer-core` (pure, no
      filesystem) plus a loader modelled on `ui/preferences.rs`: size cap,
      `deny_unknown_fields`, corrupt file is an error not a reset, atomic
      temp+rename write. Evidence: round-trip, unknown-field, oversize and
      corrupt-file tests; two writes of a default profile are byte-identical.
- [x] AP1-b (2026-08-04) credentials store `musializer.assist-credentials/v1` at
      `$XDG_CONFIG_HOME/musializer/credentials.json`, dir `0700`, file `0600`
      set before secret bytes exist, temp file created `0600`; loose-permission
      files are refused, not repaired. Evidence: `mode & 0o077 == 0` asserted on
      file and dir; a `0644` fixture loads as a permission error without being
      chmodded; Forget on a two-provider file leaves the other entry
      byte-identical.
- [x] AP1-c (2026-08-04) `Secret` newtype: no `Clone`, no derived `Debug`, best-effort zeroize
      on drop, hand-written `Debug` printing `<redacted>`, no new crate.
      Evidence: `format!("{:?}")` contains no key characters; non-`Clone` pinned.
- [x] AP1-d (2026-08-04) env import and self-strip: read `OPENROUTER_API_KEY` once at startup
      into the session store, then remove it from the process environment before
      threads start (`SAFETY:` comment + `AGENTS.md` unsafe-inventory row).
      Evidence: a child spawned from a build with a planted key shows no
      credential-named variable in `/proc/self/environ`; the helper's
      `_safe_local_env` test stays green.
- [x] AP1-e (2026-08-04) `.env` migration: the desktop path passes an explicit flag disabling
      `external_analysis.py`'s repository-`.env` fallback; the CLI path keeps
      it. Evidence: `_openrouter_env` with the flag and a populated `.env`
      fixture yields no `OPENROUTER_API_KEY`; `tools/support_bundle_check.sh`
      still asserts `"credentials": "environment only; omitted"`.
- [x] AP1-f (2026-08-04) canary leak scan `tools/secret_canary_check.sh`: plant a sentinel
      key through file, session and env-import routes, run a dry-run assist job,
      scan config dir, cache dir, `build/analysis/`, a saved `.musi`, the job
      log, the support bundle and `/proc/<pid>/cmdline` for zero occurrences;
      wire into `tools/verify.sh`. Negative control: leak the sentinel into the
      dry-run JSON, watch the scan fail, revert byte-for-byte.

### AP2 — discovery

- [x] AP2-a (2026-08-04) extend `musializer_doctor.py` with per-runtime identity for Whisper,
      the MMS/CTC aligner and any stem separator: path, version, model path,
      `sha256` where practical, language support, GPU readiness. Evidence: a
      schema test on doctor output; a missing binary yields a per-runtime
      `unavailable` state with a remediation string, not a global failure.
- [ ] AP2-b models-directory resolution per the operator rule: default
      `<install dir>/models/`, fallback to a home-directory musializer folder
      when unwritable, always overridable and always displayed. Evidence: pure
      unit tests over an injected writability probe (writable default,
      unwritable default, explicit override); the resolved path appears in
      doctor output.
- [x] AP2-c (2026-08-04; live `codex app-server` JSON-RPC probe returned 7
      models, old-Codex stub yields `Codex default`) Codex `model/list` discovery where the installed Codex supports it;
      cache non-secret catalog metadata only. Evidence: a stubbed old-Codex
      response yields exactly `Codex default` and no guessed id; a test asserts
      no Codex auth file is read.
- [x] AP2-d (2026-08-04; 34 tests in `tests/test_provider_discovery.py`, live
      fetch cached 25 audio+text models) OpenRouter catalog cache: bounded fetch of `GET /api/v1/models`
      with the stored filters, normalized to an allowlist of fields, written
      under `$XDG_CACHE_HOME/musializer` with cache schema version, source URL,
      timestamps, filters, atomic replacement. Evidence: malformed, oversized,
      duplicated-id and truncated inputs are each refused while the prior valid
      cache stays readable.
- [ ] AP2-e suitability overlay: a versioned in-repo table keyed by (model id,
      contract id) with `recommended`/`experimental`/`unsupported`, evidence
      date, prompt/schema version, scope, languages, limitations. Evidence:
      every `recommended` row names an evidence date and a benchmarked
      prompt/schema version; an absent model resolves to `experimental`, never
      `recommended`.
- [ ] AP2-f offline/stale UX (cached catalog with an honest age badge, no
      network hang) — deferred to the dialog tranche (P3), which owns the
      surface it renders on.

## P4 — honest status, stale handoffs and final gate

### F0 — robust native-size shell typography

Completed 2026-08-01 (`a664b7d`, operator-requested robustness sweep):

- [x] Replace the single 64 px UI atlas with native Space Grotesk atlases for
      every fitted 11--22 px size and the fixed 24, 28, 34, 38 and 84 px sizes.
- [x] Route all ordinary shell drawing, measurement, buttons, text input and
      panels through a distinct `UiFonts` type that cannot be used as a raw
      raylib face. Keep caption faces and Font Awesome icons as explicit raw-face
      exceptions.
- [x] Quantize fitted rows before measurement, draw UI glyphs 1:1 without
      mipmaps, and snap text origins and tracking to integer pixels.
- [x] Report the native sizes used and fail the headless gate when any shell
      label requests a scaled/non-native size.
- [x] Exercise welcome, every scene and panel, preview/export and both supported
      window sizes through the silent headless path.

Evidence: changing `Faces::ui()` from `&Face` to `&UiFonts` first produced 61
compile errors across raw draw, measure, fitted-row and helper boundaries, which
made the retrofit inventory compiler-complete instead of search-pattern based.
The full headless gate then collected 81 application font reports; every one
loaded all 17 atlases and reported `non-native-requests=0`, while preview/export
determinism and the existing scene contracts stayed green. As the runtime
negative control, changing one welcome label from 15.0 to 15.5 px still rendered
but reported three non-native requests, and the new gate rejected it. The source
perturbation was reverted before the green run.

### F1 — remove false unfinished status

- [x] Stop `toggle_panel` from emitting `ShellCommand::NotImplemented` for the real
      Export, Lyrics and Assist panels; delete the obsolete command if no genuine
      use remains.
- [x] Remove the scene-browser footer `* not ported yet` while every registered
      scene reports a ported drawing path.
- [ ] Replace stale "largest remaining gap" claims in status text with a pointer
      to this plan.
- [ ] Ensure intentionally unavailable actions name their real prerequisite
      (`helper missing`, `FFmpeg missing`, `no track`) rather than saying "stub".
- [ ] Show truthful Saved/Unsaved/Save failed/No project file state in the Tracks
      header, including lyric and route drafts.
- [ ] Show Cadence's preview-only no-timed-lyrics guidance instead of leaving an
      empty scene that resembles a broken renderer.

### F2 — retire completed agent seams without changing behavior

This is centralized dangling-work cleanup, not feature parity by itself:

- [ ] Remove or rewrite obsolete "no caller yet", "Agent X must wire this" and
      "nothing dispatches this" comments whose call sites now exist.
- [ ] Remove stale module/function `allow(dead_code)` attributes that only existed
      for the completed fan-out. Keep intentional future-scene placeholder code
      explicitly documented if it remains.
- [ ] Reconcile the old duplicate `transport_duration_text` boundary note with the
      landed core timing module; deduplicate only with numeric evidence.
- [ ] Review the old requests to hoist shared scene draw helpers. Consolidate only
      where it reduces live duplication without perturbing verified formulas.
- [ ] Keep `REWRITE_PLAN.md` as append-only historical evidence; do not rewrite its
      old session statements to pretend they were never true.

### F3 — optional structural cleanup, explicitly not a parity blocker

- [ ] Consider moving `Workspace::assist` to `Shell` and deleting its four `Cell`
      fields. Do this only with the existing Assist differential and capture suite
      green before and after.
- [ ] Treat further ownership, module and math-helper consolidation as normal
      refactoring. It does not belong on the parity critical path unless it fixes
      an observable defect.
- [ ] Fix or formally close the FFmpeg test-helper `ETXTBSY` race. The historical
      report named
      `process::ffmpeg::tests::a_completed_encode_is_published_by_rename`; on
      2026-07-31 `tools/verify.sh --quick` instead failed
      `an_encoder_that_ignores_the_pipe_closing_is_killed_after_the_grace_period`
      at helper spawn with `ExecutableFileBusy`, after which that exact test
      passed in isolation. Treat this as a shared generated-executable race, not
      as one flaky assertion, and retain a regression stress test when fixed.

### G1 — integration gates that cover the discovered holes

- [ ] Add a seeded-project headless scenario that simultaneously proves lyrics,
      semantics, manual events, scene plans and cue settings reach preview.
- [ ] Run the same project through full and windowed export and compare selected
      frame hashes/state reports.
- [ ] Add a drop-dispatch test, lyric TSV test, autosave-multiple-tracks test and
      packaged-helper discovery test.
- [ ] Add CLI ordering/side-effect, interactive scene-plan, project-preset
      adoption, Tune overflow/toggle, transactional-seek and font-application
      gates from D4-D8, B4, C5 and E3.
- [ ] Every new gate gets a negative control; record what the perturbation broke.
- [ ] Choose further differential harnesses by evidence weakness, not by counting
      modules. A property-only unit suite is the strongest candidate signal.

### G2 — final feature-parity audit

- [ ] Walk the C application surface from `docs/PHASE0_INVENTORY.md`: CLI, file
      types, panels, timeline gestures, projects, scenes, captions, Assist, fonts,
      presets, events, export, launcher and distribution.
- [ ] For every C capability, record `fit`, `improved mechanism`, `deliberate
      exclusion` or an open task id in the matrix below. No uncategorized row.
- [ ] Run `tools/verify.sh`; require 0 failures and a clean oracle at `9300af9`.
- [ ] Open a C-written `.musi` in Rust and a Rust-written `.musi` in C, including
      embedded ASCII image/font assets, lyrics, semantic/manual lanes, routes,
      presets and enabled scene cues.
- [ ] Verify an interactive session on the operator's real desktop separately from
      Xvfb: dialogs, drag/drop, fullscreen, audio controls and launcher startup.
      Audio playback and the Spectrum onset-floor fix were accepted on 2026-08-03;
      the other interaction families remain open.
- [ ] At completion, the deliberate exclusions above are the complete observable
      difference list. Update `README.md` and `AGENTS.md` from this file.

## Feature-surface ledger

| C feature family | Current Rust state | Closure |
| --- | --- | --- |
| Ten scenes and audio reaction | Fit, including saved semantic/lyric/event inputs | none; retain G1 evidence |
| Shared captions and Cadence lyrics | Render integration fit; Cadence empty-state guidance missing | F1 |
| Automatic scene plans | Fit in preview/export, CLI and the interactive Assist header | none; retain G1 evidence |
| Manual scene cues | Capture/base/pending semantics and editable timeline lane fit; boundary render evidence incomplete | B3 |
| Settings/routes/presets | Core fit; dirty/reset, project-preset adoption and Tune control gaps remain | C1, C3, C5, D6 |
| `.musi` open/save | Differential round trip fit | C1-C5 guard/autosave/adoption completion |
| Multi-track workspace | Fit interactively | C2, C4 |
| Lyrics editing | Editor fit; document import/export and broad draft guards missing | C2, D3 |
| Manual/semantic events | Scene consumption fit; timeline markers missing | D4 |
| Timeline | Waveform/zoom, transactional seek and scene-plan editing fit; lyric/event lanes and pan missing | D4 |
| ASCII image mode | CLI/project rendering fit; GUI and typed drop missing | D1-D2 |
| Assist | UI/controller/bridge and source bundle fit; live/fullscreen/helper-state validation remains | C2, E2-E4 |
| Google Fonts | Job/manifest verification exists; background polling and project application are unwired | E3 |
| FFmpeg export | Project lanes and scene plans fit; remaining boundary evidence is B3 | B3 |
| CLI | Documented flags and execution order fit; reload excluded | none |
| Tune UI | Settings/routes/presets exist; scrolling and toggle presentation differ | D6 |
| Keyboard workflows | Transport/fine-seek plus Rust additions fit; C save/export/direct-scene keys missing | D7 |
| Notices/save status | Basic transient notices fit; actionable tray and save-state visibility missing | D8, F1 |
| Linux launcher/MIME | Source install fit | E4 |
| Self-contained distribution/doctor | Missing | E4 |
| User-perspective review | Complete evidence review exists; every named defect, workflow, opportunity and blind spot is open as the next tranche | UX0 |
| Microphone/hot reload/non-Linux | Deliberately excluded | none |

## Consolidated dangling-note ledger

This table resolves the old planning fragments so they are not rediscovered as
parallel task lists.

| Inherited note | Disposition |
| --- | --- |
| `REWRITE_PLAN.md` TODO: route editor UI and persistence | Completed and differentially covered; stale historical TODO, not active work |
| Completion-plan W1/W2 and G through O | Landed; retain NOTE ENTRIES as history. Remaining integration gaps have new A-G ids above |
| Agent I: Lyrics panel call sites | Landed; only TSV import/export and broad draft guards remain, D3/C2 |
| Agent J: Assist panel seams | Landed; real helper bundle and lyric-draft conflict remain, E2/C2 |
| Agent K: font-browser seams | UI/job controller landed; background polling and transactional project application remain E3; stale comments go to F2 |
| Agent L: event/preset call sites | Event frame consumption landed; timeline markers remain D4 and project-preset adoption is C5 |
| W1 note: autosave only current track | Active as C4 |
| `Workspace::assist` ownership deferral | Optional structural cleanup F3, not a parity requirement |
| Item O: add every possible harness | Reframed as risk-based evidence work in G1 |
| Recorded intermittent FFmpeg `ETXTBSY` test-helper race | Reproduced in a second test and retained as test-reliability cleanup in F3 |
| `UX_PERSPECTIVE_REVIEW.md` | Evidence companion, fully normalized into next-task umbrella UX0; it is not a parallel queue |
| Source comments naming agents/integration-owner handoffs | Audit and remove as F2 after verifying each call site |
| Ignored `.claude/worktrees/**/REWRITE_PLAN.md` files | Historical worktree snapshots, not tracked plans and not sources of current tasks |

## How this file stays authoritative

- Start work by claiming one task id here and recording dependencies.
- When a task lands, check its boxes and append the commit plus evidence under the
  task. Do not add a second completion plan elsewhere.
- New C-only behavior discovered during implementation becomes a task here before
  it becomes code.
- A task may be removed from the critical path only as `completed`, `duplicate`,
  `not observable`, or an operator-approved deliberate exclusion, with the reason
  recorded here.
- Keep historical investigation and negative-control details in
  `REWRITE_PLAN.md` NOTE ENTRIES or focused code/test comments; keep the live queue
  here concise and current.
