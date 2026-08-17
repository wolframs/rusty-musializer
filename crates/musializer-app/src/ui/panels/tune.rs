//! The tuning inspector, and the route editor row inside it.
//!
//! **Owner: Agent G** for the route editor row. The slider list itself is
//! already ported and is only here because the row it expands lives inside it.
//!
//! The route editor is **not a new panel** — that was a misreading this
//! repository has already made once. It replaces the setting's own 30 px slider
//! zone with a taller block: the legacy editor's 262 px plus a second 26 px row
//! for post-legacy musical sources, and 24 more when the source is `band`. Draft
//! edits, Apply commits, Close discards, and dirty participates in the close
//! guards — all of which [`musializer_core::ui::route_editor_state`] already
//! implements and tests.
//!
//! # Three seams this file cannot reach, and what it does instead
//!
//! Agent G's fan-out brief forbids editing `shell.rs`, `widgets.rs`, `theme.rs`
//! and `main.rs`, which is where three things the oracle's editor needs live.
//! Each is worked around here in a way that is one line to undo, and each is
//! marked `SEAM:` at its use site so a later session can find all of them with a
//! grep rather than by reading:
//!
//! 1. **The draft has nowhere to live.** The C keeps `Route_Editor_State` in the
//!    global `Plug` (`plug.c:5646`); the natural home here is a `Shell` field.
//!    The draft lives on [`Shell`] as `route_editor`, reached through
//!    [`Shell::with_editor`]/[`Shell::peek_editor`]. It was a `thread_local`
//!    while that field did not exist — Agent G flagged it as something that
//!    should not survive the merge, and it did not.
//! 2. **Apply and Remove need `&mut RouteTable`.** The shell only sees
//!    `&Workspace`, so [`ShellCommand::ApplyRoute`] and
//!    [`ShellCommand::RemoveRoute`] carry the commit back to `main.rs` after the
//!    drawing pair closes.
//! 3. **The live source values cross the shell boundary.** [`ShellInput`] carries
//!    the same `RouteSources` the frame loop evaluates, so every button's meter
//!    previews the exact value that will drive preview and export.

use musializer_core::project::preset_store::PresetAction;
use musializer_core::scene::routes::{AnalysisSource, Interpolation, ParameterMapping, RouteTable};
use musializer_core::scene::settings::{self, SceneSettings, SettingDescriptor, SettingsSnapshot};
use musializer_core::scene::SceneId;
use musializer_core::ui::notice::Severity;
use musializer_core::ui::route_editor_state::{self, RouteEditorDraft, RouteEditorState};
use musializer_core::ui::text_edit::TextRules;
use musializer_core::ui::tune_explore::{
    self, ExploreSource, ExploreState, Side, SplitMix64, Strength, TuneTarget, TypedValueError,
};
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::AuthoredText;
use raylib::prelude::{KeyboardKey, RaylibDraw, RaylibDrawHandle, Rectangle};
use serde::{Deserialize, Serialize};

use super::super::mapping_editor::{self, AnchorPair};
use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::shell_layout::WorkspaceFrame;
use super::super::text_input::TextField;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};
use crate::workspace::Track;

// -- geometry, from `plug.c:5517-5523` ---------------------------------------

/// The seven stacked zones of the editor area, in the oracle's order
/// (`scene_route_editor_area_height`, `plug.c:5517-5523`).
///
/// Written as the sum the C writes rather than as `262.0`, because the numbers
/// are what a later reader checks against the drawing code below: each one is the
/// height of the block that consumes it, and `route_editor_row` advances its
/// cursor by exactly these.
#[rustfmt::skip]
const ROUTE_EDITOR_AREA_HEIGHT: f32 = 24.0  // live caption + meter
                                    + 52.0  // two rows of source buttons
                                    + 40.0  // the low anchor
                                    + 40.0  // the high anchor
                                    + 70.0  // the transfer graph
                                    + 26.0  // the curve stepper
                                    + 32.0  // the action row
                                    + 4.0; // trailing

/// The band stepper only exists for [`AnalysisSource::Band`] (`plug.c:5522`).
const ROUTE_EDITOR_BAND_ROW_HEIGHT: f32 = 24.0;

/// The label/readout/`~` line above the editor area.
///
/// **Divergence, deliberate.** The C's setting row is 76 px with a 29 px header
/// and 12 px of trailing rule space, so its expanded row is `area + 41`
/// (`plug.c:6113`, `:6191`). This inspector's row is 46 px with a 22 px header
/// and a 4 px gap, so its expanded row is `area + 26`. The *area* is reproduced
/// exactly — it is the part with controls in it — and only the row furniture
/// around it follows this inspector's own rhythm. Matching the C's 41 here would
/// put 15 px of nothing under a block that already ends in its own 4 px of
/// trailing space.
const ROUTE_EDITOR_ROW_HEADER: f32 = 22.0;
const ROUTE_EDITOR_ROW_GAP: f32 = 4.0;

/// `gap` in `scene_route_editor_panel` (`plug.c:5648`).
const GAP: f32 = 5.0;

// -- widget ids ---------------------------------------------------------------
//
// All inside `widgets::id::INSPECTOR`, below the 900 the reset button uses. The
// per-row `~` buttons are indexed by setting so two rows can never claim one
// another's press; the editor's own controls are singletons because at most one
// editor is open.
mod slot {
    /// `~` for setting `index`.
    pub const ROUTE_TOGGLE: u32 = 100;
    /// The collapsed routed summary, which is also a hit target (`plug.c:6210`).
    pub const ROUTED_SUMMARY: u32 = 150;
    pub const SOURCE: u32 = 200;
    pub const BAND_PREVIOUS: u32 = 210;
    pub const BAND_NEXT: u32 = 211;
    pub const INPUT_LOW: u32 = 220;
    pub const INPUT_HIGH: u32 = 221;
    pub const OUTPUT_LOW: u32 = 222;
    pub const OUTPUT_HIGH: u32 = 223;
    pub const CURVE_PREVIOUS: u32 = 230;
    pub const CURVE_NEXT: u32 = 231;
    pub const ACTION: u32 = 240;

    // -- exploration (UX0-B09, UX0-C04, UX0-C07) ------------------------------
    //
    // Indexed slots are `+ index` over at most [`MAX_CONTROLS`] = 12 rows, so
    // each band is 50 wide and cannot reach the next. `ALL_SLOTS` below names
    // every one of these for the collision test, because the lesson this
    // repository paid for is that a *hand-picked subset* proves nothing.

    /// The value chip, which opens typed entry. `+ index`.
    pub const VALUE_CHIP: u32 = 250;
    /// The label, which resets one setting to its default. `+ index`.
    pub const LABEL_RESET: u32 = 300;
    pub const NUDGE: u32 = 400;
    pub const SURPRISE: u32 = 401;
    pub const AB_COMPARE: u32 = 410;
    pub const AB_REVERT: u32 = 411;
    pub const AB_KEEP: u32 = 412;
    /// The route editor's two disabled actions, which need a hit target of
    /// their own to hover-test: `disabled_button` returns no state (B08).
    pub const ACTION_DISABLED: u32 = 420;

    /// Every constant above, named individually, plus the two bare literals the
    /// row loop uses (`index` for the slider, `900` for Reset scene).
    #[cfg(test)]
    pub const ALL_SLOTS: [(&str, u32, u32); 19] = [
        // (name, first slot, count)
        ("SLIDER", 0, 12),
        ("ROUTE_TOGGLE", ROUTE_TOGGLE, 12),
        ("ROUTED_SUMMARY", ROUTED_SUMMARY, 12),
        ("SOURCE", SOURCE, 6),
        ("BAND_PREVIOUS", BAND_PREVIOUS, 1),
        ("BAND_NEXT", BAND_NEXT, 1),
        ("INPUT_LOW", INPUT_LOW, 1),
        ("INPUT_HIGH", INPUT_HIGH, 1),
        ("OUTPUT_LOW", OUTPUT_LOW, 1),
        ("OUTPUT_HIGH", OUTPUT_HIGH, 1),
        ("CURVE_PREVIOUS", CURVE_PREVIOUS, 1),
        ("CURVE_NEXT", CURVE_NEXT, 1),
        ("ACTION", ACTION, 5),
        ("VALUE_CHIP", VALUE_CHIP, 12),
        ("LABEL_RESET", LABEL_RESET, 12),
        ("NUDGE", NUDGE, 1),
        ("SURPRISE", SURPRISE, 1),
        ("AB", AB_COMPARE, 3),
        ("ACTION_DISABLED", ACTION_DISABLED, 2),
    ];
}

/// The Reset-scene button's slot, named rather than left a literal at its use
/// site so [`slot::ALL_SLOTS`]' overlap test can see it.
const RESET_SCENE_SLOT: u32 = 900;

// -- the draft's temporary home ----------------------------------------------

/// SEAM 1: the route editor draft, and the track slot it is keyed against.
///
/// See this module's header. `track_slot` is cached here because
/// [`Shell::route_editor_height`] is asked before the row is measured and is not
/// handed the workspace, while [`RouteEditorState::targets`] needs the slot to
/// keep a hidden draft for another track from expanding a row on this one
/// (`route_editor_state.c:101-114`).
pub(crate) struct EditorHost {
    state: RouteEditorState,
    track_slot: usize,
    /// review 1.8 (UX0-A08): "Reset scene"'s arm/confirm state. It lives here,
    /// not on a new `Shell` field, for the same reason the route draft does —
    /// `Shell`'s field list belongs to `shell.rs`, which this agent cannot
    /// touch, and `EditorHost` is already the one seam `Shell` exposes into
    /// this file. It has nothing to do with the route editor; it is here
    /// because this is where the door is.
    reset_arm: ResetArm,
    /// UX0-C04/C07: the open audition, if any. Same reasoning as `reset_arm` —
    /// this is the one seam `Shell` already exposes into this file.
    explore: ExploreState,
    /// UX0-B09: the row whose value is being typed into, if any.
    typing: Option<TypedEntry>,
    /// Presses of Nudge/Surprise this session, mixed into the seed so two
    /// presses in a row cannot draw the same tuning. Not a clock: a clock would
    /// make a headless capture unreproducible, which is the whole reason
    /// `--ui-probe tune-seed=` exists.
    explore_presses: u64,
    /// `--ui-probe tune-seed=`, replacing the counter for a capture run.
    probe_seed: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteRecoveryDraft {
    track_slot: usize,
    scene: String,
    setting_index: usize,
    committed: Option<RecoveryMapping>,
    draft: RecoveryMapping,
    touched: bool,
}

impl RouteRecoveryDraft {
    fn into_session(self) -> Option<RouteEditorDraft> {
        let scene = SceneId::from_stable_name(&self.scene)?;
        let draft = self.draft.into_mapping()?;
        let committed = match self.committed {
            Some(mapping) => Some(mapping.into_mapping()?),
            None => None,
        };
        Some(RouteEditorDraft {
            track_slot: self.track_slot,
            scene,
            setting_index: self.setting_index,
            committed,
            draft,
            touched: self.touched,
        })
    }

    pub(crate) fn is_valid_for_tracks(&self, track_count: usize) -> bool {
        if self.track_slot >= track_count {
            return false;
        }
        let Some(session) = self.clone().into_session() else {
            return false;
        };
        RouteEditorState::new().restore_session(session)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryMapping {
    parameter: String,
    source: String,
    band_index: u16,
    input_min: f64,
    input_max: f64,
    output_min: f64,
    output_max: f64,
    interpolation: String,
    clamp: bool,
}

impl From<&ParameterMapping> for RecoveryMapping {
    fn from(mapping: &ParameterMapping) -> Self {
        Self {
            parameter: mapping.parameter.clone(),
            source: mapping.source.canonical_name().to_string(),
            band_index: mapping.band_index,
            input_min: mapping.input_min,
            input_max: mapping.input_max,
            output_min: mapping.output_min,
            output_max: mapping.output_max,
            interpolation: mapping.interpolation.canonical_name().to_string(),
            clamp: mapping.clamp,
        }
    }
}

impl RecoveryMapping {
    fn into_mapping(self) -> Option<ParameterMapping> {
        Some(ParameterMapping {
            parameter: self.parameter,
            source: AnalysisSource::from_canonical_name(&self.source)?,
            band_index: self.band_index,
            input_min: self.input_min,
            input_max: self.input_max,
            output_min: self.output_min,
            output_max: self.output_max,
            interpolation: Interpolation::from_canonical_name(&self.interpolation)?,
            clamp: self.clamp,
        })
    }
}

impl Default for EditorHost {
    fn default() -> Self {
        Self {
            state: RouteEditorState::new(),
            track_slot: 0,
            reset_arm: ResetArm::default(),
            explore: ExploreState::new(),
            typing: None,
            explore_presses: 0,
            probe_seed: None,
        }
    }
}

/// One row's open text field (UX0-B09).
///
/// Keyed by `(scene, index)` rather than by the row's screen position, because
/// the list re-lays out whenever a route editor expands above it and a field
/// bound to a rectangle would then be editing a different setting.
struct TypedEntry {
    scene: SceneId,
    index: usize,
    field: TextField,
}

impl TypedEntry {
    /// The field starts holding the value it is replacing, selected-at-caret
    /// rather than empty: a user who clicks a chip to see the exact number and
    /// presses Escape must get their value back, and a blank box would make the
    /// click destructive before they typed anything.
    fn open(scene: SceneId, index: usize, value: f32, precision: u32) -> Self {
        // 12 bytes is "-180.000" and change — more than any descriptor needs,
        // and short enough that the field cannot outgrow the chip.
        let mut field = TextField::new(TextRules::ascii_query(12));
        field.bind(&format!("{:.*}", precision as usize, value));
        field.set_focused(true);
        Self {
            scene,
            index,
            field,
        }
    }
}

/// The "Reset scene" button's cross-frame arm/confirm state (review 1.8,
/// UX0-A08). Copies the preset block's `Delete` button exactly
/// (`events.rs`'s `presets.delete_armed`): first click arms and relabels the
/// button, a second click while armed is the real action, and anything else
/// disarms it rather than letting a stray click land on a confirmation nobody
/// meant to give.
///
/// Keyed by `(track_slot, scene)` rather than a bare bool, because unlike the
/// preset block — one selection shared by the whole session — Tune redraws a
/// different scene's rows the instant the user switches scene or track. An
/// unqualified bool would leave "Confirm" showing on a scene the user never
/// clicked reset on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ResetArm {
    armed: bool,
    track_slot: usize,
    scene: Option<SceneId>,
}

impl ResetArm {
    /// Whether this exact button (this track, this scene) is the one armed.
    fn is_armed_for(self, track_slot: usize, scene: SceneId) -> bool {
        self.armed && self.track_slot == track_slot && self.scene == Some(scene)
    }

    /// First click.
    fn arm(&mut self, track_slot: usize, scene: SceneId) {
        self.armed = true;
        self.track_slot = track_slot;
        self.scene = Some(scene);
    }

    /// Second click, or anything else that should make a stray third click
    /// harmless: a slider edit, a preset action, or leaving the row.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Shell {
    fn with_editor<T>(&mut self, f: impl FnOnce(&mut EditorHost) -> T) -> T {
        f(&mut self.route_editor)
    }

    fn peek_editor<T>(&self, f: impl FnOnce(&EditorHost) -> T) -> T {
        f(&self.route_editor)
    }
}

impl Shell {
    /// Whether a Tune value is being typed into (UX0-B09).
    ///
    /// Read by [`Shell::text_entry_focused`] through `TextEntrySurface::TuneValue`.
    /// **This is the whole reason typing `1` into a value chip does not toggle
    /// the HUD and typing a `-` does not do whatever `-` does next** — the shell
    /// reads the keyboard before any panel draws, so the guard has to be asked
    /// there and it has to know about this field.
    pub(crate) fn tune_value_typing(&self) -> bool {
        self.peek_editor(|host| host.typing.is_some())
    }

    /// Puts a value chip in the state a user reaches by clicking it, for
    /// `shell.rs`'s `TextEntrySurface::ALL` sweep.
    ///
    /// The sweep's `focus` helper is a `match`, so this method existing is what
    /// the compiler demands before `TuneValue` can be added — which is the point
    /// of that design and the reason UX0-A06's second surface was missed.
    #[cfg(test)]
    pub(crate) fn focus_tune_value_for_test(&mut self) {
        self.inspector_open = true;
        self.route_editor.typing = Some(TypedEntry::open(SceneId::Loom, 0, 1.0, 2));
    }

    /// What a Tune edit lands on this frame (`App::settings_mut`'s three cases,
    /// as data). The audition is keyed against it so a revert can never write
    /// A's values into a cue they were never captured from.
    fn tune_target(&self, input: &ShellInput<'_>, track_slot: usize) -> TuneTarget {
        TuneTarget {
            track_slot,
            scene: input.scene,
            cue: input
                .workspace
                .current()
                .and_then(Track::active_cue)
                .map(|(position, _)| position),
        }
    }

    /// Open — or extend — the audition covering whatever is about to change.
    ///
    /// Called *before* the commands that do the changing are pushed, so the
    /// snapshot it captures is the tuning as it stands this frame.
    fn begin_audition(
        &mut self,
        target: TuneTarget,
        settings: &SceneSettings,
        source: ExploreSource,
    ) {
        let opened = self.with_editor(|host| host.explore.begin(target, settings, source));
        if !opened {
            // `capture` refuses when a value is out of its descriptor's range.
            // Say so rather than drawing a Revert button that cannot deliver.
            self.notify(
                Severity::Warning,
                "No undo for this change",
                "A setting is outside its allowed range, so the tuning before this change could not be saved.",
            );
        }
    }

    /// The seed for the next Nudge/Surprise.
    ///
    /// A counter mixed with the scene, not a clock — a capture run must be able
    /// to press Surprise and get the same picture twice, and `--ui-probe
    /// tune-seed=` replaces the counter outright for exactly that reason.
    fn next_explore_seed(&mut self, scene: SceneId) -> u64 {
        self.with_editor(|host| {
            host.explore_presses = host.explore_presses.wrapping_add(1);
            let base = host.probe_seed.unwrap_or(0x5EED_1E55_C0FF_EE01);
            base ^ (host.explore_presses.wrapping_mul(0x9E37_79B9_7F4A_7C15))
                ^ ((scene.index() as u64) << 48)
        })
    }
}

/// Turn a snapshot into the one-`SetSetting`-per-value the shell can emit.
///
/// **Not a bulk "apply these settings" command.** Going value by value through
/// the same path a slider drag takes is what makes an audition inherit
/// `App::settings_mut`'s targeting for free: with a cue driving, a revert lands
/// in that cue's captured snapshot, and `commit_active_cue_settings` persists it,
/// with no second copy of the LX3 targeting rules to keep in step.
fn push_snapshot(scene: SceneId, snapshot: &SettingsSnapshot, commands: &mut Vec<ShellCommand>) {
    for (index, _) in settings::descriptors(scene).iter().enumerate() {
        commands.push(ShellCommand::SetSetting {
            scene,
            index,
            value: snapshot.values[index],
        });
    }
}

/// Whether an open, dirty route draft should stop the application quitting.
///
/// This is `route_editor_dirty` in the C's close guard (`plug.c:7248`), which
/// `main.rs`'s `confirm_close` still weighs only dirty projects. Exposed here so
/// adding it there is one call rather than a second copy of the draft.
#[allow(
    dead_code,
    reason = "the close guard in main.rs is the integration owner's line to add; see the Agent G note in REWRITE_PLAN.md"
)]
impl Shell {
    pub(crate) fn route_edit_is_dirty(&self) -> bool {
        self.peek_editor(|host| host.state.is_dirty())
    }

    #[must_use]
    pub(crate) fn route_recovery_draft(&self) -> Option<RouteRecoveryDraft> {
        self.peek_editor(|host| {
            let session = host.state.session()?;
            host.state.is_dirty().then(|| RouteRecoveryDraft {
                track_slot: session.track_slot,
                scene: session.scene.stable_name().to_string(),
                setting_index: session.setting_index,
                committed: session.committed.as_ref().map(RecoveryMapping::from),
                draft: RecoveryMapping::from(&session.draft),
                touched: session.touched,
            })
        })
    }

    pub(crate) fn restore_route_recovery_draft(&mut self, recovered: RouteRecoveryDraft) -> bool {
        let Some(session) = recovered.into_session() else {
            return false;
        };
        self.route_editor.track_slot = session.track_slot;
        self.route_editor.state.restore_session(session)
    }

    /// Whether preview must keep the scene hosting the inline route editor
    /// visible (`route_editor_open_for_active_track`, `plug.c:574-578`).
    pub(crate) fn route_editor_open_for_active_track(&self, track_slot: Option<usize>) -> bool {
        track_slot.is_some_and(|slot| {
            self.peek_editor(|host| host.state.is_open() && host.track_slot == slot)
        })
    }
}

/// `--ui-probe route=KEY`: opens the editor on a named setting.
///
/// Applied by `main.rs` with the rest of the probe, before the first frame. It
/// exists at all because the expanded editor is 260 px of drawing whose only
/// other entrance is a mouse click, and this repository has already paid twice
/// for a surface no capture could reach.
///
/// Returns the line the run prints as evidence. A screenshot cannot say whether
/// the block under the cursor is the editor or a very tall slider, so the line
/// names which setting opened, how tall the row it asked for is, and whether it
/// opened onto a committed route or a fresh full-range draft.
/// `headless_check.sh` asserts on it.
impl Shell {
    pub(crate) fn open_route_editor_probe(
        &mut self,
        key: &str,
        drawn_scene: SceneId,
        track_slot: usize,
        committed: Option<&ParameterMapping>,
    ) -> String {
        let Some((scene, index, _)) = settings::descriptor_by_key(key) else {
            return format!("route editor: {key} UNKNOWN");
        };
        // A key from another scene is not an error: a capture may probe a scene it
        // then switches away from. It simply does not expand a row here.
        if scene != drawn_scene {
            return format!("route editor: {key} not on the drawn scene");
        }
        self.route_editor.track_slot = track_slot;
        if !self.with_editor(|host| host.state.open(track_slot, scene, index, committed)) {
            return format!("route editor: {key} REFUSED");
        }
        let (height, source) = self.peek_editor(|host| {
            let source = host
                .state
                .draft()
                .map_or("?", |draft| route_editor_state::source_label(draft.source));
            (
                ROUTE_EDITOR_ROW_HEADER
                    + ROUTE_EDITOR_AREA_HEIGHT
                    + if host
                        .state
                        .draft()
                        .is_some_and(|draft| draft.source == AnalysisSource::Band)
                    {
                        ROUTE_EDITOR_BAND_ROW_HEIGHT
                    } else {
                        0.0
                    }
                    + ROUTE_EDITOR_ROW_GAP,
                source,
            )
        });
        format!(
            "route editor: {key} open row={height}px source={source} committed={}",
            u8::from(committed.is_some())
        )
    }
}

/// `--ui-probe tune-seed=`, `tune-explore=` and `tune-type=` (PX6).
///
/// Applied by `main.rs` with the rest of the probe, before the first frame, and
/// writing into the same `SceneSettings` a slider would. Returns the lines the
/// run prints as evidence.
///
/// **Why this exists rather than more `click=` runs.** `click=` presses one
/// control per run, which is the right test for "does this button take the
/// press" and cannot state a claim about a *sequence* — and every claim
/// UX0-C04 makes is about a sequence: explore, compare, come back. The gate uses
/// both, and the two together are what separate "Revert redrew the panel" from
/// "Revert restored the exact bits".
pub(crate) fn apply_tune_probe(
    shell: &mut Shell,
    probe: &crate::cli::UiProbe,
    scene: SceneId,
    track_slot: usize,
    cue: Option<usize>,
    settings: &mut SceneSettings,
) -> Vec<String> {
    let target = TuneTarget {
        track_slot,
        scene,
        cue,
    };
    let mut lines = Vec::new();
    if let Some(seed) = probe.tune_seed {
        shell.route_editor.probe_seed = Some(seed);
    }

    if let Some(spec) = probe.tune_type.as_deref() {
        // `descriptor_by_key` already accepted the key in `cli.rs`, so an
        // unknown one cannot reach here; a key from *another scene* still can,
        // and is reported rather than silently ignored.
        let (key, text) = spec.split_once(':').unwrap_or((spec, ""));
        match settings::descriptor_by_key(key) {
            Some((owner, index, descriptor)) if owner == scene => {
                match tune_explore::parse_typed(descriptor, text) {
                    Ok(typed) => {
                        let written = settings.set(scene, index, typed.value);
                        lines.push(format!(
                            "tune typed:      {key} \"{text}\" -> {} clamped={} rounded={} written={}",
                            typed.value,
                            u8::from(typed.clamped),
                            u8::from(typed.rounded),
                            u8::from(written)
                        ));
                    }
                    Err(error) => lines.push(format!(
                        "tune typed:      {key} \"{text}\" REFUSED {error:?}"
                    )),
                }
            }
            _ => lines.push(format!("tune typed:      {key} not on the drawn scene")),
        }
    }

    if let Some(spec) = probe.tune_explore.as_deref() {
        for action in spec.split('+') {
            let applied = match action {
                "nudge" | "surprise" => {
                    let strength = if action == "surprise" {
                        Strength::Surprise
                    } else {
                        Strength::Nudge
                    };
                    let opened = shell.route_editor.explore.begin(
                        target,
                        settings,
                        if action == "surprise" {
                            ExploreSource::Surprise
                        } else {
                            ExploreSource::Nudge
                        },
                    );
                    if !opened {
                        lines.push(format!("tune explore:    {action} NO SNAPSHOT"));
                        continue;
                    }
                    let seed = shell.next_explore_seed(scene);
                    let mut rng = SplitMix64::new(seed);
                    Some(tune_explore::explore(&mut rng, scene, settings, strength))
                }
                "compare" => shell.route_editor.explore.compare(target, settings),
                "revert" => shell.route_editor.explore.revert(target),
                "keep" => {
                    shell.route_editor.explore.keep(target);
                    None
                }
                _ => None,
            };
            if let Some(snapshot) = applied {
                for (index, _) in settings::descriptors(scene).iter().enumerate() {
                    settings.set(scene, index, snapshot.values[index]);
                }
            }
            let state = shell.route_editor.explore.session(target).map_or_else(
                || "closed".to_string(),
                |s| tune_explore::audition_label(&s),
            );
            lines.push(format!("tune explore:    {action} -> {state}"));
        }
    }
    lines
}

/// Every value of the drawn scene, exactly (PX6).
///
/// Printed with `{}` rather than the descriptor's own precision **on purpose**:
/// Rust's `Display` for `f32` emits the shortest decimal that round-trips, so
/// two different bit patterns can never print the same string. That is what lets
/// the headless gate assert a revert is bit-for-bit rather than
/// "looks-the-same-to-two-places", which is the whole claim of UX0-C04.
#[must_use]
pub(crate) fn tune_values_line(scene: SceneId, settings: &SceneSettings) -> String {
    let values: Vec<String> = settings::descriptors(scene)
        .iter()
        .enumerate()
        .map(|(index, _)| settings.get(scene, index).to_string())
        .collect();
    format!("{} {}", scene.stable_name(), values.join(" "))
}

/// What the Tune panel's typed entry and audition are doing, for the report
/// (PX6).
///
/// **`click probe:` is not enough on its own.** It proves the press was cashed
/// by the widget id the chip minted, which is exactly the half EX1 says a hover
/// cannot prove — and exactly *not* the half that says the press did what the
/// control is for. A press routed to the right id through a branch that forgot
/// to open the field photographs identically. This line is the other half.
#[must_use]
pub(crate) fn tune_state_line(
    shell: &Shell,
    scene: SceneId,
    track_slot: usize,
    cue: Option<usize>,
) -> String {
    let target = TuneTarget {
        track_slot,
        scene,
        cue,
    };
    let typing = shell.peek_editor(|host| {
        host.typing.as_ref().map(|entry| {
            let label = settings::descriptor(entry.scene, entry.index)
                .map_or("?", |descriptor| descriptor.label);
            format!("{label}=\"{}\"", entry.field.edit.text())
        })
    });
    let audition = shell
        .peek_editor(|host| host.explore.session(target))
        .map(|session| tune_explore::audition_label(&session));
    format!(
        "typing {}  audition {}",
        typing.unwrap_or_else(|| "none".to_string()),
        audition.unwrap_or_else(|| "none".to_string())
    )
}

impl Shell {
    /// The tuning inspector: one slider per setting of the active scene.
    ///
    /// Bounds, defaults and precision all come from the descriptor table in
    /// [`settings`], which was checked column-by-column against the C. The
    /// inspector never invents a range.
    pub(crate) fn inspector(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        if frame.inspector.is_empty() {
            return;
        }
        // The slot the draft is keyed against. Cached before any row is measured,
        // because `route_editor_height` is asked without the workspace.
        let track_slot = input.workspace.current_index().unwrap_or(0);
        self.with_editor(|host| host.track_slot = track_slot);

        let content = widgets::panel(d, input.fonts.ui(), frame.inspector, "TUNE");
        let font = input.fonts.ui();
        let padding = metric::UI_PANEL_PADDING;
        let mut y = content.y + padding;

        widgets::draw_text(
            d,
            input.fonts.ui(),
            input.scene.display_name(),
            content.x + padding,
            y,
            metric::UI_FONT_HEADER,
            color::ui_ink(),
        );
        y += metric::UI_FONT_HEADER + metric::UI_CONTROL_GAP;

        // review 1.7 (UX0-A07): `cue_settings_active` silently redirects every
        // slider edit and Reset into a cue's captured snapshot whenever the
        // playhead sits on one, and nothing on screen said so — the header
        // named the scene, never which copy of its settings was being edited.
        // The identity and time come straight from `Track::active_cue`, not
        // from the playhead, so the label cannot drift from what a Reset or a
        // slider edit actually touches.
        // The audition is scoped to what a slider would move, which is the same
        // thing the scope line below names. A target that has moved since last
        // frame ends the session rather than letting Revert write into a
        // segment the snapshot never came from (LX3).
        let target = self.tune_target(input, track_slot);
        self.with_editor(|host| host.explore.retarget(target));

        let active_cue = input.workspace.current().and_then(Track::active_cue);
        let (segments, plan_enabled) = input.workspace.current().map_or((0, false), |track| {
            (track.scene_switches.len(), track.scene_switches.enabled)
        });
        let scope = tune_scope_label(
            active_cue.map(|(index, cue)| (index, cue.start_seconds)),
            segments,
            plan_enabled,
        );
        widgets::draw_text(
            d,
            input.fonts.ui(),
            &scope,
            content.x + padding,
            y,
            metric::UI_FONT_CAPTION,
            if active_cue.is_some() {
                color::accent()
            } else {
                color::ui_muted()
            },
        );
        y += metric::UI_FONT_CAPTION + metric::UI_CONTROL_GAP;

        // The shared preset block, between the header and the first slider
        // (`plug.c:5979-6100`): 42 px collapsed, 98 px populated, 0 if it does
        // not fit.
        let mut presets = Vec::new();
        y += self.preset_block(
            d,
            input,
            UiRect::new(content.x, y, content.width, content.y + content.height - y),
            input.presets,
            &mut presets,
        );
        if !presets.is_empty() {
            // review 1.8: a preset Apply/Replace/Delete changes what Reset would
            // act on, so an armed confirmation from a moment ago must not carry
            // over and land on a scene the user has since edited a different way.
            self.with_editor(|host| host.reset_arm.disarm());
        }
        // UX0-C04's headline case: *loading a preset* is the destructive
        // exploration the plan names, and it arrives here as an action the
        // application will perform after this frame. Capturing before it does is
        // what makes "try that preset" reversible.
        if presets
            .iter()
            .any(|action| matches!(action, PresetAction::Apply(_)))
        {
            self.begin_audition(target, input.settings, ExploreSource::Preset);
        }
        commands.extend(presets.into_iter().map(ShellCommand::Preset));

        // UX0-C04/C07. Between the presets and the sliders because that is where
        // the actions it makes reversible are: a preset Apply and a Surprise are
        // the same gesture, and a Revert anywhere else would have to be hunted
        // for after the thing it undoes has already happened.
        // The list — and the block above it — stop above "Reset scene", which is
        // pinned to the panel floor. **The row loop used to measure against
        // `content` alone**, so on a twelve-control scene at the 960x640 minimum
        // the last row and the "+N more" notice drew *underneath* the Reset
        // button. That was already true before this block existed; adding 48 px
        // of audition bar is what made it reachable at 720p too, so it is fixed
        // here rather than left as a thing the next capture rediscovers.
        let list_bottom = content.y + content.height - metric::UI_BUTTON_HEIGHT - padding * 2.0;
        y += self.explore_block(
            d,
            input,
            UiRect::new(content.x, y, content.width, (list_bottom - y).max(0.0)),
            target,
            commands,
        );

        let descriptors = settings::descriptors(input.scene);
        let row_height = 46.0f32;
        for (index, descriptor) in descriptors.iter().enumerate() {
            // The expanded route editor asks for the row before it is measured,
            // because a row it owns is taller than a slider row and the strip
            // must be checked against the height it will actually use.
            let expanded = self.route_editor_height(input.scene, index);
            let row = UiRect::new(
                content.x + padding,
                y,
                content.width - padding * 2.0,
                if expanded > 0.0 { expanded } else { row_height },
            );
            if !content.contains(row) || row.y + row.height > list_bottom {
                // Out of room. Say so rather than silently dropping the tail: a
                // truncated list that does not admit it is a feature nobody can
                // find.
                widgets::draw_text(
                    d,
                    input.fonts.ui(),
                    &format!("+{} more (enlarge the window)", descriptors.len() - index),
                    content.x + padding,
                    y,
                    metric::UI_FONT_CAPTION,
                    color::ui_warning(),
                );
                break;
            }
            y += row.height;

            let committed = committed_route(input, input.scene, index).cloned();
            let routed = committed.is_some();

            // UX0-B08: the route affordance is a **word**, not `~`.
            //
            // The oracle draws a tilde (`plug.c:6169-6186`) and the review found
            // exactly what a tilde tells a newcomer, which is nothing — it is not
            // an abbreviation of anything, has no established meaning in an
            // audio interface, and its tooltip was the only thing in the
            // application that explained it. The button now says which of its
            // three states it is in, so the row is readable with the pointer
            // parked somewhere else entirely.
            let route_label = if expanded > 0.0 {
                "Editing"
            } else if routed {
                "Routed"
            } else {
                "Route"
            };
            let route_width = (widgets::measure(font, route_label, metric::UI_FONT_CAPTION) + 14.0)
                .clamp(44.0, 74.0);
            let toggle = UiRect::new(
                row.x + row.width - route_width,
                row.y - 3.0,
                route_width,
                20.0,
            );
            let toggle_id =
                widgets::widget_id(widgets::id::INSPECTOR, slot::ROUTE_TOGGLE + index as u32);
            let toggle_state = self.widgets.text_button(
                d,
                font,
                toggle_id,
                toggle,
                route_label,
                routed || expanded > 0.0,
                ButtonStyle::Neutral,
                Some(metric::UI_FONT_CAPTION),
            );
            if toggle_state.clicked {
                self.toggle_route_editor(input, index, committed.as_ref(), expanded > 0.0);
            }
            // UX0-B08's second half: a *dynamic* tip. A routed row's tip names
            // the route it would open rather than repeating the static sentence,
            // because by then the user knows what routing is and wants to know
            // what this one does.
            self.widgets.hint(
                d,
                toggle_state,
                toggle_id,
                toggle,
                &match (expanded > 0.0, committed.as_ref()) {
                    (true, _) => "Close this route editor".to_string(),
                    (false, Some(route)) => format!(
                        "Driven by {} - click to edit",
                        ascii_fallback(
                            font.all_loaded(),
                            &route_editor_state::summary(route, descriptor.precision)
                        )
                    ),
                    (false, None) => {
                        "Let the music move this setting instead of the slider".to_string()
                    }
                },
            );

            // The label and the readout are drawn for *every* row, expanded
            // included (`plug.c:6156-6190`): the C's editor replaces the slider
            // zone, not the line that names the setting. Losing the readout there
            // would leave the one row the user is working on as the only one that
            // does not say what value it currently produces.
            let value = input.settings.get(input.scene, index);
            let effective = input
                .routed
                .map_or(value, |routed| routed.get(input.scene, index));

            // Only an unrouted, uncollapsed row gets the editable chip: a routed
            // setting's number is produced by its route, and a text field over it
            // would take a value the next frame overwrites.
            let editable = !routed && expanded <= 0.0;
            let pulse_auto = input.scene == SceneId::PulseField
                && descriptor.key == "settings.pulse.petals"
                && effective < 0.5
                && !routed;
            let readout = if pulse_auto {
                "Auto: balance".to_string()
            } else {
                format!(
                    "{:.*}{}",
                    descriptor.precision as usize,
                    effective,
                    if routed { "  routed" } else { "" }
                )
            };
            let readout_width = widgets::measure(font, &readout, metric::UI_FONT_VALUE);
            let chip_width = if editable {
                (readout_width + 14.0).clamp(46.0, 104.0)
            } else {
                readout_width
            };
            let chip = UiRect::new(toggle.x - 8.0 - chip_width, row.y - 3.0, chip_width, 20.0);

            // UX0-B09: per-setting reset, on the label.
            //
            // A third button on a 46 px row that already carries a chip and a
            // route control does not fit at the 960 px minimum, and an
            // affordance that only appears on hover is the defect LX2-a was.
            // The label is already drawn, already wide, and already names the
            // thing being reset; a leading `*` marks it as moved from its
            // default, which also makes "what have I changed on this scene"
            // answerable at a glance rather than value by value.
            let modified = value.to_bits() != descriptor.default_value.to_bits();
            let label_zone = (chip.x - 6.0 - row.x).max(24.0);
            let label_rect = UiRect::new(row.x, row.y - 3.0, label_zone, 20.0);
            let label_id =
                widgets::widget_id(widgets::id::INSPECTOR, slot::LABEL_RESET + index as u32);
            // An unmodified label claims no press: there is nothing to reset, and
            // a hit target over an inert control is a click the user cannot
            // account for.
            let label_state = if modified {
                self.widgets.button(d, label_id, label_rect)
            } else {
                widgets::ButtonState::default()
            };
            if label_state.clicked {
                self.begin_audition(target, input.settings, ExploreSource::Manual);
                commands.push(ShellCommand::SetSetting {
                    scene: input.scene,
                    index,
                    value: descriptor.default_value,
                });
                self.with_editor(|host| host.reset_arm.disarm());
            }
            if modified {
                self.widgets.hint(
                    d,
                    label_state,
                    label_id,
                    label_rect,
                    &format!(
                        "{} - click to reset to {:.*}",
                        descriptor.label, descriptor.precision as usize, descriptor.default_value
                    ),
                );
            }
            let label_text = if modified {
                format!("* {}", descriptor.label)
            } else {
                descriptor.label.to_string()
            };
            widgets::draw_text_faded(
                d,
                input.fonts.ui(),
                &label_text,
                row.x,
                row.y,
                label_zone,
                12.0,
                metric::UI_FONT_CAPTION,
                if routed || label_state.hovered {
                    // Hovered can only be true where the label is a live reset
                    // button, so the accent is the affordance: the one word that
                    // lights up under the pointer is the one that does something.
                    color::accent()
                } else if modified {
                    color::ui_ink()
                } else {
                    color::ui_muted()
                },
            );

            if editable {
                self.value_chip(
                    d, input, chip, index, descriptor, effective, &readout, target, commands,
                );
            } else {
                widgets::draw_text(
                    d,
                    input.fonts.ui(),
                    &readout,
                    chip.x,
                    row.y,
                    metric::UI_FONT_VALUE,
                    if routed {
                        color::accent()
                    } else {
                        color::ui_ink()
                    },
                );
            }

            // An expanded row replaces the whole slider zone rather than sitting
            // beside it (`plug.c:5517-5528`), so nothing below runs for it.
            if expanded > 0.0 {
                self.route_editor_row(d, input, row, input.scene, index, commands);
                continue;
            }

            // A routed setting shows the route, not a slider that appears to move
            // on its own: the summary, the live meter, and a hit target that opens
            // the editor (`plug.c:6197-6215`).
            if let Some(route) = committed.as_ref() {
                self.routed_row(d, input, row, index, descriptor, route);
                continue;
            }

            let span = descriptor.maximum - descriptor.minimum;
            let normalized = if span > 0.0 {
                (effective - descriptor.minimum) / span
            } else {
                0.0
            };
            // UX0-B09's fine step. The wheel over the row moves one unit of the
            // descriptor's own precision — 0.01 on a two-place slider, one degree
            // on a hue — and Shift moves ten. A ratio rather than two hand-picked
            // deltas, so every control takes the same number of notches to cross
            // the same fraction of its range.
            //
            // Read from the row rectangle rather than from the slider's, because
            // the value chip and the label are part of the same control as far as
            // a user aiming a wheel is concerned, and the timed lanes' wheel
            // (LX2-c) is a different rectangle entirely so neither can steal the
            // other's notch.
            let wheel = self.wheel_delta(d);
            if wheel != 0.0 {
                let pointer = input.ui_scale.mouse(d);
                if row.contains_point(pointer.x, pointer.y) {
                    let coarse = d.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
                        || d.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);
                    let steps = (wheel.round() as i32) * if coarse { 10 } else { 1 };
                    let stepped = tune_explore::step(descriptor, value, steps);
                    if stepped.to_bits() != value.to_bits() {
                        commands.push(ShellCommand::SetSetting {
                            scene: input.scene,
                            index,
                            value: stepped,
                        });
                        self.with_editor(|host| {
                            host.reset_arm.disarm();
                            host.explore.note_manual_edit(target);
                        });
                    }
                }
            }

            let track = UiRect::new(row.x, row.y + 22.0, row.width, 20.0);
            let id = widgets::widget_id(widgets::id::INSPECTOR, index as u32);
            if let Some(fraction) = self.widgets.slider(d, id, track, normalized) {
                // Every value this panel writes goes onto the descriptor's own
                // precision grid, so the readout above the slider is the number
                // in the store rather than a rounded picture of it. The C only
                // did this for `precision == 0` (`plug.c`'s integer readout),
                // which left a two-place slider showing "1.23" for 1.2345.
                let proposed =
                    tune_explore::conform(descriptor, descriptor.minimum + fraction * span);
                commands.push(ShellCommand::SetSetting {
                    scene: input.scene,
                    index,
                    value: proposed,
                });
                // A drag while an audition is open is the user refining the
                // experiment; it becomes the new "B" without starting a session
                // of its own.
                self.with_editor(|host| host.explore.note_manual_edit(target));
                // review 1.8: an edit made after arming Reset is exactly the
                // "changed my mind" case the arm/confirm step exists to protect
                // — the preset block's Delete disarms the same way on any other
                // preset action (`main.rs`'s `handle_preset`), mirrored here for
                // the same reason.
                self.with_editor(|host| host.reset_arm.disarm());
            }
        }

        let reset = UiRect::new(
            content.x + padding,
            content.y + content.height - metric::UI_BUTTON_HEIGHT - padding,
            content.width - padding * 2.0,
            metric::UI_BUTTON_HEIGHT,
        );
        if content.contains(reset) {
            // review 1.8 (UX0-A08): Reset used to fire on a single unconfirmed
            // click while the less-destructive preset Delete right above it
            // already required a second one. Same arm/confirm shape, same
            // widget-style rules: filled and named `Confirm reset` while armed,
            // `Danger` styled so the row reads as "about to do something" rather
            // than a plain toggle.
            let armed =
                self.peek_editor(|host| host.reset_arm.is_armed_for(track_slot, input.scene));
            let id = widgets::widget_id(widgets::id::INSPECTOR, RESET_SCENE_SLOT);
            if self
                .widgets
                .text_button(
                    d,
                    input.fonts.ui(),
                    id,
                    reset,
                    if armed {
                        "Confirm reset"
                    } else {
                        "Reset scene"
                    },
                    armed,
                    if armed {
                        ButtonStyle::Danger
                    } else {
                        ButtonStyle::Neutral
                    },
                    None,
                )
                .clicked
            {
                if armed {
                    self.with_editor(|host| host.reset_arm.disarm());
                    commands.push(ShellCommand::ResetScene(input.scene));
                } else {
                    self.with_editor(|host| host.reset_arm.arm(track_slot, input.scene));
                }
            }
        }
    }

    /// One row's value, as a chip that can be typed into (UX0-B09).
    ///
    /// Click it and it becomes a text field holding the current number; Enter
    /// commits it through the descriptor, Escape puts it back. Clamping and
    /// rounding are [`tune_explore::parse_typed`]'s, so the value that lands is
    /// one `SceneSettings::set` will accept — the store *rejects* rather than
    /// clamps (`scene_settings.c:143-149`), so a raw typed 99 would otherwise
    /// vanish with no explanation at all.
    #[allow(
        clippy::too_many_arguments,
        reason = "one row's worth of context: which setting, its descriptor, its value, and where the edit lands"
    )]
    fn value_chip(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        chip: UiRect,
        index: usize,
        descriptor: &SettingDescriptor,
        value: f32,
        display: &str,
        target: TuneTarget,
        commands: &mut Vec<ShellCommand>,
    ) {
        let font = input.fonts.ui();
        let typing_here = self.peek_editor(|host| {
            host.typing
                .as_ref()
                .is_some_and(|entry| entry.scene == input.scene && entry.index == index)
        });

        if typing_here {
            // Enter and Escape are read here rather than inside `TextField`,
            // which has neither: the field reports Escape only by dropping focus
            // and has no notion of a commit at all. Both are read *before* the
            // field draws, so the field's own Escape handling cannot defocus the
            // entry out from under the revert.
            let commit = d.is_key_pressed(KeyboardKey::KEY_ENTER)
                || d.is_key_pressed(KeyboardKey::KEY_KP_ENTER);
            let cancel = d.is_key_pressed(KeyboardKey::KEY_ESCAPE);

            widgets::fill(d, chip, color::ui_surface());
            d.draw_rectangle_lines_ex(
                Rectangle::new(chip.x, chip.y, chip.width, chip.height),
                1.0,
                color::accent(),
            );
            self.with_editor(|host| {
                if let Some(entry) = host.typing.as_mut() {
                    entry.field.draw_with_face(
                        d,
                        AuthoredText::from_ui(font),
                        chip,
                        metric::UI_FONT_VALUE,
                        "",
                        true,
                    );
                }
            });

            if cancel {
                self.with_editor(|host| host.typing = None);
                return;
            }
            // A click elsewhere defocuses the field (`TextField` sets focus from
            // every left press). Treat that as a cancel rather than a commit: the
            // user pressed something else, and silently writing a half-typed
            // number on the way past is the surprise this whole panel is trying
            // to remove.
            let still_focused =
                self.peek_editor(|host| host.typing.as_ref().is_some_and(|e| e.field.is_focused()));
            if !still_focused && !commit {
                self.with_editor(|host| host.typing = None);
                return;
            }
            if !commit {
                return;
            }

            let text = self.peek_editor(|host| {
                host.typing
                    .as_ref()
                    .map(|entry| entry.field.edit.text().to_string())
                    .unwrap_or_default()
            });
            self.with_editor(|host| host.typing = None);
            match tune_explore::parse_typed(descriptor, &text) {
                Ok(typed) => {
                    if typed.value.to_bits() != value.to_bits() {
                        commands.push(ShellCommand::SetSetting {
                            scene: input.scene,
                            index,
                            value: typed.value,
                        });
                        self.with_editor(|host| {
                            host.reset_arm.disarm();
                            host.explore.note_manual_edit(target);
                        });
                    }
                    // Say when the number written is not the number typed. A
                    // silently clamped 99 reads as a broken text field.
                    if typed.clamped {
                        self.notify(
                            Severity::Info,
                            "Value clamped",
                            &format!(
                                "{} accepts {:.*} to {:.*}; it is now {:.*}.",
                                descriptor.label,
                                descriptor.precision as usize,
                                descriptor.minimum,
                                descriptor.precision as usize,
                                descriptor.maximum,
                                descriptor.precision as usize,
                                typed.value
                            ),
                        );
                    }
                }
                // An empty field is a cancel, not a mistake, and says nothing.
                Err(TypedValueError::Empty) => {}
                Err(TypedValueError::NotANumber) => self.notify(
                    Severity::Warning,
                    "Not a number",
                    &format!("\"{text}\" is not a value {} can take.", descriptor.label),
                ),
            }
            return;
        }

        // Not typing: the chip is a readout that says it can be typed into.
        let id = widgets::widget_id(widgets::id::INSPECTOR, slot::VALUE_CHIP + index as u32);
        let state = self.widgets.button(d, id, chip);
        widgets::fill(
            d,
            chip,
            if state.hovered {
                color::ui_surface()
            } else {
                color::ui_raised()
            },
        );
        d.draw_rectangle_lines_ex(
            Rectangle::new(chip.x, chip.y, chip.width, chip.height),
            1.0,
            if state.hovered {
                color::accent()
            } else {
                color::ui_rule()
            },
        );
        let width = widgets::measure(font, display, metric::UI_FONT_VALUE);
        widgets::draw_text(
            d,
            font,
            display,
            chip.x + (chip.width - width) * 0.5,
            chip.y + (chip.height - metric::UI_FONT_VALUE) * 0.5,
            metric::UI_FONT_VALUE,
            color::ui_ink(),
        );
        if state.clicked {
            let precision = descriptor.precision;
            let scene = input.scene;
            self.with_editor(|host| {
                host.typing = Some(TypedEntry::open(scene, index, value, precision));
            });
        }
        self.widgets.hint(
            d,
            state,
            id,
            chip,
            &format!(
                "Type a value ({:.*} to {:.*})  -  wheel steps by {}, Shift by ten",
                descriptor.precision as usize,
                descriptor.minimum,
                descriptor.precision as usize,
                descriptor.maximum,
                format_args!(
                    "{:.*}",
                    descriptor.precision as usize,
                    tune_explore::step_size(descriptor)
                )
            ),
        );
    }

    /// Nudge / Surprise, and the audition bar that makes them safe
    /// (UX0-C04, UX0-C07).
    ///
    /// Returns the height it consumed, `0.0` when it does not fit — the same
    /// contract `preset_block` has, and for the same reason: at the 960x640
    /// minimum the inspector is already truncating its slider list, and a block
    /// that took its space unconditionally would push settings off the bottom to
    /// make room for a button.
    fn explore_block(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        target: TuneTarget,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        const BUTTON_H: f32 = 24.0;
        const SENTENCE_H: f32 = 18.0;
        const GAP_AFTER: f32 = 6.0;
        // One slider row's worth of settings must survive underneath, or the
        // block has eaten the thing it exists to explore.
        const ROOM_FOR_ONE_ROW: f32 = 46.0;

        let font = input.fonts.ui();
        let session = self.peek_editor(|host| host.explore.session(target));
        let needed = BUTTON_H
            + GAP_AFTER
            + if session.is_some() {
                SENTENCE_H + BUTTON_H + 4.0
            } else {
                0.0
            };
        if area.height < needed + ROOM_FOR_ONE_ROW {
            return 0.0;
        }

        let mut y = area.y;
        // Two buttons, equal halves. Surprise is on the right — it is the bolder
        // of the pair, and the pair reads left to right as increasing daring.
        let half = (area.width - GAP) * 0.5;
        for (slot_index, label, strength, tip) in [
            (
                slot::NUDGE,
                "Nudge",
                Strength::Nudge,
                "Move every setting a little, at random. Revert brings it back.",
            ),
            (
                slot::SURPRISE,
                "Surprise",
                Strength::Surprise,
                "Re-tune this scene at random, inside every setting's own range. Revert brings it back.",
            ),
        ] {
            let boundary = UiRect::new(
                area.x + f32::from(slot_index == slot::SURPRISE) * (half + GAP),
                y,
                half,
                BUTTON_H,
            );
            let id = widgets::widget_id(widgets::id::INSPECTOR, slot_index);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                boundary,
                label,
                false,
                ButtonStyle::Neutral,
                Some(metric::UI_FONT_CAPTION),
            );
            if state.clicked {
                self.begin_audition(
                    target,
                    input.settings,
                    if strength == Strength::Surprise {
                        ExploreSource::Surprise
                    } else {
                        ExploreSource::Nudge
                    },
                );
                let seed = self.next_explore_seed(input.scene);
                let mut rng = SplitMix64::new(seed);
                let snapshot =
                    tune_explore::explore(&mut rng, input.scene, input.settings, strength);
                push_snapshot(input.scene, &snapshot, commands);
                self.with_editor(|host| host.reset_arm.disarm());
            }
            self.widgets.hint(d, state, id, boundary, tip);
        }
        y += BUTTON_H;

        if let Some(session) = session {
            y += 4.0;
            widgets::draw_text(
                d,
                font,
                &tune_explore::audition_label(&session),
                area.x,
                y,
                metric::UI_FONT_CAPTION,
                color::accent(),
            );
            y += SENTENCE_H;

            let third = (area.width - GAP * 2.0) / 3.0;
            let boundary = |slot_index: usize| {
                UiRect::new(
                    area.x + slot_index as f32 * (third + GAP),
                    y,
                    third,
                    BUTTON_H,
                )
            };
            let showing_before = session.showing == Side::Before;
            let labels = ["A/B", "Revert", "Keep"];
            let font_size = widgets::row_font_size(font, &labels, &[third; 3], BUTTON_H);

            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::AB_COMPARE);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                boundary(0),
                labels[0],
                showing_before,
                ButtonStyle::Neutral,
                Some(font_size),
            );
            if state.clicked {
                let swap = self.with_editor(|host| host.explore.compare(target, input.settings));
                if let Some(snapshot) = swap {
                    push_snapshot(input.scene, &snapshot, commands);
                }
            }
            self.widgets.hint(
                d,
                state,
                id,
                boundary(0),
                if showing_before {
                    "Showing the tuning you started with - click to hear the new one"
                } else {
                    "Showing the new tuning - click to compare against the one you started with"
                },
            );

            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::AB_REVERT);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                boundary(1),
                labels[1],
                false,
                ButtonStyle::Danger,
                Some(font_size),
            );
            if state.clicked {
                if let Some(snapshot) = self.with_editor(|host| host.explore.revert(target)) {
                    push_snapshot(input.scene, &snapshot, commands);
                }
                self.with_editor(|host| host.reset_arm.disarm());
            }
            self.widgets.hint(
                d,
                state,
                id,
                boundary(1),
                "Put every setting back exactly as it was before you started",
            );

            // Keep is refused while the original is on screen, because it would
            // throw the experiment away — the opposite of what the word says.
            // Disabled with a reason rather than hidden (UX0-B08's rule, applied
            // to a control that is not a route).
            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::AB_KEEP);
            if showing_before {
                self.widgets
                    .disabled_button(d, font, boundary(2), labels[2], Some(font_size));
                let hover = self.widgets.button(d, id, boundary(2));
                self.widgets.hint(
                    d,
                    hover,
                    id,
                    boundary(2),
                    "A/B back to the new tuning first - Keep here would discard it",
                );
            } else {
                let state = self.widgets.text_button(
                    d,
                    font,
                    id,
                    boundary(2),
                    labels[2],
                    false,
                    ButtonStyle::Neutral,
                    Some(font_size),
                );
                if state.clicked {
                    self.with_editor(|host| host.explore.keep(target));
                }
                self.widgets.hint(
                    d,
                    state,
                    id,
                    boundary(2),
                    "Settle on this tuning and close the comparison",
                );
            }
            y += BUTTON_H;
        }

        y - area.y + GAP_AFTER
    }

    /// Open, close, or refuse — the route button's three answers
    /// (`plug.c:6174-6186`).
    ///
    /// A *dirty* draft refuses rather than being replaced, and says why. That is
    /// the whole reason the draft exists: a half-authored route must not be lost
    /// to a click on a different row.
    fn toggle_route_editor(
        &mut self,
        input: &ShellInput<'_>,
        index: usize,
        committed: Option<&ParameterMapping>,
        editing_this_row: bool,
    ) {
        let track_slot = input.workspace.current_index().unwrap_or(0);
        let dirty = self.peek_editor(|host| host.state.is_dirty());
        if dirty {
            self.notify(
                Severity::Warning,
                "Route edit in progress",
                "Apply or discard the open route draft first.",
            );
            return;
        }
        if editing_this_row {
            self.with_editor(|host| host.state.close());
            return;
        }
        let opened =
            self.with_editor(|host| host.state.open(track_slot, input.scene, index, committed));
        if !opened {
            // `open` only refuses for a setting that has no descriptor or a
            // committed route that does not belong to it — both of which mean the
            // table and the descriptor tables disagree. Say so rather than leaving
            // a button that did nothing.
            self.notify(
                Severity::Error,
                "Route editor could not open",
                "This setting's committed route does not match its descriptor.",
            );
        }
    }

    /// The collapsed routed row: summary, live meter, and a click target that
    /// opens the editor (`plug.c:6197-6215`).
    fn routed_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        index: usize,
        descriptor: &SettingDescriptor,
        route: &ParameterMapping,
    ) {
        let font = input.fonts.ui();
        let summary = ascii_fallback(
            font.all_loaded(),
            &route_editor_state::summary(route, descriptor.precision),
        );
        widgets::draw_text(
            d,
            font,
            &summary,
            row.x,
            row.y + 20.0,
            metric::UI_FONT_CAPTION,
            color::ui_ink(),
        );
        let meter = UiRect::new(row.x, row.y + 37.0, row.width, 7.0);
        self.route_meter(d, input, meter, route);

        // The whole zone is the hit target, not just the `~` button
        // (`plug.c:6208-6215`).
        let hit = UiRect::new(row.x, row.y + 18.0, row.width, 28.0);
        let id = widgets::widget_id(widgets::id::INSPECTOR, slot::ROUTED_SUMMARY + index as u32);
        if self.widgets.button(d, id, hit).clicked {
            self.toggle_route_editor(input, index, Some(route), false);
        }
    }

    /// How tall the route editor row for this setting is, or `0.0` when it is
    /// not open for it.
    ///
    /// The oracle's editor *area* is `24+26+40+40+70+26+32+4` px, plus 24 more
    /// when the source is `band` (`plug.c:5517-5523`), and it is asked *before*
    /// the row is measured so a row that will not fit is never drawn — the layout
    /// rule this repository has already paid for. See
    /// [`ROUTE_EDITOR_ROW_HEADER`] for why the row is that plus 26 rather than
    /// the C's plus 41.
    pub(crate) fn route_editor_height(&self, scene: SceneId, index: usize) -> f32 {
        self.peek_editor(|host| {
            if !host.state.targets(host.track_slot, scene, index) {
                return 0.0;
            }
            let band = host
                .state
                .draft()
                .is_some_and(|draft| draft.source == AnalysisSource::Band);
            ROUTE_EDITOR_ROW_HEADER
                + ROUTE_EDITOR_AREA_HEIGHT
                + if band {
                    ROUTE_EDITOR_BAND_ROW_HEIGHT
                } else {
                    0.0
                }
                + ROUTE_EDITOR_ROW_GAP
        })
    }

    /// The expanded route editor, drawn in place of one setting's slider zone
    /// (`scene_route_editor_panel`, `plug.c:5643-5872`).
    ///
    /// Returns the height it consumed, which is the contract the caller advances
    /// by instead of by the slider row's height — how the C makes one row taller
    /// without a second layout pass.
    #[allow(
        clippy::too_many_arguments,
        reason = "the stub's signature, which the inspector's row loop calls"
    )]
    pub(crate) fn route_editor_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        scene: SceneId,
        index: usize,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        let Some(descriptor) = settings::descriptor(scene, index) else {
            return 0.0;
        };
        let Some(draft) = self.peek_editor(|host| host.state.draft().cloned()) else {
            return 0.0;
        };
        let font = input.fonts.ui();

        // The label, the readout and the `~` button on the line above were all
        // drawn by the caller, for every row alike. This function owns the zone
        // the slider would have had, and nothing else.
        let area = UiRect::new(
            row.x,
            row.y + ROUTE_EDITOR_ROW_HEADER,
            row.width,
            row.height - ROUTE_EDITOR_ROW_HEADER - ROUTE_EDITOR_ROW_GAP,
        );
        // The oracle draws the block straight onto the panel; a faint raised
        // ground is what tells a reader that 260 px of controls belong to one row
        // rather than to the list.
        widgets::fill(d, area, color::ui_surface());

        let mut cursor = area.y;
        let live = self.live_value(input, draft.source, draft.band_index);

        // 1. The live caption and the input-window meter (`plug.c:5654-5681`).
        let mapped = live.and_then(|value| draft.output_value(descriptor, value));
        let caption = match (live, mapped) {
            (Some(live), Some(mapped)) => format!(
                "LIVE {} {:.3} {} {:.*}",
                route_editor_state::source_label(draft.source),
                live,
                mapping_editor::arrow(font.all_loaded()),
                descriptor.precision as usize,
                mapped
            ),
            (Some(live), None) => format!(
                "LIVE {}  {:.3}",
                route_editor_state::source_label(draft.source),
                live
            ),
            _ => format!(
                "LIVE {}  no signal",
                route_editor_state::source_label(draft.source)
            ),
        };
        widgets::draw_text(d, font, &caption, area.x, cursor, 12.0, color::ui_muted());
        self.route_meter(
            d,
            input,
            UiRect::new(area.x, cursor + 14.0, area.width, 7.0),
            &draft,
        );
        cursor += 24.0;

        // 2. The sources. Post-legacy musical summaries make this a two-row
        // palette: five comfortably readable choices per row instead of ten
        // cryptic slivers. `ALL` remains the authority so another appended source
        // cannot silently disappear.
        const SOURCE_COLUMNS: usize = 5;
        let source_width =
            (area.width - GAP * (SOURCE_COLUMNS as f32 - 1.0)) / SOURCE_COLUMNS as f32;
        for (slot_index, source) in AnalysisSource::ALL.into_iter().enumerate() {
            let column = slot_index % SOURCE_COLUMNS;
            let source_row = slot_index / SOURCE_COLUMNS;
            let button = UiRect::new(
                area.x + column as f32 * (source_width + GAP),
                cursor + source_row as f32 * 26.0,
                source_width,
                22.0,
            );
            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::SOURCE + slot_index as u32);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                button,
                route_editor_state::source_label(source),
                draft.source == source,
                ButtonStyle::Neutral,
                Some(metric::UI_FONT_CAPTION),
            );
            if state.clicked {
                self.with_editor(|host| host.state.set_source(source));
            }
            self.widgets.hint(
                d,
                state,
                id,
                button,
                match source {
                    AnalysisSource::Rms => "Overall loudness, smoothed",
                    AnalysisSource::Peak => "The loudest instant in the window",
                    AnalysisSource::SpectralFlux => "How much the spectrum is moving",
                    AnalysisSource::BeatPhase => "Position inside the current beat, 0 to 1",
                    AnalysisSource::Band => "One analyzer band, chosen below",
                    AnalysisSource::Time => "An eight-second triangle clock; needs no audio",
                    AnalysisSource::Bass => "Mean energy in the lowest fifth of the spectrum",
                    AnalysisSource::Mids => "Mean energy between the low and high fifths",
                    AnalysisSource::Treble => "Mean energy in the highest fifth of the spectrum",
                    AnalysisSource::Balance => {
                        "Treble share of bass plus treble: 0 bass-heavy, 1 treble-heavy"
                    }
                },
            );
        }
        cursor += 52.0;

        // 3. The band stepper, only for the band source (`plug.c:5699-5721`).
        if draft.source == AnalysisSource::Band {
            // The live analyzer's band count, capped by the analyzer's own
            // maximum inside `step_band`. Zero means no analysis has arrived yet,
            // and the C falls back to the cap rather than refusing to step.
            let band_limit = if input.band_count > 0 {
                input.band_count
            } else {
                musializer_core::audio::analyzer::MAX_BANDS
            };
            let previous = UiRect::new(area.x, cursor, 34.0, 20.0);
            let next = UiRect::new(area.x + area.width - 34.0, cursor, 34.0, 20.0);
            for (id_slot, delta, label, boundary) in [
                (slot::BAND_PREVIOUS, -1, "<", previous),
                (slot::BAND_NEXT, 1, ">", next),
            ] {
                let id = widgets::widget_id(widgets::id::INSPECTOR, id_slot);
                if self
                    .widgets
                    .text_button(
                        d,
                        font,
                        id,
                        boundary,
                        label,
                        false,
                        ButtonStyle::Neutral,
                        Some(metric::UI_FONT_CAPTION),
                    )
                    .clicked
                {
                    self.with_editor(|host| host.state.step_band(delta, band_limit));
                }
            }
            let caption = format!("Band {} of {} (low to high)", draft.band_index, band_limit);
            let width = widgets::measure(font, &caption, 13.0);
            widgets::draw_text(
                d,
                font,
                &caption,
                area.x + (area.width - width) * 0.5,
                cursor + (20.0 - 13.0) * 0.5,
                13.0,
                color::ui_ink(),
            );
            cursor += ROUTE_EDITOR_BAND_ROW_HEIGHT;
        }

        // 4. The two anchors (`plug.c:5723-5776`).
        //
        // Each anchor pairs one source level with one output value, and the
        // physical grouping mirrors that pairing. The C notes why: the layout that
        // grouped the four sliders by axis hid the diagonal input→output
        // relationship the route actually is.
        let span = descriptor.maximum - descriptor.minimum;
        let span = if span <= 0.0 { 1.0 } else { span };
        for anchor in 0..2u32 {
            let high = anchor == 1;
            let name = route_editor_state::anchor_label(draft.source, high);
            let anchor_input = if high {
                draft.input_max
            } else {
                draft.input_min
            };
            let anchor_output = if high {
                draft.output_max
            } else {
                draft.output_min
            };
            let caption = format!(
                "{name}  {anchor_input:.2} {} {:.*}",
                mapping_editor::arrow(font.all_loaded()),
                descriptor.precision as usize,
                anchor_output
            );
            let (input_slot, output_slot) = if high {
                (slot::INPUT_HIGH, slot::OUTPUT_HIGH)
            } else {
                (slot::INPUT_LOW, slot::OUTPUT_LOW)
            };
            let edit = mapping_editor::anchor_pair(
                d,
                &mut self.widgets,
                font,
                area.x,
                cursor,
                area.width,
                &AnchorPair {
                    caption: &caption,
                    input_fraction: anchor_input as f32,
                    output_fraction: (anchor_output as f32 - descriptor.minimum) / span,
                    input_id: widgets::widget_id(widgets::id::INSPECTOR, input_slot),
                    output_id: widgets::widget_id(widgets::id::INSPECTOR, output_slot),
                    arrow_ok: font.all_loaded(),
                },
            );
            if let Some(fraction) = edit.input {
                self.with_editor(|host| {
                    if high {
                        host.state.set_input_max(f64::from(fraction))
                    } else {
                        host.state.set_input_min(f64::from(fraction))
                    }
                });
            }
            if let Some(fraction) = edit.output {
                let value = f64::from(descriptor.minimum + fraction * span);
                self.with_editor(|host| {
                    if high {
                        host.state.set_output_high(value)
                    } else {
                        host.state.set_output_low(value)
                    }
                });
            }
            cursor += 40.0;
        }

        // 5. The transfer graph (`plug.c:5778-5782`). The sample closure is
        // `output_value` — the same function the frame loop maps with — so
        // curve shape, swapped outputs, clamping and toggle quantization draw
        // exactly as they will play.
        let plot = UiRect::new(area.x, cursor, area.width, 64.0);
        mapping_editor::transfer_graph(
            d,
            plot,
            (draft.input_min, draft.input_max),
            f64::from(descriptor.minimum),
            f64::from(span),
            live,
            &|source| draft.output_value(descriptor, source),
        );
        cursor += 70.0;

        // 6. The curve stepper (`plug.c:5784-5808`).
        if let Some(next) = mapping_editor::curve_stepper(
            d,
            &mut self.widgets,
            font,
            area.x,
            cursor,
            area.width,
            widgets::widget_id(widgets::id::INSPECTOR, slot::CURVE_PREVIOUS),
            widgets::widget_id(widgets::id::INSPECTOR, slot::CURVE_NEXT),
            draft.interpolation,
        ) {
            self.with_editor(|host| host.state.set_curve(next));
        }
        cursor += 26.0;

        // 7. The action row (`plug.c:5810-5871`).
        self.route_actions(d, input, area, cursor, &draft, commands);
        row.height
    }

    /// Clamp / Swap / Apply / Remove / Close (`plug.c:5810-5871`).
    ///
    /// The enable rules are the oracle's: Apply is live only for a draft that is
    /// valid *and* either dirty or never committed, Remove only for a draft with a
    /// committed route behind it, and Close reads `Discard` while the draft is
    /// dirty so the destructive answer names itself.
    fn route_actions(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        cursor: f32,
        draft: &ParameterMapping,
        commands: &mut Vec<ShellCommand>,
    ) {
        let font = input.fonts.ui();
        let (dirty, can_apply, has_committed) = self.peek_editor(|host| {
            (
                host.state.is_dirty(),
                host.state.can_apply(),
                host.state
                    .session()
                    .is_some_and(|session| session.committed.is_some()),
            )
        });
        let apply_label = if has_committed { "Applied" } else { "Apply" };
        let close_label = if dirty { "Discard" } else { "Close" };
        let labels = ["Clamp", "Swap", apply_label, "Remove", close_label];

        let action_width = (area.width - GAP * 4.0) / 5.0;
        let widths = [action_width; 5];
        let font_size = widgets::row_font_size(font, &labels, &widths, 28.0);
        let boundary = |slot_index: usize| {
            UiRect::new(
                area.x + slot_index as f32 * (action_width + GAP),
                cursor,
                action_width,
                28.0,
            )
        };
        let id =
            |slot_index: u32| widgets::widget_id(widgets::id::INSPECTOR, slot::ACTION + slot_index);

        let state = self.widgets.text_button(
            d,
            font,
            id(0),
            boundary(0),
            labels[0],
            draft.clamp,
            ButtonStyle::Neutral,
            Some(font_size),
        );
        if state.clicked {
            let next = !draft.clamp;
            self.with_editor(|host| host.state.set_clamp(next));
        }
        self.widgets.hint(
            d,
            state,
            id(0),
            boundary(0),
            "Hold the source inside the quiet/loud window; off lets the curve extrapolate",
        );
        let state = self.widgets.text_button(
            d,
            font,
            id(1),
            boundary(1),
            labels[1],
            false,
            ButtonStyle::Neutral,
            Some(font_size),
        );
        if state.clicked {
            self.with_editor(|host| host.state.swap_output());
        }
        self.widgets.hint(
            d,
            state,
            id(1),
            boundary(1),
            "Exchange the two output values, inverting the response",
        );

        // SEAM 2: committing needs `&mut RouteTable`, which the shell never sees.
        // The enable rules are the oracle's so the row still teaches what Apply
        // costs; the click says the commit is not built rather than doing nothing.
        if can_apply && (dirty || !has_committed) {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id(2),
                    boundary(2),
                    "Apply",
                    true,
                    ButtonStyle::Neutral,
                    Some(font_size),
                )
                .clicked
            {
                commands.push(ShellCommand::ApplyRoute {
                    scene: input.scene,
                    route: draft.clone(),
                });
                self.with_editor(|host| host.state.close());
            }
        } else {
            // UX0-B08: a disabled action says *why*. `disabled_button` returns no
            // state, so the reason needs a hit target of its own — which also
            // stops a click falling through to whatever is behind it.
            self.widgets
                .disabled_button(d, font, boundary(2), apply_label, Some(font_size));
            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::ACTION_DISABLED);
            let hover = self.widgets.button(d, id, boundary(2));
            self.widgets.hint(
                d,
                hover,
                id,
                boundary(2),
                if !can_apply {
                    "The quiet and loud levels are the same, so this route has nothing to map"
                } else {
                    "Already applied - change something above to apply it again"
                },
            );
        }

        if has_committed {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id(3),
                    boundary(3),
                    labels[3],
                    false,
                    ButtonStyle::Danger,
                    Some(font_size),
                )
                .clicked
            {
                commands.push(ShellCommand::RemoveRoute {
                    scene: input.scene,
                    parameter: draft.parameter.clone(),
                });
                self.with_editor(|host| host.state.close());
            }
        } else {
            self.widgets
                .disabled_button(d, font, boundary(3), labels[3], Some(font_size));
            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::ACTION_DISABLED + 1);
            let hover = self.widgets.button(d, id, boundary(3));
            self.widgets.hint(
                d,
                hover,
                id,
                boundary(3),
                "Nothing to remove - this setting has no route yet. Apply one first.",
            );
        }

        if self
            .widgets
            .text_button(
                d,
                font,
                id(4),
                boundary(4),
                close_label,
                false,
                if dirty {
                    ButtonStyle::Danger
                } else {
                    ButtonStyle::Neutral
                },
                Some(font_size),
            )
            .clicked
        {
            self.with_editor(|host| host.state.close());
        }
    }

    /// The live-source meter: where this frame's source value sits inside the
    /// route's input window (`scene_route_meter`, `plug.c:5574-5590`).
    ///
    /// Draws empty when no signal is available, which is the state the C
    /// describes and — SEAM 3 — the state four of the five sources are in here.
    fn route_meter(
        &self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        bar: UiRect,
        route: &ParameterMapping,
    ) {
        let fill = self
            .live_value(input, route.source, route.band_index)
            .map_or(0.0, |live| route_editor_state::meter_position(route, live));
        mapping_editor::meter(d, bar, fill);
    }

    /// SEAM 3: this frame's value for one analysis source.
    ///
    /// `main.rs` already builds a [`musializer_core::scene::routes::RouteSources`]
    /// for the frame loop, but [`ShellInput`] carries only `rms`. RMS therefore
    /// reads live and the rest report no signal — which the meter and the caption
    /// both render as the C renders an unavailable source, rather than as a
    /// confident zero.
    fn live_value(
        &self,
        input: &ShellInput<'_>,
        source: AnalysisSource,
        band_index: u16,
    ) -> Option<f64> {
        // Every source now, not just RMS: `ShellInput` carries the same
        // `RouteSources` the frame loop evaluated the routes from, so the meter
        // and the route agree by construction rather than by coincidence.
        input.route_sources.value(source, band_index)
    }
}

/// The committed route driving one setting of the current track, if any
/// (`route_editor_find_route`, `route_editor_state.c:296-309`).
///
/// Reads the *track's* table rather than the effective settings, because "this
/// value differs from the slider" and "this setting is routed" are different
/// questions and only the second one may open the editor.
fn committed_route<'a>(
    input: &'a ShellInput<'_>,
    scene: SceneId,
    index: usize,
) -> Option<&'a ParameterMapping> {
    let table: &RouteTable = &input.workspace.current()?.scene_routes;
    route_editor_state::find_route(table, scene, index)
}

/// The same guard for [`route_editor_state::summary`], which carries U+00B7 and
/// U+2192 from the C's string literal.
fn ascii_fallback(loaded: bool, text: &str) -> String {
    if loaded {
        return text.to_string();
    }
    text.replace('\u{2192}', "->").replace('\u{00B7}', "-")
}

/// The Tune header's scope line (review 1.7, UX0-A07): whether a slider edit
/// and Reset land on the base scene or on one cue's captured snapshot.
///
/// `cue` is `(position, start_seconds)` for the active cue — its 0-based index
/// in the plan and its own recorded start time, both read from
/// [`crate::workspace::Track::active_cue`] rather than re-derived here, so this
/// function cannot disagree with what a Reset actually touches. `None` means
/// the base scene.
///
/// Pure and pinned by a test rather than only drawn, because a label a capture
/// happens to show is not the same claim as a label a test pins byte for byte —
/// this repository's own rule for a value the oracle would have pinned, applied
/// to a value that has no oracle at all.
pub(crate) fn tune_scope_label(
    cue: Option<(usize, f64)>,
    segments: usize,
    plan_enabled: bool,
) -> String {
    match cue {
        // 1-based in the label: "segment 1" for the first one reads as an
        // ordinal position, which is what a user counting segments on the
        // timeline would call it, not the plan's internal zero-based index.
        // "segment" rather than the older "cue", so the word matches the scene
        // lane, its notices and the operator's own "scene split" (LX3-b).
        Some((position, start_seconds)) => format!(
            "Editing: segment {} of {segments} ({})",
            position + 1,
            widgets::format_timestamp(start_seconds)
        ),
        None if segments == 0 => "Editing: base scene".to_string(),
        // A plan on screen and a base-scene edit is the state the operator
        // could not read (LX3-b): the lane shows six blocks, the sliders move,
        // and nothing said the blocks were not the thing being moved. Both
        // branches name the count, because the count is what is on screen.
        // The plan is driving, the playhead is inside a segment, and that
        // segment has no snapshot of its own — so the edit really does go to
        // the shared table every uncaptured segment of this scene falls back
        // to. Naming it is also the hint: "Capture tuning" is the button that
        // changes the answer.
        None if plan_enabled => "Editing: base scene - this segment captured no tuning".to_string(),
        None => format!("Editing: base scene - {segments} segments are paused"),
    }
}

/// The Reset notice's routed-count sentence (review 1.8, UX0-A08): `ResetScene`
/// only ever touches settings, so a routed row keeps showing its route's
/// output no matter what the reset just did to the slider underneath it. For a
/// heavily-routed scene that reads as "the button did nothing" unless
/// something says otherwise.
///
/// `None` when there is nothing to explain: a reset on a scene with no routed
/// settings really did just reset everything, and a notice would be noise.
#[must_use]
pub(crate) fn reset_routed_notice(routed_count: usize) -> Option<String> {
    match routed_count {
        0 => None,
        1 => Some("1 routed setting keeps its routed value.".to_string()),
        n => Some(format!("{n} routed settings keep their routed values.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::scene::settings::index;

    // The draft is a field of `Shell`, so each test gets a fresh one from
    // `Shell::new()` and there is nothing to leak between them. It used to be a
    // `thread_local`, which is why these tests were once wrapped in a
    // close-before-and-after helper; that helper is gone with the global.

    #[test]
    fn a_closed_editor_leaves_every_row_a_slider() {
        let shell = Shell::new();
        for index in 0..settings::descriptors(SceneId::Loom).len() {
            assert_eq!(shell.route_editor_height(SceneId::Loom, index), 0.0);
        }
    }

    #[test]
    fn recovery_restores_a_route_edit_as_a_dirty_draft() {
        let mut shell = Shell::new();
        let weight = index::loom::WEIGHT;
        assert!(shell
            .route_editor
            .state
            .open(2, SceneId::Loom, weight, None));
        assert!(shell
            .route_editor
            .state
            .set_curve(Interpolation::Smoothstep));
        let recovery = shell.route_recovery_draft().expect("dirty draft");
        assert!(!recovery.is_valid_for_tracks(2));
        assert!(recovery.is_valid_for_tracks(3));

        let mut restored = Shell::new();
        assert!(restored.restore_route_recovery_draft(recovery));
        assert!(restored.route_edit_is_dirty());
        let session = restored.route_editor.state.session().unwrap();
        assert_eq!(session.track_slot, 2);
        assert_eq!(session.scene, SceneId::Loom);
        assert_eq!(session.setting_index, weight);
        assert_eq!(session.draft.interpolation, Interpolation::Smoothstep);
    }

    #[test]
    fn the_expanded_row_accounts_for_the_extended_source_palette() {
        // The oracle's `scene_route_editor_area_height` (`plug.c:5517-5523`),
        // plus one deliberate post-legacy source row. Written out so a geometry
        // change has to update the accounting rather than clipping controls.
        const ORACLE_AREA: f32 = 24.0 + 26.0 + 40.0 + 40.0 + 70.0 + 26.0 + 32.0 + 4.0;
        const MUSICAL_SOURCE_ROW: f32 = 26.0;
        assert_eq!(ROUTE_EDITOR_AREA_HEIGHT, ORACLE_AREA + MUSICAL_SOURCE_ROW);

        let mut shell = Shell::new();
        let weight = index::loom::WEIGHT;
        assert!(shell
            .route_editor
            .state
            .open(0, SceneId::Loom, weight, None));
        assert_eq!(
            shell.route_editor_height(SceneId::Loom, weight),
            ROUTE_EDITOR_AREA_HEIGHT + ROUTE_EDITOR_ROW_HEADER + ROUTE_EDITOR_ROW_GAP
        );
        // Only the row that hosts the draft grows.
        assert_eq!(
            shell.route_editor_height(SceneId::Loom, index::loom::DENSITY),
            0.0
        );
        // And only for the scene it was opened on.
        assert_eq!(shell.route_editor_height(SceneId::SongAtlas, weight), 0.0);

        // The band source adds its stepper row, and the height is asked before
        // the row is measured — so the change has to be visible here.
        assert!(shell.route_editor.state.set_source(AnalysisSource::Band));
        assert_eq!(
            shell.route_editor_height(SceneId::Loom, weight),
            ROUTE_EDITOR_AREA_HEIGHT
                + ROUTE_EDITOR_BAND_ROW_HEIGHT
                + ROUTE_EDITOR_ROW_HEADER
                + ROUTE_EDITOR_ROW_GAP
        );
    }

    #[test]
    fn a_draft_for_another_track_does_not_expand_this_tracks_row() {
        // `route_editor_targets` is track-scoped for exactly this reason
        // (`route_editor_state.c:101-114`): a hidden draft must not reshape the
        // list of a track it does not belong to.
        let mut shell = Shell::new();
        let weight = index::loom::WEIGHT;
        assert!(shell
            .route_editor
            .state
            .open(1, SceneId::Loom, weight, None));
        shell.route_editor.track_slot = 0;
        assert_eq!(shell.route_editor_height(SceneId::Loom, weight), 0.0);
        shell.route_editor.track_slot = 1;
        assert!(shell.route_editor_height(SceneId::Loom, weight) > 0.0);
    }

    #[test]
    fn the_expanded_row_fits_the_inspector_at_the_minimum_window() {
        // The layout rule as a number. `WorkspaceFrame::layout` at the 960x640
        // minimum with the inspector open has to leave a content rectangle the
        // expanded row can sit in, or the row is never drawn and the feature is
        // unreachable at a size the application permits.
        let frame = WorkspaceFrame::layout(960.0, 640.0, true, 1, 150.0);
        let padding = metric::UI_PANEL_PADDING;
        // The panel header rule, then the scene name, then the scope line
        // (review 1.7 added this one), then the first row.
        let available = frame.inspector.height
            - 27.0
            - padding
            - metric::UI_FONT_HEADER
            - metric::UI_CONTROL_GAP
            - metric::UI_FONT_CAPTION
            - metric::UI_CONTROL_GAP;
        assert!(
            available >= ROUTE_EDITOR_AREA_HEIGHT + ROUTE_EDITOR_ROW_HEADER + ROUTE_EDITOR_ROW_GAP,
            "the expanded row needs {} px and the 960x640 inspector offers {available}",
            ROUTE_EDITOR_AREA_HEIGHT + ROUTE_EDITOR_ROW_HEADER + ROUTE_EDITOR_ROW_GAP
        );
    }

    #[test]
    fn the_fallback_face_never_prints_a_missing_glyph_box() {
        // Both substitutions are guarded on `is_loaded`, and the guard is what a
        // capture caught the last time it was missing. Here only the ASCII side is
        // reachable without a window, which is the side that has to be correct.
        assert_eq!(
            ascii_fallback(false, "Band 2 \u{00B7} Smooth \u{00B7} 0.40 \u{2192} 2.20"),
            "Band 2 - Smooth - 0.40 -> 2.20"
        );
    }

    #[test]
    fn tune_scope_label_names_the_base_scene_when_there_is_no_plan_at_all() {
        assert_eq!(tune_scope_label(None, 0, false), "Editing: base scene");
        // `enabled` cannot be true over an empty plan — `set_auto_scenes`
        // refuses it — but the label must not invent segments if it ever is.
        assert_eq!(tune_scope_label(None, 0, true), "Editing: base scene");
    }

    #[test]
    fn tune_scope_label_names_the_segment_and_reuses_the_apps_own_time_format() {
        // Position is 0-based coming out of `Track::active_cue`; the label
        // reads 1-based, "segment 1" for the first one. The time format is
        // `widgets::format_timestamp`'s, not a new one: MM:SS.mmm.
        assert_eq!(
            tune_scope_label(Some((0, 42.0)), 6, true),
            format!(
                "Editing: segment 1 of 6 ({})",
                widgets::format_timestamp(42.0)
            )
        );
        assert_eq!(
            tune_scope_label(Some((0, 42.0)), 6, true),
            "Editing: segment 1 of 6 (00:42.000)"
        );
        assert_eq!(
            tune_scope_label(Some((2, 90.5)), 6, true),
            "Editing: segment 3 of 6 (01:30.500)"
        );
    }

    #[test]
    fn tune_scope_label_distinguishes_a_paused_plan_from_a_driving_one() {
        // LX3-b. The operator's report was "I'm not sure which one gets the
        // tuning applied to", and the state they were in is this one: six
        // segments drawn in the lane, the plan switched off behind their back
        // by a scene click, and a header that said "base scene" without saying
        // that the six blocks on screen were not it.
        assert_eq!(
            tune_scope_label(None, 6, false),
            "Editing: base scene - 6 segments are paused"
        );
        assert_eq!(
            tune_scope_label(None, 6, true),
            "Editing: base scene - this segment captured no tuning"
        );
    }

    #[test]
    fn reset_routed_notice_is_silent_when_nothing_is_routed() {
        assert_eq!(reset_routed_notice(0), None);
    }

    #[test]
    fn reset_routed_notice_names_the_count_and_pluralizes() {
        assert_eq!(
            reset_routed_notice(1).as_deref(),
            Some("1 routed setting keeps its routed value.")
        );
        assert_eq!(
            reset_routed_notice(3).as_deref(),
            Some("3 routed settings keep their routed values.")
        );
    }

    #[test]
    fn reset_arm_requires_the_same_track_and_scene_to_confirm() {
        // Mirrors `preset_delete_armed`'s shape (`main.rs`'s `handle_preset`):
        // arm, then only the matching second click is the real action.
        let mut arm = ResetArm::default();
        assert!(!arm.is_armed_for(0, SceneId::Loom));

        arm.arm(0, SceneId::Loom);
        assert!(arm.is_armed_for(0, SceneId::Loom));
        // A different track, or a different scene on the same track, must not
        // read as armed — that would confirm a reset the user never asked for.
        assert!(!arm.is_armed_for(1, SceneId::Loom));
        assert!(!arm.is_armed_for(0, SceneId::Spectrum));

        arm.disarm();
        assert!(!arm.is_armed_for(0, SceneId::Loom));
    }

    // -- exploration (PX6: UX0-B08, UX0-B09, UX0-C04, UX0-C07) ---------------

    /// The widget-id lesson, applied *inside* a namespace.
    ///
    /// `widgets::id`'s own test proves no two namespaces collide. It cannot see
    /// this file, where every control lives in `INSPECTOR` and several are
    /// `base + index` over up to [`settings::MAX_CONTROLS`] rows — so the way to
    /// break this panel is to mint a base that another band grows into, which is
    /// exactly what `EXPORT`/`SEEK` did one level up. The table is beside the
    /// constants and this sweeps all of it, rather than a hand-written subset.
    #[test]
    fn no_two_inspector_slot_bands_overlap() {
        let mut taken: Vec<(u32, &str)> = Vec::new();
        for (name, first, count) in slot::ALL_SLOTS {
            for offset in 0..count {
                let value = first + offset;
                if let Some((_, other)) = taken.iter().find(|(slot, _)| *slot == value) {
                    panic!("slot {value} is claimed by both {name} and {other}");
                }
                taken.push((value, name));
            }
        }
        // And the one bare literal left: Reset scene.
        assert!(
            !taken.iter().any(|(slot, _)| *slot == RESET_SCENE_SLOT),
            "the Reset scene slot {RESET_SCENE_SLOT} is inside an indexed band"
        );
        // Every scene's control count must fit the 12-wide indexed bands.
        for scene in SceneId::ALL {
            assert!(settings::count(scene) <= 12);
        }
    }

    fn loom_target() -> TuneTarget {
        TuneTarget {
            track_slot: 0,
            scene: SceneId::Loom,
            cue: None,
        }
    }

    fn probe(spec: &str) -> crate::cli::UiProbe {
        crate::cli::parse_ui_probe_spec(spec).expect("a valid probe spec")
    }

    #[test]
    fn the_probe_keys_parse_and_a_typo_fails_the_command_line() {
        assert_eq!(probe("tune-seed=7").tune_seed, Some(7));
        assert_eq!(
            probe("tune-explore=surprise+revert")
                .tune_explore
                .as_deref(),
            Some("surprise+revert")
        );
        // The key is resolved against the descriptor tables at parse time, and
        // the `settings.` prefix is optional exactly as `route=`'s is.
        assert_eq!(
            probe("tune-type=loom.weight:1.42").tune_type.as_deref(),
            Some("settings.loom.weight:1.42")
        );
        // A misspelled action or an unknown key must fail here rather than
        // quietly photographing an unexplored scene.
        assert!(crate::cli::parse_ui_probe_spec("tune-explore=suprise").is_err());
        assert!(crate::cli::parse_ui_probe_spec("tune-type=loom.nope:1").is_err());
        assert!(crate::cli::parse_ui_probe_spec("tune-type=loom.weight:").is_err());
    }

    /// UX0-C04's claim, end to end through the application's own probe path
    /// rather than only through the core module: Surprise then Revert leaves
    /// **the same bits**, not the same picture.
    #[test]
    fn surprise_then_revert_restores_the_exact_bits_through_the_probe() {
        let mut shell = Shell::new();
        let mut settings = SceneSettings::default();
        settings.set(SceneId::Loom, index::loom::WEIGHT, 1.37);
        settings.set(SceneId::Loom, index::loom::DENSITY, 0.83);
        let before = tune_values_line(SceneId::Loom, &settings);

        let spec = probe("tune-seed=4242,tune-explore=surprise");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        let explored = tune_values_line(SceneId::Loom, &settings);
        assert_ne!(explored, before, "Surprise changed nothing to revert");

        let spec = probe("tune-explore=revert");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert_eq!(tune_values_line(SceneId::Loom, &settings), before);
        // And the report line is a round-tripping form, so equality of the two
        // strings really is equality of the bits.
        for (i, _) in settings::descriptors(SceneId::Loom).iter().enumerate() {
            let text = settings.get(SceneId::Loom, i).to_string();
            assert_eq!(
                text.parse::<f32>().unwrap().to_bits(),
                settings.get(SceneId::Loom, i).to_bits(),
                "value {i} does not round-trip through its printed form"
            );
        }
    }

    #[test]
    fn the_seed_is_what_makes_a_surprise_capture_reproducible() {
        let run = |seed: &str| {
            let mut shell = Shell::new();
            let mut settings = SceneSettings::default();
            let spec = probe(&format!("tune-seed={seed},tune-explore=surprise"));
            apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
            tune_values_line(SceneId::Loom, &settings)
        };
        assert_eq!(run("11"), run("11"));
        assert_ne!(run("11"), run("12"));
    }

    #[test]
    fn a_b_puts_the_original_back_on_screen_and_then_returns_the_experiment() {
        let mut shell = Shell::new();
        let mut settings = SceneSettings::default();
        settings.set(SceneId::Loom, index::loom::WEIGHT, 1.37);
        let original = tune_values_line(SceneId::Loom, &settings);

        let spec = probe("tune-seed=9,tune-explore=surprise");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        let explored = tune_values_line(SceneId::Loom, &settings);

        let spec = probe("tune-explore=compare");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert_eq!(tune_values_line(SceneId::Loom, &settings), original);

        let spec = probe("tune-explore=compare");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert_eq!(tune_values_line(SceneId::Loom, &settings), explored);
    }

    #[test]
    fn keep_closes_the_session_so_revert_stops_being_offered() {
        let mut shell = Shell::new();
        let mut settings = SceneSettings::default();
        let spec = probe("tune-seed=5,tune-explore=surprise+keep");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        let kept = tune_values_line(SceneId::Loom, &settings);
        assert!(shell.route_editor.explore.session(loom_target()).is_none());

        // A revert with no session must change nothing rather than reaching for
        // whatever snapshot happens to be lying around.
        let spec = probe("tune-explore=revert");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert_eq!(tune_values_line(SceneId::Loom, &settings), kept);
    }

    /// The LX3 rule, applied to the audition: a snapshot taken against the base
    /// scene must never be written into a cue it was not captured from.
    #[test]
    fn an_audition_does_not_survive_the_target_moving() {
        let mut shell = Shell::new();
        let mut settings = SceneSettings::default();
        let spec = probe("tune-seed=3,tune-explore=surprise");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert!(shell.route_editor.explore.session(loom_target()).is_some());

        // The playhead moves into a segment: a different target entirely.
        let moved = TuneTarget {
            cue: Some(1),
            ..loom_target()
        };
        assert!(shell.route_editor.explore.session(moved).is_none());
        let explored = tune_values_line(SceneId::Loom, &settings);
        let spec = probe("tune-explore=revert");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, Some(1), &mut settings);
        assert_eq!(
            tune_values_line(SceneId::Loom, &settings),
            explored,
            "a revert reached across into another segment"
        );
    }

    #[test]
    fn a_typed_value_is_clamped_by_the_descriptor_and_actually_written() {
        let mut shell = Shell::new();
        let mut settings = SceneSettings::default();
        // 0.50..2.00 for Loom's thread density. `SceneSettings::set` *rejects*
        // rather than clamps, so an unclamped 99 would be dropped in silence —
        // which is what the panel used to do to any out-of-range number.
        let spec = probe("tune-type=loom.density:99");
        let lines = apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert_eq!(settings.get(SceneId::Loom, index::loom::DENSITY), 2.00);
        assert!(
            lines[0].contains("clamped=1") && lines[0].contains("written=1"),
            "{}",
            lines[0]
        );

        let spec = probe("tune-type=loom.density:1.239");
        apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert_eq!(settings.get(SceneId::Loom, index::loom::DENSITY), 1.24);
    }

    #[test]
    fn a_typed_key_from_another_scene_says_so_rather_than_writing_it() {
        let mut shell = Shell::new();
        let mut settings = SceneSettings::default();
        let spec = probe("tune-type=spectrum.amplitude:1.5");
        let lines = apply_tune_probe(&mut shell, &spec, SceneId::Loom, 0, None, &mut settings);
        assert!(lines[0].contains("not on the drawn scene"), "{}", lines[0]);
        assert_eq!(settings, SceneSettings::default());
    }

    #[test]
    fn the_values_line_distinguishes_two_floats_a_readout_would_not() {
        // The whole reason the line prints `{}` and not `{:.2}`: 1.37 and the
        // float one ulp above it both *read* as "1.37".
        let mut a = SceneSettings::default();
        let mut b = SceneSettings::default();
        a.set(SceneId::Loom, index::loom::WEIGHT, 1.37);
        b.set(
            SceneId::Loom,
            index::loom::WEIGHT,
            f32::from_bits(1.37f32.to_bits() + 1),
        );
        assert_ne!(
            tune_values_line(SceneId::Loom, &a),
            tune_values_line(SceneId::Loom, &b)
        );
        assert_eq!(
            format!("{:.2}", a.get(SceneId::Loom, index::loom::WEIGHT)),
            format!("{:.2}", b.get(SceneId::Loom, index::loom::WEIGHT)),
            "the two values are indistinguishable at the readout's precision"
        );
    }

    #[test]
    fn reset_arm_rearming_moves_to_the_new_target() {
        // Switching scene while armed and clicking Reset there re-arms rather
        // than confirming the old scene's reset — the same "movement disarms"
        // rule the preset block's Delete button follows.
        let mut arm = ResetArm::default();
        arm.arm(0, SceneId::Loom);
        arm.arm(0, SceneId::Spectrum);
        assert!(!arm.is_armed_for(0, SceneId::Loom));
        assert!(arm.is_armed_for(0, SceneId::Spectrum));
    }
}
