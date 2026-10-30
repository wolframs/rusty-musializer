//! The tuning inspector, and the route editor row inside it.
//!
//! **Owner: Agent G** for the route editor row. The slider list itself is
//! already ported and is only here because the row it expands lives inside it.
//!
//! Opening a route gives its setting the available Tune area, temporarily
//! hiding presets and other settings so Apply/Discard cannot fall below the
//! panel. Closing returns to all settings. A dirty draft on another scene or
//! track remains visible through Edit draft / Discard draft actions. Manual
//! values remain independent of route drafts. The body scrolls with the wheel
//! or scrollbar; Alt+wheel steps a value. The pure draft state lives in
//! [`musializer_core::ui::route_editor_state`].
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
use musializer_core::scene::settings::{
    self, SceneSettings, SettingDescriptor, SettingsSnapshot, PRESETS_PER_SCENE,
};
use musializer_core::scene::SceneId;
use musializer_core::ui::notice::Severity;
use musializer_core::ui::route_editor_state::{self, RouteEditorDraft, RouteEditorState};
use musializer_core::ui::text_edit::TextRules;
use musializer_core::ui::tune_explore::{
    self, ExploreSource, ExploreState, Side, SplitMix64, Strength, TuneTarget, TypedValueError,
};
use musializer_core::ui::tune_groups;
#[path = "tune/pixel.rs"]
mod pixel;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::AuthoredText;
use raylib::prelude::{KeyboardKey, RaylibDraw, RaylibDrawHandle, Rectangle};
use serde::{Deserialize, Serialize};

use super::super::mapping_editor::{self, AnchorPair};
use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::text_input::TextField;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};
use crate::workspace::{Track, Workspace};

// -- geometry, from `plug.c:5517-5523` ---------------------------------------

// -- widget ids ---------------------------------------------------------------
//
// All inside `widgets::id::INSPECTOR`, below the 900 the reset button uses. The
// per-row `~` buttons are indexed by setting so two rows can never claim one
// another's press; the editor's own controls are singletons because at most one
// editor is open.
mod slot {
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
    /// The route editor's two disabled actions, which need a hit target of
    /// their own to hover-test: `disabled_button` returns no state (B08).
    pub const ACTION_DISABLED: u32 = 420;
    pub const HIDDEN_DRAFT: u32 = 450;

    /// Every constant above, named individually, plus the two bare literals the
    /// row loop uses (`index` for the slider, `900` for Reset scene).
    #[cfg(test)]
    pub const ALL_SLOTS: [(&str, u32, u32); 18] = [
        // (name, first slot, count)
        ("SLIDER", 0, 12),
        // Ten: every `AnalysisSource`, one per item (it read 6 while ten
        // were claimed; nothing sat in 206..209, so it never bit).
        ("SOURCE", SOURCE, 10),
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
        ("ACTION_DISABLED", ACTION_DISABLED, 2),
        ("HIDDEN_DRAFT", HIDDEN_DRAFT, 2),
        // WX-5d's column (`column_slot`): twelve rows of four ids each.
        ("COLUMN_ROW", super::column_slot::ROW, 48),
        ("COLUMN_TILES", super::column_slot::NUDGE, 2),
        ("COLUMN_TRIAL", super::column_slot::COMPARE, 7),
        ("RESET_SCENE (column too)", 900, 0),
    ];
}

/// The Reset-scene button's slot, named rather than left a literal at its use
/// site so [`slot::ALL_SLOTS`]' overlap test can see it.
const RESET_SCENE_SLOT: u32 = 900;
const ROUTE_DRAFT_WARNING: &str = "Route edit in progress";

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

/// Ordinary wheel movement navigates; editing a value is an explicit modifier.
fn tune_wheel(wheel: f32, alt: bool) -> (f32, f32) {
    if alt {
        (0.0, wheel)
    } else {
        (wheel, 0.0)
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
            || (self.column_view() == super::super::shell::ColumnView::Look
                && self.look_column.pixel_typing.is_some())
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
            self.peek_editor(|host| {
                host.state
                    .session()
                    .is_some_and(|draft| draft.track_slot == slot)
            })
        })
    }

    pub(crate) fn focused_route_row(&self, track_slot: usize, scene: SceneId) -> Option<usize> {
        self.peek_editor(|host| {
            host.state
                .session()
                .filter(|draft| draft.track_slot == track_slot && draft.scene == scene)
                .map(|draft| draft.setting_index)
        })
    }

    fn warn_route_draft(&mut self) {
        if self
            .notices
            .notices()
            .iter()
            .any(|notice| notice.title == ROUTE_DRAFT_WARNING)
        {
            return;
        }
        let Some(draft) = self.route_editor.state.session() else {
            return;
        };
        let label = settings::descriptor(draft.scene, draft.setting_index)
            .map_or("setting", |descriptor| descriptor.label);
        let detail = format!(
            "Unapplied route: {} / {label}, track {}. Use Edit draft or Discard draft in Tune.",
            draft.scene.display_name(),
            draft.track_slot + 1
        );
        self.notify(Severity::Warning, ROUTE_DRAFT_WARNING, &detail);
    }

    fn clear_route_warning(&mut self) {
        let ids: Vec<_> = self
            .notices
            .notices()
            .iter()
            .filter(|notice| notice.title == ROUTE_DRAFT_WARNING)
            .map(|notice| notice.id)
            .collect();
        for id in ids {
            self.notices.dismiss(id);
        }
    }

    fn discard_route_draft(&mut self) {
        self.route_editor.state.close();
        self.clear_route_warning();
    }

    /// Keep a draft that belongs elsewhere visible and recoverable. Merely
    /// viewing another scene must neither lose it nor leave an invisible lock.
    fn hidden_route_draft_bar(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        let Some(draft) = self.route_editor.state.session().cloned() else {
            return 0.0;
        };
        let current = input.workspace.current_index().unwrap_or(0);
        if !self.route_editor.state.is_dirty()
            || (draft.track_slot == current && draft.scene == input.scene)
        {
            return 0.0;
        }
        let label = settings::descriptor(draft.scene, draft.setting_index)
            .map_or("setting", |descriptor| descriptor.label);
        widgets::draw_text_faded(
            d,
            input.fonts.ui(),
            &format!(
                "Unapplied route: {} / {label} (track {})",
                draft.scene.display_name(),
                draft.track_slot + 1
            ),
            area.x,
            area.y,
            area.width,
            24.0,
            metric::UI_FONT_CAPTION,
            color::ui_warning(),
        );
        let width = (area.width - 8.0) / 2.0;
        for (index, label) in ["Edit draft", "Discard draft"].iter().enumerate() {
            let button = UiRect::new(
                area.x + index as f32 * (width + 8.0),
                area.y + 20.0,
                width,
                28.0,
            );
            if index == 0 && input.workspace.get(draft.track_slot).is_none() {
                self.widgets
                    .disabled_button(d, input.fonts.ui(), button, label, None);
                continue;
            }
            if self
                .widgets
                .text_button(
                    d,
                    input.fonts.ui(),
                    widgets::widget_id(widgets::id::INSPECTOR, slot::HIDDEN_DRAFT + index as u32),
                    button,
                    label,
                    false,
                    ButtonStyle::Neutral,
                    None,
                )
                .clicked
            {
                if index == 0 {
                    if draft.track_slot != current {
                        commands.push(ShellCommand::SelectTrack(draft.track_slot));
                    }
                    commands.push(ShellCommand::ResumeRouteDraft {
                        track_slot: draft.track_slot,
                        scene: draft.scene,
                    });
                    self.clear_route_warning();
                } else {
                    self.discard_route_draft();
                }
            }
        }
        58.0
    }
}

/// `--ui-probe route=KEY`: opens the editor on a named setting.
///
/// Applied by `main.rs` with the rest of the probe, before the first frame. It
/// exists at all because the expanded editor is 260 px of drawing whose only
/// other entrance is a mouse click, and this repository has already paid twice
/// for a surface no capture could reach.
///
/// Returns the startup request line. Geometry is not known until Look draws:
/// the final `route editor:` line reads the actual card, alongside `tune layout:`.
/// A request must not report the deleted band's row budget as drawn geometry.
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
        let source = self.peek_editor(|host| {
            host.state
                .draft()
                .map_or("?", |draft| route_editor_state::source_label(draft.source))
        });
        format!(
            "route editor request: {key} open source={source} committed={} geometry=after-draw",
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

/// Look's column state across frames, kept on `Shell::look_column` (WX-5d).
/// It lives in this file so this view never edits `shell.rs`.
#[derive(Clone, Debug, Default)]
pub struct LookColumn {
    /// The list's scroll offset, in logical px.
    offset: f32,
    pixel_section: bool,
    pixel_reveal: bool,
    pixel_typing: Option<TextField>,
    /// What the offset is for: a new track or scene starts at the top.
    context: Option<(usize, SceneId)>,
    /// Scroll the open route editor into view on the next frame.
    reveal_editor: bool,
    /// The row whose editor the last frame showed. An editor that opens by any
    /// route — `~`, Edit draft, a probe — is scrolled into view once.
    editor_shown: Option<usize>,
    /// The last track press, `(descriptor index, time)`, for double-click reset.
    last_track_press: Option<(usize, f64)>,
    /// The `tune layout:` line for the last frame Look drew in the column.
    report: String,
    route_report: String,
    pixel_report: String,
    appearance_report: String,
}

/// Two presses on one track within this many seconds reset it (§3.3).
const DOUBLE_PRESS_SECONDS: f64 = 0.4;
/// How far one wheel notch scrolls the list.
const LOOK_WHEEL_STEP: f32 = 48.0;
/// The scene name's type size (the design's 20 px header).
const LOOK_TITLE_SIZE: f32 = 20.0;

/// Slots in `widgets::id::INSPECTOR` for the column (WX-5d), past the 900
/// Reset scene keeps. Rows stride by `widgets::SLIDER_ROW_IDS`.
mod column_slot {
    pub const ROW: u32 = 1000;
    pub const NUDGE: u32 = 1100;
    pub const SURPRISE: u32 = 1101;
    /// The A / B switch: two consecutive items.
    pub const COMPARE: u32 = 1110;
    pub const REVERT: u32 = 1112;
    pub const KEEP: u32 = 1113;
    pub const AGAIN: u32 = 1114;
    pub const ANOTHER: u32 = 1115;
    /// Keep's hover target while A is on screen and Keep is refused.
    pub const KEEP_REFUSED: u32 = 1116;
}

/// The row label column's width for a row `width` wide: a third of the row,
/// held between what a two-word label needs and what leaves the track room
/// (118 at the mockup's 1280 width, 147 at 1920).
#[must_use]
pub(crate) fn look_label_width(width: f32) -> f32 {
    (width * 0.34).round().clamp(96.0, 150.0)
}

/// Look's scope sentence (WX-5d §3.3): where a change lands, in words. The
/// column twin of [`tune_scope_label`], with "section" for the plan's unit as
/// the plan lane now names it.
#[must_use]
pub(crate) fn look_scope_sentence(
    cue: Option<(usize, f64)>,
    sections: usize,
    plan_enabled: bool,
) -> String {
    match cue {
        Some((position, start_seconds)) => format!(
            "Changes apply to section {}, from {}",
            position + 1,
            widgets::format_timestamp(start_seconds)
        ),
        None if sections == 0 => "Changes apply to the base scene".to_string(),
        None if plan_enabled => {
            "Changes apply to the base scene \u{00b7} this section captured no tuning".to_string()
        }
        None => format!("Changes apply to the base scene \u{00b7} {sections} sections are paused"),
    }
}

impl Shell {
    /// The `tune layout:` report line (WX-5d §5): the drawn scene, its groups
    /// with row counts, each row's rect with its track, value and route, the
    /// explore tiles and the presets. **The Look & Assist builder's**; empty
    /// until built, and an empty line is not printed.
    #[must_use]
    pub(crate) fn describe_tune_layout(&self, _workspace: &Workspace) -> String {
        if self.column_view() != super::super::shell::ColumnView::Look {
            return String::new();
        }
        self.look_column.report.clone()
    }

    pub(crate) fn describe_route_editor(&self) -> String {
        self.look_column.route_report.clone()
    }

    /// The tuning inspector: one slider per setting of the active scene.
    ///
    /// Bounds, defaults and precision all come from the descriptor table in
    /// [`settings`], which was checked column-by-column against the C. The
    /// inspector never invents a range.
    pub(crate) fn inspector(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        let column = self.column_rect();
        self.look_view(d, column, input, commands);
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
                color::ui_selected_edge(),
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
                color::ui_hover()
            } else {
                color::ui_raised()
            },
        );
        d.draw_rectangle_lines_ex(
            Rectangle::new(chip.x, chip.y, chip.width, chip.height),
            1.0,
            if state.hovered {
                color::ui_selected_edge()
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
                "Type a value ({:.*} to {:.*})  -  Alt+wheel steps by {}, Shift by ten",
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

    /// Open, close, or refuse — the route button's three answers
    /// (`plug.c:6174-6186`).
    ///
    /// A *dirty* draft refuses rather than being replaced, and says why. That is
    /// the whole reason the draft exists: a half-authored route must not be lost
    /// to a click on a different row.
    fn toggle_route_editor(
        &mut self,
        track_slot: usize,
        scene: SceneId,
        index: usize,
        committed: Option<&ParameterMapping>,
    ) {
        if self.focused_route_row(track_slot, scene) == Some(index) {
            self.discard_route_draft();
            return;
        }
        let dirty = self.peek_editor(|host| host.state.is_dirty());
        if dirty {
            self.warn_route_draft();
            return;
        }
        let opened = self.with_editor(|host| host.state.open(track_slot, scene, index, committed));
        if opened {
            // A route click leaves manual text entry, just like clicking any
            // other control. Its now-hidden field must not keep keyboard focus.
            self.route_editor.typing = None;
            self.route_editor.reset_arm.disarm();
            self.clear_route_warning();
        }
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

// -- Look in the column (WX-5d §3.3) -------------------------------------------

impl Shell {
    /// Look, drawn in the right column: the scene, the scope in words, the
    /// two explore tiles (or the trial card), saved looks, the parameters in
    /// groups with the route editor inline, and the footer. Every rect comes
    /// from [`tune_groups::look_layout`] and
    /// [`route_editor_state::column_editor_layout`].
    ///
    /// The same state and the same commands as the inspector above: a row's
    /// drag, its typed value, its reset and its route all go through the
    /// paths the inspector's controls use, so nothing here can write
    /// somewhere a slider could not.
    pub(crate) fn look_view(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        column: UiRect,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        if column.is_empty() {
            self.look_column.report.clear();
            return;
        }
        let font = input.fonts.ui();
        let scene = input.scene;
        let track_slot = input.workspace.current_index().unwrap_or(0);
        self.with_editor(|host| host.track_slot = track_slot);
        let target = self.tune_target(input, track_slot);
        self.with_editor(|host| host.explore.retarget(target));
        if self.look_column.context != Some((track_slot, scene)) {
            self.look_column.context = Some((track_slot, scene));
            self.look_column.pixel_typing = None;
            self.look_column.offset = 0.0;
        }

        // The head: LOOK, and where the change lands as a count.
        let active_cue = input.workspace.current().and_then(Track::active_cue);
        let (sections, plan_enabled) = input.workspace.current().map_or((0, false), |track| {
            (track.scene_switches.len(), track.scene_switches.enabled)
        });
        let head = widgets::column_head(d, font, column, &[widgets::MicroRun::muted("Look")]);
        let position_text = active_cue.map(|(index, _)| format!("{}", index + 1));
        let total_text = format!("{sections}");
        let trailing: Vec<widgets::MicroRun<'_>> = match position_text.as_deref() {
            Some(position) => vec![
                widgets::MicroRun::muted("Section "),
                widgets::MicroRun::ink(position),
                widgets::MicroRun::muted(" of "),
                widgets::MicroRun::ink(&total_text),
            ],
            None => vec![widgets::MicroRun::muted("Base scene")],
        };
        let trailing_width = widgets::micro_width(font, &trailing);
        let trailing_box = head.trailing_right(trailing_width, metric::UI_FONT_MICRO);
        widgets::draw_micro(d, font, trailing_box.x, trailing_box.y, &trailing);

        // What the list holds this frame.
        let groups = tune_groups::groups(scene);
        let order: Vec<usize> = groups
            .iter()
            .flat_map(|rows| rows.indices.iter().copied())
            .collect();
        let focused = self.focused_route_row(track_slot, scene);
        if focused != self.look_column.editor_shown {
            self.look_column.editor_shown = focused;
            self.look_column.reveal_editor |= focused.is_some();
        }
        let draft_bar = self.route_editor.state.is_dirty() && focused.is_none();
        let audition = self.peek_editor(|host| host.explore.session(target));
        let looks = super::events::SavedLooks::measure(font, &input.presets, scene);
        let card_width = (head.body.width - tune_groups::LOOK_PADDING * 2.0).max(0.0);
        let band = self
            .route_editor
            .state
            .draft()
            .is_some_and(|draft| draft.source == AnalysisSource::Band);
        let spec = tune_groups::LookSpec {
            pixel_look: true,
            groups: groups
                .iter()
                .map(|rows| (rows.group, rows.indices.len()))
                .collect(),
            draft_bar,
            audition: audition.is_some(),
            chip_widths: looks.chip_widths.clone(),
            save_width: looks.save_width,
            empty_hint_width: looks.hint_width,
            preset_action_widths: looks.action_widths.clone(),
            editor: focused.and_then(|index| {
                order.iter().position(|&row| row == index).map(|position| {
                    (
                        position,
                        route_editor_state::column_editor_height(band, card_width),
                    )
                })
            }),
        };

        // Scroll: the wheel moves the list unless Alt asks for a value step.
        let mouse = input.ui_scale.mouse(d);
        let alt = d.is_key_down(KeyboardKey::KEY_LEFT_ALT)
            || d.is_key_down(KeyboardKey::KEY_RIGHT_ALT)
            || self.probe_wheel_alt;
        let (scroll_wheel, value_wheel) = tune_wheel(self.wheel_delta(d), alt);
        let mut layout = tune_groups::look_layout(head.body, &spec, self.look_column.offset);
        if scroll_wheel != 0.0 && layout.viewport.contains_point(mouse.x, mouse.y) {
            layout = tune_groups::look_layout(
                head.body,
                &spec,
                layout.offset - scroll_wheel * LOOK_WHEEL_STEP,
            );
        }
        if std::mem::take(&mut self.look_column.pixel_reveal) {
            if let Some(area) = layout.pixel {
                let offset = tune_groups::reveal_offset(&layout, area);
                layout = tune_groups::look_layout(head.body, &spec, offset);
            }
        }
        if self.look_column.reveal_editor {
            self.look_column.reveal_editor = false;
            if let Some(editor) = layout.editor {
                let offset = tune_groups::reveal_offset(&layout, editor);
                layout = tune_groups::look_layout(head.body, &spec, offset);
            }
        }
        self.look_column.offset = layout.offset;

        // The fixed header: the scene's name and the scope sentence.
        widgets::draw_text(
            d,
            font,
            scene.display_name(),
            layout.title.x,
            layout.title.y + (layout.title.height - LOOK_TITLE_SIZE) * 0.5,
            LOOK_TITLE_SIZE,
            color::ui_ink(),
        );
        let scope = look_scope_sentence(
            active_cue.map(|(index, cue)| (index, cue.start_seconds)),
            sections,
            plan_enabled,
        );
        widgets::draw_text_faded(
            d,
            font,
            &ascii_fallback(font.all_loaded(), &scope),
            layout.scope.x,
            layout.scope.y,
            layout.scope.width,
            24.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );

        let old_clip = self.widgets.set_input_clip(Some(layout.viewport));
        let mut report_rows = Vec::with_capacity(order.len());
        let explore_report;
        let mut editor_report = String::from("none");
        self.look_column.route_report.clear();
        {
            let mut clip = widgets::begin_scissor(d, layout.viewport, input.ui_scale);
            let d = &mut *clip;
            if let Some(bar) = layout.draft_bar {
                self.hidden_route_draft_bar(d, input, bar, commands);
            }
            explore_report =
                self.look_explore(d, input, &layout.explore, target, audition, commands);

            let mut presets = Vec::new();
            self.saved_looks(
                d,
                input,
                &looks,
                layout.presets_head,
                &layout.preset_actions,
                &layout.chips,
                layout.save,
                layout.empty_hint,
                &mut presets,
            );
            if !presets.is_empty() {
                self.with_editor(|host| host.reset_arm.disarm());
            }
            if presets
                .iter()
                .any(|action| matches!(action, PresetAction::Apply(_)))
            {
                self.begin_audition(target, input.settings, ExploreSource::Preset);
            }
            commands.extend(presets.into_iter().map(ShellCommand::Preset));

            if let Some(area) = layout.pixel {
                self.pixel_look_column(d, input, area, commands);
            }
            let label_width = look_label_width(card_width);
            for group in &layout.groups {
                widgets::section_head(
                    d,
                    font,
                    group.head,
                    &[widgets::MicroRun::muted(group.group.title())],
                );
                for &(position, row) in &group.rows {
                    let index = order[position];
                    let line = self.look_row(
                        d,
                        input,
                        row,
                        index,
                        label_width,
                        target,
                        value_wheel,
                        layout.viewport,
                        commands,
                    );
                    report_rows.push(line);
                }
            }
            if let (Some(card), Some(index)) = (layout.editor, focused) {
                editor_report = self.route_editor_column(d, input, card, scene, index, commands);
            }
        }
        self.widgets.set_input_clip(old_clip);
        if !layout.scrollbar.is_empty() {
            widgets::rounded_box(
                d,
                layout.scrollbar,
                2.0,
                color::ui_control_edge(),
                color::ui_control_edge(),
                1.0,
            );
        }

        // The footer: Reset scene… (two presses) and the hint.
        widgets::fill(d, layout.footer, color::ui_surface());
        widgets::fill(
            d,
            UiRect::new(layout.footer.x, layout.footer.y, layout.footer.width, 1.0),
            color::ui_rule(),
        );
        let armed = self.peek_editor(|host| host.reset_arm.is_armed_for(track_slot, scene));
        let reset_id = widgets::widget_id(widgets::id::INSPECTOR, RESET_SCENE_SLOT);
        let reset_state = if armed {
            self.widgets.text_button(
                d,
                font,
                reset_id,
                layout.reset,
                "Confirm reset",
                true,
                ButtonStyle::Danger,
                Some(metric::UI_FONT_CAPTION),
            )
        } else {
            self.text_action(
                d,
                font,
                reset_id,
                layout.reset,
                "Reset scene\u{2026}",
                color::ui_danger(),
            )
        };
        if reset_state.clicked {
            if armed {
                self.with_editor(|host| host.reset_arm.disarm());
                commands.push(ShellCommand::ResetScene(scene));
            } else {
                self.with_editor(|host| host.reset_arm.arm(track_slot, scene));
            }
        }
        self.widgets.hint(
            d,
            reset_state,
            reset_id,
            layout.reset,
            if armed {
                "Press again to put every value of this scene back to its default"
            } else {
                "Every value back to its default; a second press confirms"
            },
        );
        self.appearance_control(d, input, layout.footer_hint, commands);
        let hint = [widgets::MicroRun::muted(
            "~ follows the music \u{00b7} double-click resets",
        )];
        let hint_width = widgets::micro_width(font, &hint);
        if hint_width <= layout.footer_hint.width - 132.0 {
            widgets::draw_micro(
                d,
                font,
                layout.footer_hint.x + layout.footer_hint.width - hint_width,
                layout.footer_hint.y + (layout.footer_hint.height - metric::UI_FONT_MICRO) * 0.5,
                &hint,
            );
        }

        // The report line (§5), from the rects just drawn.
        let rect = |r: UiRect| format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height);
        let groups_text: Vec<String> = groups
            .iter()
            .map(|rows| format!("{}:{}", rows.group.token(), rows.indices.len()))
            .collect();
        let chips_text: Vec<String> = layout.chips.iter().map(|chip| rect(*chip)).collect();
        self.look_column.report = format!(
            "scene={} head={} title={} viewport={} offset={:.0}/{:.0} groups={} rows={} explore={} presets={}/{} chips={} save@{} reset@{}{} editor={}",
            scene.stable_name(),
            rect(head.head),
            rect(layout.title),
            rect(layout.viewport),
            layout.offset,
            layout.max_offset,
            groups_text.join(","),
            report_rows.join(";"),
            explore_report,
            looks.labels.len(),
            PRESETS_PER_SCENE,
            if chips_text.is_empty() {
                "none".to_string()
            } else {
                chips_text.join(";")
            },
            rect(layout.save),
            rect(layout.reset),
            if armed { "*" } else { "" },
            editor_report,
        );
    }

    /// A word that is a button: a hover box, no frame, the label in `tint`
    /// (Reset scene…, the danger-text actions of §3).
    fn text_action(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        id: u64,
        rect: UiRect,
        label: &str,
        tint: raylib::prelude::Color,
    ) -> widgets::ButtonState {
        let state = self.widgets.button(d, id, rect);
        if state.hovered {
            widgets::control_box(d, rect, color::ui_hover(), color::ui_hover(), 1.0);
        }
        widgets::text_label(d, font, rect, label, metric::UI_FONT_CAPTION, tint);
        state
    }

    /// The explore block: Nudge and Surprise as tiles, or the trial card.
    /// Returns its part of the report line.
    fn look_explore(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        explore: &tune_groups::ExploreLayout,
        target: TuneTarget,
        audition: Option<tune_explore::Audition>,
        commands: &mut Vec<ShellCommand>,
    ) -> String {
        let font = input.fonts.ui();
        let rect = |r: UiRect| format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height);
        let press = |shell: &mut Shell, strength: Strength, commands: &mut Vec<ShellCommand>| {
            shell.begin_audition(
                target,
                input.settings,
                if strength == Strength::Surprise {
                    ExploreSource::Surprise
                } else {
                    ExploreSource::Nudge
                },
            );
            let seed = shell.next_explore_seed(input.scene);
            let mut rng = SplitMix64::new(seed);
            let snapshot = tune_explore::explore(&mut rng, input.scene, input.settings, strength);
            push_snapshot(input.scene, &snapshot, commands);
            shell.with_editor(|host| host.reset_arm.disarm());
        };
        match (explore, audition) {
            (tune_groups::ExploreLayout::Tiles { nudge, surprise }, _) => {
                for (tile, slot_index, strength, icon, title, line, short, tip) in [
                    (
                        *nudge,
                        column_slot::NUDGE,
                        Strength::Nudge,
                        musializer_runtime::font::Icon::Sliders,
                        "Nudge",
                        "Small steps around this look",
                        "Small steps",
                        "Move every value a little, at random. The trial card brings it back.",
                    ),
                    (
                        *surprise,
                        column_slot::SURPRISE,
                        Strength::Surprise,
                        musializer_runtime::font::Icon::Random,
                        "Surprise",
                        "A bold new look",
                        "A bold new look",
                        "Re-tune this scene at random, inside every value's own range. The trial card brings it back.",
                    ),
                ] {
                    let id = widgets::widget_id(widgets::id::INSPECTOR, slot_index);
                    let state = self.widgets.button(d, id, tile);
                    widgets::rounded_box(
                        d,
                        tile,
                        6.0,
                        if state.pressed {
                            color::ui_selected()
                        } else if state.hovered {
                            color::ui_hover()
                        } else {
                            color::ui_raised()
                        },
                        if state.hovered {
                            color::ui_selected_edge()
                        } else {
                            color::ui_control_edge()
                        },
                        1.0,
                    );
                    let mut text_x = tile.x + 12.0;
                    if input.fonts.icons_available() {
                        d.draw_text_ex(
                            input.fonts.icons(),
                            &icon.glyph().to_string(),
                            raylib::prelude::Vector2::new(tile.x + 12.0, tile.y + 14.0),
                            15.0,
                            0.0,
                            color::ui_ink(),
                        );
                        text_x += 22.0;
                    }
                    widgets::draw_text(
                        d,
                        font,
                        title,
                        text_x,
                        tile.y + 12.0,
                        15.0,
                        color::ui_ink(),
                    );
                    // The full invitation when it fits; at the 320 column the
                    // first words carry it rather than a faded half sentence.
                    let room = (tile.width - 24.0).max(0.0);
                    let line = if widgets::measure(font, line, metric::UI_FONT_CAPTION) <= room {
                        line
                    } else {
                        short
                    };
                    widgets::draw_text_faded(
                        d,
                        font,
                        line,
                        tile.x + 12.0,
                        tile.y + 37.0,
                        (tile.width - 24.0).max(0.0),
                        18.0,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                    if state.clicked {
                        press(self, strength, commands);
                    }
                    self.widgets.hint(d, state, id, tile, tip);
                }
                format!("nudge@{},surprise@{}", rect(*nudge), rect(*surprise))
            }
            (tune_groups::ExploreLayout::Audition(card), Some(session)) => {
                widgets::rounded_box(
                    d,
                    card.card,
                    6.0,
                    color::ui_surface(),
                    color::ui_selected_edge(),
                    1.0,
                );
                let head = tune_explore::audition_head(&session);
                let moved = session.moved(input.scene, input.settings);
                let moved_text =
                    format!("{moved} value{} moved", if moved == 1 { "" } else { "s" });
                // The verb muted, what is tried in ink (`TRYING SURPRISE 3`). The space
                // stays inside the first run: a run of one space measures as nothing
                // once tracked.
                let (verb, rest) = head
                    .split_once(' ')
                    .map_or((String::new(), head.clone()), |(verb, rest)| {
                        (format!("{verb} "), rest.to_string())
                    });
                let runs = [
                    widgets::MicroRun::muted(&verb),
                    widgets::MicroRun::ink(&rest),
                ];
                let y = card.line.y + (card.line.height - metric::UI_FONT_MICRO) * 0.5;
                widgets::draw_micro(d, font, card.line.x, y, &runs);
                let moved_runs = [widgets::MicroRun::muted(&moved_text)];
                let moved_width = widgets::micro_width(font, &moved_runs);
                widgets::draw_micro(
                    d,
                    font,
                    card.line.x + card.line.width - moved_width,
                    y,
                    &moved_runs,
                );

                let showing_before = session.showing == Side::Before;
                let segmented = self.widgets.segmented(
                    d,
                    font,
                    widgets::id::INSPECTOR,
                    column_slot::COMPARE,
                    card.compare,
                    &["A \u{00b7} before", "B \u{00b7} this"],
                    Some(usize::from(!showing_before)),
                    widgets::SegmentedFit::Hug,
                );
                if let Some(pick) = segmented.clicked {
                    if (pick == 0) != showing_before {
                        let swap =
                            self.with_editor(|host| host.explore.compare(target, input.settings));
                        if let Some(snapshot) = swap {
                            push_snapshot(input.scene, &snapshot, commands);
                        }
                    }
                }

                let id = widgets::widget_id(widgets::id::INSPECTOR, column_slot::REVERT);
                let state = self.widgets.text_button(
                    d,
                    font,
                    id,
                    card.revert,
                    "Revert",
                    false,
                    ButtonStyle::Ghost,
                    Some(metric::UI_FONT_CAPTION),
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
                    card.revert,
                    "Put every value back exactly as it was before you started",
                );
                if showing_before {
                    self.widgets.disabled_button(
                        d,
                        font,
                        card.keep,
                        "Keep",
                        Some(metric::UI_FONT_CAPTION),
                    );
                    let id = widgets::widget_id(widgets::id::INSPECTOR, column_slot::KEEP_REFUSED);
                    let hover = self.widgets.button(d, id, card.keep);
                    self.widgets.hint(
                        d,
                        hover,
                        id,
                        card.keep,
                        "Switch back to B first - Keep here would discard the trial",
                    );
                } else {
                    let id = widgets::widget_id(widgets::id::INSPECTOR, column_slot::KEEP);
                    let state = self.widgets.text_button(
                        d,
                        font,
                        id,
                        card.keep,
                        "Keep",
                        false,
                        ButtonStyle::Strong,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    if state.clicked {
                        self.with_editor(|host| host.explore.keep(target));
                    }
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        card.keep,
                        "Settle on this look and close the trial",
                    );
                }
                for (button, slot_index, strength, label) in [
                    (
                        card.again,
                        column_slot::AGAIN,
                        Strength::Nudge,
                        "Nudge again",
                    ),
                    (
                        card.another,
                        column_slot::ANOTHER,
                        Strength::Surprise,
                        "Another",
                    ),
                ] {
                    let id = widgets::widget_id(widgets::id::INSPECTOR, slot_index);
                    let state = self.widgets.text_button(
                        d,
                        font,
                        id,
                        button,
                        label,
                        false,
                        ButtonStyle::Neutral,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    if state.clicked {
                        press(self, strength, commands);
                    }
                }
                let items: Vec<String> = segmented
                    .layout
                    .items
                    .iter()
                    .map(|item| rect(*item))
                    .collect();
                format!(
                    "trial@{} a@{} b@{} revert@{} keep@{} again@{} another@{} moved={moved}",
                    rect(card.card),
                    items.first().map_or("-", String::as_str),
                    items.get(1).map_or("-", String::as_str),
                    rect(card.revert),
                    rect(card.keep),
                    rect(card.again),
                    rect(card.another),
                )
            }
            (tune_groups::ExploreLayout::Audition(_), None) => String::from("none"),
        }
    }

    /// One parameter row (§3.3): label with the changed dot, the track with
    /// its default tick, the value (press to type), and `~`. Returns its
    /// entry in the report line.
    #[allow(
        clippy::too_many_arguments,
        reason = "one row's worth of context: where, which setting, and where the edit lands"
    )]
    fn look_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        index: usize,
        label_width: f32,
        target: TuneTarget,
        value_wheel: f32,
        viewport: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> String {
        let font = input.fonts.ui();
        let scene = input.scene;
        let Some(descriptor) = settings::descriptor(scene, index) else {
            return String::new();
        };
        let track_slot = input.workspace.current_index().unwrap_or(0);
        let committed = committed_route(input, scene, index).cloned();
        let routed = committed.is_some();
        let open = self.focused_route_row(track_slot, scene) == Some(index);
        let value = input.settings.get(scene, index);
        let effective = input
            .routed
            .map_or(value, |routed| routed.get(scene, index));
        let span = descriptor.maximum - descriptor.minimum;
        let fraction = |v: f32| {
            if span > 0.0 {
                ((v - descriptor.minimum) / span).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let modified = value.to_bits() != descriptor.default_value.to_bits();
        let pulse_auto = scene == SceneId::PulseField
            && descriptor.key == "settings.pulse.petals"
            && effective < 0.5
            && !routed;
        let value_text = if pulse_auto {
            "Auto".to_string()
        } else {
            format!("{:.*}", descriptor.precision as usize, effective)
        };
        let label = ascii_fallback(font.all_loaded(), descriptor.label);
        let first = column_slot::ROW + index as u32 * widgets::SLIDER_ROW_IDS;
        let result = self.widgets.slider_row(
            d,
            font,
            widgets::id::INSPECTOR,
            first,
            row,
            &widgets::SliderRow {
                label: &label,
                value_text: &value_text,
                normalized: fraction(effective),
                default: Some(fraction(descriptor.default_value)),
                changed: modified,
                route: Some(routed || open),
                label_width,
            },
        );
        let layout = result.layout;
        let id = |offset: u32| widgets::widget_id(widgets::id::INSPECTOR, first + offset);
        let editable = !routed && !open;

        // The label resets a changed value, as it did in the inspector.
        if modified && result.label.clicked {
            self.begin_audition(target, input.settings, ExploreSource::Manual);
            commands.push(ShellCommand::SetSetting {
                scene,
                index,
                value: descriptor.default_value,
            });
            self.with_editor(|host| host.reset_arm.disarm());
        }
        self.widgets.hint(
            d,
            result.label,
            id(1),
            layout.label,
            &if modified {
                format!(
                    "{} - click to reset to {:.*}",
                    descriptor.label, descriptor.precision as usize, descriptor.default_value
                )
            } else {
                format!("{} - at its default", descriptor.label)
            },
        );

        // The track: a drag writes onto the descriptor's grid; two presses
        // reset. A routed value belongs to its route, so the track only shows it.
        if editable {
            if let Some(fraction) = result.drag {
                let proposed =
                    tune_explore::conform(descriptor, descriptor.minimum + fraction * span);
                if proposed.to_bits() != value.to_bits() {
                    commands.push(ShellCommand::SetSetting {
                        scene,
                        index,
                        value: proposed,
                    });
                    self.with_editor(|host| {
                        host.explore.note_manual_edit(target);
                        host.reset_arm.disarm();
                    });
                }
            }
            if result.track.clicked {
                let now = d.get_time();
                let double = self
                    .look_column
                    .last_track_press
                    .is_some_and(|(last, at)| last == index && now - at <= DOUBLE_PRESS_SECONDS);
                if double {
                    self.look_column.last_track_press = None;
                    self.begin_audition(target, input.settings, ExploreSource::Manual);
                    commands.push(ShellCommand::SetSetting {
                        scene,
                        index,
                        value: descriptor.default_value,
                    });
                } else {
                    self.look_column.last_track_press = Some((index, now));
                }
            }
            // Alt+wheel steps the value under the pointer; Shift makes ten.
            if value_wheel != 0.0 {
                let pointer = input.ui_scale.mouse(d);
                if row.contains_point(pointer.x, pointer.y)
                    && viewport.contains_point(pointer.x, pointer.y)
                {
                    let coarse = d.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
                        || d.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT)
                        || self.probe_wheel_shift;
                    let steps = (value_wheel.round() as i32) * if coarse { 10 } else { 1 };
                    let stepped = tune_explore::step(descriptor, value, steps);
                    if stepped.to_bits() != value.to_bits() {
                        commands.push(ShellCommand::SetSetting {
                            scene,
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
        }

        // The value: a press opens typed entry; while typing, the field sits
        // over the value, a little wider so a long number fits.
        let typing_here = self.peek_editor(|host| {
            host.typing
                .as_ref()
                .is_some_and(|entry| entry.scene == scene && entry.index == index)
        });
        let field = UiRect::new(
            layout.value.x - 22.0,
            layout.value.y + 4.0,
            layout.value.width + 22.0,
            layout.value.height - 8.0,
        );
        if typing_here {
            if !viewport.contains(field) {
                self.with_editor(|host| host.typing = None);
            } else {
                self.value_chip(
                    d,
                    input,
                    field,
                    index,
                    descriptor,
                    effective,
                    &value_text,
                    target,
                    commands,
                );
            }
        } else if editable && result.value.clicked {
            let precision = descriptor.precision;
            self.with_editor(|host| {
                host.typing = Some(TypedEntry::open(scene, index, value, precision));
            });
        }
        self.widgets.hint(
            d,
            result.value,
            id(2),
            layout.value,
            &if editable {
                format!(
                    "Type a value ({:.*} to {:.*}) - Alt+wheel steps, Shift by ten",
                    descriptor.precision as usize,
                    descriptor.minimum,
                    descriptor.precision as usize,
                    descriptor.maximum,
                )
            } else {
                "This value follows the music; open ~ to change how".to_string()
            },
        );

        // `~`: open, close or refuse, as the inspector's route button.
        if result.route.clicked {
            self.toggle_route_editor(track_slot, scene, index, committed.as_ref());
            if self.focused_route_row(track_slot, scene) == Some(index) {
                self.look_column.reveal_editor = true;
            }
        }
        self.widgets.hint(
            d,
            result.route,
            id(3),
            layout.route,
            &match (open, committed.as_ref()) {
                (true, _) if self.route_edit_is_dirty() => {
                    "Discard this unapplied route".to_string()
                }
                (true, _) => "Close the route editor".to_string(),
                (false, Some(route)) => format!(
                    "Follows {} - click to edit",
                    ascii_fallback(
                        font.all_loaded(),
                        &route_editor_state::summary(route, descriptor.precision)
                    )
                ),
                (false, None) => "Let the music move this value".to_string(),
            },
        );

        let rect = |r: UiRect| format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height);
        format!(
            "{}{}@{}(track@{},value@{},route@{},label@{})",
            descriptor.key,
            match (modified, routed) {
                (true, true) => "*~",
                (true, false) => "*",
                (false, true) => "~",
                (false, false) => "",
            },
            rect(layout.row),
            rect(layout.track),
            rect(layout.value),
            rect(layout.route),
            rect(layout.label),
        )
    }

    /// The route editor as the column draws it (§3.3): the same draft and
    /// commands as [`Self::route_editor_row`], in the column's vocabulary.
    /// Returns its part of the report line.
    fn route_editor_column(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        card: UiRect,
        scene: SceneId,
        index: usize,
        commands: &mut Vec<ShellCommand>,
    ) -> String {
        let Some(descriptor) = settings::descriptor(scene, index) else {
            return "none".to_string();
        };
        let Some(draft) = self.peek_editor(|host| host.state.draft().cloned()) else {
            return "none".to_string();
        };
        let font = input.fonts.ui();
        let layout =
            route_editor_state::column_editor_layout(card, draft.source == AnalysisSource::Band);
        let (dirty, can_apply, has_committed) = self.peek_editor(|host| {
            (
                host.state.is_dirty(),
                host.state.can_apply(),
                host.state
                    .session()
                    .is_some_and(|session| session.committed.is_some()),
            )
        });
        self.look_column.route_report = format!(
            "{} open row={}px source={} committed={} card={:.0},{:.0},{:.0},{:.0}",
            descriptor.key,
            layout.card.height,
            route_editor_state::source_label(draft.source),
            u8::from(has_committed),
            layout.card.x,
            layout.card.y,
            layout.card.width,
            layout.card.height,
        );
        widgets::rounded_box(
            d,
            layout.card,
            6.0,
            color::ui_raised(),
            color::ui_control_edge(),
            1.0,
        );

        // Head: what follows the music, and whether it is applied.
        let label = ascii_fallback(font.all_loaded(), descriptor.label);
        let head = [
            widgets::MicroRun::muted("Follows the music \u{00b7} "),
            widgets::MicroRun::ink(&label),
        ];
        let y = layout.head.y + (layout.head.height - metric::UI_FONT_MICRO) * 0.5;
        let state_word = if has_committed && !dirty {
            "Applied"
        } else {
            "Not applied yet"
        };
        let state_runs = [widgets::MicroRun::muted(state_word)];
        let state_width = widgets::micro_width(font, &state_runs);
        widgets::draw_micro(d, font, layout.head.x, y, &head);
        if widgets::micro_width(font, &head) + state_width + 12.0 <= layout.head.width {
            widgets::draw_micro(
                d,
                font,
                layout.head.x + layout.head.width - state_width,
                y,
                &state_runs,
            );
        }

        // Sources: two segmented rows of five.
        let selected = AnalysisSource::ALL
            .iter()
            .position(|source| *source == draft.source);
        let mut source_items = Vec::new();
        for (row_index, boundary) in layout.sources.iter().enumerate() {
            let start = row_index * route_editor_state::COLUMN_EDITOR_SOURCES_PER_ROW;
            let end = (start + route_editor_state::COLUMN_EDITOR_SOURCES_PER_ROW)
                .min(AnalysisSource::ALL.len());
            let labels: Vec<&str> = AnalysisSource::ALL[start..end]
                .iter()
                .map(|source| route_editor_state::source_label(*source))
                .collect();
            let segmented = self.widgets.segmented(
                d,
                font,
                widgets::id::INSPECTOR,
                slot::SOURCE + start as u32,
                *boundary,
                &labels,
                selected.and_then(|picked| picked.checked_sub(start)),
                widgets::SegmentedFit::Fill,
            );
            if let Some(pick) = segmented.clicked {
                let source = AnalysisSource::ALL[start + pick];
                self.with_editor(|host| host.state.set_source(source));
            }
            source_items.extend(segmented.layout.items);
        }

        // The band stepper, only for the band source.
        if draft.source == AnalysisSource::Band {
            let band_limit = if input.band_count > 0 {
                input.band_count
            } else {
                musializer_core::audio::analyzer::MAX_BANDS
            };
            let row = layout.band;
            for (id_slot, delta, label, boundary) in [
                (
                    slot::BAND_PREVIOUS,
                    -1,
                    "<",
                    UiRect::new(row.x, row.y, 34.0, row.height),
                ),
                (
                    slot::BAND_NEXT,
                    1,
                    ">",
                    UiRect::new(row.x + row.width - 34.0, row.y, 34.0, row.height),
                ),
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
            widgets::text_label(
                d,
                font,
                row,
                &format!("Band {} of {} (low to high)", draft.band_index, band_limit),
                metric::UI_FONT_CAPTION,
                color::ui_ink(),
            );
        }

        // The live level: the source's name, where it sits in the window, the
        // reading.
        let live = self.live_value(input, draft.source, draft.band_index);
        let name = [widgets::MicroRun::muted(route_editor_state::source_label(
            draft.source,
        ))];
        let level_y = layout.level.y + (layout.level.height - metric::UI_FONT_MICRO) * 0.5;
        widgets::draw_micro(d, font, layout.level.x, level_y, &name);
        let reading = live.map_or_else(|| "no signal".to_string(), |value| format!("{value:.2}"));
        let reading_width = widgets::measure_tabular(font, &reading, metric::UI_FONT_CAPTION);
        widgets::draw_text_tabular(
            d,
            font,
            &reading,
            layout.level.x + layout.level.width - reading_width,
            layout.level.y + (layout.level.height - metric::UI_FONT_CAPTION) * 0.5,
            metric::UI_FONT_CAPTION,
            color::ui_ink(),
        );
        let meter = UiRect::new(
            layout.level.x + 52.0,
            layout.level.y + (layout.level.height - 4.0) * 0.5,
            (layout.level.width - 52.0 - reading_width - 12.0).max(0.0),
            4.0,
        );
        self.route_meter(d, input, meter, &draft);

        // The transfer plot: the live reading is the accent dot (5a's rule).
        let span = descriptor.maximum - descriptor.minimum;
        let span = if span <= 0.0 { 1.0 } else { span };
        mapping_editor::transfer_graph(
            d,
            layout.plot,
            (draft.input_min, draft.input_max),
            f64::from(descriptor.minimum),
            f64::from(span),
            live,
            &|source| draft.output_value(descriptor, source),
        );

        // The two anchors: the shared pair, so Lyrics' `~ Tune` matches.
        for (anchor, rect) in layout.anchors.iter().enumerate() {
            let high = anchor == 1;
            let name = route_editor_state::anchor_label(draft.source, high);
            let (anchor_input, anchor_output) = if high {
                (draft.input_max, draft.output_max)
            } else {
                (draft.input_min, draft.output_min)
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
                rect.x,
                rect.y,
                rect.width,
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
        }

        if let Some(next) = mapping_editor::curve_stepper(
            d,
            &mut self.widgets,
            font,
            layout.curve.x,
            layout.curve.y,
            layout.curve.width,
            widgets::widget_id(widgets::id::INSPECTOR, slot::CURVE_PREVIOUS),
            widgets::widget_id(widgets::id::INSPECTOR, slot::CURVE_NEXT),
            draft.interpolation,
        ) {
            self.with_editor(|host| host.state.set_curve(next));
        }

        // The actions: Apply strong; Discard; Remove for a committed route;
        // Clamp and Swap at the right.
        let size = Some(metric::UI_FONT_CAPTION);
        let action =
            |slot_index: u32| widgets::widget_id(widgets::id::INSPECTOR, slot::ACTION + slot_index);
        let apply_live = can_apply && (dirty || !has_committed);
        if apply_live {
            let state = self.widgets.text_button(
                d,
                font,
                action(2),
                layout.apply,
                "Apply",
                false,
                ButtonStyle::Strong,
                size,
            );
            if state.clicked {
                commands.push(ShellCommand::ApplyRoute {
                    scene,
                    route: draft.clone(),
                });
                self.discard_route_draft();
            }
        } else {
            self.widgets.disabled_button(
                d,
                font,
                layout.apply,
                if has_committed { "Applied" } else { "Apply" },
                size,
            );
            let id = widgets::widget_id(widgets::id::INSPECTOR, slot::ACTION_DISABLED);
            let hover = self.widgets.button(d, id, layout.apply);
            self.widgets.hint(
                d,
                hover,
                id,
                layout.apply,
                if !can_apply {
                    "The quiet and loud levels are the same, so this route has nothing to map"
                } else {
                    "Already applied - change something above to apply it again"
                },
            );
        }
        let close = self.widgets.text_button(
            d,
            font,
            action(4),
            layout.discard,
            if dirty { "Discard" } else { "Close" },
            false,
            ButtonStyle::Ghost,
            size,
        );
        if close.clicked {
            self.discard_route_draft();
        }
        if has_committed {
            let state = self.text_action(
                d,
                font,
                action(3),
                layout.remove,
                "Remove",
                color::ui_danger(),
            );
            if state.clicked {
                commands.push(ShellCommand::RemoveRoute {
                    scene,
                    parameter: draft.parameter.clone(),
                });
                self.discard_route_draft();
            }
        }
        let clamp = self.widgets.text_button(
            d,
            font,
            action(0),
            layout.clamp,
            "Clamp",
            draft.clamp,
            ButtonStyle::Ghost,
            size,
        );
        if clamp.clicked {
            let next = !draft.clamp;
            self.with_editor(|host| host.state.set_clamp(next));
        }
        self.widgets.hint(
            d,
            clamp,
            action(0),
            layout.clamp,
            "Hold the source inside the quiet/loud window; off lets the curve extrapolate",
        );
        let swap = self.widgets.text_button(
            d,
            font,
            action(1),
            layout.swap,
            "Swap",
            false,
            ButtonStyle::Ghost,
            size,
        );
        if swap.clicked {
            self.with_editor(|host| host.state.swap_output());
        }
        self.widgets.hint(
            d,
            swap,
            action(1),
            layout.swap,
            "Exchange the two output values, inverting the response",
        );

        let rect = |r: UiRect| format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height);
        let sources: Vec<String> = AnalysisSource::ALL
            .iter()
            .zip(&source_items)
            .map(|(source, item)| {
                format!(
                    "{}{}@{}",
                    source.canonical_name(),
                    if *source == draft.source { "*" } else { "" },
                    rect(*item)
                )
            })
            .collect();
        format!(
            "{}@{}(sources={} plot@{} apply@{}{} discard@{} clamp@{} swap@{} dirty={})",
            descriptor.key,
            rect(layout.card),
            sources.join(";"),
            rect(layout.plot),
            rect(layout.apply),
            if apply_live { "" } else { "!" },
            rect(layout.discard),
            rect(layout.clamp),
            rect(layout.swap),
            u8::from(dirty),
        )
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
    fn manual_hue_motion_leaves_other_routes_available() {
        let mut shell = Shell::new();
        let mut values = SceneSettings::default();
        assert!(values.set(SceneId::SongAtlas, index::atlas::HUE_MOTION, 1.0));
        shell.route_editor.explore.note_manual_edit(TuneTarget {
            track_slot: 0,
            scene: SceneId::SongAtlas,
            cue: None,
        });
        assert!(!shell.route_edit_is_dirty());
        shell.toggle_route_editor(0, SceneId::SongAtlas, index::atlas::COLOR, None);
        assert_eq!(
            shell.focused_route_row(0, SceneId::SongAtlas),
            Some(index::atlas::COLOR)
        );
        assert!(!shell.route_edit_is_dirty());
        assert!(shell.notices.notices().is_empty());
    }

    #[test]
    fn hidden_draft_is_preserved_and_warns_only_once_until_explicitly_discarded() {
        let mut shell = Shell::new();
        assert!(!ShellCommand::ResumeRouteDraft {
            track_slot: 1,
            scene: SceneId::Loom,
        }
        .mutates_project());
        shell.toggle_route_editor(1, SceneId::Loom, index::loom::WEIGHT, None);
        shell
            .route_editor
            .state
            .set_curve(Interpolation::Smoothstep);
        let original = shell.route_recovery_draft().unwrap();
        // The inspector most recently drew track zero; that is not ownership.
        shell.route_editor.track_slot = 0;
        assert!(!shell.route_editor_open_for_active_track(Some(0)));
        assert!(shell.route_editor_open_for_active_track(Some(1)));
        assert_eq!(shell.focused_route_row(0, SceneId::SongAtlas), None);
        for _ in 0..8 {
            shell.toggle_route_editor(0, SceneId::SongAtlas, index::atlas::COLOR, None);
        }
        assert_eq!(shell.route_recovery_draft().unwrap(), original);
        assert_eq!(shell.notices.notices().len(), 1);
        assert!(shell.notices.notices()[0].detail.contains("Loom"));
        assert!(shell.notices.notices()[0].detail.contains("track 2"));
        shell.discard_route_draft();
        assert!(shell.notices.notices().is_empty());
        shell.toggle_route_editor(0, SceneId::SongAtlas, index::atlas::COLOR, None);
        assert_eq!(
            shell.focused_route_row(0, SceneId::SongAtlas),
            Some(index::atlas::COLOR)
        );
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
    fn the_column_rows_stride_by_the_widgets_ids_per_row() {
        assert_eq!(widgets::SLIDER_ROW_IDS, 4);
        assert_eq!(
            slot::ALL_SLOTS
                .iter()
                .find(|(name, _, _)| *name == "COLUMN_ROW")
                .map(|(_, _, count)| *count),
            Some(12 * widgets::SLIDER_ROW_IDS)
        );
        assert_eq!(column_slot::ANOTHER + 1, column_slot::KEEP_REFUSED);
    }

    #[test]
    fn look_scope_sentence_names_the_section_or_the_base_scene() {
        assert_eq!(
            look_scope_sentence(Some((20, 92.583)), 70, true),
            "Changes apply to section 21, from 01:32.583"
        );
        assert_eq!(
            look_scope_sentence(None, 0, false),
            "Changes apply to the base scene"
        );
        assert_eq!(
            look_scope_sentence(None, 70, true),
            "Changes apply to the base scene \u{00b7} this section captured no tuning"
        );
        assert_eq!(
            look_scope_sentence(None, 6, false),
            "Changes apply to the base scene \u{00b7} 6 sections are paused"
        );
    }

    #[test]
    fn the_label_column_is_a_third_of_the_row_within_its_bounds() {
        assert_eq!(look_label_width(330.0), 112.0);
        assert_eq!(look_label_width(432.0), 147.0);
        assert_eq!(look_label_width(292.0), 99.0);
        assert_eq!(look_label_width(200.0), 96.0);
        assert_eq!(look_label_width(900.0), 150.0);
    }

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
