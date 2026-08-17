//! Route editor state over `core::scene::routes`.
//!
//! **Owner: Agent F.** Port of `route_editor_state.c`/`.h` from the frozen C oracle
//! at `../musializer` (commit `9300af9`, read-only).
//!
//! Headless draft state for the Tune-inspector route editor. Editing happens on a
//! draft copy so a half-finished route never touches the committed table; Apply
//! commits the draft, Close discards it. A dirty draft participates in the
//! application's close/context-change guards like other drafts
//! (`route_editor_state.h:10-13`).
//!
//! Two shape changes from the C, both of them deletions:
//!
//! - C stored a zeroed struct with a `bool open` flag, and half its functions
//!   began `if (!state->open) return false`. Here the open/closed distinction is
//!   structural — an [`Option<RouteEditorDraft>`] — so the closed case is one
//!   pattern match rather than a rule every method has to remember. The
//!   false-on-misuse contract is kept exactly: a closed editor accepts no edits.
//! - C indexed scenes with `size_t scene_index` and checked it against
//!   `SCENE_SETTINGS_SCENE_COUNT` on every entry point. [`SceneId`] makes the
//!   invalid-index branch unrepresentable. `setting_index` stays a `usize` and
//!   keeps its bounds check, because a scene really can be asked for a control it
//!   does not have.

use crate::scene::routes::{AnalysisSource, Interpolation, ParameterMapping, RouteTable};
use crate::scene::settings::{self, SettingDescriptor};
use crate::scene::SceneId;

/// Minimum input-range width the editor maintains while either endpoint is
/// dragged (`route_editor_state.h:15-17`).
///
/// `ParameterMapping::is_valid_for` only demands `input_max > input_min`; this
/// keeps the sliders from pinching the range into uselessness.
pub const ROUTE_EDITOR_INPUT_GAP: f64 = 0.01;

/// One open draft: the setting being edited and the route being authored for it
/// (`route_editor_state.h:19-31`).
///
/// C's `has_committed` flag collapses into [`Self::committed`] being `Some`: it
/// means the edited setting already had a committed route at open (or after
/// Apply), and that route is what the dirty comparison runs against.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteEditorDraft {
    pub track_slot: usize,
    pub scene: SceneId,
    pub setting_index: usize,
    /// The committed route this draft is compared against, when there is one.
    pub committed: Option<ParameterMapping>,
    pub draft: ParameterMapping,
    /// A fresh draft only guards once an edit actually changed a field.
    pub touched: bool,
}

/// The Tune inspector's route editor (`Route_Editor_State`,
/// `route_editor_state.h:19-31`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouteEditorState {
    session: Option<RouteEditorDraft>,
}

/// Clamps an output endpoint into the descriptor's range and quantizes it for
/// integer-looking settings (`route_editor_snap_output`,
/// `route_editor_state.c:33-40`).
///
/// C wrote the clamp as two `if`s; `clamp` is equivalent here because every caller
/// has already rejected a non-finite value, and a descriptor's minimum is never
/// above its maximum.
fn snap_output(descriptor: &SettingDescriptor, value: f64) -> f64 {
    let value = value.clamp(descriptor.minimum as f64, descriptor.maximum as f64);
    if descriptor.precision == 0 {
        value.round()
    } else {
        value
    }
}

/// The index of the route for `parameter` within a scene
/// (`route_editor_table_find`, `route_editor_state.c:245-260`).
fn table_find(table: &RouteTable, scene: SceneId, parameter: &str) -> Option<usize> {
    table
        .scene(scene)
        .items()
        .iter()
        .position(|route| route.parameter == parameter)
}

impl RouteEditorState {
    /// A closed editor (`route_editor_init`, `route_editor_state.c:42-46`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a draft for one setting of one track's scene
    /// (`route_editor_state.c:48-89`).
    ///
    /// With `existing == None` the draft starts as a full-range RMS route (input
    /// 0..1, output spanning the descriptor range, linear, clamped) so the first
    /// Apply is visible. Pulse Field's petal fold is the one semantic exception:
    /// its legacy zero already means `Balance -> 3..9`, so the fresh draft exposes
    /// that exact hidden rule instead of proposing RMS -> 0..12. A `Some` route
    /// must be valid for exactly this setting and becomes both the draft and the
    /// committed snapshot. Opening replaces any previous draft; a *failed* open
    /// leaves the previous draft alone, as in C, where the guards run before the
    /// struct is zeroed.
    ///
    /// The descriptor is looked up on demand rather than stored, because in C the
    /// descriptor tables were static data inside the hot-reloaded module and a
    /// cached pointer would dangle across a reload while the indices stayed stable
    /// (`route_editor_state.c:9-17`). Here it is simply cheap and always current.
    pub fn open(
        &mut self,
        track_slot: usize,
        scene: SceneId,
        setting_index: usize,
        existing: Option<&ParameterMapping>,
    ) -> bool {
        let Some(descriptor) = settings::descriptor(scene, setting_index) else {
            return false;
        };
        if let Some(existing) = existing {
            if !existing.is_valid_for(scene) || existing.parameter != descriptor.key {
                return false;
            }
            self.session = Some(RouteEditorDraft {
                track_slot,
                scene,
                setting_index,
                committed: Some(existing.clone()),
                draft: existing.clone(),
                touched: false,
            });
            return true;
        }
        // C had to guard against the descriptor key overflowing the mapping's
        // fixed-size `parameter` buffer; a `String` removes that failure mode.
        let pulse_fold = scene == SceneId::PulseField && descriptor.key == "settings.pulse.petals";
        self.session = Some(RouteEditorDraft {
            track_slot,
            scene,
            setting_index,
            committed: None,
            draft: ParameterMapping {
                parameter: descriptor.key.to_string(),
                source: if pulse_fold {
                    AnalysisSource::Balance
                } else {
                    AnalysisSource::Rms
                },
                band_index: 0,
                input_min: 0.0,
                input_max: 1.0,
                // Zero is Pulse Field's legacy shorthand for its hidden auto
                // formula, not a useful routed petal count. A fresh route makes
                // that formula explicit and editable without passing through 0.
                output_min: if pulse_fold {
                    3.0
                } else {
                    descriptor.minimum as f64
                },
                output_max: if pulse_fold {
                    9.0
                } else {
                    descriptor.maximum as f64
                },
                interpolation: Interpolation::Linear,
                clamp: true,
            },
            touched: false,
        });
        true
    }

    /// Restores an in-flight editor draft from an app-owned recovery snapshot.
    /// The same validation as [`Self::open`] is repeated for both copies so a
    /// malformed recovery file cannot construct a state the live editor could
    /// never reach.
    pub fn restore_session(&mut self, recovered: RouteEditorDraft) -> bool {
        let Some(descriptor) = settings::descriptor(recovered.scene, recovered.setting_index)
        else {
            return false;
        };
        if recovered.draft.parameter != descriptor.key
            || !recovered.draft.is_valid_for(recovered.scene)
            || recovered.committed.as_ref().is_some_and(|committed| {
                committed.parameter != descriptor.key || !committed.is_valid_for(recovered.scene)
            })
        {
            return false;
        }
        self.session = Some(recovered);
        true
    }

    /// Discards the draft (`route_editor_close`, `route_editor_state.c:91-94`).
    pub fn close(&mut self) {
        self.session = None;
    }

    /// Whether a draft is open (`route_editor_state.c:96-99`).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// The open draft and its context, or `None` when closed.
    #[must_use]
    pub fn session(&self) -> Option<&RouteEditorDraft> {
        self.session.as_ref()
    }

    /// The route being authored, or `None` when closed.
    #[must_use]
    pub fn draft(&self) -> Option<&ParameterMapping> {
        self.session.as_ref().map(|session| &session.draft)
    }

    /// Whether the open draft belongs to this exact setting
    /// (`route_editor_state.c:101-106`).
    ///
    /// The inspector uses this to decide which row hosts the editor.
    #[must_use]
    pub fn targets(&self, track_slot: usize, scene: SceneId, setting_index: usize) -> bool {
        self.matches_context(track_slot, scene)
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.setting_index == setting_index)
    }

    /// Whether the open draft belongs to this track and scene
    /// (`route_editor_state.c:108-114`).
    ///
    /// This is what decides whether the draft survives a scene or track switch out
    /// of sight.
    #[must_use]
    pub fn matches_context(&self, track_slot: usize, scene: SceneId) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.track_slot == track_slot && session.scene == scene)
    }

    /// Selects the analysis source (`route_editor_state.c:116-126`).
    ///
    /// Switching away from [`AnalysisSource::Band`] resets `band_index` to zero,
    /// which is a schema rule rather than tidiness: a non-band route with a
    /// non-zero band index is invalid.
    ///
    /// C also rejected an out-of-range enum value; `AnalysisSource` cannot hold
    /// one.
    pub fn set_source(&mut self, source: AnalysisSource) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.draft.source == source {
            return true;
        }
        session.draft.source = source;
        if source != AnalysisSource::Band {
            session.draft.band_index = 0;
        }
        session.touched = true;
        true
    }

    /// Steps the selected band (`route_editor_state.c:128-145`).
    ///
    /// Only meaningful for the band source, so it refuses otherwise. `band_limit`
    /// is the live analyzer band count, itself capped at
    /// [`crate::audio::analyzer::MAX_BANDS`], so a stale larger limit cannot author
    /// a band the analyzer will never produce.
    pub fn step_band(&mut self, delta: i32, band_limit: usize) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.draft.source != AnalysisSource::Band || band_limit == 0 {
            return false;
        }
        let limit = band_limit.min(crate::audio::analyzer::MAX_BANDS);
        let mut band = i64::from(session.draft.band_index) + i64::from(delta);
        if band < 0 {
            band = 0;
        }
        if band > (limit - 1) as i64 {
            band = (limit - 1) as i64;
        }
        let band = band as u16;
        if band == session.draft.band_index {
            return true;
        }
        session.draft.band_index = band;
        session.touched = true;
        true
    }

    /// Moves the input window's lower endpoint (`route_editor_state.c:147-157`).
    ///
    /// Stays inside 0..1 and maintains [`ROUTE_EDITOR_INPUT_GAP`] below
    /// `input_max`, so dragging the endpoints past each other narrows the window
    /// rather than inverting or collapsing it.
    pub fn set_input_min(&mut self, mut value: f64) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        if value < 0.0 {
            value = 0.0;
        }
        let ceiling = session.draft.input_max - ROUTE_EDITOR_INPUT_GAP;
        if value > ceiling {
            value = ceiling;
        }
        if value == session.draft.input_min {
            return true;
        }
        session.draft.input_min = value;
        session.touched = true;
        true
    }

    /// Moves the input window's upper endpoint (`route_editor_state.c:159-169`).
    pub fn set_input_max(&mut self, mut value: f64) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        if value > 1.0 {
            value = 1.0;
        }
        let floor_value = session.draft.input_min + ROUTE_EDITOR_INPUT_GAP;
        if value < floor_value {
            value = floor_value;
        }
        if value == session.draft.input_max {
            return true;
        }
        session.draft.input_max = value;
        session.touched = true;
        true
    }

    /// Sets the value produced at `input_min` (`route_editor_state.c:171-180`).
    ///
    /// Clamped to the descriptor range and rounded for precision-0 settings, so the
    /// endpoints read like the slider the route replaces. `low > high` is legal and
    /// means an inverted response.
    pub fn set_output_low(&mut self, value: f64) -> bool {
        let Some((session, descriptor)) = self.session_with_descriptor() else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        let value = snap_output(descriptor, value);
        if value == session.draft.output_min {
            return true;
        }
        session.draft.output_min = value;
        session.touched = true;
        true
    }

    /// Sets the value produced at `input_max` (`route_editor_state.c:182-191`).
    pub fn set_output_high(&mut self, value: f64) -> bool {
        let Some((session, descriptor)) = self.session_with_descriptor() else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        let value = snap_output(descriptor, value);
        if value == session.draft.output_max {
            return true;
        }
        session.draft.output_max = value;
        session.touched = true;
        true
    }

    /// Flips the response direction (`route_editor_state.c:193-202`).
    ///
    /// A no-op — reported as success — when the endpoints are already equal, since
    /// there is no direction to flip.
    pub fn swap_output(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.draft.output_min == session.draft.output_max {
            return true;
        }
        std::mem::swap(&mut session.draft.output_min, &mut session.draft.output_max);
        session.touched = true;
        true
    }

    /// Selects the curve (`route_editor_state.c:204-213`).
    ///
    /// C also rejected an out-of-range enum value; `Interpolation` cannot hold one.
    pub fn set_curve(&mut self, curve: Interpolation) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.draft.interpolation == curve {
            return true;
        }
        session.draft.interpolation = curve;
        session.touched = true;
        true
    }

    /// Turns output clamping on or off (`route_editor_state.c:215-222`).
    pub fn set_clamp(&mut self, clamp: bool) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if session.draft.clamp == clamp {
            return true;
        }
        session.draft.clamp = clamp;
        session.touched = true;
        true
    }

    /// The guard predicate: an open draft that differs from its committed route, or
    /// a fresh draft the user actually edited (`route_editor_state.c:224-231`).
    ///
    /// `touched` is only set when a field actually changed, which is what keeps a
    /// freshly opened, untouched draft from tripping the dirty guard on close.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        match session.committed.as_ref() {
            Some(committed) => session.draft != *committed,
            None => session.touched,
        }
    }

    /// Track-scoped dirtiness, used by save/autosave and context-change policy
    /// (`route_editor_state.c:233-237`).
    ///
    /// A hidden draft for another track must not make the active track claim its own
    /// file is dirty, while the owning track must never be reported as fully saved.
    #[must_use]
    pub fn is_dirty_for_track(&self, track_slot: usize) -> bool {
        self.is_dirty()
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.track_slot == track_slot)
    }

    /// Whether the draft is a well-formed route for its scene
    /// (`route_editor_state.c:239-243`).
    #[must_use]
    pub fn can_apply(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.draft.is_valid_for(session.scene))
    }

    /// Commits the draft into the table, replacing a committed route for the same
    /// parameter in place (`route_editor_state.c:262-280`).
    ///
    /// On success the draft becomes the committed snapshot and the editor stays
    /// open, clean. False — with state and table unchanged — when the draft is
    /// invalid or a new route no longer fits the scene's capacity.
    ///
    /// The remove-then-add order is deliberate: freeing the parameter's own slot
    /// first means the re-add can only fail on programmer error, never on capacity.
    pub fn apply(&mut self, table: &mut RouteTable) -> bool {
        if !self.can_apply() {
            return false;
        }
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let scene = session.scene;
        if let Some(index) = table_find(table, scene, &session.draft.parameter) {
            if !table.remove(scene, index) {
                return false;
            }
        }
        if table.add(scene, session.draft.clone()).is_err() {
            return false;
        }
        session.committed = Some(session.draft.clone());
        session.touched = false;
        true
    }

    /// Deletes the committed route from the table and closes the editor, returning
    /// the setting to its slider (`route_editor_state.c:282-294`).
    ///
    /// False when nothing was removed — including for a fresh draft that never
    /// committed, which has nothing in the table to delete.
    pub fn remove(&mut self, table: &mut RouteTable) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let Some(committed) = session.committed.as_ref() else {
            return false;
        };
        let scene = session.scene;
        let Some(index) = table_find(table, scene, &committed.parameter) else {
            return false;
        };
        if !table.remove(scene, index) {
            return false;
        }
        self.close();
        true
    }

    /// The open draft together with its descriptor, mirroring C's
    /// `route_editor_descriptor` guard (`route_editor_state.c:9-17`).
    fn session_with_descriptor(
        &mut self,
    ) -> Option<(&mut RouteEditorDraft, &'static SettingDescriptor)> {
        let session = self.session.as_mut()?;
        let descriptor = settings::descriptor(session.scene, session.setting_index)?;
        Some((session, descriptor))
    }
}

/// The committed route driving one setting, or `None` when the setting is a plain
/// slider (`route_editor_state.c:296-309`).
///
/// Powers the per-row routed affordance.
#[must_use]
pub fn find_route(
    table: &RouteTable,
    scene: SceneId,
    setting_index: usize,
) -> Option<&ParameterMapping> {
    let descriptor = settings::descriptor(scene, setting_index)?;
    let index = table_find(table, scene, descriptor.key)?;
    table.scene(scene).items().get(index)
}

/// The source's short label (`route_editor_state.c:311-321`).
#[must_use]
pub fn source_label(source: AnalysisSource) -> &'static str {
    match source {
        AnalysisSource::Rms => "RMS",
        AnalysisSource::Peak => "Peak",
        AnalysisSource::SpectralFlux => "Flux",
        AnalysisSource::BeatPhase => "Beat",
        AnalysisSource::Band => "Band",
        AnalysisSource::Time => "Time",
        AnalysisSource::Bass => "Bass",
        AnalysisSource::Mids => "Mids",
        AnalysisSource::Treble => "Treble",
        AnalysisSource::Balance => "Balance",
    }
}

/// The name of one of a route's two anchors (`route_editor_state.c:323-333`).
///
/// A route is authored as two (source -> output) anchors. Anchor names follow the
/// source so editor rows read as sentences; a generic "low/high" would be wrong for
/// beat phase, whose axis is time inside the beat, not loudness
/// (`route_editor_state.h:103-106`).
#[must_use]
pub fn anchor_label(source: AnalysisSource, high: bool) -> &'static str {
    match source {
        AnalysisSource::Rms
        | AnalysisSource::Peak
        | AnalysisSource::Band
        | AnalysisSource::Bass
        | AnalysisSource::Mids
        | AnalysisSource::Treble => {
            if high {
                "Loud"
            } else {
                "Quiet"
            }
        }
        AnalysisSource::SpectralFlux => {
            if high {
                "Busy"
            } else {
                "Calm"
            }
        }
        AnalysisSource::BeatPhase => {
            if high {
                "Beat end"
            } else {
                "Beat start"
            }
        }
        // The Time triangle's axis is the eight-second cycle, not loudness: 0 at
        // the cycle boundaries, 1 at the midpoint.
        AnalysisSource::Time => {
            if high {
                "Cycle peak"
            } else {
                "Cycle low"
            }
        }
        AnalysisSource::Balance => {
            if high {
                "Treble-heavy"
            } else {
                "Bass-heavy"
            }
        }
    }
}

/// The curve's short label (`route_editor_state.c:335-345`).
#[must_use]
pub fn curve_label(curve: Interpolation) -> &'static str {
    match curve {
        Interpolation::Step => "Step",
        Interpolation::Linear => "Linear",
        Interpolation::Smoothstep => "Smooth",
        Interpolation::EaseIn => "Ease in",
        Interpolation::EaseOut => "Ease out",
    }
}

/// Compact row summary, e.g. `Band 2 · Smooth · 0.40 → 2.20`
/// (`route_editor_state.c:347-366`).
///
/// An ` · unclamped` suffix is appended when clamping is off. `precision` comes
/// from the setting descriptor so the values read like the slider they replace;
/// Rust's `{:.*}` is C's `%.*f`. The separators are U+00B7 and U+2192, as in the C
/// string literal's `\xC2\xB7` and `\xE2\x86\x92`.
#[must_use]
pub fn summary(route: &ParameterMapping, precision: u32) -> String {
    let source = if route.source == AnalysisSource::Band {
        format!("Band {}", route.band_index)
    } else {
        source_label(route.source).to_string()
    };
    let precision = precision as usize;
    format!(
        "{} · {} · {:.*} → {:.*}{}",
        source,
        curve_label(route.interpolation),
        precision,
        route.output_min,
        precision,
        route.output_max,
        if route.clamp { "" } else { " · unclamped" }
    )
}

/// Where the live source value sits inside the route's input range, 0..1, for the
/// meter strip (`route_editor_state.c:368-379`).
///
/// Zero when the value or the range is unusable. Stays `f32` because it is consumed
/// directly as a drawing fraction.
#[must_use]
pub fn meter_position(route: &ParameterMapping, source_value: f64) -> f32 {
    if !source_value.is_finite()
        || !route.input_min.is_finite()
        || !route.input_max.is_finite()
        // C wrote `!(input_max > input_min)`; with both known finite this is the
        // same test without the negated partial comparison.
        || route.input_max <= route.input_min
    {
        return 0.0;
    }
    let normalized = (source_value - route.input_min) / (route.input_max - route.input_min);
    normalized.clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::index;

    // C's headless tests could not include scene.h, so they mirrored the Scene_Id
    // ordering by hand. Here the real ids are usable: SceneId::SongAtlas == 4 and
    // SceneId::Loom == 8, the two the C test named.
    const TRACK_A: usize = 0;
    const TRACK_B: usize = 1;

    fn committed_band_route() -> ParameterMapping {
        ParameterMapping {
            parameter: "settings.loom.weight".to_string(),
            source: AnalysisSource::Band,
            band_index: 2,
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.4,
            output_max: 2.2,
            interpolation: Interpolation::Smoothstep,
            clamp: true,
        }
    }

    #[test]
    fn route_editor_open_seeds_full_range_draft() {
        let weight = index::loom::WEIGHT;
        let mut state = RouteEditorState::new();
        assert!(!state.is_open());

        assert!(state.open(TRACK_A, SceneId::Loom, weight, None));
        assert!(state.is_open());
        assert!(state.targets(TRACK_A, SceneId::Loom, weight));
        assert!(!state.targets(TRACK_B, SceneId::Loom, weight));
        assert!(!state.matches_context(TRACK_A, SceneId::SongAtlas));

        let draft = state.draft().unwrap();
        assert_eq!(draft.parameter, "settings.loom.weight");
        assert_eq!(draft.source, AnalysisSource::Rms);
        assert_eq!(draft.band_index, 0);
        assert!((draft.input_min - 0.0).abs() < 1e-9);
        assert!((draft.input_max - 1.0).abs() < 1e-9);
        // Fresh drafts span the descriptor range so the first Apply is visible.
        assert!((draft.output_min - 0.40).abs() < 1e-6);
        assert!((draft.output_max - 2.50).abs() < 1e-6);
        assert_eq!(draft.interpolation, Interpolation::Linear);
        assert!(draft.clamp);
        // Untouched fresh drafts do not guard and are already valid.
        assert!(!state.is_dirty());
        assert!(state.can_apply());
    }

    #[test]
    fn route_editor_open_rejects_bad_targets() {
        let mut state = RouteEditorState::new();
        // C also rejected `scene_index == COUNT_SCENES`; `SceneId` makes that
        // unrepresentable, which is the point of using it here.
        assert!(!state.open(TRACK_A, SceneId::Loom, settings::MAX_CONTROLS, None));

        // An existing route must belong to exactly the opened setting.
        let route = committed_band_route();
        let density = index::loom::DENSITY;
        assert!(!state.open(TRACK_A, SceneId::Loom, density, Some(&route)));
        let mut poisoned = committed_band_route();
        poisoned.input_max = poisoned.input_min;
        let weight = index::loom::WEIGHT;
        assert!(!state.open(TRACK_A, SceneId::Loom, weight, Some(&poisoned)));
        assert!(!state.is_open());
    }

    #[test]
    fn pulse_fold_opens_as_its_existing_balance_auto_mapping() {
        let mut state = RouteEditorState::new();
        assert!(state.open(TRACK_A, SceneId::PulseField, index::pulse::PETALS, None));
        let draft = state.draft().unwrap();
        assert_eq!(draft.source, AnalysisSource::Balance);
        assert_eq!(draft.output_min, 3.0);
        assert_eq!(draft.output_max, 9.0);
        assert_eq!(draft.interpolation, Interpolation::Linear);
        assert!(state.can_apply());
    }

    #[test]
    fn route_editor_open_with_existing_route_starts_clean() {
        let weight = index::loom::WEIGHT;
        let route = committed_band_route();
        let mut state = RouteEditorState::new();
        assert!(state.open(TRACK_A, SceneId::Loom, weight, Some(&route)));
        assert!(state.session().unwrap().committed.is_some());
        assert!(!state.is_dirty());

        // Editing dirties; restoring the committed value cleans again.
        assert!(state.set_curve(Interpolation::Linear));
        assert!(state.is_dirty());
        assert!(state.set_curve(Interpolation::Smoothstep));
        assert!(!state.is_dirty());
        // No-change writes never dirty.
        assert!(state.set_clamp(true));
        assert!(state.set_source(AnalysisSource::Band));
        assert!(!state.is_dirty());
    }

    #[test]
    fn route_editor_setters_keep_draft_valid() {
        let weight = index::loom::WEIGHT;
        let mut state = RouteEditorState::new();
        assert!(state.open(TRACK_A, SceneId::Loom, weight, None));

        // The input window keeps a minimum width while endpoints drag past
        // each other, and stays inside 0..1.
        assert!(state.set_input_max(0.30));
        assert!(state.set_input_min(0.90));
        assert!((state.draft().unwrap().input_min - (0.30 - ROUTE_EDITOR_INPUT_GAP)).abs() < 1e-9);
        assert!(state.set_input_min(-4.0));
        assert!((state.draft().unwrap().input_min - 0.0).abs() < 1e-9);
        assert!(state.set_input_max(7.0));
        assert!((state.draft().unwrap().input_max - 1.0).abs() < 1e-9);
        assert!(!state.set_input_min(f64::NAN));
        assert!(!state.set_input_max(f64::INFINITY));

        // Output endpoints clamp to the descriptor range; inversion is legal.
        assert!(state.set_output_low(99.0));
        assert!((state.draft().unwrap().output_min - 2.50).abs() < 1e-6);
        assert!(state.set_output_high(-99.0));
        assert!((state.draft().unwrap().output_max - 0.40).abs() < 1e-6);
        assert!(state.can_apply());
        assert!(state.swap_output());
        assert!((state.draft().unwrap().output_min - 0.40).abs() < 1e-6);
        assert!((state.draft().unwrap().output_max - 2.50).abs() < 1e-6);

        // Equal endpoints are a slider constant in .musi v1, not a route. Keep
        // Apply disabled instead of letting authoring identity disappear on open.
        let low = state.draft().unwrap().output_min;
        assert!(state.set_output_high(low));
        assert!(!state.can_apply());
        assert!(state.set_output_high(2.50));
        assert!(state.can_apply());

        // C also rejected out-of-range `Musi_Interpolation` and
        // `Musi_Analysis_Source` values here; Rust's enums cannot hold one, so the
        // misuse branch is gone rather than merely untested.
        assert!(state.can_apply());

        // A closed editor accepts no edits.
        state.close();
        assert!(!state.set_input_min(0.2));
        assert!(!state.is_dirty());
    }

    #[test]
    fn route_editor_integer_settings_snap_outputs() {
        let color = index::atlas::COLOR;
        let mut state = RouteEditorState::new();
        assert!(state.open(TRACK_A, SceneId::SongAtlas, color, None));
        assert!(state.set_output_low(-42.4));
        assert!((state.draft().unwrap().output_min - (-42.0)).abs() < 1e-9);
        assert!(state.set_output_high(260.0));
        assert!((state.draft().unwrap().output_max - 180.0).abs() < 1e-9);
    }

    #[test]
    fn route_editor_band_stepping_respects_source_and_limit() {
        let weight = index::loom::WEIGHT;
        let mut state = RouteEditorState::new();
        assert!(state.open(TRACK_A, SceneId::Loom, weight, None));

        // Band stepping is only meaningful for the band source.
        assert!(!state.step_band(1, 24));
        assert!(state.set_source(AnalysisSource::Band));
        assert!(state.step_band(5, 24));
        assert_eq!(state.draft().unwrap().band_index, 5);
        assert!(state.step_band(100, 24));
        assert_eq!(state.draft().unwrap().band_index, 23);
        assert!(state.step_band(-100, 24));
        assert_eq!(state.draft().unwrap().band_index, 0);
        assert!(!state.step_band(1, 0));

        // Leaving the band source drops the index (schema: zero unless band).
        assert!(state.step_band(3, 24));
        assert!(state.set_source(AnalysisSource::Peak));
        assert_eq!(state.draft().unwrap().band_index, 0);
        assert!(state.can_apply());
    }

    #[test]
    fn route_editor_apply_commits_and_replaces_in_table() {
        let weight = index::loom::WEIGHT;
        let mut table = RouteTable::new();
        let mut state = RouteEditorState::new();
        assert!(state.open(TRACK_A, SceneId::Loom, weight, None));
        assert!(state.set_source(AnalysisSource::Band));
        assert!(state.step_band(2, 24));
        assert!(state.is_dirty());
        assert!(state.is_dirty_for_track(TRACK_A));
        assert!(!state.is_dirty_for_track(TRACK_B));

        assert!(state.apply(&mut table));
        assert_eq!(table.scene(SceneId::Loom).len(), 1);
        assert!(state.is_open());
        assert!(state.session().unwrap().committed.is_some());
        assert!(!state.is_dirty());
        assert!(!state.is_dirty_for_track(TRACK_A));
        let routed = find_route(&table, SceneId::Loom, weight);
        assert!(routed.is_some());
        assert_eq!(routed.unwrap().band_index, 2);

        // Re-applying an edited draft replaces the parameter's slot in place.
        assert!(state.set_curve(Interpolation::EaseOut));
        assert!(state.apply(&mut table));
        assert_eq!(table.scene(SceneId::Loom).len(), 1);
        let routed = find_route(&table, SceneId::Loom, weight);
        assert!(routed.is_some());
        assert_eq!(routed.unwrap().interpolation, Interpolation::EaseOut);
        // C asserted `scene_route_table_valid(&table)`; the Rust table has no
        // whole-table predicate, so check what it meant: every route still valid
        // for the scene that holds it.
        assert!(table
            .scene(SceneId::Loom)
            .items()
            .iter()
            .all(|route| route.is_valid_for(SceneId::Loom)));

        let density = index::loom::DENSITY;
        assert!(find_route(&table, SceneId::Loom, density).is_none());
    }

    #[test]
    fn route_editor_remove_deletes_route_and_closes() {
        let weight = index::loom::WEIGHT;
        let mut table = RouteTable::new();
        let route = committed_band_route();
        assert!(table.add(SceneId::Loom, route.clone()).is_ok());

        let mut state = RouteEditorState::new();
        // A fresh draft that never committed has nothing to remove.
        assert!(state.open(TRACK_A, SceneId::Loom, weight, None));
        assert!(!state.remove(&mut table));
        assert_eq!(table.scene(SceneId::Loom).len(), 1);

        assert!(state.open(TRACK_A, SceneId::Loom, weight, Some(&route)));
        assert!(state.remove(&mut table));
        assert_eq!(table.scene(SceneId::Loom).len(), 0);
        assert!(!state.is_open());
        assert!(find_route(&table, SceneId::Loom, weight).is_none());
    }

    #[test]
    fn route_editor_anchor_labels_follow_the_source() {
        // Loudness sources use loudness words; flux and beat phase measure
        // different axes and must not claim "quiet" or "loud".
        assert_eq!(anchor_label(AnalysisSource::Rms, false), "Quiet");
        assert_eq!(anchor_label(AnalysisSource::Rms, true), "Loud");
        assert_eq!(anchor_label(AnalysisSource::Peak, false), "Quiet");
        assert_eq!(anchor_label(AnalysisSource::Band, true), "Loud");
        assert_eq!(anchor_label(AnalysisSource::SpectralFlux, false), "Calm");
        assert_eq!(anchor_label(AnalysisSource::SpectralFlux, true), "Busy");
        assert_eq!(anchor_label(AnalysisSource::BeatPhase, false), "Beat start");
        assert_eq!(anchor_label(AnalysisSource::BeatPhase, true), "Beat end");
        assert_eq!(anchor_label(AnalysisSource::Bass, false), "Quiet");
        assert_eq!(anchor_label(AnalysisSource::Treble, true), "Loud");
        assert_eq!(anchor_label(AnalysisSource::Balance, false), "Bass-heavy");
        assert_eq!(anchor_label(AnalysisSource::Balance, true), "Treble-heavy");
    }

    #[test]
    fn route_editor_summary_and_meter_read_back() {
        let mut route = committed_band_route();
        assert_eq!(summary(&route, 2), "Band 2 · Smooth · 0.40 → 2.20");
        route.clamp = false;
        route.source = AnalysisSource::BeatPhase;
        route.band_index = 0;
        assert_eq!(
            summary(&route, 2),
            "Beat · Smooth · 0.40 → 2.20 · unclamped"
        );
        // C also handled a NULL route by emptying the buffer; `&ParameterMapping`
        // cannot be null.

        route.input_min = 0.2;
        route.input_max = 0.7;
        assert!((meter_position(&route, 0.45) - 0.5).abs() < 1e-6);
        assert!((meter_position(&route, -3.0) - 0.0).abs() < 1e-6);
        assert!((meter_position(&route, 9.0) - 1.0).abs() < 1e-6);
        assert!((meter_position(&route, f64::NAN) - 0.0).abs() < 1e-6);
    }
}
