//! The three-pane lyrics editor: cue list, cue form, caption typography.
//!
//! **Owner: Agent I.** C sources: `lyrics_editor_ui.c`, `lyrics_editor_layout.c`,
//! `lyric_lane_edit.c` — the last two were already ported, along with
//! `caption_layout` and the caption face `runtime::font` loads. This file is the
//! first one.
//!
//! Four panes share the panel, exactly as the oracle's do, and never two at once
//! (`lyrics_editor_ui.c:1142-1185`): the cue list beside the editing form, the
//! caption style form across the whole panel, and [`super::fonts`]'s browser —
//! **Agent K's**, called from here because the oracle drives it with
//! `p->lyric_editor.font_pane` (`plug.c:3786`) rather than as a panel of its own.
//!
//! # The two structural divergences, and why
//!
//! **The cue lane sits at the top of this panel, not inside the always-visible
//! lane stack yet.** It shares `timed_lane` geometry with the scene-plan lane and
//! reads the same `Shell::timeline`, so blocks, ticks and playhead stay aligned.
//! Its rectangle remains disjoint from the waveform scrubber; the shell-level
//! timeline gesture owner introduced for scene boundaries is the seam for moving
//! this lane into the common stack when the always-visible lyric lane lands.
//!
//! **Every edit leaves as a [`LyricsEdit`], not as a mutation.** The panel is
//! handed `&Workspace`, so it could not write to the document if it wanted to.
//! That is the same shape as [`ShellCommand`] and for the same reason —
//! `main.rs` owns the model — but it travels in the editor rather than in the
//! command vector because these are a panel's private vocabulary and adding six
//! variants to a shared enum is a merge conflict in the file every agent touches.

use std::path::PathBuf;

use musializer_core::project::editor_draft::{LyricDraftState, LyricDraftValues};
use musializer_core::project::lyrics::{self, LyricCue, LyricsDocument, LyricsError};
use musializer_core::project::model::{
    caption, caption_fx, CaptionAnchor, CaptionBox, CaptionFace, CaptionStyle, EffectDrive,
};
use musializer_core::ui::lyric_lane_edit::{
    self, LyricLaneClick, LyricLaneSelection, LyricLaneZone,
};
use musializer_core::ui::lyrics_editor_layout::{self, LYRIC_EDITOR_ROW_HEIGHT};
use musializer_core::ui::notice::Severity;
use musializer_core::ui::text_edit::{TextEditError, TextRules};
use musializer_core::ui::timed_lane;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::{GlyphRepertoire, UiFonts};
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::super::shell::{draw_timeline_playhead, Shell, ShellCommand, ShellInput};
use super::super::shell_layout::DEFAULT_TIMELINE_HEIGHT;
use super::super::text_input::TextField;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};
use crate::cli::{CaptionPickerProbe, UiProbe};
use crate::workspace::{Track, Workspace};

/// The cue lane's height, and the gap between it and the editor below.
///
/// 22 px is the oracle's lane: `plug.c` caps it there, which is also why its
/// scene names — gated on a lane at least 28 px high — never appeared at all
/// (`lyrics_editor_ui.c:1060-1066`).
pub const LANE_HEIGHT: f32 = 22.0;
/// 5 px, and the 5 is not a taste decision — see [`CHROME_ABOVE_PANEL`].
const LANE_GAP: f32 = 5.0;

/// Everything the timeline band spends above this panel: its own 27 px header,
/// 10 px of padding, the 56 px strip, and the 28 px the zoom readout occupies
/// below it.
///
/// Derived from `Shell::timeline_strip`, not guessed. If that arithmetic
/// changes, [`LyricEditor::timeline_height`] is where it shows up — as a panel
/// too short for its form, which the tests at the bottom of this file assert
/// against directly.
const CHROME_ABOVE_PANEL: f32 = 121.0;
/// The panel's own bottom margin inside the band.
const PANEL_BOTTOM_PADDING: f32 = 10.0;

/// This shell's chrome around the panel is **exactly** the oracle's
/// `LYRIC_EDITOR_TIMELINE_CHROME` (`lyrics_editor_layout.h:35-37`), which is what
/// lets [`lyrics_editor_layout::panel_height`]'s sidebar protection be used
/// unchanged: that function subtracts 158 from the window before deciding how
/// much the panel may take, so a shell spending more than 158 quietly breaks the
/// guarantee.
///
/// It did, by one pixel, and a capture is what showed it: at 1280x720 with six
/// cues the tracks panel vanished entirely, because the sidebar came out at 300
/// against a 301 floor. Hence a lane gap of 5 rather than 6, and hence this
/// assertion rather than a comment asking the next reader to add up four
/// constants.
const _: () = assert!(
    CHROME_ABOVE_PANEL + PANEL_BOTTOM_PADDING + LANE_HEIGHT + LANE_GAP
        == lyrics_editor_layout::LYRIC_EDITOR_TIMELINE_CHROME
);

/// Smallest editor that can host its own first row of controls.
///
/// `workspace_layout.h:7-19`'s rule applied to this panel: a surface that cannot
/// contain its own controls is not drawn at all, because the widgets would claim
/// presses aimed at whatever is painted over them and — as a capture of this very
/// panel showed — draw past the bottom of the framebuffer besides.
///
/// The number is the arithmetic below, not a guess: the form starts 10 px inside
/// the editor, its action row starts 3 px above that and is 34 px tall, the cue
/// list's header sits at the same 10 px with its first row 28 px below, and a row
/// is [`LYRIC_EDITOR_ROW_HEIGHT`]. `10 + 28 + 36 + 8` is the shortest editor with
/// a header, one row and a margin.
const EDITOR_MINIMUM_HEIGHT: f32 = 82.0;

/// Smallest pane the caption controls fit in, and the width its two columns need
/// (`CAPTION_STYLE_FORM_HEIGHT`/`:838`, `lyrics_editor_ui.c:454`).
///
/// **The height is [`CaptionFormLayout::EXTENT`], not the oracle's 152.** At 152
/// the form drew, and then the "Import a face..." button — placed 158 px down —
/// was skipped by a `fits inside the form?` guard, so at every form height in
/// `[152, 184)` the only route to an imported caption face silently did not
/// exist. The real layout produces exactly one such height and it is the one a
/// 960x640 window gets, which is how the operator found it and no test did.
/// A control the form draws is now part of the number the form is measured by.
const CAPTION_FORM_HEIGHT: f32 = CaptionFormLayout::EXTENT;
const CAPTION_FORM_WIDTH: f32 = 520.0;

/// What the shortest panel the layout can produce leaves for the caption form.
///
/// `lyrics_editor_layout::panel_height` never returns less than
/// `LYRIC_EDITOR_FORM_MINIMUM`, and [`Shell::caption_pane`] spends
/// `UI_PANEL_PADDING` twice plus the 38 px header row before handing the rest to
/// the form. Derived rather than measured, so raising the form's own extent past
/// what the panel can ever supply fails to compile instead of reintroducing the
/// "Enlarge the window" message at the minimum supported size.
const CAPTION_FORM_AVAILABLE: f32 =
    lyrics_editor_layout::LYRIC_EDITOR_FORM_MINIMUM - metric::UI_PANEL_PADDING * 2.0 - 38.0;

const _: () = assert!(CAPTION_FORM_HEIGHT <= CAPTION_FORM_AVAILABLE);

/// Widget id namespaces for this panel.
///
/// Local constants rather than entries in [`widgets::id`], which is a shared file
/// this fan-out does not edit. They sit outside the range that module uses
/// (1..=6); the request to fold them in is in Agent I's report.
mod ns {
    /// The pane toggles and the document buttons.
    pub const ACTIONS: u32 = 16;
    /// The editing form: the two time rows and Apply/Discard/Delete.
    pub const FORM: u32 = 17;
    /// The caption pane's choices, anchor grid, swatches and face buttons.
    pub const CAPTION: u32 = 18;
    /// One per cue, keyed by cue id, so a row keeps its id across a scroll.
    pub const ROWS: u32 = 19;
}

/// Indices inside [`ns::FORM`].
///
/// These are deliberately separated by row. Delete used to be index 2 while
/// the START row also began at index 2; the START decrement button therefore
/// consumed Delete's release before Delete was drawn, making the visible
/// control impossible to click.
mod form_id {
    pub const APPLY: u32 = 0;
    pub const DISCARD: u32 = 1;
    pub const DELETE: u32 = 2;
    pub const START_TIME: u32 = 10;
    pub const END_TIME: u32 = 20;
}

/// Indices inside [`ns::CAPTION`], separated by control group for the same reason
/// [`form_id`]'s are separated by row.
///
/// Each group leaves room for one more entry than it uses: the INK row grew a
/// seventh cell when free colour picking landed, and had the swatch base been
/// packed tight against the next group that cell would have inherited the anchor
/// grid's id and released its press.
mod caption_id {
    /// Face choice: three cells, since a project may carry an imported family.
    pub const FACE: u32 = 0;
    /// Backing choice: three cells.
    pub const BACKING: u32 = 8;
    /// The 3x3 anchor grid.
    pub const ANCHOR: u32 = 16;
    pub const IMPORT_FACE: u32 = 32;
    pub const REMOVE_FACE: u32 = 33;
    /// The INK row: one per swatch, then the custom cell.
    pub const INK_SWATCHES: u32 = 40;
    /// The PLATE row, the same shape.
    pub const PLATE_SWATCHES: u32 = 48;
    /// The free picker's Done button.
    pub const PICKER_DONE: u32 = 56;
    /// The effects form's PULSE drive row: six cells.
    pub const PULSE_DRIVE: u32 = 64;
    /// The effects form's HUE drive row: six cells.
    pub const HUE_DRIVE: u32 = 72;
    /// The glow colour cell on the effects form.
    pub const GLOW_COLOR: u32 = 80;
    /// The Style/Effects toggle in the caption pane header.
    pub const EFFECTS_TOGGLE: u32 = 88;
}

/// The amber a lyric block is drawn in (`lyrics_editor_ui.c:998`).
///
/// Not in `theme::rgba` because that is a shared file. It is a block fill rather
/// than text, so nothing in the contrast suite would have an opinion about it;
/// the request to move it there is in the report.
const LYRIC_BLOCK: Color = Color::new(242, 190, 66, 255);

fn lyric_lane_hover_allowed(lane: UiRect, pointer: Vector2, drag_active: bool) -> bool {
    !drag_active && lane.contains_point(pointer.x, pointer.y)
}

/// The pale wash behind a selected row (`GetColor(0xE7ECFAFF)`, `:1238`).
const SELECTED_ROW: u32 = 0xE7EC_FAFF;

/// One change the editor wants made to the model.
///
/// Applied by `main.rs` through [`Self::apply`], which is why the variants carry
/// values rather than references: by the time it runs, the frame that produced it
/// is over.
#[derive(Clone, Debug, PartialEq)]
pub enum LyricsEdit {
    /// Apply, with a new cue (`lyrics_insert`, `lyrics_editor_ui.c:137-145`).
    Insert {
        start_seconds: f64,
        end_seconds: f64,
        text: String,
    },
    /// Apply, with an existing cue (`lyrics_update`, `:147-153`).
    Update {
        id: u64,
        start_seconds: f64,
        end_seconds: f64,
        text: String,
    },
    /// Delete (`:1412`).
    Delete { id: u64 },
    /// A committed body drag in the lane (`lyrics_shift_many`, `:323`).
    ShiftMany { ids: Vec<u64>, delta_seconds: f64 },
    /// A committed edge drag in the lane (`lyrics_retime`, `:337`).
    Retime {
        id: u64,
        start_seconds: f64,
        end_seconds: f64,
    },
    /// Any caption control (`draw_caption_style_form`'s `changed` flag, `:980`).
    ///
    /// Boxed because `CaptionStyle` is far larger than every other variant, and
    /// an enum sized by its rarest member would be carried by every lane drag.
    SetCaptionStyle(Box<CaptionStyle>),
}

impl LyricsEdit {
    /// Writes the change into a track, returning the id of a newly inserted cue.
    ///
    /// Called by `main.rs` on what [`LyricEditor::take_pending`] hands back —
    /// which is only ever the edits whose owning track is the one being written
    /// to (review 1.3).
    ///
    /// Every variant goes through the model's own validating entry point, so an
    /// edit the panel should not have offered fails here rather than corrupting
    /// the document. That is what makes it safe for the panel to be optimistic
    /// about geometry it clamped a frame ago.
    pub fn apply(self, track: &mut Track) -> Result<Option<u64>, LyricsError> {
        match self {
            LyricsEdit::Insert {
                start_seconds,
                end_seconds,
                text,
            } => track
                .lyrics
                .insert(LyricCue {
                    id: 0,
                    start_seconds,
                    end_seconds,
                    text,
                })
                .map(Some),
            LyricsEdit::Update {
                id,
                start_seconds,
                end_seconds,
                text,
            } => track
                .lyrics
                .update(id, start_seconds, end_seconds, &text)
                .map(|()| None),
            LyricsEdit::Delete { id } => track.lyrics.delete(id).map(|()| None),
            LyricsEdit::ShiftMany { ids, delta_seconds } => {
                track.lyrics.shift_many(&ids, delta_seconds).map(|()| None)
            }
            LyricsEdit::Retime {
                id,
                start_seconds,
                end_seconds,
            } => track
                .lyrics
                .retime(id, start_seconds, end_seconds)
                .map(|()| None),
            LyricsEdit::SetCaptionStyle(style) => {
                track.caption_style = *style;
                Ok(None)
            }
        }
    }
}

/// One deferred edit and the workspace slot it was authored against
/// (review 1.3).
///
/// The slot travels *with* the edit rather than being read at flush time,
/// because those are two different questions: the edit belongs to whichever
/// track the draft was bound to when the user clicked Apply, and the flush
/// happens against whichever track is current when `main.rs` drains. Cue ids
/// restart at 1 in every document, so with only the bare `id` those two tracks
/// are indistinguishable and one track's draft lands on another's cue.
#[derive(Clone, Debug, PartialEq)]
struct PendingLyricEdit {
    owner_slot: Option<usize>,
    edit: LyricsEdit,
}

/// What one drain yielded (review 1.3).
///
/// `refused` is a count rather than a discarded silence: an edit dropped because
/// its track went away is work the user believes they applied, and the tray has
/// to say so.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricEditDrain {
    pub edits: Vec<LyricsEdit>,
    pub refused: usize,
}

/// The lane gesture in flight (`Lyric_Editor`'s `lane_drag_*`,
/// `lyrics_editor_ui.h:45-57`).
///
/// The origin is kept in **seconds as well as pixels**: the pixel origin decides
/// a click from a drag, and the seconds origin survives a zoom or a pan
/// mid-gesture, which a pixel delta would not.
#[derive(Clone, Debug, Default, PartialEq)]
struct LaneDrag {
    active: bool,
    zone: Option<LyricLaneZone>,
    id: u64,
    origin_seconds: f64,
    origin_x: f32,
    delta_seconds: f64,
    edge_seconds: f64,
    moved: bool,
}

/// Which face served project-authored text last frame, and what it could not draw.
///
/// Recorded during the draw rather than derived in [`LyricEditor::describe`],
/// because the answer depends on which atlases rasterized — a question only the
/// frame knows and a headless report line has to carry. UX0-A05: the panel drew
/// Greek and Cyrillic lyrics as rows of `?` for as long as it went through the
/// chrome atlas, and no capture caught it because a row of question marks is a
/// perfectly plausible picture of a font that simply lacks a glyph.
///
/// `missing` is not a promise that every script works. Hebrew, Arabic and CJK
/// are outside [`musializer_core::project::caption_layout::FONT_CODEPOINT_RANGES`]
/// altogether — the bundled Alegreya has no glyphs for them, and requesting
/// blocks the face cannot draw would only trade one row of boxes for another.
/// Those need an imported face, which is a separate feature; what this number
/// says is that the *bundled* repertoire reached the editor intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AuthoredEvidence {
    /// Face name serving the cue list rows.
    rows: &'static str,
    /// Face name serving the editable text field.
    field: &'static str,
    /// Characters the serving faces cannot draw, over every cue in the document
    /// plus the draft in the field.
    missing: usize,
}

impl Default for AuthoredEvidence {
    fn default() -> Self {
        // Not `caption`/0: before the first draw nothing has been served, and a
        // report line that claims a glyph-complete face it never used is the
        // exact failure this field exists to make impossible.
        Self {
            rows: "none",
            field: "none",
            missing: 0,
        }
    }
}

/// Count what the serving faces cannot draw, over the whole document.
///
/// Pure, and separated from the draw so the claim is assertable without a GPU:
/// hand it the chrome repertoire and a Greek cue and it reports a non-zero
/// count; hand it the curated one and it reports zero. That pair is the test.
///
/// Every cue rather than the visible rows only. A scrolled-away line is still
/// text this document contains and this panel must be able to show, and a count
/// that changed with the scroll position would be evidence about the viewport
/// rather than about the face.
fn authored_evidence<'a>(
    rows: (&'static str, GlyphRepertoire),
    field: (&'static str, GlyphRepertoire),
    cues: impl IntoIterator<Item = &'a str>,
    draft: &str,
) -> AuthoredEvidence {
    let missing = cues
        .into_iter()
        .map(|text| rows.1.missing(text))
        .sum::<usize>()
        + field.1.missing(draft);
    AuthoredEvidence {
        rows: rows.0,
        field: field.0,
        missing,
    }
}

/// Draft state for the lyrics editor (`Lyric_Editor`, `lyrics_editor_ui.h:24-73`).
///
/// The canonical document lives on the [`Track`]; everything here is the current
/// interface draft. `text` is a whole [`TextField`] rather than the C's
/// `char[512]`, because a caret and a selection are state and the C has neither.
pub struct LyricEditor {
    /// The cue the form is bound to. `0` is the C's "nothing selected".
    pub selected_id: u64,
    /// A cue being composed, which has no canonical counterpart yet.
    pub draft_new: bool,
    draft_start: f64,
    draft_end: f64,
    text: TextField,
    list_first: usize,
    list_follow_selection: bool,
    /// Which editor the panel shows: the cue list and form, or caption
    /// typography. Never both — the style form needs two columns across the whole
    /// panel, and the cue list is not part of that decision.
    pub style_pane: bool,
    /// The font browser, a third whole-panel pane. Agent K's.
    pub font_pane: bool,
    /// The caption effects form, a sibling body inside the style pane
    /// (post-legacy, 2026-08-03). `font_pane` wins when both are set, matching
    /// the browser's precedence over the style form.
    pub effects_pane: bool,
    /// Which caption slider or picker surface a drag started on, `0` for none.
    /// Held rather than recomputed from the pointer so a drag that leaves the
    /// track keeps moving the control it started on. `1..=3` are the SIZE, WIDTH
    /// and INSET sliders; `4..=6` are the picker's square, hue bar and alpha
    /// bar; `10..=16` are the effects form's seven sliders.
    style_drag: u8,
    /// The free colour picker, closed unless a custom swatch opened it.
    picker: CaptionPicker,
    lane_selection: LyricLaneSelection,
    lane_drag: LaneDrag,
    /// Cue count as of the last frame, so [`Self::timeline_height`] can ask the
    /// oracle's `lyric_editor_panel_height` for the right number of rows without
    /// the shell reaching into the workspace.
    cue_count: usize,
    /// Which workspace slot the draft above belongs to (review 1.3).
    ///
    /// The house pattern is `RouteEditorState`'s `track_slot`
    /// (`route_editor_state.h:19-31`), and this editor is the one draft that did
    /// not have it: `selected_id` is a bare `u64`, ids restart at 1 in every
    /// document, and so a draft opened on track A addressed a real — different —
    /// cue on track B. It is also the same defect `Track::manual_clear` was made
    /// per-track to avoid, one lane over.
    ///
    /// `None` means no track has been entered yet, which is not the same as
    /// slot 0: an edit authored before any track was current must not apply to
    /// the first one that appears.
    owner_slot: Option<usize>,
    pending: Vec<PendingLyricEdit>,
    /// UX0-A05 evidence, written by [`Shell::cue_list`] every frame it draws.
    authored: AuthoredEvidence,
}

impl Default for LyricEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl LyricEditor {
    /// Whether the cue text field is taking keystrokes.
    ///
    /// Exposed so [`crate::ui::shell::Shell`] can stand its global shortcuts down
    /// while someone is typing. Without it, composing a lyric would toggle
    /// playback on every space, mute on every `m`, and scrub the track on every
    /// arrow — the shell reads those keys before this panel is drawn, so the panel
    /// cannot defend itself.
    ///
    /// A predicate rather than a public field: the shell has no business reaching
    /// into the editor's text state, and this is the only question it needs.
    #[must_use]
    pub fn is_typing(&self) -> bool {
        self.text.is_focused()
    }

    #[must_use]
    pub fn new() -> Self {
        Self {
            selected_id: 0,
            draft_new: false,
            draft_start: 0.0,
            draft_end: 0.0,
            text: TextField::new(TextRules::LYRIC_CUE),
            list_first: 0,
            // `lyric_editor_ui_init` sets exactly this one field
            // (`lyrics_editor_ui.c:51-56`).
            list_follow_selection: true,
            style_pane: false,
            font_pane: false,
            effects_pane: false,
            style_drag: 0,
            picker: CaptionPicker::default(),
            lane_selection: LyricLaneSelection::new(),
            lane_drag: LaneDrag::default(),
            cue_count: 0,
            owner_slot: None,
            pending: Vec::new(),
            authored: AuthoredEvidence::default(),
        }
    }

    /// The workspace slot this draft belongs to, or `None` before any track has
    /// been entered (review 1.3).
    #[must_use]
    pub fn draft_owner(&self) -> Option<usize> {
        self.owner_slot
    }

    /// Puts the editor in the state a user reaches by opening a cue on `slot`
    /// and typing one character into it.
    ///
    /// A hook rather than field access, so that `ui::shell`'s tests drive the
    /// same transition the panel does: `select_single` is where the cue id is
    /// copied out of the document, and a test that set the fields by hand would
    /// stop covering the thing that goes wrong.
    #[cfg(test)]
    pub(crate) fn open_dirty_draft_for_test(
        &mut self,
        slot: usize,
        document: &LyricsDocument,
        id: u64,
    ) {
        self.enter_track(Some(slot));
        self.select_single(document, id);
        self.text
            .edit
            .insert_char('!')
            .expect("a plain character is accepted by the cue rules");
        debug_assert!(self.has_unsaved_draft(document));
    }

    /// Clicks Apply on whatever the draft currently is, as the form's button
    /// does.
    #[cfg(test)]
    pub(crate) fn apply_draft_for_test(&mut self, track: &Track) {
        let edit = self.draft_edit().expect("the draft is applicable");
        apply_draft(self, track, edit);
    }

    /// Rebinds the editor to whichever track is now current, dropping a draft
    /// that belonged to another slot (review 1.3).
    ///
    /// The oracle does the same thing in the same place — `plug.c:5274` clears
    /// the draft immediately after a track switch is allowed, and `plug.c:5037`
    /// after a project opens. Doing it here as well as guarding the switch is
    /// what makes the stale binding unreachable rather than merely unlikely: a
    /// path that forgets the guard finds a cleared draft instead of one bound to
    /// a cue id that means something else now.
    ///
    /// Returns whether it dropped an unsaved *new* cue — the one draft that is
    /// dirty by rule (`editor_draft.c:5-15`) and therefore lost work no matter
    /// what the owning document held.
    pub fn enter_track(&mut self, slot: Option<usize>) -> bool {
        if self.owner_slot == slot {
            return false;
        }
        let lost = self.draft_new;
        self.owner_slot = slot;
        self.clear_draft();
        // The lane selection is a list of cue ids from the old document, and the
        // ids it holds all resolve against the new one.
        self.lane_selection.clear();
        self.list_first = 0;
        lost
    }

    /// `lyric_editor_ui_clear_draft` (`:58-66`).
    pub fn clear_draft(&mut self) {
        self.selected_id = 0;
        self.draft_new = false;
        self.draft_start = 0.0;
        self.draft_end = 0.0;
        self.text.bind("");
        self.text.set_focused(false);
    }

    /// Binds the form to a cue (`lyric_editor_ui_select`, `:108-118`).
    ///
    /// The lane selection is left alone, because the lane applies its own
    /// ctrl/shift rules before calling this.
    pub fn select(&mut self, document: &LyricsDocument, id: u64) {
        let Some(cue) = document.find(id) else {
            return;
        };
        self.selected_id = cue.id;
        self.draft_new = false;
        self.draft_start = cue.start_seconds;
        self.draft_end = cue.end_seconds;
        let text = cue.text.clone();
        self.text.bind(&text);
        self.list_follow_selection = true;
    }

    /// Binds the form *and* makes that cue the whole lane selection
    /// (`lyric_editor_ui_select_single`, `:96-106`).
    ///
    /// Everything outside the lane — the cue list, the probe — uses this. Without
    /// it the lane draws the form's cue with its bound outline and the pale
    /// unselected fill at once, and a drag would then move nothing.
    pub fn select_single(&mut self, document: &LyricsDocument, id: u64) {
        if document.find(id).is_none() {
            return;
        }
        self.select(document, id);
        let _ = self
            .lane_selection
            .apply(document, id, LyricLaneClick::Replace);
    }

    /// `lyric_editor_ui_begin_new` (`:120-131`).
    ///
    /// The default two-second block also stops at the next cue (review 1.14).
    /// The oracle's does not, and an overlap it walks into is invisible: the
    /// later cue simply wins for the length of the overlap and the new line
    /// never displays there. Only the **default** is clamped — every control in
    /// the form and the lane can still make an overlap on purpose, because
    /// overlaps are legal and sometimes wanted.
    pub fn begin_new(&mut self, document: &LyricsDocument, playhead: f64) {
        let duration = document.duration_seconds();
        let mut start = playhead;
        if start + 0.05 > duration {
            start = (duration - 2.0).max(0.0);
        }
        let mut end = duration.min(start + 2.0);
        if let Some(next) = document
            .cues()
            .iter()
            .find(|cue| cue.start_seconds > start)
            .map(|cue| cue.start_seconds)
        {
            end = end.min(next);
        }
        // A cue crowded up against the next one is still a cue, not a sliver the
        // lane could never grab again.
        end = end.max(duration.min(start + lyric_lane_edit::LYRIC_MIN_CUE_SECONDS));
        self.selected_id = 0;
        self.draft_new = true;
        self.draft_start = start;
        self.draft_end = end;
        self.text.bind("");
        self.text.set_focused(true);
    }

    /// `lyric_editor_ui_has_unsaved_draft` (`:68-84`), through Agent B's port of
    /// the rule.
    #[must_use]
    pub fn has_unsaved_draft(&self, document: &LyricsDocument) -> bool {
        LyricDraftState {
            is_new: self.draft_new,
            selected_id: self.selected_id,
            canonical: document.find(self.selected_id).map(|cue| LyricDraftValues {
                start_seconds: cue.start_seconds,
                end_seconds: cue.end_seconds,
                text: cue.text.clone(),
            }),
            draft: LyricDraftValues {
                start_seconds: self.draft_start,
                end_seconds: self.draft_end,
                text: self.text.edit.text().to_string(),
            },
        }
        .is_dirty()
    }

    /// The edit Apply would make, or `None` when the draft is not one the model
    /// would accept.
    ///
    /// Asked *before* the button is drawn, so a draft that cannot be applied gets
    /// a disabled Apply rather than an error notice afterwards. The empty buffer
    /// is the case that matters: the field must let you empty it to retype, and
    /// `validate_text` refuses empty text.
    #[must_use]
    pub fn draft_edit(&self) -> Option<LyricsEdit> {
        let text = self.text.edit.text();
        if lyrics::validate_text(text).is_err() {
            return None;
        }
        if self.draft_new {
            return Some(LyricsEdit::Insert {
                start_seconds: self.draft_start,
                end_seconds: self.draft_end,
                text: text.to_string(),
            });
        }
        if self.selected_id == 0 {
            return None;
        }
        Some(LyricsEdit::Update {
            id: self.selected_id,
            start_seconds: self.draft_start,
            end_seconds: self.draft_end,
            text: text.to_string(),
        })
    }

    fn push(&mut self, edit: LyricsEdit) {
        // Stamped from the editor's own binding, not from whichever track is
        // current: those two can differ, and when they do the difference is
        // review 1.3.
        self.pending.push(PendingLyricEdit {
            owner_slot: self.owner_slot,
            edit,
        });
    }

    /// Everything the editor asked for this frame, taken by `main.rs`.
    ///
    /// Draining rather than reading is the contract: an edit left behind would be
    /// applied twice, and applying a `ShiftMany` twice moves the cues twice.
    ///
    /// `current_slot` is what the edits are checked against (review 1.3). An edit
    /// whose owner is not the current track is **dropped, never applied**: the
    /// two documents share cue ids, so applying it would either overwrite a lyric
    /// the user never opened or fail with a raw out-of-range error against a
    /// shorter track. This is the last line of defence — the guards above it
    /// should mean the count is always zero — and it is deliberately not a
    /// remap-to-the-right-track, because by flush time there is nothing left that
    /// says the user still wants it.
    pub fn take_pending(&mut self, current_slot: Option<usize>) -> LyricEditDrain {
        let mut drain = LyricEditDrain::default();
        for pending in std::mem::take(&mut self.pending) {
            if pending.owner_slot == current_slot {
                drain.edits.push(pending.edit);
            } else {
                drain.refused += 1;
            }
        }
        drain
    }

    #[must_use]
    #[allow(
        dead_code,
        reason = "part of the `main.rs` integration surface this fan-out does not wire; see the report"
    )]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The timeline band height this panel asks for.
    ///
    /// The panel body is the oracle's `lyric_editor_panel_height`
    /// (`lyrics_editor_layout.c:14-28`), which already protects the sidebar and
    /// already refuses to go below the form's 223 px; the rest is this shell's
    /// own chrome plus the lane. Capped the way the export panel caps itself, so
    /// a short window keeps a preview rather than being all timeline.
    #[must_use]
    #[allow(
        dead_code,
        reason = "part of the `main.rs` integration surface this fan-out does not wire; see the report"
    )]
    pub fn timeline_height(&self, window_height: f32, extra_chrome: f32) -> f32 {
        let wanted = CHROME_ABOVE_PANEL
            + PANEL_BOTTOM_PADDING
            + LANE_HEIGHT
            + LANE_GAP
            // `panel_height`'s own ceiling is `screen_height - SIDEBAR_MINIMUM -
            // TIMELINE_CHROME`, which is the guarantee that the sidebar keeps its
            // floor. `extra_chrome` is whatever else the band spends above this
            // editor that the oracle's chrome constant does not know about —
            // today the manual event row, which landed in the same fan-out as
            // this panel. Subtracting it from the screen height *here* is what
            // keeps that guarantee true of the whole band rather than of this
            // panel alone; a capture caught the alternative, with the tracks
            // panel silently below its floor and not drawn.
            + lyrics_editor_layout::panel_height(window_height - extra_chrome, self.cue_count);
        let ceiling = window_height - metric::HUD_BUTTON_SIZE - DEFAULT_TIMELINE_HEIGHT;
        if wanted > ceiling {
            DEFAULT_TIMELINE_HEIGHT.max(ceiling)
        } else {
            wanted
        }
    }

    /// Honours the `--ui-probe` keys that name a lyrics-editor state.
    ///
    /// Returns whether everything asked for was reachable, so `main.rs` can fail
    /// the exit status when it was not — a capture script that photographs the
    /// wrong state is the failure the flag exists to prevent
    /// (`musializer.c:128-130`).
    ///
    /// `lyrics-file=` is set here only because both this panel and Assist would
    /// otherwise claim it; the field it writes is Agent J's.
    #[allow(
        dead_code,
        reason = "part of the `main.rs` integration surface this fan-out does not wire; see the report"
    )]
    pub fn apply_probe(&mut self, probe: &UiProbe, track: &mut Track) -> bool {
        let mut honoured = true;
        if probe.caption_style_pane {
            self.style_pane = true;
        }
        if probe.caption_effects_pane {
            self.style_pane = true;
            self.effects_pane = true;
        }
        if probe.font_browser.is_some() {
            // The browser is a pane *inside* the style pane, and its own Back
            // button returns there (`lyrics_editor_ui.c:1165-1174`).
            self.style_pane = true;
            self.font_pane = true;
        }
        if let Some(target) = probe.caption_picker {
            // A disclosure inside the style pane, so it turns the pane on the
            // same way `fonts=` does — and turns the browser off, because the
            // browser draws over the whole form and the frame would show neither.
            self.style_pane = true;
            self.font_pane = false;
            honoured &= probe.font_browser.is_none();
            // The glow colour lives on the effects form, so its picker opens
            // that body; the other two live on the style form and close it.
            self.effects_pane = target == CaptionPickerProbe::Glow;
            self.picker.toggle(match target {
                CaptionPickerProbe::Ink => CaptionColor::Ink,
                CaptionPickerProbe::Plate => CaptionColor::Plate,
                CaptionPickerProbe::Glow => CaptionColor::Glow,
            });
        }
        if let Some(selection) = probe.lyric_selection {
            match track
                .lyrics
                .cues()
                .get(selection.saturating_sub(1) as usize)
                .map(|cue| cue.id)
            {
                Some(id) => self.select_single(&track.lyrics, id),
                None => honoured = false,
            }
        }
        if let Some(path) = probe.lyrics_reference_path.as_ref() {
            track.lyrics_reference_path = Some(PathBuf::from(path));
        }
        honoured
    }

    /// One line for the slice report, distinguishing "the panel drew" from "the
    /// panel is showing the state that was asked for".
    ///
    /// Follows the `fonts:` and `reopen:` lines: everything a capture script would
    /// otherwise have to infer from a picture.
    #[must_use]
    #[allow(
        dead_code,
        reason = "part of the `main.rs` integration surface this fan-out does not wire; see the report"
    )]
    pub fn describe(&self, track: Option<&Track>) -> String {
        let Some(track) = track else {
            return "no track".to_string();
        };
        let pane = if self.font_pane && self.style_pane {
            "fonts"
        } else if self.style_pane && self.effects_pane {
            "caption-effects"
        } else if self.style_pane {
            "caption-style"
        } else {
            "cues"
        };
        let selected = if self.draft_new {
            "new".to_string()
        } else if self.selected_id == 0 {
            "none".to_string()
        } else {
            match track.lyrics.find(self.selected_id) {
                Some(cue) => format!(
                    "#{} at {}",
                    self.selected_id,
                    widgets::format_timestamp(cue.start_seconds)
                ),
                None => format!("#{} (stale)", self.selected_id),
            }
        };
        format!(
            "{} cues, pane {}, selected {}, owner {}, lane {}, draft {}, shadowed {}, \
             face={} field={} missing={} picker={}",
            track.lyrics.len(),
            pane,
            selected,
            // Which track the draft is bound to (review 1.3). A capture cannot
            // photograph a binding, and "selected #3" reads identically whether
            // the 3 came from this document or the one before it.
            self.owner_slot
                .map_or_else(|| "none".to_string(), |slot| slot.to_string()),
            self.lane_selection.len(),
            if self.has_unsaved_draft(&track.lyrics) {
                "dirty"
            } else {
                "clean"
            },
            // A block whose hatching a capture would otherwise have to be read
            // for (review 1.14): the count says whether the lane drew any.
            track.lyrics.shadowed_cues().len(),
            // UX0-A05. `face=` names the atlas that drew the cue rows, `field=`
            // the one behind the editable line, and `missing=` counts the
            // characters those two faces cannot draw across the whole document.
            // A picture cannot distinguish "this font has no alpha" from "this
            // atlas was never asked for one", so the number is the evidence.
            self.authored.rows,
            self.authored.field,
            self.authored.missing,
            // Which free picker is open, and on what. The picker replaces the
            // left column while it is up, so a capture of the caption pane is
            // ambiguous about whether it photographed the controls or the
            // picker; `picker=ink` is what makes the frame self-describing, and
            // `picker=none` is what proves the toggle closes again.
            self.picker.target.map_or("none", |target| target.token()),
        )
    }
}

impl Shell {
    /// The lyrics editor, in the timeline strip's place.
    pub(crate) fn lyrics_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        // Split borrow: the editor and the widget set are both fields of
        // `self`, and `draw_lyrics` needs them at once.
        let mut editor = std::mem::take(&mut self.lyrics);
        self.draw_lyrics(d, input, &mut editor, content, strip, commands);
        self.lyrics = editor;
    }

    /// One frame of the editor.
    ///
    /// Split from [`Self::lyrics_panel`] so every method below takes the editor
    /// by reference rather than reaching through `self`, which is what lets the
    /// panel borrow the widget set and the editor at the same time.
    fn draw_lyrics(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let font = input.fonts.ui();
        // Before anything is drawn or clicked: whatever the guards did or did not
        // do, the editor must be bound to the track it is about to edit
        // (review 1.3). A draft carried in from another slot is dropped here, and
        // said out loud when it was a new cue, because that is the only draft
        // whose loss the user cannot see for themselves in the list.
        if editor.enter_track(input.workspace.current_index()) {
            self.notify(
                Severity::Warning,
                "Unsaved lyric draft was dropped",
                "The new cue belonged to the track that was open before this one.",
            );
        }
        // The panel starts below the strip, the rule `super::stub` records: the
        // panel and the strip share one band, and the height an open panel asks
        // for is the space underneath.
        let top = strip.y + strip.height + 28.0;
        let panel = UiRect::new(
            content.x + metric::UI_PANEL_PADDING,
            top,
            content.width - metric::UI_PANEL_PADDING * 2.0,
            (content.y + content.height - top - PANEL_BOTTOM_PADDING).max(0.0),
        );
        if panel.is_empty() {
            return;
        }

        let Some(track) = input.workspace.current() else {
            // Reachable: the toolbar disables the Lyrics button without a track,
            // but a probe can still open the panel.
            widgets::fill(d, panel, color::ui_surface());
            widgets::draw_text(
                d,
                font,
                "Open a track to edit its lyrics.",
                panel.x + 8.0,
                panel.y + 8.0,
                metric::UI_FONT_LABEL,
                color::ui_muted(),
            );
            return;
        };
        editor.cue_count = track.lyrics.len();
        // A stale id makes a bulk shift reject the whole move — correctly, but
        // the user would only see dragging stop working (`:989-992`).
        editor.lane_selection.prune(&track.lyrics);

        let lane = UiRect::new(panel.x, panel.y, panel.width, LANE_HEIGHT);
        let boundary = UiRect::new(
            panel.x,
            panel.y + LANE_HEIGHT + LANE_GAP,
            panel.width,
            (panel.height - LANE_HEIGHT - LANE_GAP).max(0.0),
        );
        self.lyric_lane(d, input, editor, track, lane);
        if boundary.is_empty() {
            return;
        }

        widgets::fill(d, boundary, color::ui_surface());
        d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 1.0, color::ui_rule());

        // The panel that cannot host its own controls draws none of them
        // (`workspace_layout.h:7-19`). Reachable today for a reason worth naming
        // rather than hiding: `Shell::timeline_height` still answers
        // `DEFAULT_TIMELINE_HEIGHT` for `UiPanel::Lyrics`, so the band is 180 px
        // and this editor gets 21. The one-line change that fixes it is in Agent
        // I's report; until it lands, this says so instead of drawing the whole
        // form off the bottom of the framebuffer — which a capture caught.
        if boundary.height < EDITOR_MINIMUM_HEIGHT {
            widgets::draw_text(
                d,
                font,
                "The lyrics editor needs a taller timeline than this build reserves for it.",
                boundary.x + 8.0,
                boundary.y + 6.0,
                metric::UI_FONT_CAPTION,
                color::ui_warning(),
            );
            return;
        }

        let padding = metric::UI_PANEL_PADDING;
        let gap = metric::UI_CONTROL_GAP;
        let list = UiRect::new(
            boundary.x + padding,
            boundary.y + padding,
            boundary.width * 0.48 - padding - gap * 0.5,
            boundary.height - padding * 2.0,
        );
        let form = UiRect::new(
            list.x + list.width + gap,
            list.y,
            boundary.x + boundary.width - padding - (list.x + list.width + gap),
            list.height,
        );

        // The caption pane takes the whole panel: typography needs two columns of
        // controls and the cue list is not part of the decision being made, so
        // hiding it is what makes the controls fit at all (`:1162-1185`).
        if editor.style_pane {
            self.caption_pane(d, input, editor, track, boundary, list, commands);
            return;
        }

        self.cue_list(d, input, editor, track, list);
        self.cue_form(d, input, editor, track, boundary, form);
    }

    // -----------------------------------------------------------------------
    // The cue lane
    // -----------------------------------------------------------------------

    /// The direct-manipulation lane (`lyric_editor_ui_draw_lane`, `:983-1082`).
    fn lyric_lane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        lane: UiRect,
    ) {
        if lane.is_empty() {
            return;
        }
        self.timeline_pan_gesture(d, input, lane);
        self.lane_gesture(d, input.ui_scale, editor, track, lane);

        widgets::fill(d, lane, color::ui_raised());
        d.draw_line_ex(
            Vector2::new(lane.x, lane.y),
            Vector2::new(lane.x + lane.width, lane.y),
            1.0,
            color::ui_rule(),
        );

        let view = self.timeline;
        let mouse = input.ui_scale.mouse(d);
        // No hover highlight while a gesture is in flight, which is the C's
        // `*active_button_id == 0` guard (`:1006-1009`) expressed against the one
        // claim this lane actually has.
        let hover = if !lyric_lane_hover_allowed(lane, mouse, editor.lane_drag.active) {
            None
        } else {
            lyric_lane_edit::hit_test(
                &track.lyrics,
                &view,
                f64::from(lane.x),
                f64::from(lane.width),
                f64::from(mouse.x),
            )
        };

        // Which blocks lose time to a later one (review 1.14). Computed once for
        // the whole lane, and not at all mid-drag: the intervals are in stored
        // time, so hatching them over blocks that are following the pointer would
        // paint the marks in the wrong place.
        let shadows = if editor.lane_drag.moved {
            Vec::new()
        } else {
            track.lyrics.shadowed_cues()
        };

        for cue in track.lyrics.cues() {
            let (preview_start, preview_end) = lane_preview_span(editor, cue);
            let Some(geometry) = timed_lane::block_geometry(
                &view,
                f64::from(lane.x),
                f64::from(lane.width),
                preview_start,
                preview_end,
            ) else {
                continue;
            };
            // Clipped to the lane by the same geometry the hit test reads. A
            // boundary which scrolled off-screen is therefore never recreated
            // as a handle at the lane border.
            let block = UiRect::new(
                geometry.left as f32,
                lane.y + 3.0,
                geometry.width() as f32,
                lane.height - 6.0,
            );

            let in_selection = editor.lane_selection.contains(cue.id);
            let hovered = hover.is_some_and(|hit| hit.id == cue.id);
            let alpha = if in_selection {
                0.82
            } else if hovered {
                0.68
            } else {
                0.38
            };
            widgets::fill(d, block, fade(LYRIC_BLOCK, alpha));
            d.draw_rectangle_lines_ex(widgets::rectangle(block), 1.0, LYRIC_BLOCK);
            // A span the preview and the export will never show this cue in is
            // greyed and hatched, so two blocks that draw the same amber stop
            // meaning the same thing (review 1.14).
            if let Some(shadow) = shadows.iter().find(|shadow| shadow.id == cue.id) {
                for &(from, to) in &shadow.hidden {
                    let Some(hidden) = timed_lane::block_geometry(
                        &view,
                        f64::from(lane.x),
                        f64::from(lane.width),
                        from,
                        to,
                    ) else {
                        continue;
                    };
                    let region = UiRect::new(
                        hidden.left as f32,
                        block.y,
                        hidden.width() as f32,
                        block.height,
                    );
                    widgets::fill(d, region, fade(color::ui_surface(), 0.62));
                    hatch(d, region, fade(color::ui_warning(), 0.85));
                }
            }
            // The cue the form is bound to gets a second, inset outline.
            // Selection and form target are different things now that a drag can
            // move several cues at once, and one shade of amber cannot say both.
            if cue.id == editor.selected_id {
                d.draw_rectangle_lines_ex(
                    widgets::rectangle(UiRect::new(
                        block.x + 1.0,
                        block.y + 1.0,
                        block.width - 2.0,
                        block.height - 2.0,
                    )),
                    1.0,
                    color::ui_ink(),
                );
            }
            // Handles are shown only where a press would actually take them, so
            // the affordance and the hit test cannot drift apart.
            if let Some(hit) =
                hover.filter(|hit| hit.id == cue.id && hit.zone != LyricLaneZone::Body)
            {
                let handle_x = if hit.zone == LyricLaneZone::StartEdge {
                    block.x
                } else {
                    block.x + block.width - 2.0
                };
                widgets::fill(
                    d,
                    UiRect::new(handle_x, block.y, 2.0, block.height),
                    color::ui_ink(),
                );
            }
        }

        // The playhead, over the blocks. The strip above draws its own; this lane
        // is a second view of the same span and would be hard to read without one.
        draw_timeline_playhead(
            d,
            view,
            lane,
            self.timeline_playhead_seconds(input.time_seconds),
            1.0,
            false,
        );
        if track.lyrics.is_empty() {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                "no cues in this track yet",
                lane.x + 6.0,
                lane.y + 4.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }
    }

    /// The press, the drag and the release (`lane_gesture_update`, `:353-449`).
    fn lane_gesture(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        ui_scale: super::super::scale::UiScale,
        editor: &mut LyricEditor,
        track: &Track,
        lane: UiRect,
    ) {
        use raylib::consts::KeyboardKey as Key;
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        let mouse = ui_scale.mouse(d);
        let view = self.timeline;
        let mode = if d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL)
        {
            LyricLaneClick::Toggle
        } else if d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT) {
            LyricLaneClick::Extend
        } else {
            LyricLaneClick::Replace
        };

        if !editor.lane_drag.active {
            if !(lane.contains_point(mouse.x, mouse.y)
                && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT))
            {
                return;
            }
            let hit = lyric_lane_edit::hit_test(
                &track.lyrics,
                &view,
                f64::from(lane.x),
                f64::from(lane.width),
                f64::from(mouse.x),
            );
            // Claimed regardless of what was hit: an empty-lane press must not
            // fall through to whatever is underneath, or clearing a selection
            // would also do something else.
            editor.lane_drag = LaneDrag {
                active: true,
                origin_x: mouse.x,
                origin_seconds: view.seconds_at(
                    f64::from(mouse.x),
                    f64::from(lane.x),
                    f64::from(lane.width),
                    track.lyrics.duration_seconds(),
                ),
                ..LaneDrag::default()
            };
            let Some(hit) = hit else {
                editor.lane_selection.clear();
                return;
            };
            // A press that would rebind the form goes through the same
            // unsaved-draft guard as every other context change.
            if hit.id != editor.selected_id && !self.allow_context_change(editor, track) {
                return;
            }
            editor.lane_drag.zone = Some(hit.zone);
            editor.lane_drag.id = hit.id;

            // A plain press on a block already in the selection keeps the
            // selection so the whole set can be dragged; the collapse to one cue
            // happens on release, and only if nothing was dragged. The `||`
            // short-circuit is load-bearing — `apply` must not run in that case,
            // because a Replace would collapse the very selection being kept.
            let keeps_selection =
                mode == LyricLaneClick::Replace && editor.lane_selection.contains(hit.id);
            let accepted =
                keeps_selection || editor.lane_selection.apply(&track.lyrics, hit.id, mode);
            if !accepted {
                self.notify(
                    Severity::Warning,
                    "That range is too large",
                    "A lane selection holds at most 64 cues.",
                );
                editor.lane_drag = LaneDrag::default();
                return;
            }
            editor.select(&track.lyrics, hit.id);
            return;
        }

        if d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
            let Some(zone) = editor.lane_drag.zone else {
                return;
            };
            if (mouse.x - editor.lane_drag.origin_x).abs()
                >= lyric_lane_edit::LYRIC_LANE_DRAG_THRESHOLD_PIXELS as f32
            {
                editor.lane_drag.moved = true;
            }
            if !editor.lane_drag.moved {
                return;
            }
            let pointer = view.seconds_at(
                f64::from(mouse.x),
                f64::from(lane.x),
                f64::from(lane.width),
                track.lyrics.duration_seconds(),
            );
            if zone == LyricLaneZone::Body {
                editor.lane_drag.delta_seconds = lyric_lane_edit::clamp_move(
                    &track.lyrics,
                    &editor.lane_selection,
                    pointer - editor.lane_drag.origin_seconds,
                );
            } else if let Some((start, end)) = lyric_lane_edit::clamp_resize(
                &track.lyrics,
                editor.lane_drag.id,
                zone == LyricLaneZone::StartEdge,
                pointer,
            ) {
                editor.lane_drag.edge_seconds = if zone == LyricLaneZone::StartEdge {
                    start
                } else {
                    end
                };
            }
            return;
        }

        // Released — or the button went up while the window was unfocused and no
        // release edge ever arrived. Either way the gesture ends here.
        if editor.lane_drag.moved {
            self.lane_commit(editor, track);
        } else if editor.lane_drag.id != 0 && mode == LyricLaneClick::Replace {
            let _ = editor.lane_selection.apply(
                &track.lyrics,
                editor.lane_drag.id,
                LyricLaneClick::Replace,
            );
        }
        editor.lane_drag = LaneDrag::default();
    }

    /// `lane_commit_drag` (`:317-351`).
    fn lane_commit(&mut self, editor: &mut LyricEditor, track: &Track) {
        let Some(zone) = editor.lane_drag.zone else {
            return;
        };
        if zone == LyricLaneZone::Body {
            if editor.lane_drag.delta_seconds == 0.0 {
                return;
            }
            editor.push(LyricsEdit::ShiftMany {
                ids: editor.lane_selection.ids().to_vec(),
                delta_seconds: editor.lane_drag.delta_seconds,
            });
        } else {
            let Some(cue) = track.lyrics.find(editor.lane_drag.id) else {
                return;
            };
            let (mut start, mut end) = (cue.start_seconds, cue.end_seconds);
            if zone == LyricLaneZone::StartEdge {
                start = editor.lane_drag.edge_seconds;
            } else {
                end = editor.lane_drag.edge_seconds;
            }
            if start == cue.start_seconds && end == cue.end_seconds {
                return;
            }
            editor.push(LyricsEdit::Retime {
                id: editor.lane_drag.id,
                start_seconds: start,
                end_seconds: end,
            });
        }
        // The form holds a copy of the cue it is bound to. Leaving it behind
        // would make an untouched form read as a dirty draft and block every
        // panel change until the user discarded an edit they never made. The C
        // re-reads the cue after committing (`:347-349`); here the commit has not
        // landed yet, so the draft is moved by the same numbers the edit carries.
        if editor.selected_id != 0 {
            match zone {
                LyricLaneZone::Body if editor.lane_selection.contains(editor.selected_id) => {
                    editor.draft_start += editor.lane_drag.delta_seconds;
                    editor.draft_end += editor.lane_drag.delta_seconds;
                }
                LyricLaneZone::StartEdge if editor.lane_drag.id == editor.selected_id => {
                    editor.draft_start = editor.lane_drag.edge_seconds;
                }
                LyricLaneZone::EndEdge if editor.lane_drag.id == editor.selected_id => {
                    editor.draft_end = editor.lane_drag.edge_seconds;
                }
                _ => {}
            }
        }
    }

    /// `lyric_editor_ui_allow_context_change` (`:86-94`).
    ///
    /// In-panel navigation only — the editor is drawn against the track it owns,
    /// so `track` is the owning document by construction. The outer paths (a
    /// track row, a panel button, an Open) go through
    /// [`Self::lyric_draft_allows_context_change`], which has to find the owner
    /// first.
    fn allow_context_change(&mut self, editor: &LyricEditor, track: &Track) -> bool {
        if !editor.has_unsaved_draft(&track.lyrics) {
            return true;
        }
        self.refuse_lyric_context_change();
        false
    }

    /// The same guard for the paths *outside* the panel: the Tracks row, the
    /// bottom-panel buttons, and the Open actions (review 1.3).
    ///
    /// The difference that matters is which document the draft is compared
    /// against. `allow_context_change` above is handed the current track and that
    /// is correct while the panel is drawing; here the draft may already belong
    /// to a slot the user is trying to leave, and asking the *current* track
    /// whether cue #3 changed is exactly the confusion this guard exists to
    /// prevent. So the owner is resolved first, the way
    /// `RouteEditorState::is_dirty_for_track` resolves it
    /// (`route_editor_state.c:233-237`).
    ///
    /// Same words, same refuse-with-a-notice behaviour as the in-panel guard, so
    /// that clicking a cue row and clicking a track row fail the same way.
    pub(crate) fn lyric_draft_allows_context_change(&mut self, workspace: &Workspace) -> bool {
        if !self.lyric_draft_is_dirty(workspace) {
            return true;
        }
        self.refuse_lyric_context_change();
        false
    }

    /// The same question without the notice, for the quit confirmation
    /// (`plug.c:7215`), which weighs it against five other conditions and asks
    /// once.
    #[must_use]
    pub(crate) fn lyric_draft_is_dirty(&self, workspace: &Workspace) -> bool {
        self.lyrics.draft_owner().is_some_and(|slot| {
            workspace
                .get(slot)
                .is_some_and(|owner| self.lyrics.has_unsaved_draft(&owner.lyrics))
        })
    }

    /// Drains the editor's deferred edits for the track `main.rs` is about to
    /// write, and reports whatever the owner check refused (review 1.3).
    ///
    /// The drain lives here rather than in `main.rs` so that the refusal is a
    /// notice the shell's own tests can see. A silently dropped edit is the
    /// failure mode this whole task is about: the user believes they applied
    /// something, and the only two honest outcomes are that it lands on the track
    /// they authored it against or that they are told it did not land at all.
    pub(crate) fn drain_lyric_edits(&mut self, current_slot: Option<usize>) -> Vec<LyricsEdit> {
        let drain = self.lyrics.take_pending(current_slot);
        if drain.refused > 0 {
            self.notify(
                Severity::Warning,
                "Lyric edit was dropped",
                &format!(
                    "{} edit(s) belonged to a track that is no longer current and were not applied.",
                    drain.refused
                ),
            );
        }
        drain.edits
    }

    /// One place for the refusal, so the two guards cannot drift apart.
    fn refuse_lyric_context_change(&mut self) {
        self.notify(
            Severity::Warning,
            "Finish the lyric edit first",
            "Apply or discard the current draft before changing tracks or panels.",
        );
    }

    // -----------------------------------------------------------------------
    // The cue list
    // -----------------------------------------------------------------------

    /// The scrolling cue list (`lyric_editor_ui_draw`'s list half, `:1187-1272`).
    fn cue_list(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        list: UiRect,
    ) {
        if list.is_empty() {
            return;
        }
        let font = input.fonts.ui();
        // Chrome keeps the native-size bank; the user's own words do not. The
        // chrome atlas is `ui_codepoint`-filtered precisely so it stays small,
        // and rebuilding it over the full curated set is not the alternative:
        // seventeen native sizes times 1784 codepoints is hundreds of megabytes
        // of atlas at the 1.75x rung. The caption atlas is already resident, is
        // 64 px and mipmapped, and the shell draws under a `Camera2D` whose zoom
        // is the UI scale — so every rung minifies from it and none magnifies a
        // 1x bitmap.
        let authored = input.fonts.authored();
        // The editable field draws through `draw_with_face` with the same
        // authored face as the rows, so the report can claim it truthfully.
        let field = (authored.name(), authored.repertoire());
        editor.authored = authored_evidence(
            (authored.name(), authored.repertoire()),
            field,
            track.lyrics.cues().iter().map(|cue| cue.text.as_str()),
            editor.text.edit.text(),
        );
        let playhead = input.time_seconds;
        widgets::draw_text(
            d,
            font,
            "LYRIC CUES",
            list.x,
            list.y,
            metric::UI_FONT_HEADER,
            color::accent(),
        );
        let count = format!("{} / {}", track.lyrics.len(), lyrics::CUE_CAPACITY);
        let count_width = widgets::measure(font, &count, metric::UI_FONT_VALUE);
        widgets::draw_text(
            d,
            font,
            &count,
            list.x + list.width - count_width,
            list.y + 2.0,
            metric::UI_FONT_VALUE,
            color::ui_muted(),
        );

        let visible = if list.height > LYRIC_EDITOR_ROW_HEIGHT {
            ((list.height - 32.0) / LYRIC_EDITOR_ROW_HEIGHT).max(0.0) as usize
        } else {
            0
        };
        // The row the list wants to keep in view: the bound cue, or the cue under
        // the playhead when nothing is bound.
        let focus = track
            .lyrics
            .cues()
            .iter()
            .position(|cue| {
                cue.id == editor.selected_id
                    || (editor.selected_id == 0
                        && cue.start_seconds <= playhead
                        && playhead < cue.end_seconds)
            })
            .unwrap_or(0);
        let mut first = editor.list_first;
        if editor.list_follow_selection {
            first = focus.saturating_sub(visible / 2);
        }
        let mouse = input.ui_scale.mouse(d);
        if list.contains_point(mouse.x, mouse.y) {
            let wheel = d.get_mouse_wheel_move();
            if wheel != 0.0 {
                editor.list_follow_selection = false;
                first = (first as i64 - (wheel * 3.0) as i64).max(0) as usize;
            }
        }
        if first + visible > track.lyrics.len() {
            first = track.lyrics.len().saturating_sub(visible);
        }
        editor.list_first = first;

        if track.lyrics.is_empty() {
            widgets::draw_text(
                d,
                font,
                "No lyric cues. Add one at the playhead.",
                list.x,
                list.y + 38.0,
                metric::UI_FONT_LABEL,
                color::ui_muted(),
            );
        }

        for row in 0..visible {
            let Some(cue) = track.lyrics.cues().get(first + row) else {
                break;
            };
            let boundary = UiRect::new(
                list.x,
                list.y + 28.0 + row as f32 * LYRIC_EDITOR_ROW_HEIGHT,
                list.width,
                LYRIC_EDITOR_ROW_HEIGHT - 2.0,
            );
            // Keyed by cue id, not by row, so a list that scrolls under a held
            // press cannot let one row release another's claim.
            let id = widgets::widget_id(ns::ROWS, cue.id as u32);
            let state = self.widgets.button(d, id, boundary);
            let selected = cue.id == editor.selected_id;
            let current = cue.start_seconds <= playhead && playhead < cue.end_seconds;
            let background = if state.hovered {
                color::track_button_hoverover()
            } else if selected {
                Color::get_color(SELECTED_ROW)
            } else {
                color::ui_raised()
            };
            widgets::fill(d, boundary, background);
            d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 1.0, color::ui_rule());
            if current {
                widgets::fill(
                    d,
                    UiRect::new(boundary.x, boundary.y, 3.0, boundary.height),
                    color::accent(),
                );
            }
            widgets::draw_text(
                d,
                font,
                &widgets::format_timestamp(cue.start_seconds),
                boundary.x + 8.0,
                boundary.y + 5.0,
                metric::UI_FONT_VALUE,
                color::ui_muted(),
            );
            // The cue's own words, so they go through the authored face rather
            // than the chrome bank (UX0-A05). The scissor stays as a backstop
            // against a half-pixel of measurement rounding, but it is no longer
            // what decides where the line ends.
            //
            // Ellipsized rather than hard-clipped, which is a deliberate
            // divergence from the C (`:1249-1254` shows as much as the row
            // holds). Nothing observable in a `.musi` file or a rendered frame
            // depends on it — it is chrome — and a hard clip cuts the last glyph
            // mid-stroke, which in a script the reader cannot spot-check by
            // shape is indistinguishable from the missing-glyph rendering this
            // very fix is about. The marker says "there is more" instead.
            let text_x = boundary.x + 90.0;
            if boundary.width > 100.0 {
                // The scissor spans `text_x .. text_x + width - 94`, and the
                // text starts 4 px into it.
                let available = boundary.width - 98.0;
                let fitted = authored.ellipsize(&cue.text, available, metric::UI_FONT_VALUE, 0.0);
                let mut clip = widgets::begin_scissor(
                    d,
                    UiRect::new(text_x, boundary.y, boundary.width - 94.0, boundary.height),
                    input.ui_scale,
                );
                authored.draw_text(
                    &mut clip,
                    &fitted,
                    Vector2::new(text_x + 4.0, boundary.y + 5.0),
                    metric::UI_FONT_VALUE,
                    0.0,
                    color::ui_ink(),
                );
                drop(clip);
            }
            if state.clicked && (selected || self.allow_context_change(editor, track)) {
                editor.select_single(&track.lyrics, cue.id);
            }
        }

        if track.lyrics.len() > visible && visible > 0 {
            let thumb =
                24.0f32.max((list.height - 32.0) * visible as f32 / track.lyrics.len() as f32);
            let travel = list.height - 32.0 - thumb;
            let amount = first as f32 / (track.lyrics.len() - visible) as f32;
            widgets::fill(
                d,
                UiRect::new(
                    list.x + list.width - 3.0,
                    list.y + 28.0 + travel * amount,
                    3.0,
                    thumb,
                ),
                color::accent(),
            );
        }
    }

    // -----------------------------------------------------------------------
    // The editing form
    // -----------------------------------------------------------------------

    /// The right-hand form (`lyric_editor_ui_draw`'s form half, `:1274-1424`).
    fn cue_form(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        boundary: UiRect,
        form: UiRect,
    ) {
        use raylib::consts::KeyboardKey as Key;

        if form.is_empty() {
            return;
        }
        let font = input.fonts.ui();
        // The pane toggle takes the header's place: a caption style and a cue
        // edit are never wanted at once, and the panel has room for one.
        let toggle = UiRect::new(form.x, form.y - 3.0, 58.0, 34.0);
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ns::ACTIONS, 0),
                toggle,
                "Style",
                editor.style_pane,
                ButtonStyle::Neutral,
                Some(14.0),
            )
            .clicked
            && self.allow_context_change(editor, track)
        {
            // Switching pane hides the draft, so it goes through the same guard
            // as every other context change.
            editor.style_pane = true;
            editor.text.set_focused(false);
        }

        let add = UiRect::new(form.x + form.width - 92.0, form.y - 3.0, 92.0, 34.0);
        let import = UiRect::new(add.x - 83.0, add.y, 77.0, add.height);
        let export = UiRect::new(import.x - 83.0, add.y, 77.0, add.height);
        let header = if editor.draft_new {
            "NEW CUE"
        } else {
            "SELECTED CUE"
        };
        // Drawn only where it fits between the toggle and the document buttons; a
        // header printed through Export is how this row would fail on a narrow
        // panel, and the toggle already names the pane.
        let header_x = toggle.x + toggle.width + 8.0;
        if header_x + widgets::measure(font, header, metric::UI_FONT_HEADER)
            < form.x + form.width - 258.0 - 8.0
        {
            widgets::draw_text(
                d,
                font,
                header,
                header_x,
                form.y,
                metric::UI_FONT_HEADER,
                color::accent(),
            );
        }

        let labels: [&str; 3] = ["Export", "Import", "Add cue"];
        let widths = [export.width, import.width, add.width];
        let row_font = widgets::row_font_size(font, &labels, &widths, add.height);
        // Export and Import move a `.lyrics.tsv` through a native save/open
        // dialog (`export_lyrics_document`, `:1084-1140`). A dialog is a
        // `ShellCommand` in this rewrite and there is no variant for one yet, so
        // these say so rather than doing nothing — the request is in Agent I's
        // report. `LyricsDocument::bridge_export`/`bridge_import` are already
        // ported and tested; what is missing is the two commands and their
        // file plumbing.
        self.widgets
            .disabled_button(d, font, export, labels[0], Some(row_font));
        self.widgets
            .disabled_button(d, font, import, labels[1], Some(row_font));
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ns::ACTIONS, 1),
                add,
                labels[2],
                false,
                ButtonStyle::Neutral,
                Some(row_font),
            )
            .clicked
            && self.allow_context_change(editor, track)
        {
            editor.begin_new(&track.lyrics, input.time_seconds);
        }

        // A panel too short for the form shows the list alone rather than drawing
        // Apply, Discard and Delete past its own bottom edge, which is what the
        // oracle shipped at every supported window size (`:1340-1346`).
        if !lyrics_editor_layout::form_fits(boundary.height) {
            widgets::draw_text(
                d,
                font,
                "Enlarge the window to edit a cue.",
                form.x,
                form.y + 34.0,
                metric::UI_FONT_VALUE,
                color::ui_muted(),
            );
            return;
        }
        if !editor.draft_new && editor.selected_id == 0 {
            widgets::draw_text(
                d,
                font,
                "Select a cue or add one at the current playhead.",
                form.x,
                form.y + 42.0,
                metric::UI_FONT_LABEL,
                color::ui_muted(),
            );
            return;
        }

        let duration = track.lyrics.duration_seconds();
        let playhead = input.time_seconds;
        let mut start = editor.draft_start;
        let mut end = editor.draft_end;
        self.time_row(
            d,
            font,
            UiRect::new(form.x, form.y + 30.0, form.width, 30.0),
            "START",
            &mut start,
            end,
            true,
            playhead,
            duration,
            form_id::START_TIME,
        );
        self.time_row(
            d,
            font,
            UiRect::new(form.x, form.y + 64.0, form.width, 30.0),
            "END",
            &mut end,
            start,
            false,
            playhead,
            duration,
            form_id::END_TIME,
        );
        editor.draft_start = start;
        editor.draft_end = end;

        let field = UiRect::new(form.x, form.y + 101.0, form.width, 37.0);
        // The user's own words go through the caption atlas while being typed,
        // for the same reason the cue list does (review 1.5).
        let event = editor.text.draw_with_face(
            d,
            input.fonts.authored(),
            field,
            17.0,
            "Type lyric content",
            true,
        );
        if event.flattened {
            self.notify(
                Severity::Info,
                "Pasted as one line",
                "A cue is a single line, so line breaks became spaces.",
            );
        }
        if let Some(error) = event.rejected {
            self.notify(
                Severity::Warning,
                "Nothing was pasted",
                match error {
                    TextEditError::TooLong => {
                        "The clipboard is longer than a cue can hold, and a cue is never truncated."
                    }
                    TextEditError::Rejected => {
                        "The clipboard holds a control character a cue cannot contain."
                    }
                    TextEditError::Empty => "There was nothing on the clipboard.",
                },
            );
        }
        if event.clipboard_unavailable {
            self.notify(
                Severity::Warning,
                "Nothing was pasted",
                "The clipboard could not be read as text.",
            );
        }

        let apply = UiRect::new(form.x, form.y + 146.0, 92.0, metric::UI_BUTTON_HEIGHT);
        let discard = UiRect::new(
            apply.x + apply.width + metric::UI_CONTROL_GAP,
            apply.y,
            104.0,
            apply.height,
        );
        let remove = UiRect::new(
            discard.x + discard.width + metric::UI_CONTROL_GAP,
            apply.y,
            92.0,
            apply.height,
        );
        let discard_label = if editor.draft_new {
            "Cancel edit"
        } else {
            "Discard edit"
        };
        let draft_labels: [&str; 3] = ["Apply", discard_label, "Delete"];
        let draft_widths = [apply.width, discard.width, remove.width];
        let draft_font = widgets::row_font_size(font, &draft_labels, &draft_widths, apply.height);

        // Apply is withheld for a draft the model would refuse rather than
        // offered and then answered with an error notice. The only reachable case
        // is empty text, which the field must let you type through.
        match editor.draft_edit() {
            Some(edit) => {
                if self
                    .widgets
                    .text_button(
                        d,
                        font,
                        widgets::widget_id(ns::FORM, form_id::APPLY),
                        apply,
                        draft_labels[0],
                        false,
                        ButtonStyle::Neutral,
                        Some(draft_font),
                    )
                    .clicked
                {
                    apply_draft(editor, track, edit);
                }
            }
            None => self
                .widgets
                .disabled_button(d, font, apply, draft_labels[0], Some(draft_font)),
        }
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ns::FORM, form_id::DISCARD),
                discard,
                discard_label,
                false,
                ButtonStyle::Neutral,
                Some(draft_font),
            )
            .clicked
        {
            editor.clear_draft();
        }
        if !editor.draft_new
            && self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(ns::FORM, form_id::DELETE),
                    remove,
                    draft_labels[2],
                    false,
                    ButtonStyle::Danger,
                    Some(draft_font),
                )
                .clicked
        {
            editor.push(LyricsEdit::Delete {
                id: editor.selected_id,
            });
            editor.clear_draft();
        }
        // The shortcut hint's row carries the overlap warning instead when there
        // is one (review 1.14). There is no spare row in this form, and a line
        // that will never reach the screen outranks a reminder about Ctrl+V.
        // It describes the *stored* cue, not the draft: it is telling the user
        // what the project does today.
        let notice = (!editor.draft_new)
            .then(|| shadow_notice(&track.lyrics, editor.selected_id))
            .flatten();
        widgets::draw_text(
            d,
            font,
            notice
                .as_deref()
                .unwrap_or("Ctrl+Enter applies  |  Ctrl+V pastes  |  Ctrl+A selects all"),
            form.x,
            apply.y + apply.height + 7.0,
            metric::UI_FONT_LABEL - 2.0,
            if notice.is_some() {
                color::ui_warning()
            } else {
                color::ui_muted()
            },
        );
        // Ctrl+Enter, the oracle's shortcut (`:1420-1423`).
        if editor.text.is_focused()
            && (d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL))
            && d.is_key_pressed(Key::KEY_ENTER)
        {
            if let Some(edit) = editor.draft_edit() {
                apply_draft(editor, track, edit);
            }
        }
    }

    /// One `START`/`END` row (`lyric_time_row`, `:219-250`).
    #[allow(
        clippy::too_many_arguments,
        reason = "the oracle's `lyric_time_row` shape: box, label, the value, its neighbour, which end it is, the playhead, the track length"
    )]
    fn time_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        boundary: UiRect,
        label: &str,
        value: &mut f64,
        other: f64,
        is_start: bool,
        playhead: f64,
        duration: f64,
        id_base: u32,
    ) {
        widgets::draw_text(
            d,
            font,
            label,
            boundary.x,
            boundary.y + 8.0,
            metric::UI_FONT_LABEL,
            color::ui_muted(),
        );
        widgets::draw_text(
            d,
            font,
            &widgets::format_timestamp(*value),
            boundary.x + 58.0,
            boundary.y + 7.0,
            18.0,
            color::ui_ink(),
        );
        let gap = 4.0;
        let minus = UiRect::new(boundary.x + 154.0, boundary.y, 42.0, boundary.height);
        let plus = UiRect::new(
            minus.x + minus.width + gap,
            boundary.y,
            42.0,
            boundary.height,
        );
        let set = UiRect::new(plus.x + plus.width + gap, boundary.y, 78.0, boundary.height);
        let labels: [&str; 3] = ["-0.1", "+0.1", "Set here"];
        let widths = [minus.width, plus.width, set.width];
        let size = widgets::row_font_size(font, &labels, &widths, boundary.height);
        for (index, (rect, label)) in [(minus, labels[0]), (plus, labels[1]), (set, labels[2])]
            .into_iter()
            .enumerate()
        {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(ns::FORM, id_base + index as u32),
                    rect,
                    label,
                    false,
                    ButtonStyle::Neutral,
                    Some(size),
                )
                .clicked
            {
                match index {
                    0 => *value -= 0.1,
                    1 => *value += 0.1,
                    _ => *value = playhead,
                }
            }
        }
        // Clamped after the buttons, as the oracle does, so a nudge past the
        // other end stops there instead of producing a cue the model refuses.
        //
        // The arithmetic itself lives in `core` and is total (review 1.1). It was
        // `f64::clamp` here, which panics when its bounds invert — and they invert
        // for any cue shorter than the minimum gap, which the model loads
        // happily. See `clamp_form_start`.
        if is_start {
            *value = lyric_lane_edit::clamp_form_start(*value, other);
        } else {
            *value = lyric_lane_edit::clamp_form_end(*value, other, duration);
        }
    }

    // -----------------------------------------------------------------------
    // The caption typography pane
    // -----------------------------------------------------------------------

    /// The whole-panel caption pane, and the font browser inside it
    /// (`lyric_editor_ui_draw`'s style branch, `:1162-1185`).
    #[allow(
        clippy::too_many_arguments,
        reason = "one rectangle per nested surface, plus the command sink the font pane needs"
    )]
    fn caption_pane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        boundary: UiRect,
        list: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let font = input.fonts.ui();
        let browsing = editor.font_pane;
        let toggle = UiRect::new(list.x, list.y - 3.0, 58.0, 34.0);
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ns::ACTIONS, 2),
                toggle,
                if browsing { "Back" } else { "Cues" },
                false,
                ButtonStyle::Neutral,
                Some(14.0),
            )
            .clicked
        {
            if browsing {
                // Back leaves the browser without leaving the style pane, which
                // is where the face the browser just imported is chosen.
                editor.font_pane = false;
            } else {
                editor.style_pane = false;
                // Leaving typography behind closes the browser with it: coming
                // back to a half-finished search nobody remembers starting is
                // worse than coming back to the controls.
                editor.font_pane = false;
                editor.effects_pane = false;
            }
        }
        widgets::draw_text(
            d,
            font,
            if browsing {
                "CAPTION FACE"
            } else if editor.effects_pane {
                "CAPTION EFFECTS"
            } else {
                "CAPTION STYLE"
            },
            toggle.x + toggle.width + 8.0,
            list.y,
            metric::UI_FONT_HEADER,
            color::accent(),
        );
        // The Style/Effects flip, hidden while the browser is up because the
        // browser's Back already means "return to where you were".
        if !browsing {
            let flip = UiRect::new(toggle.x + toggle.width + 178.0, list.y - 3.0, 74.0, 34.0);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(ns::CAPTION, caption_id::EFFECTS_TOGGLE),
                    flip,
                    if editor.effects_pane {
                        "Style"
                    } else {
                        "Effects"
                    },
                    false,
                    ButtonStyle::Neutral,
                    Some(14.0),
                )
                .clicked
            {
                editor.effects_pane = !editor.effects_pane;
                // Whichever picker was open belongs to the form being left; an
                // open picker floating over the other form would write into a
                // control that is no longer on screen.
                editor.picker.target = None;
            }
        }
        let form = UiRect::new(
            boundary.x + metric::UI_PANEL_PADDING,
            list.y + 38.0,
            boundary.width - metric::UI_PANEL_PADDING * 2.0,
            boundary.height - metric::UI_PANEL_PADDING * 2.0 - 38.0,
        );
        if form.is_empty() {
            return;
        }
        if browsing {
            // Agent K's, called with the rectangle this pane's layout gives it.
            self.font_browser_pane(d, input, form, commands);
            return;
        }
        if editor.effects_pane {
            self.caption_effects_form(d, input, editor, track, form);
            return;
        }
        self.caption_style_form(d, input, editor, track, form);
    }

    /// `draw_caption_style_form` (`:833-981`), plus the free picker the oracle
    /// has no counterpart for.
    fn caption_style_form(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        form: UiRect,
    ) {
        let font = input.fonts.ui();
        if form.height < CAPTION_FORM_HEIGHT || form.width < CAPTION_FORM_WIDTH {
            widgets::draw_text(
                d,
                font,
                "Enlarge the window to edit caption style.",
                form.x,
                form.y,
                metric::UI_FONT_VALUE,
                color::ui_muted(),
            );
            return;
        }
        // The track is authoritative and is up to date by the time this frame
        // runs, so the draft is taken fresh each frame rather than held between
        // them. A held copy would be a second source of truth, and the one that
        // goes stale when an edit is refused.
        let mut style = track.caption_style.clone();
        let mut changed = false;

        // Two columns across the whole panel. The cue list is hidden while this
        // pane is open, which is what buys the width: stacking these controls
        // into the 48% form column needed height the panel does not have.
        let layout = CaptionFormLayout::compose(form);

        // The imported face is offered only when the project actually carries
        // one. A third choice selecting a face the file does not have would be a
        // control whose only outcome is a validation failure on save.
        let imported = style.font.as_ref().map(|asset| asset.family.clone());

        // Left column, or the picker in its place. Never both: a control painted
        // under another claims the press first, which is the defect
        // `workspace_layout`'s no-draw rule exists for, and the picker needs more
        // height than any window this panel supports can spare beside them.
        match editor.picker.target {
            Some(target) => {
                if self.caption_picker(
                    d,
                    font,
                    input.ui_scale,
                    editor,
                    &layout.picker,
                    target,
                    &mut style,
                ) {
                    changed = true;
                }
            }
            None => {
                if self.caption_left_column(
                    d,
                    font,
                    editor,
                    &layout,
                    imported.as_deref(),
                    &mut style,
                ) {
                    changed = true;
                }
            }
        }

        let readout_width = 44.0;
        #[rustfmt::skip]
        let sliders: [(&str, f64, f64, u8); 3] = [
            ("SIZE",  caption::SIZE_MINIMUM,   caption::SIZE_MAXIMUM,   1),
            ("WIDTH", caption::WIDTH_MINIMUM,  caption::WIDTH_MAXIMUM,  2),
            ("INSET", caption::MARGIN_MINIMUM, caption::MARGIN_MAXIMUM, 3),
        ];
        for (index, (label, minimum, maximum, drag_id)) in sliders.into_iter().enumerate() {
            let bar = layout.sliders[index];
            caption_label(d, font, label, layout.right_label_x, bar.y, bar.height);
            if bar.width < 60.0 {
                continue;
            }
            let value = match index {
                0 => &mut style.size_scale,
                1 => &mut style.width_scale,
                _ => &mut style.margin_scale,
            };
            if caption_slider(
                d,
                input.ui_scale,
                &mut editor.style_drag,
                drag_id,
                bar,
                minimum,
                maximum,
                value,
            ) {
                changed = true;
            }
            let readout = format!("{:.1}%", *value * 100.0);
            let width = widgets::measure(font, &readout, 12.0).min(readout_width);
            widgets::draw_text(
                d,
                font,
                &readout,
                layout.readout_right - width,
                bar.y + 5.0,
                12.0,
                color::ui_ink(),
            );
        }

        caption_label(
            d,
            font,
            "INK",
            layout.right_label_x,
            layout.ink.y,
            layout.ink.height,
        );
        match self.swatch_row(
            d,
            widgets::widget_id(ns::CAPTION, caption_id::INK_SWATCHES),
            layout.ink,
            &CAPTION_TEXT_SWATCHES,
            style.text_rgba,
        ) {
            Some(SwatchChoice::Packed(chosen)) => {
                style.text_rgba = chosen;
                changed = true;
            }
            Some(SwatchChoice::Custom) => editor.picker.toggle(CaptionColor::Ink),
            None => {}
        }

        // Kept live even with backing "None": switching back should restore the
        // colour that was chosen rather than reset it.
        caption_label(
            d,
            font,
            "PLATE",
            layout.right_label_x,
            layout.plate.y,
            layout.plate.height,
        );
        match self.swatch_row(
            d,
            widgets::widget_id(ns::CAPTION, caption_id::PLATE_SWATCHES),
            layout.plate,
            &CAPTION_BOX_SWATCHES,
            style.box_rgba,
        ) {
            Some(SwatchChoice::Packed(chosen)) => {
                style.box_rgba = chosen;
                changed = true;
            }
            Some(SwatchChoice::Custom) => editor.picker.toggle(CaptionColor::Plate),
            None => {}
        }

        widgets::draw_text(
            d,
            font,
            "Sizes are fractions of the frame, so exports match the preview.",
            layout.note.x,
            layout.note.y,
            12.0,
            color::ui_muted(),
        );

        if changed {
            editor.push(LyricsEdit::SetCaptionStyle(Box::new(style)));
        }
    }

    /// FACE, BACKING, PLACE and the two face-asset buttons.
    ///
    /// Returns whether the style changed. Split out of [`Self::caption_style_form`]
    /// because the free picker draws in exactly this space and the two must be
    /// mutually exclusive by construction rather than by remembering to skip.
    fn caption_left_column(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        editor: &mut LyricEditor,
        layout: &CaptionFormLayout,
        imported: Option<&str>,
        style: &mut CaptionStyle,
    ) -> bool {
        let mut changed = false;

        let face_labels: Vec<&str> = match imported {
            Some(family) => vec!["Alegreya", "Space Grotesk", family],
            None => vec!["Alegreya", "Space Grotesk"],
        };
        caption_label(
            d,
            font,
            "FACE",
            layout.label_x,
            layout.face.y,
            layout.face.height,
        );
        let face_index = index_of(CaptionFace::ALL, style.face);
        if let Some(chosen) = self.choice_row(
            d,
            font,
            widgets::widget_id(ns::CAPTION, caption_id::FACE),
            layout.face,
            &face_labels,
            face_index,
        ) {
            if let Some(face) = CaptionFace::ALL.get(chosen) {
                if *face != style.face {
                    style.face = *face;
                    changed = true;
                }
            }
        }

        let box_labels: [&str; 3] = ["None", "Shadow", "Plate"];
        caption_label(
            d,
            font,
            "BACKING",
            layout.label_x,
            layout.backing.y,
            layout.backing.height,
        );
        let box_index = index_of(CaptionBox::ALL, style.box_style);
        if let Some(chosen) = self.choice_row(
            d,
            font,
            widgets::widget_id(ns::CAPTION, caption_id::BACKING),
            layout.backing,
            &box_labels,
            box_index,
        ) {
            if let Some(value) = CaptionBox::ALL.get(chosen) {
                if *value != style.box_style {
                    style.box_style = *value;
                    changed = true;
                }
            }
        }

        // The anchor grid mirrors the frame: the top-left cell puts captions in
        // the top-left of the video. The choice is spatial, and a row of nine
        // names would be worse than nine 22 px cells. The enum counts up from the
        // bottom row because bottom-centre is the default and belongs at zero,
        // while the grid draws downward from the top (`:886-902`).
        caption_label(d, font, "PLACE", layout.label_x, layout.anchor[0].y, 66.0);
        let anchor_index = index_of(CaptionAnchor::ALL, style.anchor);
        for (cell, rect) in layout.anchor.into_iter().enumerate() {
            let value = (2 - cell / 3) * 3 + cell % 3;
            widgets::fill(
                d,
                rect,
                if anchor_index == value {
                    color::accent()
                } else {
                    color::ui_raised()
                },
            );
            d.draw_rectangle_lines_ex(widgets::rectangle(rect), 1.0, color::ui_rule());
            #[allow(
                clippy::cast_possible_truncation,
                reason = "nine cells, indexed by the loop that draws them"
            )]
            let id = widgets::widget_id(ns::CAPTION, caption_id::ANCHOR + cell as u32);
            if self.widgets.button(d, id, rect).clicked {
                if let Some(anchor) = CaptionAnchor::ALL.get(value) {
                    if *anchor != style.anchor {
                        style.anchor = *anchor;
                        changed = true;
                    }
                }
            }
        }

        // Reaching the browser from the face row is what makes the third choice
        // obtainable at all — which is why it is no longer drawn conditionally.
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ns::CAPTION, caption_id::IMPORT_FACE),
                layout.import_face,
                "Import a face...",
                false,
                ButtonStyle::Neutral,
                Some(13.0),
            )
            .clicked
        {
            editor.font_pane = true;
        }
        if imported.is_some()
            && self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(ns::CAPTION, caption_id::REMOVE_FACE),
                    layout.remove_face,
                    "Remove",
                    false,
                    ButtonStyle::Neutral,
                    Some(13.0),
                )
                .clicked
        {
            // Dropping the asset must drop the face that named it in the same
            // step, or the project would validate as an imported face with
            // nothing behind it.
            style.face = CaptionFace::Alegreya;
            style.font = None;
            changed = true;
        }

        changed
    }

    /// The free colour picker: a saturation/value square, a hue bar, an alpha bar
    /// and a hex readout, in the space the left column otherwise occupies.
    ///
    /// **Invented, and an operator decision of 2026-08-03** — the frozen C offers
    /// six ink swatches and five plate swatches and nothing else. The swatches
    /// stay as the quick picks they always were; this is the escape hatch.
    ///
    /// It claims the left column rather than opening below the swatch rows
    /// because there is nowhere below them: the tallest caption form any
    /// supported window produces is 282 px, and the swatch rows already end at
    /// 136. Replacing rather than overlaying is also what keeps the press claim
    /// honest — an overlaid picker would sit on top of buttons that were drawn
    /// first and took the press.
    ///
    /// Returns whether the style changed, which during a drag is every frame:
    /// the same behaviour the three caption sliders already have, and there is
    /// no undo stack for caption style for it to flood.
    #[allow(
        clippy::too_many_arguments,
        reason = "the caption-form shape: the face, the scale, the drag slots, the layout, the target, the style"
    )]
    fn caption_picker(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        ui_scale: super::super::scale::UiScale,
        editor: &mut LyricEditor,
        picker: &CaptionPickerLayout,
        target: CaptionColor,
        style: &mut CaptionStyle,
    ) -> bool {
        // Whatever the model holds now, including a preset swatch clicked while
        // this was open.
        editor.picker.sync(target.read(style));

        widgets::draw_text(
            d,
            font,
            target.heading(),
            picker.heading.x,
            picker.heading.y,
            12.0,
            color::accent(),
        );
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ns::CAPTION, caption_id::PICKER_DONE),
                picker.done,
                "Done",
                false,
                ButtonStyle::Neutral,
                Some(13.0),
            )
            .clicked
        {
            editor.picker.target = None;
        }

        widgets::saturation_value_field(d, picker.field, editor.picker.hue);
        d.draw_rectangle_lines_ex(widgets::rectangle(picker.field), 1.0, color::ui_rule());
        widgets::hue_bar(d, picker.hue);
        d.draw_rectangle_lines_ex(widgets::rectangle(picker.hue), 1.0, color::ui_rule());

        let (red, green, blue) = widgets::rgb_from_hsv(
            editor.picker.hue,
            editor.picker.saturation,
            editor.picker.value,
        );
        widgets::alpha_bar(d, picker.alpha, Color::new(red, green, blue, 255));
        d.draw_rectangle_lines_ex(widgets::rectangle(picker.alpha), 1.0, color::ui_rule());

        // Knobs after the fills, so none of them is painted over.
        let field_knob = Vector2::new(
            picker.field.x + editor.picker.saturation * picker.field.width,
            picker.field.y + (1.0 - editor.picker.value) * picker.field.height,
        );
        let ink = widgets::contrasting_ink(Color::new(red, green, blue, 255));
        d.draw_circle_lines(field_knob.x as i32, field_knob.y as i32, 5.0, ink);
        let hue_y = picker.hue.y + (editor.picker.hue / 360.0) * picker.hue.height;
        d.draw_rectangle_lines_ex(
            widgets::rectangle(UiRect::new(
                picker.hue.x - 2.0,
                hue_y - 3.0,
                picker.hue.width + 4.0,
                6.0,
            )),
            2.0,
            color::ui_ink(),
        );
        let alpha_x = picker.alpha.x + editor.picker.alpha * picker.alpha.width;
        d.draw_rectangle_lines_ex(
            widgets::rectangle(UiRect::new(
                alpha_x - 3.0,
                picker.alpha.y - 2.0,
                6.0,
                picker.alpha.height + 4.0,
            )),
            2.0,
            color::ui_ink(),
        );

        // The three drags. Ids 4..=6 continue the caption sliders' 1..=3 in the
        // same slot, so no two of them can be live at once — and like them, none
        // claims a widget id, because a gesture that leaves its rectangle would
        // otherwise strand the one claim the whole interface shares.
        if let Some((across, down)) =
            caption_drag(d, ui_scale, &mut editor.style_drag, 4, picker.field)
        {
            editor.picker.saturation = across;
            editor.picker.value = 1.0 - down;
        }
        if let Some((_, down)) = caption_drag(d, ui_scale, &mut editor.style_drag, 5, picker.hue) {
            // 359.99 rather than 360: they are the same colour, and letting the
            // stored hue reach 360 would make the knob leave the bottom of the bar.
            editor.picker.hue = (down * 360.0).min(359.99);
        }
        if let Some((across, _)) =
            caption_drag(d, ui_scale, &mut editor.style_drag, 6, picker.alpha)
        {
            editor.picker.alpha = across;
        }

        let (red, green, blue, alpha) = widgets::unpack_rgba(editor.picker.packed());
        widgets::draw_text(
            d,
            font,
            &format!("#{red:02X}{green:02X}{blue:02X}"),
            picker.readout.x,
            picker.readout.y,
            13.0,
            color::ui_ink(),
        );
        widgets::draw_text(
            d,
            font,
            &format!("ALPHA {:.0}%", f32::from(alpha) / 255.0 * 100.0),
            picker.readout.x,
            picker.readout.y + 18.0,
            12.0,
            color::ui_muted(),
        );

        let chosen = editor.picker.packed();
        if chosen == target.read(style) {
            return false;
        }
        target.write(style, chosen);
        // Recorded as agreed, or the next frame's `sync` would read the value
        // back out and lose the hue the drag is holding.
        editor.picker.adopt(chosen);
        true
    }

    /// The caption effects form (post-legacy, 2026-08-03): glow, drives, soft
    /// shadow and plate shape. A sibling body to [`Self::caption_style_form`]
    /// inside the same pane, sharing its footprint, drag slot and free picker.
    fn caption_effects_form(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        form: UiRect,
    ) {
        let font = input.fonts.ui();
        if form.height < CAPTION_FORM_HEIGHT || form.width < CAPTION_FORM_WIDTH {
            widgets::draw_text(
                d,
                font,
                "Enlarge the window to edit caption effects.",
                form.x,
                form.y,
                metric::UI_FONT_VALUE,
                color::ui_muted(),
            );
            return;
        }
        let mut style = track.caption_style.clone();
        let mut changed = false;
        let layout = CaptionEffectsLayout::compose(form);

        // The left column, or the glow picker in its place — the same
        // mutual exclusion the style form has, for the same press-claim reason.
        // An Ink or Plate picker cannot be open here: the pane toggle closes the
        // picker, and this form's only opener targets Glow.
        if editor.picker.target == Some(CaptionColor::Glow) {
            if self.caption_picker(
                d,
                font,
                input.ui_scale,
                editor,
                &layout.picker,
                CaptionColor::Glow,
                &mut style,
            ) {
                changed = true;
            }
        } else {
            changed |= self.effects_left_column(d, input, editor, &layout, &mut style);
        }

        // The right column: the hue drive and four sliders, drawn regardless of
        // the picker so a colour drag shows its consequence live.
        caption_label(
            d,
            font,
            "HUE",
            layout.right_label_x,
            layout.hue.y,
            layout.hue.height,
        );
        let hue_index = index_of(EffectDrive::ALL, style.effects.glow_hue_drive);
        if let Some(chosen) = self.choice_row(
            d,
            font,
            widgets::widget_id(ns::CAPTION, caption_id::HUE_DRIVE),
            layout.hue,
            &DRIVE_LABELS,
            hue_index,
        ) {
            if let Some(drive) = EffectDrive::ALL.get(chosen) {
                if *drive != style.effects.glow_hue_drive {
                    style.effects.glow_hue_drive = *drive;
                    changed = true;
                }
            }
        }

        #[rustfmt::skip]
        let right_sliders: [(&str, UiRect, f64, f64, u8, usize); 4] = [
            ("SWEEP", layout.sweep, caption_fx::HUE_RANGE_MINIMUM,      caption_fx::HUE_RANGE_MAXIMUM,      13, 0),
            ("BLUR",  layout.blur,  caption_fx::SHADOW_BLUR_MINIMUM,    caption_fx::SHADOW_BLUR_MAXIMUM,    14, 1),
            ("SHADE", layout.shade, caption_fx::SHADOW_OPACITY_MINIMUM, caption_fx::SHADOW_OPACITY_MAXIMUM, 15, 2),
            ("ROUND", layout.round, caption_fx::PLATE_ROUNDNESS_MINIMUM, caption_fx::PLATE_ROUNDNESS_MAXIMUM, 16, 3),
        ];
        for (label, bar, minimum, maximum, drag_id, which) in right_sliders {
            caption_label(d, font, label, layout.right_label_x, bar.y, bar.height);
            if bar.width < 60.0 {
                continue;
            }
            let value = match which {
                0 => &mut style.effects.glow_hue_range,
                1 => &mut style.effects.shadow_blur,
                2 => &mut style.effects.shadow_opacity,
                _ => &mut style.effects.plate_roundness,
            };
            if caption_slider(
                d,
                input.ui_scale,
                &mut editor.style_drag,
                drag_id,
                bar,
                minimum,
                maximum,
                value,
            ) {
                changed = true;
            }
            let readout = if which == 0 {
                format!("{:.0}\u{b0}", *value)
            } else {
                format!("{:.1}%", *value * 100.0)
            };
            let width = widgets::measure(font, &readout, 12.0).min(44.0);
            widgets::draw_text(
                d,
                font,
                &readout,
                layout.readout_right - width,
                bar.y + 5.0,
                12.0,
                color::ui_ink(),
            );
        }

        widgets::draw_text(
            d,
            font,
            "Drives follow the music. BLUR and SHADE soften the Shadow backing; ROUND shapes the Plate.",
            layout.note.x,
            layout.note.y,
            12.0,
            color::ui_muted(),
        );

        if changed {
            editor.push(LyricsEdit::SetCaptionStyle(Box::new(style)));
        }
    }

    /// GLOW, RADIUS, COLOR, PULSE and DEPTH — the space the glow picker claims.
    fn effects_left_column(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        layout: &CaptionEffectsLayout,
        style: &mut CaptionStyle,
    ) -> bool {
        let font = input.fonts.ui();
        let mut changed = false;

        #[rustfmt::skip]
        let sliders: [(&str, UiRect, f64, f64, u8, usize); 3] = [
            ("GLOW",   layout.glow,   caption_fx::GLOW_STRENGTH_MINIMUM, caption_fx::GLOW_STRENGTH_MAXIMUM, 10, 0),
            ("RADIUS", layout.radius, caption_fx::GLOW_RADIUS_MINIMUM,   caption_fx::GLOW_RADIUS_MAXIMUM,   11, 1),
            ("DEPTH",  layout.depth,  caption_fx::PULSE_DEPTH_MINIMUM,   caption_fx::PULSE_DEPTH_MAXIMUM,   12, 2),
        ];
        for (label, bar, minimum, maximum, drag_id, which) in sliders {
            caption_label(d, font, label, layout.label_x, bar.y, bar.height);
            if bar.width < 60.0 {
                continue;
            }
            let value = match which {
                0 => &mut style.effects.glow_strength,
                1 => &mut style.effects.glow_radius,
                _ => &mut style.effects.glow_pulse_depth,
            };
            if caption_slider(
                d,
                input.ui_scale,
                &mut editor.style_drag,
                drag_id,
                bar,
                minimum,
                maximum,
                value,
            ) {
                changed = true;
            }
            let readout = format!("{:.1}%", *value * 100.0);
            let width = widgets::measure(font, &readout, 12.0).min(44.0);
            widgets::draw_text(
                d,
                font,
                &readout,
                layout.left_readout_right - width,
                bar.y + 5.0,
                12.0,
                color::ui_ink(),
            );
        }

        // The glow colour cell: checkerboard under it so a translucent glow
        // reads as translucent, and the free picker behind a click.
        caption_label(
            d,
            font,
            "COLOR",
            layout.label_x,
            layout.color_cell.y,
            layout.color_cell.height,
        );
        widgets::checkerboard(d, layout.color_cell, layout.color_cell.width * 0.25);
        widgets::fill(
            d,
            layout.color_cell,
            Color::get_color(style.effects.glow_rgba),
        );
        d.draw_rectangle_lines_ex(widgets::rectangle(layout.color_cell), 1.0, color::ui_rule());
        if self
            .widgets
            .button(
                d,
                widgets::widget_id(ns::CAPTION, caption_id::GLOW_COLOR),
                layout.color_cell,
            )
            .clicked
        {
            editor.picker.toggle(CaptionColor::Glow);
        }
        let (red, green, blue, _) = widgets::unpack_rgba(style.effects.glow_rgba);
        widgets::draw_text(
            d,
            font,
            &format!("#{red:02X}{green:02X}{blue:02X}"),
            layout.color_cell.x + layout.color_cell.width + 8.0,
            layout.color_cell.y + 5.0,
            12.0,
            color::ui_muted(),
        );

        caption_label(
            d,
            font,
            "PULSE",
            layout.label_x,
            layout.pulse.y,
            layout.pulse.height,
        );
        let pulse_index = index_of(EffectDrive::ALL, style.effects.glow_pulse);
        if let Some(chosen) = self.choice_row(
            d,
            font,
            widgets::widget_id(ns::CAPTION, caption_id::PULSE_DRIVE),
            layout.pulse,
            &DRIVE_LABELS,
            pulse_index,
        ) {
            if let Some(drive) = EffectDrive::ALL.get(chosen) {
                if *drive != style.effects.glow_pulse {
                    style.effects.glow_pulse = *drive;
                    changed = true;
                }
            }
        }
        changed
    }

    /// One row of mutually exclusive choices (`caption_choice_row`, `:502-521`).
    fn choice_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        id: u64,
        row: UiRect,
        labels: &[&str],
        current: usize,
    ) -> Option<usize> {
        if labels.is_empty() || row.width <= 0.0 {
            return None;
        }
        let gap = 4.0;
        let width = (row.width - gap * (labels.len() - 1) as f32) / labels.len() as f32;
        if width < 24.0 {
            return None;
        }
        let widths = vec![width; labels.len()];
        let size = widgets::row_font_size(font, labels, &widths, row.height);
        let mut chosen = None;
        for (index, label) in labels.iter().enumerate() {
            let box_rect = UiRect::new(
                row.x + index as f32 * (width + gap),
                row.y,
                width,
                row.height,
            );
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    id + index as u64,
                    box_rect,
                    label,
                    index == current,
                    ButtonStyle::Neutral,
                    Some(size),
                )
                .clicked
            {
                chosen = Some(index);
            }
        }
        chosen
    }

    /// A row of colour swatches, and the custom cell that opens the free picker
    /// (`caption_swatch_row`, `:541-576`).
    ///
    /// The swatches are the oracle's and stay the quick picks they were: a
    /// caption has to be legible over moving material, so a short opinionated
    /// list is the right first offer, and the alpha is baked into each entry
    /// because "white at 40%" is a different decision from "grey". What is new is
    /// the last cell — it shows the colour currently in force, carries a `+`, and
    /// reads as selected whenever the value matches no swatch, which is the only
    /// honest way to show that a hand-picked colour is the live one.
    fn swatch_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        id: u64,
        row: UiRect,
        swatches: &[u32],
        current: u32,
    ) -> Option<SwatchChoice> {
        let gap = 4.0;
        let cells = swatches.len() + 1;
        let width = (row.width - gap * (cells - 1) as f32) / cells as f32;
        if width < 12.0 {
            return None;
        }
        let custom = !swatches.contains(&current);
        let mut chosen = None;
        for index in 0..cells {
            let box_rect = UiRect::new(
                row.x + index as f32 * (width + gap),
                row.y,
                width,
                row.height,
            );
            let is_custom = index == swatches.len();
            let packed = if is_custom { current } else { swatches[index] };
            // A checkerboard behind every swatch, so a translucent choice reads
            // as translucent rather than as a slightly different flat colour.
            widgets::checkerboard(d, box_rect, (box_rect.height * 0.5).max(2.0));
            let tint = Color::get_color(packed);
            widgets::fill(d, box_rect, tint);
            if is_custom {
                // A `+` rather than a colour wheel glyph: the icon face is not
                // guaranteed loaded (`Faces::icons_available`), and two strokes
                // drawn in whatever contrasts with the swatch cannot go missing.
                let ink = widgets::contrasting_ink(tint);
                let arm = (width * 0.28).min(box_rect.height * 0.28);
                let (cx, cy) = (
                    box_rect.x + box_rect.width * 0.5,
                    box_rect.y + box_rect.height * 0.5,
                );
                widgets::fill(d, UiRect::new(cx - arm, cy - 1.0, arm * 2.0, 2.0), ink);
                widgets::fill(d, UiRect::new(cx - 1.0, cy - arm, 2.0, arm * 2.0), ink);
            }
            let selected = if is_custom { custom } else { packed == current };
            d.draw_rectangle_lines_ex(
                widgets::rectangle(box_rect),
                if selected { 2.0 } else { 1.0 },
                if selected {
                    color::accent()
                } else {
                    color::ui_rule()
                },
            );
            if self.widgets.button(d, id + index as u64, box_rect).clicked {
                chosen = Some(if is_custom {
                    SwatchChoice::Custom
                } else {
                    SwatchChoice::Packed(packed)
                });
            }
        }
        chosen
    }
}

/// `lyric_editor_ui_apply` (`:133-164`), minus the failure branch: the draft is
/// checked before Apply is offered, so the only failures left are the ones
/// `main.rs` reports when it writes the edit.
///
/// A free function rather than a method because it touches no shell state, and
/// keeping it out of `impl Shell` is what lets the tests below drive it.
fn apply_draft(editor: &mut LyricEditor, track: &Track, edit: LyricsEdit) {
    // An insert's id is the document's next one, which is deterministic:
    // `LyricsDocument::insert` allocates `next_id` when `id == 0`. Predicting it
    // is what lets the form stay bound to the cue it just created without a round
    // trip through `main.rs` — the C gets the id back from `lyrics_insert`
    // (`:143-145`), which this panel cannot, because the write happens later.
    let inserted = matches!(edit, LyricsEdit::Insert { .. }).then(|| track.lyrics.next_id());
    editor.push(edit);
    editor.text.set_focused(false);
    match inserted {
        Some(id) => {
            // The document does not hold it yet, so the form is bound by hand
            // rather than through `select`; the lane learns about it next frame,
            // once the insert has landed.
            editor.selected_id = id;
            editor.draft_new = false;
            editor.list_follow_selection = true;
        }
        None => editor.draft_new = false,
    }
}

/// `CAPTION_TEXT_SWATCHES` (`lyrics_editor_ui.c:534-536`).
#[rustfmt::skip]
const CAPTION_TEXT_SWATCHES: [u32; 6] = [
    0xFFFF_FFFF, 0xF2BE_42FF, 0x7FB2_FFFF, 0x9BE8_B0FF, 0x1A1A_1EFF, 0xFFFF_FFC0,
];
/// `CAPTION_BOX_SWATCHES` (`:537-539`).
#[rustfmt::skip]
const CAPTION_BOX_SWATCHES: [u32; 5] = [
    0x0000_00B7, 0x0000_0066, 0x0000_00FF, 0xFFFF_FFCC, 0x1B2A_5AC0,
];

// ---------------------------------------------------------------------------
// The caption style form's geometry, composed before anything is drawn
// ---------------------------------------------------------------------------

/// Which caption colour the free picker is editing.
///
/// Two colours rather than a generic slot, because the pair is what the model
/// stores and naming them is what lets the report line and the probe say which
/// one is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionColor {
    Ink,
    Plate,
    /// The glow's base colour, which lives on the effects form rather than a
    /// swatch row — glow colours are not "few and opinionated", they are the
    /// point of the feature.
    Glow,
}

impl CaptionColor {
    /// For the report line and the `--ui-probe` key.
    const fn token(self) -> &'static str {
        match self {
            Self::Ink => "ink",
            Self::Plate => "plate",
            Self::Glow => "glow",
        }
    }

    /// The picker's own heading. It has to name the colour, because opening the
    /// picker replaces the controls that were in that space.
    const fn heading(self) -> &'static str {
        match self {
            Self::Ink => "CUSTOM INK",
            Self::Plate => "CUSTOM PLATE",
            Self::Glow => "GLOW COLOUR",
        }
    }

    const fn read(self, style: &CaptionStyle) -> u32 {
        match self {
            Self::Ink => style.text_rgba,
            Self::Plate => style.box_rgba,
            Self::Glow => style.effects.glow_rgba,
        }
    }

    fn write(self, style: &mut CaptionStyle, packed: u32) {
        match self {
            Self::Ink => style.text_rgba = packed,
            Self::Plate => style.box_rgba = packed,
            Self::Glow => style.effects.glow_rgba = packed,
        }
    }
}

/// The free colour picker's state while it is open.
///
/// **Hue, saturation and value are held here rather than re-derived from the
/// RGBA every frame, and that is the whole reason this struct exists.** Grey has
/// no hue and black has no saturation, so a picker that read its own bar
/// positions back out of the packed colour would snap to red the instant a drag
/// touched the left or the bottom edge of the square — and then stay there,
/// because the colour it wrote back has no hue to recover either.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CaptionPicker {
    /// `None` when the picker is closed, which is also its resting state: it is a
    /// disclosure under the swatch row, not a mode.
    target: Option<CaptionColor>,
    /// Degrees in `[0, 360)`.
    hue: f32,
    saturation: f32,
    value: f32,
    alpha: f32,
    /// The packed colour the four above were last agreed with, so a preset
    /// swatch clicked while the picker is open moves the picker, and the
    /// picker's own writes do not move it back.
    source: Option<u32>,
}

impl CaptionPicker {
    /// Opens on `target`, or closes if it is already the one open.
    ///
    /// Clearing `source` is what makes reopening re-derive: the same colour may
    /// have been reached by a route that lost its hue while the picker was shut.
    fn toggle(&mut self, target: CaptionColor) {
        if self.target == Some(target) {
            self.target = None;
        } else {
            self.target = Some(target);
            self.source = None;
        }
    }

    /// Adopts `packed` unless it is already the value this picker agreed with.
    fn sync(&mut self, packed: u32) {
        if self.source == Some(packed) {
            return;
        }
        let (red, green, blue, alpha) = widgets::unpack_rgba(packed);
        let (hue, saturation, value) = widgets::hsv_from_rgb(red, green, blue);
        // A colour with no chroma reports hue 0. Keeping the bar where the user
        // left it is the difference between "I dragged to black and back" and "I
        // dragged to black and lost my colour".
        if saturation > 0.0 && value > 0.0 {
            self.hue = hue;
        }
        self.saturation = saturation;
        self.value = value;
        self.alpha = f32::from(alpha) / 255.0;
        self.source = Some(packed);
    }

    /// What the three bars currently mean, as the model stores it.
    fn packed(&self) -> u32 {
        let (red, green, blue) = widgets::rgb_from_hsv(self.hue, self.saturation, self.value);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to [0, 1] before scaling"
        )]
        let alpha = (self.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        widgets::pack_rgba(red, green, blue, alpha)
    }

    /// Records `packed` as agreed, so [`Self::sync`] does not undo the drag that
    /// produced it.
    fn adopt(&mut self, packed: u32) {
        self.source = Some(packed);
    }
}

/// Every rectangle the caption style form draws, composed in one place before
/// anything is drawn.
///
/// **This exists because of the "Import a face..." defect.** The button was
/// positioned at `form.y + 158` and then drawn only `if it happened to fit`, so
/// for every form height in `[152, 184)` — which is exactly what a 960x640
/// window produces — it was silently not there, and neither was the Remove
/// button beside it. A conditional draw is a control that pretends, and this
/// repository's rule is that a missing feature says so by name. Composing the
/// geometry as a value is what lets a test assert *every* rect against the form
/// rectangle, rather than each draw call re-deriving a position nothing checks.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CaptionFormLayout {
    /// Where the left column's 12 px labels start.
    label_x: f32,
    /// Where the right column's labels start.
    right_label_x: f32,
    /// Half the form, minus the gap. The left column's controls must end inside
    /// `form.x + column_width`.
    column_width: f32,
    face: UiRect,
    backing: UiRect,
    /// Row-major from the **top**, which is not the enum's order — see the
    /// anchor loop.
    anchor: [UiRect; 9],
    import_face: UiRect,
    remove_face: UiRect,
    /// SIZE, WIDTH, INSET.
    sliders: [UiRect; 3],
    /// Where each slider's percentage readout is right-aligned to.
    readout_right: f32,
    ink: UiRect,
    plate: UiRect,
    /// The "sizes are fractions of the frame" note, moved out of the left column
    /// so the import button could come up into the space it used.
    note: UiRect,
    /// The free picker, which claims the left column when it is open.
    picker: CaptionPickerLayout,
}

/// The free picker's rectangles, all inside the left column.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CaptionPickerLayout {
    heading: UiRect,
    done: UiRect,
    /// The saturation/value square.
    field: UiRect,
    hue: UiRect,
    alpha: UiRect,
    /// The hex readout and the alpha percentage beside the square.
    readout: UiRect,
}

impl CaptionFormLayout {
    /// One choice row, one anchor cell stride, one button.
    const ROW: f32 = 26.0;
    const CELL: f32 = 22.0;
    const CELL_STRIDE: f32 = 24.0;
    /// The gutter the 12 px labels sit in, left of both columns.
    const LABEL_WIDTH: f32 = 62.0;
    const COLUMN_GAP: f32 = 24.0;

    /// How far below `form.y` the lowest control reaches.
    ///
    /// The left column is the taller one: the import row at `+134` plus its
    /// 26 px is 160, against the right column's note ending at 152 and the
    /// picker's alpha bar ending at 146. This is [`CAPTION_FORM_HEIGHT`], and a
    /// test walks every rect to prove the number is not a guess.
    const EXTENT: f32 = 160.0;

    fn compose(form: UiRect) -> Self {
        let column_width = (form.width - Self::COLUMN_GAP) * 0.5;
        let left_x = form.x + Self::LABEL_WIDTH;
        let left_width = column_width - Self::LABEL_WIDTH;
        let right_label_x = form.x + column_width + Self::COLUMN_GAP;
        let right_x = right_label_x + Self::LABEL_WIDTH;
        let right_width = column_width - Self::LABEL_WIDTH;

        let mut anchor = [UiRect::new(0.0, 0.0, 0.0, 0.0); 9];
        for (cell, rect) in anchor.iter_mut().enumerate() {
            *rect = UiRect::new(
                left_x + (cell % 3) as f32 * Self::CELL_STRIDE,
                form.y + 60.0 + (cell / 3) as f32 * Self::CELL_STRIDE,
                Self::CELL,
                Self::CELL,
            );
        }

        // 134, not the oracle's 158: dropping the note out of this column and
        // tightening the two choice rows by 2 px each is what brings the only
        // route to an imported face inside the form at the minimum window size.
        //
        // And it starts at the form's own edge rather than in the control
        // column, because the 62 px gutter is for the 12 px labels beside FACE,
        // BACKING and PLACE. A button has its own label. Spending the gutter on
        // it is what left Remove hanging 26 px past the column at the minimum
        // width — where the old code simply did not draw it, which is the same
        // silent disappearance one row up.
        let import_face = UiRect::new(form.x, form.y + 134.0, 132.0, Self::ROW);
        let remove_face = UiRect::new(
            import_face.x + import_face.width + 6.0,
            import_face.y,
            74.0,
            Self::ROW,
        );

        let readout_width = 44.0;
        let slider_width = right_width - readout_width - 6.0;
        let mut sliders = [UiRect::new(0.0, 0.0, 0.0, 0.0); 3];
        for (index, rect) in sliders.iter_mut().enumerate() {
            *rect = UiRect::new(
                right_x,
                form.y + index as f32 * Self::ROW,
                slider_width,
                Self::CELL,
            );
        }

        Self {
            label_x: form.x,
            right_label_x,
            column_width,
            face: UiRect::new(left_x, form.y, left_width, Self::ROW),
            backing: UiRect::new(left_x, form.y + 30.0, left_width, Self::ROW),
            anchor,
            import_face,
            remove_face,
            sliders,
            readout_right: right_x + right_width,
            ink: UiRect::new(right_x, form.y + 86.0, right_width, Self::CELL),
            plate: UiRect::new(right_x, form.y + 114.0, right_width, Self::CELL),
            note: UiRect::new(right_label_x, form.y + 140.0, column_width, 12.0),
            picker: CaptionPickerLayout {
                heading: UiRect::new(form.x, form.y + 4.0, column_width - 62.0, 12.0),
                done: UiRect::new(form.x + column_width - 56.0, form.y, 56.0, Self::CELL),
                field: UiRect::new(form.x, form.y + 26.0, 96.0, 96.0),
                hue: UiRect::new(form.x + 104.0, form.y + 26.0, 16.0, 96.0),
                alpha: UiRect::new(form.x, form.y + 128.0, 120.0, 18.0),
                readout: UiRect::new(form.x + 128.0, form.y + 26.0, column_width - 128.0, 34.0),
            },
        }
    }

    /// Every rectangle in draw order, for the containment test.
    ///
    /// The picker's rects are included even though they and the left column's are
    /// never drawn in the same frame: both have to fit, and a list that omitted
    /// one state would be the same blind spot the import button lived in.
    #[cfg(test)]
    fn every_rect(&self) -> Vec<(&'static str, UiRect)> {
        let mut rects = vec![
            ("face", self.face),
            ("backing", self.backing),
            ("import_face", self.import_face),
            ("remove_face", self.remove_face),
            ("ink", self.ink),
            ("plate", self.plate),
            ("note", self.note),
            ("picker.heading", self.picker.heading),
            ("picker.done", self.picker.done),
            ("picker.field", self.picker.field),
            ("picker.hue", self.picker.hue),
            ("picker.alpha", self.picker.alpha),
            ("picker.readout", self.picker.readout),
        ];
        for rect in self.anchor {
            rects.push(("anchor", rect));
        }
        for rect in self.sliders {
            rects.push(("slider", rect));
        }
        rects
    }
}

/// Every rectangle the caption *effects* form draws, composed like
/// [`CaptionFormLayout`] and for the same reason: a control positioned inline
/// and drawn "if it fits" is the import-button defect again. The form shares
/// the style form's footprint ([`CaptionFormLayout::EXTENT`]), and a test walks
/// every rect to keep that true.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CaptionEffectsLayout {
    label_x: f32,
    right_label_x: f32,
    /// The left column's right edge, for the slider readouts.
    left_readout_right: f32,
    /// The right column's right edge.
    readout_right: f32,
    /// Glow strength.
    glow: UiRect,
    /// Glow radius, as a fraction of the font size.
    radius: UiRect,
    /// The glow colour cell; clicking it opens the free picker on
    /// [`CaptionColor::Glow`].
    color_cell: UiRect,
    /// The pulse drive choice row.
    pulse: UiRect,
    /// Pulse depth.
    depth: UiRect,
    /// The hue drive choice row.
    hue: UiRect,
    /// Hue sweep, in degrees.
    sweep: UiRect,
    /// Shadow blur.
    blur: UiRect,
    /// Shadow opacity.
    shade: UiRect,
    /// Plate roundness.
    round: UiRect,
    note: UiRect,
    /// The free picker claims the left column here exactly as it does on the
    /// style form, so the rects are the same ones.
    picker: CaptionPickerLayout,
}

impl CaptionEffectsLayout {
    fn compose(form: UiRect) -> Self {
        const ROW: f32 = CaptionFormLayout::ROW;
        let column_width = (form.width - CaptionFormLayout::COLUMN_GAP) * 0.5;
        let left_x = form.x + CaptionFormLayout::LABEL_WIDTH;
        let left_width = column_width - CaptionFormLayout::LABEL_WIDTH;
        let right_label_x = form.x + column_width + CaptionFormLayout::COLUMN_GAP;
        let right_x = right_label_x + CaptionFormLayout::LABEL_WIDTH;
        let right_width = column_width - CaptionFormLayout::LABEL_WIDTH;

        let readout_width = 44.0;
        let left_slider = left_width - readout_width - 6.0;
        let right_slider = right_width - readout_width - 6.0;
        Self {
            label_x: form.x,
            right_label_x,
            left_readout_right: left_x + left_width,
            readout_right: right_x + right_width,
            glow: UiRect::new(left_x, form.y, left_slider, CaptionFormLayout::CELL),
            radius: UiRect::new(left_x, form.y + ROW, left_slider, CaptionFormLayout::CELL),
            color_cell: UiRect::new(
                left_x,
                form.y + ROW * 2.0,
                CaptionFormLayout::CELL,
                CaptionFormLayout::CELL,
            ),
            pulse: UiRect::new(left_x, form.y + 82.0, left_width, CaptionFormLayout::ROW),
            depth: UiRect::new(left_x, form.y + 108.0, left_slider, CaptionFormLayout::CELL),
            hue: UiRect::new(right_x, form.y, right_width, CaptionFormLayout::ROW),
            sweep: UiRect::new(right_x, form.y + ROW, right_slider, CaptionFormLayout::CELL),
            blur: UiRect::new(
                right_x,
                form.y + ROW * 2.0,
                right_slider,
                CaptionFormLayout::CELL,
            ),
            shade: UiRect::new(
                right_x,
                form.y + ROW * 3.0,
                right_slider,
                CaptionFormLayout::CELL,
            ),
            round: UiRect::new(
                right_x,
                form.y + ROW * 4.0,
                right_slider,
                CaptionFormLayout::CELL,
            ),
            note: UiRect::new(form.x, form.y + 140.0, form.width, 12.0),
            picker: CaptionFormLayout::compose(form).picker,
        }
    }

    /// Every rectangle in draw order, for the containment test.
    #[cfg(test)]
    fn every_rect(&self) -> Vec<(&'static str, UiRect)> {
        vec![
            ("glow", self.glow),
            ("radius", self.radius),
            ("color_cell", self.color_cell),
            ("pulse", self.pulse),
            ("depth", self.depth),
            ("hue", self.hue),
            ("sweep", self.sweep),
            ("blur", self.blur),
            ("shade", self.shade),
            ("round", self.round),
            ("note", self.note),
            ("picker.heading", self.picker.heading),
            ("picker.done", self.picker.done),
            ("picker.field", self.picker.field),
            ("picker.hue", self.picker.hue),
            ("picker.alpha", self.picker.alpha),
            ("picker.readout", self.picker.readout),
        ]
    }
}

/// The drive rows' labels, in [`EffectDrive::ALL`] order.
const DRIVE_LABELS: [&str; 6] = ["Off", "RMS", "Bass", "Beat", "Flux", "Time"];

/// What a swatch row press asked for.
///
/// A named pair rather than a sentinel packed value: every `u32` is a colour
/// somebody may legitimately have chosen, so "the custom cell" cannot be encoded
/// as one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwatchChoice {
    Packed(u32),
    Custom,
}

/// Position of `value` in a `named_enum!`'s `ALL`, which is the C enum's order
/// and therefore the index a choice row uses.
fn index_of<T: PartialEq + Copy>(all: &[T], value: T) -> usize {
    all.iter().position(|entry| *entry == value).unwrap_or(0)
}

/// A muted 12 px label, vertically centred against a control (`caption_label`,
/// `:523-529`).
fn caption_label(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    text: &str,
    x: f32,
    y: f32,
    height: f32,
) {
    widgets::draw_text(
        d,
        font,
        text,
        x,
        y + (height - 12.0) * 0.5,
        12.0,
        color::ui_muted(),
    );
}

/// A plain slider (`caption_slider`, `:459-499`).
///
/// Deliberately not [`super::super::widgets::Widgets::slider`]: that one claims a
/// widget id, and a slider drag holding the claim every button in the
/// application needs is the stranded-claim failure this codebase already knows
/// about. The C makes the same choice for the same reason and says so — it owns
/// no `active_button_id`, so nothing can be stranded here.
#[allow(
    clippy::too_many_arguments,
    reason = "the oracle's `caption_slider` shape: the drag slot, its id, the track, the range, the value"
)]
fn caption_slider(
    d: &mut RaylibDrawHandle<'_>,
    ui_scale: super::super::scale::UiScale,
    drag: &mut u8,
    id: u8,
    track_rect: UiRect,
    minimum: f64,
    maximum: f64,
    value: &mut f64,
) -> bool {
    use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

    let radius = track_rect.height * 0.5;
    let left = track_rect.x + radius;
    let span = track_rect.width - radius * 2.0;
    if span <= 0.0 {
        return false;
    }
    let range = maximum - minimum;
    if range <= 0.0 {
        return false;
    }
    let fraction = (((*value - minimum) / range) as f32).clamp(0.0, 1.0);

    d.draw_line_ex(
        Vector2::new(left, track_rect.y + radius),
        Vector2::new(left + span, track_rect.y + radius),
        2.0,
        color::ui_rule(),
    );
    let handle_x = left + fraction * span;
    d.draw_line_ex(
        Vector2::new(left, track_rect.y + radius),
        Vector2::new(handle_x, track_rect.y + radius),
        2.0,
        color::accent(),
    );
    d.draw_circle_v(
        Vector2::new(handle_x, track_rect.y + radius),
        radius * 0.62,
        color::accent(),
    );

    let mouse = ui_scale.mouse(d);
    if *drag == 0
        && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT)
        && track_rect.contains_point(mouse.x, mouse.y)
    {
        *drag = id;
    }
    if *drag != id {
        return false;
    }
    // Released, or the button went up while the window was unfocused. This
    // widget owns no claim, so nothing can be stranded, but the drag must still
    // end on its own.
    if !d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
        *drag = 0;
        return false;
    }
    let moved = (minimum + f64::from((mouse.x - left) / span) * range).clamp(minimum, maximum);
    if moved == *value {
        return false;
    }
    *value = moved;
    true
}

/// A press-and-drag anywhere inside `rect`, reported as a position in `[0, 1]`
/// on both axes.
///
/// The two-dimensional sibling of [`caption_slider`], and it shares the drag slot
/// and the reason: it claims no widget id, so a gesture that wanders out of the
/// square cannot strand the one `active_button_id` every button in the
/// application needs. It is also why the picker's three surfaces cannot be
/// dragged at once — one slot, one gesture.
///
/// `None` means "this drag is not yours", including on the frame the button comes
/// up; the caller decides whether the position it last saw was a change.
fn caption_drag(
    d: &mut RaylibDrawHandle<'_>,
    ui_scale: super::super::scale::UiScale,
    drag: &mut u8,
    id: u8,
    rect: UiRect,
) -> Option<(f32, f32)> {
    use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

    if rect.is_empty() {
        return None;
    }
    let mouse = ui_scale.mouse(d);
    if *drag == 0
        && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT)
        && rect.contains_point(mouse.x, mouse.y)
    {
        *drag = id;
    }
    if *drag != id {
        return None;
    }
    // Released, or the button went up while the window was unfocused.
    if !d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
        *drag = 0;
        return None;
    }
    Some((
        ((mouse.x - rect.x) / rect.width).clamp(0.0, 1.0),
        ((mouse.y - rect.y) / rect.height).clamp(0.0, 1.0),
    ))
}

/// Where a cue should be drawn right now: its stored timing, unless the current
/// drag proposes something else (`lane_preview_span`, `:298-315`).
///
/// Preview and commit read the same clamped numbers, so what the blocks show
/// during a drag is what gets written.
fn lane_preview_span(editor: &LyricEditor, cue: &LyricCue) -> (f64, f64) {
    let (mut start, mut end) = (cue.start_seconds, cue.end_seconds);
    if !editor.lane_drag.moved {
        return (start, end);
    }
    match editor.lane_drag.zone {
        Some(LyricLaneZone::Body) => {
            if editor.lane_selection.contains(cue.id) {
                start += editor.lane_drag.delta_seconds;
                end += editor.lane_drag.delta_seconds;
            }
        }
        Some(LyricLaneZone::StartEdge) if cue.id == editor.lane_drag.id => {
            start = editor.lane_drag.edge_seconds;
        }
        Some(LyricLaneZone::EndEdge) if cue.id == editor.lane_drag.id => {
            end = editor.lane_drag.edge_seconds;
        }
        _ => {}
    }
    (start, end)
}

/// The form's one-line warning for a cue a later one talks over (review 1.14).
///
/// Cue numbers are 1-based canonical indices, which is how [`LyricsValidation`]
/// already names a cue to the user (`lyrics.rs`'s `Display`).
///
/// [`LyricsValidation`]: musializer_core::project::lyrics::LyricsValidation
fn shadow_notice(document: &LyricsDocument, id: u64) -> Option<String> {
    let index = document.index_of(id)?;
    let cue = document.cues().get(index)?;
    let shadow = document.cue_shadow(index)?;
    let number = shadow.shadowed_by_index + 1;
    let from = widgets::format_timestamp(shadow.from_seconds());
    if shadow.fully {
        return Some(format!("Overlaps cue {number} — this line never displays"));
    }
    // A cue that reappears after the overlap gets both ends named; saying only
    // "after" would claim the rest of the line is lost as well.
    let (_, until) = shadow.hidden[0];
    if shadow.hidden.len() == 1 && until >= cue.end_seconds {
        Some(format!(
            "Overlaps cue {number} — this line will not display after {from}"
        ))
    } else {
        Some(format!(
            "Overlaps cue {number} — this line will not display from {from} to {}",
            widgets::format_timestamp(until)
        ))
    }
}

/// Spacing between the hatch strokes that mark a shadowed span.
const HATCH_SPACING: f32 = 5.0;

/// Diagonal hatching inside `rect`, clipped by construction (review 1.14).
///
/// No scissor: each stroke is a segment of the 45° line through the rect's bottom
/// edge, and the parameter range is solved for the rect rather than drawn long
/// and cut. A lane block is a few pixels wide on a long track, and starting a
/// scissor per block would be a state change per cue per frame.
fn hatch(d: &mut RaylibDrawHandle<'_>, rect: UiRect, tint: Color) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let bottom = rect.y + rect.height;
    let mut origin = rect.x - rect.height;
    while origin < rect.x + rect.width {
        // Points on the stroke are `(origin + t, bottom - t)` for `t` in
        // `[0, height]`; these two bounds are where it is also inside the rect.
        let low = (rect.x - origin).max(0.0);
        let high = (rect.x + rect.width - origin).min(rect.height);
        if high > low {
            d.draw_line_ex(
                Vector2::new(origin + low, bottom - low),
                Vector2::new(origin + high, bottom - high),
                1.0,
                tint,
            );
        }
        origin += HATCH_SPACING;
    }
}

/// raylib's `ColorAlpha`, for a compile-time colour.
fn fade(color: Color, alpha: f32) -> Color {
    Color::new(
        color.r,
        color.g,
        color.b,
        (alpha.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use musializer_core::scene::SceneId;

    use super::*;

    fn document(count: usize) -> LyricsDocument {
        let mut document = LyricsDocument::new(120.0).expect("a positive duration");
        for index in 0..count {
            let start = 10.0 + index as f64 * 5.0;
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: start + 2.0,
                    text: format!("line {index}"),
                })
                .expect("the fixture cues are valid");
        }
        document
    }

    #[test]
    fn lyric_hover_requires_both_axes_of_the_actual_cue_lane() {
        let lane = UiRect::new(100.0, 400.0, 500.0, 22.0);
        assert!(lyric_lane_hover_allowed(
            lane,
            Vector2::new(250.0, 410.0),
            false
        ));
        // Same X as a cue, but in the scene preview and PCM lane respectively.
        assert!(!lyric_lane_hover_allowed(
            lane,
            Vector2::new(250.0, 200.0),
            false
        ));
        assert!(!lyric_lane_hover_allowed(
            lane,
            Vector2::new(250.0, 360.0),
            false
        ));
        assert!(!lyric_lane_hover_allowed(
            lane,
            Vector2::new(250.0, 410.0),
            true
        ));
    }

    #[test]
    fn binding_to_a_cue_copies_it_and_leaves_the_draft_clean() {
        let document = document(3);
        let id = document.cues()[1].id;
        let mut editor = LyricEditor::new();
        editor.select_single(&document, id);
        assert_eq!(editor.selected_id, id);
        assert_eq!(editor.text.edit.text(), "line 1");
        assert!(!editor.has_unsaved_draft(&document));
        // The lane agrees about what is selected, which is what stops a block
        // being drawn bound-but-unselected and a drag then moving nothing.
        assert!(editor.describe(None) == "no track");
    }

    #[test]
    fn a_typed_character_makes_the_draft_dirty_and_apply_available() {
        let document = document(3);
        let id = document.cues()[0].id;
        let mut editor = LyricEditor::new();
        editor.select_single(&document, id);
        editor
            .text
            .edit
            .insert_char('!')
            .expect("a plain character");
        assert!(editor.has_unsaved_draft(&document));
        assert_eq!(
            editor.draft_edit(),
            Some(LyricsEdit::Update {
                id,
                start_seconds: 10.0,
                end_seconds: 12.0,
                text: "line 0!".to_string(),
            })
        );
    }

    #[test]
    fn an_empty_buffer_offers_no_apply_because_the_model_would_refuse_it() {
        // The one case the field must allow and the model must not: you cannot
        // retype a cue without first emptying it. `validate_text` rejects empty
        // text, so Apply is withheld rather than offered and then answered with
        // an error notice.
        let document = document(1);
        let id = document.cues()[0].id;
        let mut editor = LyricEditor::new();
        editor.select_single(&document, id);
        editor.text.edit.select_all();
        assert!(editor.text.edit.backspace());
        assert_eq!(editor.draft_edit(), None);
        assert!(editor.has_unsaved_draft(&document));
    }

    #[test]
    fn a_new_cue_starts_at_the_playhead_and_is_always_dirty() {
        let document = document(0);
        let mut editor = LyricEditor::new();
        editor.begin_new(&document, 30.0);
        assert!(editor.draft_new);
        assert_eq!(editor.draft_start, 30.0);
        assert_eq!(editor.draft_end, 32.0);
        // Nothing to compare against, so discarding it would lose everything
        // typed: dirty by rule (`editor_draft.c:5-15`).
        assert!(editor.has_unsaved_draft(&document));
        // ... but not applicable until it has text.
        assert_eq!(editor.draft_edit(), None);
        editor.text.edit.insert("hold on").expect("plain text");
        assert!(matches!(
            editor.draft_edit(),
            Some(LyricsEdit::Insert { .. })
        ));
    }

    #[test]
    fn a_new_cue_near_the_end_backs_off_rather_than_running_past_it() {
        // `lyric_editor_ui_begin_new`'s guard (`:124`).
        let document = document(0);
        let mut editor = LyricEditor::new();
        editor.begin_new(&document, 119.99);
        assert_eq!(editor.draft_start, 118.0);
        assert_eq!(editor.draft_end, 120.0);
    }

    #[test]
    fn the_form_predicts_exactly_the_id_the_document_will_allocate() {
        // What lets the form stay bound to a cue it created a frame before the
        // insert lands. If these ever diverge the form silently binds to nothing
        // and the next Apply writes an Update against a missing id.
        let mut document = document(3);
        let predicted = document.next_id();
        let allocated = document
            .insert(LyricCue {
                id: 0,
                start_seconds: 1.0,
                end_seconds: 3.0,
                text: "new".to_string(),
            })
            .expect("a valid cue");
        assert_eq!(predicted, allocated);
    }

    #[test]
    fn every_edit_the_panel_can_produce_goes_through_the_models_own_gate() {
        // The panel is optimistic about geometry it clamped a frame ago, which is
        // only safe because every variant lands through a validating entry point.
        let mut document = document(3);
        let ids: Vec<u64> = document.cues().iter().map(|cue| cue.id).collect();
        assert!(document.shift_many(&ids, 1.0).is_ok());
        assert!(document.retime(ids[0], 9.0, 13.0).is_ok());
        assert!(document.update(ids[1], 20.0, 22.0, "changed").is_ok());
        assert!(document.delete(ids[2]).is_ok());
        assert!(document.validate().is_ok());
        // And the refusals the panel relies on being refusals.
        assert!(document.retime(ids[0], 5.0, 4.0).is_err());
        assert!(document.update(ids[0], 0.0, 1.0, "").is_err());
    }

    #[test]
    fn the_panel_asks_for_a_band_its_form_actually_fits_in() {
        // The definition of done's two sizes and everything between, and the
        // whole reason `lyrics_editor_layout` exists: the shipped C drew Apply,
        // Discard and Delete below the bottom of the framebuffer at every
        // supported window size.
        let mut editor = LyricEditor::new();
        for count in [0usize, 1, 8, 64, 1024] {
            editor.cue_count = count;
            for window in [640.0f32, 720.0, 800.0, 1080.0, 1440.0, 2160.0] {
                let band = editor.timeline_height(window, 0.0);
                let panel = band - CHROME_ABOVE_PANEL - PANEL_BOTTOM_PADDING;
                let boundary = panel - LANE_HEIGHT - LANE_GAP;
                assert!(
                    lyrics_editor_layout::form_fits(boundary),
                    "{window}px window, {count} cues: the form got {boundary}px"
                );
                // And the band never eats the whole window: the preview is what
                // the application exists to show.
                assert!(
                    band + metric::HUD_BUTTON_SIZE < window,
                    "{window}px window: the band asked for {band}px"
                );
            }
        }
    }

    #[test]
    fn a_taller_window_shows_more_cues_rather_than_a_fixed_three() {
        let mut editor = LyricEditor::new();
        editor.cue_count = 12;
        let at_640 = editor.timeline_height(640.0, 0.0);
        let at_720 = editor.timeline_height(720.0, 0.0);
        let at_1080 = editor.timeline_height(1080.0, 0.0);
        assert!(at_720 > at_640);
        assert!(at_1080 > at_720);
    }

    #[test]
    fn pending_edits_are_taken_once() {
        // Applying a ShiftMany twice moves the cues twice, so the drain is the
        // contract rather than a convenience.
        let mut editor = LyricEditor::new();
        editor.enter_track(Some(0));
        editor.push(LyricsEdit::Delete { id: 4 });
        assert!(editor.has_pending());
        assert_eq!(editor.take_pending(Some(0)).edits.len(), 1);
        assert!(!editor.has_pending());
        assert!(editor.take_pending(Some(0)).edits.is_empty());
    }

    fn test_track(name: &str, cues: usize) -> Track {
        let mut track = Track::new(PathBuf::from(name), 120.0, SceneId::Spectrum, 7)
            .expect("120s is a valid duration");
        track.lyrics = document(cues);
        track
    }

    /// review 1.3 (UX0-A03). Cue ids restart at 1 in every document, so track A's
    /// cue #3 and track B's cue #3 are the same `u64`. The editor remembered only
    /// that number, and the deferred Update landed on whichever track was current
    /// when `main.rs` drained — overwriting a lyric the user never opened, or
    /// failing with a raw out-of-range error when B was shorter.
    #[test]
    fn an_edit_authored_on_one_track_is_refused_against_another() {
        let a = test_track("/tmp/a.wav", 4);
        // Deliberately shorter: the two documents share ids 1..=2 and B has no
        // #3 at all, which is the variant that used to surface as a raw error.
        let b = test_track("/tmp/b.wav", 2);
        let before = b.lyrics.clone();

        let mut editor = LyricEditor::new();
        let id = a.lyrics.cues()[2].id;
        editor.open_dirty_draft_for_test(0, &a.lyrics, id);
        editor.apply_draft_for_test(&a);
        assert!(editor.has_pending());

        // The old bug route: the workspace moved on without the guard, and the
        // drain happened afterwards.
        let drain = editor.take_pending(Some(1));
        assert_eq!(drain.refused, 1);
        assert!(
            drain.edits.is_empty(),
            "an edit owned by slot 0 reached slot 1"
        );

        // Nothing the drain yielded can touch B, so B is what it was.
        let mut b = b;
        for edit in drain.edits {
            let _ = edit.apply(&mut b);
        }
        assert_eq!(b.lyrics, before);
    }

    /// The other half: the owner matching is not incidentally always false.
    #[test]
    fn an_edit_reaches_the_track_that_authored_it() {
        let mut a = test_track("/tmp/a.wav", 4);
        let id = a.lyrics.cues()[2].id;
        let mut editor = LyricEditor::new();
        editor.open_dirty_draft_for_test(0, &a.lyrics, id);
        editor.apply_draft_for_test(&a);

        let drain = editor.take_pending(Some(0));
        assert_eq!(drain.refused, 0);
        assert_eq!(drain.edits.len(), 1);
        for edit in drain.edits {
            edit.apply(&mut a).expect("the model accepts its own draft");
        }
        assert_eq!(a.lyrics.find(id).expect("the cue survives").text, "line 2!");
    }

    /// Entering a track is the point the binding can never be stale past
    /// (`plug.c:5274`), so it has to drop the draft rather than reinterpret it.
    #[test]
    fn entering_another_track_drops_a_draft_it_does_not_own() {
        let a = test_track("/tmp/a.wav", 4);
        let mut editor = LyricEditor::new();
        let id = a.lyrics.cues()[2].id;
        editor.open_dirty_draft_for_test(0, &a.lyrics, id);
        assert_eq!(editor.draft_owner(), Some(0));

        editor.enter_track(Some(1));
        assert_eq!(editor.draft_owner(), Some(1));
        assert_eq!(editor.selected_id, 0);
        assert!(!editor.draft_new);
        assert!(editor.lane_selection.is_empty());

        // And re-entering the track already owned changes nothing, because a
        // clear on every frame would delete the draft as fast as it is typed.
        editor.open_dirty_draft_for_test(1, &a.lyrics, id);
        editor.enter_track(Some(1));
        assert_eq!(editor.selected_id, id);
    }

    /// A new cue is dirty by rule, so dropping one is unsaved work and the panel
    /// says so; an unbound editor has nothing to report.
    #[test]
    fn only_a_dropped_new_cue_is_reported_as_lost() {
        let a = test_track("/tmp/a.wav", 4);
        let mut editor = LyricEditor::new();
        editor.enter_track(Some(0));
        editor.begin_new(&a.lyrics, 30.0);
        assert!(editor.enter_track(Some(1)));

        let mut editor = LyricEditor::new();
        editor.open_dirty_draft_for_test(0, &a.lyrics, a.lyrics.cues()[0].id);
        assert!(
            !editor.enter_track(Some(1)),
            "an edit to a stored cue is still in the document it came from"
        );
    }

    /// Requirement 3 of UX0-A03: Apply, Discard and Cancel each have to leave the
    /// draft clean, or the guard they exist to satisfy stays shut behind them.
    #[test]
    fn apply_discard_and_cancel_all_leave_the_draft_clean() {
        let mut a = test_track("/tmp/a.wav", 4);
        let id = a.lyrics.cues()[1].id;

        // Apply, then the write `main.rs` performs at the end of the frame.
        let mut editor = LyricEditor::new();
        editor.open_dirty_draft_for_test(0, &a.lyrics, id);
        editor.apply_draft_for_test(&a);
        for edit in editor.take_pending(Some(0)).edits {
            edit.apply(&mut a).expect("the model accepts its own draft");
        }
        assert!(!editor.has_unsaved_draft(&a.lyrics), "Apply left it dirty");

        // Discard, on an existing cue.
        editor.open_dirty_draft_for_test(0, &a.lyrics, id);
        editor.clear_draft();
        assert!(
            !editor.has_unsaved_draft(&a.lyrics),
            "Discard left it dirty"
        );

        // Cancel, on a new cue that was never applied.
        editor.begin_new(&a.lyrics, 30.0);
        assert!(editor.has_unsaved_draft(&a.lyrics));
        editor.clear_draft();
        assert!(!editor.has_unsaved_draft(&a.lyrics), "Cancel left it dirty");
    }

    #[test]
    fn every_lyrics_form_control_has_a_distinct_widget_id() {
        let mut ids = vec![
            form_id::APPLY,
            form_id::DISCARD,
            form_id::DELETE,
            form_id::START_TIME,
            form_id::START_TIME + 1,
            form_id::START_TIME + 2,
            form_id::END_TIME,
            form_id::END_TIME + 1,
            form_id::END_TIME + 2,
        ];
        let control_count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), control_count);
    }

    #[test]
    fn the_report_line_says_which_state_the_panel_is_in() {
        // "evidence, not existence": a capture script asserts on this rather than
        // on a picture.
        let editor = LyricEditor::new();
        assert_eq!(editor.describe(None), "no track");
    }

    /// UX0-A05, as a number a headless run can assert.
    ///
    /// The reported picture was a preview typesetting the line perfectly while
    /// the cue list and the field one panel below showed rows of `?`. Both
    /// surfaces held the same string, so the only variable was the atlas — which
    /// is exactly what this drives: the same cues through two repertoires.
    #[test]
    fn authored_text_evidence_counts_what_the_serving_face_cannot_draw() {
        const GREEK: &str = "Καλημέρα κόσμε";
        const CYRILLIC: &str = "мир продолжает звучать";

        // The fix: the caption atlas is requested over the full curated set.
        let served = authored_evidence(
            ("caption", GlyphRepertoire::Curated),
            ("caption", GlyphRepertoire::Curated),
            [GREEK, CYRILLIC],
            "ещё одна строка",
        );
        assert_eq!(served.rows, "caption");
        assert_eq!(served.field, "caption");
        assert_eq!(served.missing, 0);

        // The negative control, which is the defect itself rather than a
        // synthetic perturbation: force the chrome atlas back onto authored text
        // and the count must not stay at zero. If this ever reads 0, the
        // interface subset has quietly grown and `missing=` has stopped being
        // evidence of anything.
        let starved = authored_evidence(
            ("ui", GlyphRepertoire::InterfaceSubset),
            ("ui", GlyphRepertoire::InterfaceSubset),
            [GREEK, CYRILLIC],
            "ещё одна строка",
        );
        assert_eq!(starved.rows, "ui");
        assert!(
            starved.missing > 0,
            "the chrome atlas cannot draw Greek or Cyrillic, so the count must say so"
        );
        // Every letter, and only the letters: the spaces survive either way, and
        // so does an em dash, which is why the bug report described a line of
        // question marks with the dash still in it.
        assert_eq!(
            starved.missing,
            [GREEK, CYRILLIC, "ещё одна строка"]
                .iter()
                .flat_map(|text| text.chars())
                .filter(|ch| !ch.is_ascii())
                .count()
        );

        // Half-fixed is the state this change actually ships in, because the
        // field's face is threaded in `ui/text_input.rs`. It must report as
        // half-fixed rather than as done.
        let partial = authored_evidence(
            ("caption", GlyphRepertoire::Curated),
            ("ui", GlyphRepertoire::InterfaceSubset),
            [GREEK, CYRILLIC],
            "ещё одна строка",
        );
        assert_eq!(partial.rows, "caption");
        assert_eq!(partial.field, "ui");
        assert_eq!(partial.missing, "ещё одна строка".chars().count() - 2);

        // A Latin document is clean through either atlas, so `missing=0` on an
        // ASCII fixture proves nothing and the gate has to use a non-Latin one.
        assert_eq!(
            authored_evidence(
                ("ui", GlyphRepertoire::InterfaceSubset),
                ("ui", GlyphRepertoire::InterfaceSubset),
                ["plain latin lyric"],
                "",
            )
            .missing,
            0
        );
    }

    #[test]
    fn the_report_line_names_the_face_that_served_the_user_s_own_words() {
        let mut document = LyricsDocument::new(120.0).expect("a positive duration");
        document
            .insert(LyricCue {
                id: 0,
                start_seconds: 0.0,
                end_seconds: 1.0,
                text: "Καλημέρα κόσμε".to_string(),
            })
            .expect("the model accepts a Greek cue");
        let mut track = test_track("/tmp/greek.wav", 0);
        track.lyrics = document;

        let mut editor = LyricEditor::new();
        // Before any frame the panel must not claim a face it never used.
        assert!(editor
            .describe(Some(&track))
            .contains("face=none field=none"));

        editor.authored = authored_evidence(
            ("caption", GlyphRepertoire::Curated),
            ("caption", GlyphRepertoire::Curated),
            track.lyrics.cues().iter().map(|cue| cue.text.as_str()),
            "",
        );
        let line = editor.describe(Some(&track));
        assert!(
            line.contains("face=caption field=caption missing=0"),
            "{line:?}"
        );
    }

    /// Review 1.1: selecting a cue shorter than a millisecond used to panic the
    /// process on the next frame that drew the timing rows.
    ///
    /// `validate_cue` asks only for `end > start`, so every cue built here is one
    /// a `.musi`, a TSV import or an aligner can hand the editor. The rows
    /// themselves need a draw handle, so this drives the arithmetic they now run
    /// — and then feeds the result back through the model's own gate, because
    /// "did not panic" is not the same as "produced a cue that can be applied".
    #[test]
    fn a_sub_millisecond_cue_can_be_selected_and_nudged_without_panicking() {
        let mut document = LyricsDocument::new(120.0).expect("a positive duration");
        // Half a millisecond long, at the start of the track and at its end, plus
        // two neighbours half a millisecond apart.
        for (start, end) in [
            (0.0, 0.000_5),
            (60.0, 60.000_5),
            (60.001, 62.0),
            (119.999_5, 120.0),
        ] {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: end,
                    text: "tight".to_string(),
                })
                .expect("the model accepts every one of these");
        }

        let mut editor = LyricEditor::new();
        for cue in document.cues() {
            editor.select_single(&document, cue.id);
            // The rows clamp every frame, and both nudge buttons on both rows.
            for nudge in [0.0, 0.1, -0.1] {
                let start =
                    lyric_lane_edit::clamp_form_start(editor.draft_start + nudge, editor.draft_end);
                let end = lyric_lane_edit::clamp_form_end(
                    editor.draft_end + nudge,
                    start,
                    document.duration_seconds(),
                );
                assert!(start >= 0.0 && end > start && end <= document.duration_seconds());
                let mut applied = document.clone();
                applied
                    .update(cue.id, start, end, "tight")
                    .expect("what the rows produce is what the model takes");
            }
        }
    }

    /// Review 1.14, the `Add cue` half.
    #[test]
    fn a_new_cues_default_end_stops_at_the_next_cue() {
        let document = document(3);
        let mut editor = LyricEditor::new();
        // Cues sit at 10, 15 and 20 s. Two seconds from 14 s would bury the one
        // at 15 s under the new line for its whole length.
        editor.begin_new(&document, 14.0);
        assert_eq!(editor.draft_start, 14.0);
        assert_eq!(editor.draft_end, 15.0);
        // Clear of everything, the default is still two seconds.
        editor.begin_new(&document, 30.0);
        assert_eq!(editor.draft_end, 32.0);
        // Crowded against the next cue, it is short rather than inverted.
        editor.begin_new(&document, 19.995);
        assert!(editor.draft_end > editor.draft_start);
        assert!(editor.draft_end <= 20.0 + lyric_lane_edit::LYRIC_MIN_CUE_SECONDS);
    }

    /// Review 1.14, the warning half.
    #[test]
    fn the_form_names_the_cue_that_swallows_the_selected_line() {
        let clear = document(3);
        let mut document = LyricsDocument::new(120.0).expect("a positive duration");
        document
            .insert(LyricCue {
                id: 0,
                start_seconds: 40.0,
                end_seconds: 44.0,
                text: "swallowed tail".to_string(),
            })
            .expect("valid");
        document
            .insert(LyricCue {
                id: 0,
                start_seconds: 42.1,
                end_seconds: 46.0,
                text: "the one that wins".to_string(),
            })
            .expect("valid");

        let notice = shadow_notice(&document, 1).expect("cue 1 loses its tail");
        assert_eq!(
            notice,
            "Overlaps cue 2 — this line will not display after 00:42.100"
        );
        // The cue doing the shadowing is not warned about, and neither is a
        // document without an overlap in it.
        assert_eq!(shadow_notice(&document, 2), None);
        assert_eq!(shadow_notice(&clear, 1), None);

        // A line nothing of which ever reaches the screen says so outright.
        let mut buried = LyricsDocument::new(120.0).expect("a positive duration");
        for end in [12.0, 14.0] {
            buried
                .insert(LyricCue {
                    id: 0,
                    start_seconds: 10.0,
                    end_seconds: end,
                    text: "line".to_string(),
                })
                .expect("valid");
        }
        assert_eq!(
            shadow_notice(&buried, 1).as_deref(),
            Some("Overlaps cue 2 — this line never displays")
        );
    }

    #[test]
    fn a_lane_drag_previews_exactly_what_it_will_commit() {
        // The property that stops blocks following the pointer and snapping back
        // when the commit is refused: the preview reads the same clamped number
        // the commit carries.
        let document = document(3);
        let ids: Vec<u64> = document.cues().iter().map(|cue| cue.id).collect();
        let mut editor = LyricEditor::new();
        editor.select_single(&document, ids[0]);
        editor.lane_drag = LaneDrag {
            active: true,
            zone: Some(LyricLaneZone::Body),
            id: ids[0],
            moved: true,
            delta_seconds: lyric_lane_edit::clamp_move(&document, &editor.lane_selection, -500.0),
            ..LaneDrag::default()
        };
        // Clamped to the start of the track: cue 0 begins at 10 s.
        assert!((editor.lane_drag.delta_seconds + 10.0).abs() < 1e-9);
        let (start, end) = lane_preview_span(&editor, &document.cues()[0]);
        assert!((start - 0.0).abs() < 1e-9);
        assert!((end - 2.0).abs() < 1e-9);
        // A cue outside the selection is not previewed at all.
        let (other_start, _) = lane_preview_span(&editor, &document.cues()[1]);
        assert!((other_start - 15.0).abs() < 1e-9);
        // And the same delta is what a commit would be given.
        let mut committed = document.clone();
        assert!(committed
            .shift_many(editor.lane_selection.ids(), editor.lane_drag.delta_seconds)
            .is_ok());
        assert!((committed.cues()[0].start_seconds - 0.0).abs() < 1e-9);
    }

    // -----------------------------------------------------------------------
    // The caption style form: geometry, and the free colour picker
    // -----------------------------------------------------------------------

    /// The defect the operator found, as a number.
    ///
    /// "Import a face..." was positioned at `form.y + 158` and then drawn only
    /// `if import_face.y + height <= form.y + form.height`, against a form
    /// minimum of 152. Every height in `[152, 184)` therefore drew the form and
    /// silently omitted the only route to an imported caption face — and the
    /// real layout hands the caption pane exactly one height at the minimum
    /// supported window, 165, which is inside that window. No capture could
    /// catch it, because a form missing its last row is a plausible picture of a
    /// form.
    ///
    /// So the geometry is composed as a value and every rectangle in it is
    /// checked against the form, in both states the form has.
    #[test]
    fn every_caption_control_fits_the_smallest_form_the_pane_will_draw() {
        // Deliberately not at the origin: an off-by-one that used `form.height`
        // where it meant `form.y + form.height` passes at (0, 0).
        let form = UiRect::new(17.0, 23.0, CAPTION_FORM_WIDTH, CAPTION_FORM_HEIGHT);
        let layout = CaptionFormLayout::compose(form);
        for (name, rect) in layout.every_rect() {
            assert!(
                form.contains(rect),
                "{name} at {rect:?} escapes the form {form:?}"
            );
        }

        // Both halves stay in their own column. The picker replaces the left
        // one, so a picker rectangle that reached across would be drawn over the
        // swatch row it was opened from — and the swatch row is drawn second,
        // which means it would take the press.
        let left = UiRect::new(form.x, form.y, layout.column_width, form.height);
        let right = UiRect::new(
            layout.right_label_x,
            form.y,
            form.x + form.width - layout.right_label_x,
            form.height,
        );
        for (name, rect) in layout.every_rect() {
            let column = if rect.x < layout.right_label_x {
                ("left", left)
            } else {
                ("right", right)
            };
            assert!(
                column.1.contains(rect),
                "{name} at {rect:?} overruns the {} column {:?}",
                column.0,
                column.1
            );
        }
    }

    /// The other half of the fix: the number the form is measured by must be one
    /// the panel can actually supply, or the repair would be an "Enlarge the
    /// window" message at the minimum supported size instead of a cut button.
    ///
    /// Driven through the same chain the panel uses — `timeline_height`, the
    /// shell's chrome, the pane's own padding — rather than against a constant,
    /// because it is the chain that decided 165 and nobody wrote 165 down.
    #[test]
    fn the_caption_form_never_falls_back_to_the_enlarge_message() {
        let mut editor = LyricEditor::new();
        let mut smallest = f32::INFINITY;
        for count in [0usize, 1, 8, 64, 1024] {
            editor.cue_count = count;
            for window in [640.0f32, 720.0, 800.0, 1080.0, 1440.0, 2160.0] {
                let band = editor.timeline_height(window, 0.0);
                let boundary =
                    band - CHROME_ABOVE_PANEL - PANEL_BOTTOM_PADDING - LANE_HEIGHT - LANE_GAP;
                // `Shell::caption_pane`'s arithmetic for the form it hands on.
                let form_height = boundary - metric::UI_PANEL_PADDING * 2.0 - 38.0;
                assert!(
                    form_height >= CAPTION_FORM_HEIGHT,
                    "{window}px window, {count} cues: the caption form got {form_height}px \
                     against a minimum of {CAPTION_FORM_HEIGHT}"
                );
                smallest = smallest.min(form_height);
            }
        }
        // The floor is the panel's form minimum, and it is what
        // `CAPTION_FORM_AVAILABLE` claims at compile time. If the two ever
        // disagree the const assertion above is measuring the wrong thing.
        assert!(
            (smallest - CAPTION_FORM_AVAILABLE).abs() < 0.001,
            "{smallest}"
        );
    }

    /// Ids are what claim a press, and a collision lets one control release
    /// another's. The caption pane grew two cells and a button in this change,
    /// and the swatch rows are the risk: they are indexed from a base, so a base
    /// too close to the next group silently overlaps it.
    #[test]
    fn every_caption_control_has_a_distinct_widget_id() {
        let mut ids = vec![
            caption_id::IMPORT_FACE,
            caption_id::REMOVE_FACE,
            caption_id::PICKER_DONE,
            caption_id::GLOW_COLOR,
            caption_id::EFFECTS_TOGGLE,
        ];
        // Three face choices (a project may carry an imported family), three
        // backings, nine anchor cells.
        ids.extend((0..3).map(|index| caption_id::FACE + index));
        ids.extend((0..3).map(|index| caption_id::BACKING + index));
        ids.extend((0..9).map(|index| caption_id::ANCHOR + index));
        // One per swatch, plus the custom cell at the end of each row.
        ids.extend((0..=CAPTION_TEXT_SWATCHES.len() as u32).map(|i| caption_id::INK_SWATCHES + i));
        ids.extend((0..=CAPTION_BOX_SWATCHES.len() as u32).map(|i| caption_id::PLATE_SWATCHES + i));
        // The effects form's two drive rows, one cell per drive.
        ids.extend((0..DRIVE_LABELS.len() as u32).map(|i| caption_id::PULSE_DRIVE + i));
        ids.extend((0..DRIVE_LABELS.len() as u32).map(|i| caption_id::HUE_DRIVE + i));

        let control_count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), control_count, "two caption controls share an id");

        // And the pane's namespace is disjoint from the other panels', which is
        // checked rather than assumed because these are local constants.
        let mine: std::collections::HashSet<u64> = ids
            .iter()
            .map(|index| widgets::widget_id(ns::CAPTION, *index))
            .collect();
        for namespace in [ns::ACTIONS, ns::FORM, ns::ROWS, widgets::id::TOOLBAR] {
            for index in 0..4096u32 {
                assert!(!mine.contains(&widgets::widget_id(namespace, index)));
            }
        }
    }

    /// The effects form shares the style form's footprint, so it inherits the
    /// same obligation: every control fits the smallest form the pane will
    /// draw, in both of its states, and each stays in its own column.
    #[test]
    fn every_effects_control_fits_the_smallest_form_the_pane_will_draw() {
        let form = UiRect::new(17.0, 23.0, CAPTION_FORM_WIDTH, CAPTION_FORM_HEIGHT);
        let layout = CaptionEffectsLayout::compose(form);
        for (name, rect) in layout.every_rect() {
            assert!(
                form.contains(rect),
                "{name} at {rect:?} escapes the form {form:?}"
            );
            let boundary = if rect.x < layout.right_label_x {
                UiRect::new(form.x, form.y, layout.right_label_x - form.x, form.height)
            } else {
                UiRect::new(
                    layout.right_label_x,
                    form.y,
                    form.x + form.width - layout.right_label_x,
                    form.height,
                )
            };
            // The note deliberately spans the whole form.
            if name != "note" {
                assert!(
                    boundary.contains(rect),
                    "{name} at {rect:?} overruns its column {boundary:?}"
                );
            }
        }
        // Six drive cells stay wide enough for `choice_row` to draw at all — a
        // row under 24 px per cell returns `None`, which is a silently missing
        // control, the exact defect this file's layout values exist to prevent.
        let per_cell = (layout.pulse.width - 4.0 * 5.0) / 6.0;
        assert!(per_cell >= 24.0, "pulse cells are {per_cell}px");
    }

    /// The Style/Effects flip and the probe both land on the effects body, and
    /// leaving the pane resets it, so a later "style=caption" capture cannot
    /// photograph the wrong form.
    #[test]
    fn the_effects_pane_is_reachable_and_leaves_with_the_style_pane() {
        let mut editor = LyricEditor::new();
        let mut track = test_track("/tmp/effects.wav", 0);
        let probe = UiProbe {
            caption_effects_pane: true,
            ..UiProbe::default()
        };
        assert!(editor.apply_probe(&probe, &mut track));
        assert!(editor.style_pane && editor.effects_pane);
        assert!(editor.describe(Some(&track)).contains("caption-effects"));

        let glow = UiProbe {
            caption_picker: Some(CaptionPickerProbe::Glow),
            ..UiProbe::default()
        };
        let mut fresh = LyricEditor::new();
        assert!(fresh.apply_probe(&glow, &mut track));
        assert!(
            fresh.effects_pane,
            "the glow picker lives on the effects form"
        );
        assert_eq!(fresh.picker.target, Some(CaptionColor::Glow));
    }

    /// The one thing an RGBA colour cannot carry, and the reason the picker holds
    /// hue, saturation and value of its own.
    ///
    /// Drag the square to the bottom edge and the colour is black; black has no
    /// hue and no saturation. A picker that re-read its bar positions out of the
    /// packed value every frame would jump to red and stay there, so dragging
    /// back up would return a *different* colour from the one you started with.
    #[test]
    fn a_drag_through_black_keeps_the_hue_it_was_dragged_from() {
        let mut picker = CaptionPicker::default();
        picker.toggle(CaptionColor::Ink);
        // Cyan at full opacity.
        picker.sync(0x00FF_FFFF);
        assert!((picker.hue - 180.0).abs() < 0.01, "{}", picker.hue);
        assert_eq!(picker.packed(), 0x00FF_FFFF);

        // Down to black, which is what the bottom edge of the square means.
        picker.value = 0.0;
        let black = picker.packed();
        assert_eq!(widgets::unpack_rgba(black), (0, 0, 0, 255));
        picker.adopt(black);
        // The frame after: the model holds black and the picker is asked to
        // agree with it. Nothing about the bar may move.
        picker.sync(black);
        assert!((picker.hue - 180.0).abs() < 0.01, "{}", picker.hue);

        // And straight back up is the colour that was there, not red.
        picker.value = 1.0;
        assert_eq!(picker.packed(), 0x00FF_FFFF);
    }

    /// The other direction: a preset swatch clicked while the picker is open has
    /// to move the picker, or the next drag would snap back to the colour the
    /// picker still thought was live.
    #[test]
    fn a_preset_swatch_chosen_while_the_picker_is_open_moves_it() {
        let mut picker = CaptionPicker::default();
        picker.toggle(CaptionColor::Ink);
        picker.sync(0xFFFF_FFFF);
        // The amber swatch, chosen from the row rather than from the square.
        picker.sync(0xF2BE_42FF);
        assert_eq!(picker.packed(), 0xF2BE_42FF);
        assert!(picker.saturation > 0.5);

        // Toggling it shut and open again re-derives, because the value may have
        // travelled a route that lost its hue while the picker was closed.
        picker.toggle(CaptionColor::Ink);
        assert_eq!(picker.target, None);
        picker.toggle(CaptionColor::Ink);
        assert_eq!(picker.source, None);
        // Another target closes the first rather than opening two.
        picker.toggle(CaptionColor::Plate);
        assert_eq!(picker.target, Some(CaptionColor::Plate));
    }

    /// Alpha is a whole byte, and a picker that lost the low bits would make the
    /// six oracle swatches unreachable by hand.
    #[test]
    fn every_swatch_in_both_rows_round_trips_through_the_picker() {
        for packed in CAPTION_TEXT_SWATCHES.iter().chain(&CAPTION_BOX_SWATCHES) {
            let mut picker = CaptionPicker::default();
            picker.sync(*packed);
            assert_eq!(
                picker.packed(),
                *packed,
                "{packed:#010X} did not survive the picker"
            );
        }
    }

    /// `picker=` and the report line, which is what a capture asserts on.
    #[test]
    fn the_report_line_says_which_free_picker_is_open() {
        let track = test_track("/tmp/a.wav", 3);
        let mut editor = LyricEditor::new();
        assert!(editor.describe(Some(&track)).contains("picker=none"));

        let probe = UiProbe {
            caption_picker: Some(CaptionPickerProbe::Plate),
            ..UiProbe::default()
        };
        let mut track = track;
        assert!(editor.apply_probe(&probe, &mut track));
        assert!(editor.style_pane, "the picker lives inside the style pane");
        assert!(!editor.font_pane);
        let line = editor.describe(Some(&track));
        assert!(line.contains("pane caption-style"), "{line:?}");
        assert!(line.contains("picker=plate"), "{line:?}");

        // The font browser draws over the whole form, so the two together are a
        // capture that would photograph neither. It has to fail the exit status
        // rather than quietly pick one.
        let mut editor = LyricEditor::new();
        let probe = UiProbe {
            caption_picker: Some(CaptionPickerProbe::Ink),
            font_browser: Some(crate::cli::FontBrowserProbe::Consent),
            ..UiProbe::default()
        };
        assert!(!editor.apply_probe(&probe, &mut track));
    }
}
