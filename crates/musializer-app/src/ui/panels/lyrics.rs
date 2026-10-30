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

use musializer_core::project::caption_effects::drive_value;
use musializer_core::project::editor_draft::{LyricDraftState, LyricDraftValues};
use musializer_core::project::lyrics::{
    self, CueOrigin, LyricCue, LyricHistory, LyricsDocument, LyricsError,
};
use musializer_core::project::model::{
    caption, CaptionAnchor, CaptionBox, CaptionFace, CaptionStyle, DriveTuning, EffectDrive,
};
use musializer_core::ui::contrast;
use musializer_core::ui::lyric_clipboard::LyricClipboard;
use musializer_core::ui::lyric_lane_edit::{
    self, LyricLaneClick, LyricLaneSelection, LyricLaneZone, LyricTap, TapRefusal,
    LYRIC_LANE_EDGE_GRAB_PIXELS, LYRIC_LANE_EDGE_MIN_BLOCK_PIXELS, LYRIC_MIN_CUE_SECONDS,
};
use musializer_core::ui::lyric_lane_stack::{row_geometry, shade_multiplier, LyricStack};
use musializer_core::ui::notice::Severity;
use musializer_core::ui::text_edit::{TextEditError, TextRules};
use musializer_core::ui::timed_lane;
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::{GlyphRepertoire, UiFonts};
use raylib::consts::MouseCursor;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};
use serde::{Deserialize, Serialize};

use super::super::mapping_editor;
use super::super::shell::{draw_timeline_playhead, Shell, ShellCommand, ShellInput};
use super::super::text_input::TextField;
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};
use crate::cli::{CaptionPickerProbe, CaptionTuneProbe, UiProbe};
use crate::workspace::{Track, Workspace};

mod column;
use column::LyricsPane;

/// The cue lane's default height, and the gap between it and the editor below.
///
/// 22 px was the oracle's lane ([`ORACLE_LANE_HEIGHT`]). **50 is 2.25x that**
/// (49.5, rounded up to a whole pixel), and the number is the operator's,
/// revised upward on 2026-08-06 after seeing the first attempt on a real track.
///
/// The first attempt was 1.5x, and the feedback names exactly why it was not
/// enough: *"what happened to 1.5x-sizing the area that matters (where the user
/// clicks)? Otherwise this is a pixel aiming fest."* The whole lane height is the
/// click target — [`Shell::lane_gesture`] hit-tests the lane rect, not the block
/// — so this constant **is** the affordance, and on a 3½ minute track zoomed to
/// the whole song a cue is about 18 px wide. There is nothing to be done about
/// the width at that zoom; the height is the axis that can be given away, so it
/// is given away generously.
pub const LANE_HEIGHT: f32 = 50.0;
/// The tallest the lane's bottom-edge drag may make it: 3x the oracle's lane.
///
/// A ceiling rather than "as much as you like" because every pixel here is a
/// pixel off the editing form below, which has a hard 223 px minimum
/// ([`lyrics_editor_layout::LYRIC_EDITOR_FORM_MINIMUM`]) — past this the lane
/// would start deleting the controls it exists to feed.
pub const LANE_HEIGHT_MAX: f32 = 66.0;
/// The shortest the lane may be squeezed to on a window that cannot seat the
/// default *and* the editing form.
///
/// The oracle's own lane, which is the honest floor: it is the height this panel
/// shipped at for the whole rewrite, so a window that can only afford it is no
/// worse off than it was. Below this the edge handles stop being distinguishable
/// by hand ([`lyric_lane_edit::LYRIC_LANE_EDGE_MIN_BLOCK_PIXELS`] is 18 px wide
/// for the same reason).
const LANE_HEIGHT_MIN: f32 = ORACLE_LANE_HEIGHT;
/// The oracle's lane, kept only so [`the chrome assertion`](self) below still
/// states what it always stated: this shell's fixed chrome plus a 22 px lane is
/// exactly `LYRIC_EDITOR_TIMELINE_CHROME`.
///
/// `plug.c` caps its lane at 22, which is also why its scene names — gated on a
/// lane at least 28 px high — never appeared at all
/// (`lyrics_editor_ui.c:1060-1066`). Ours is now taller than that gate.
const ORACLE_LANE_HEIGHT: f32 = 22.0;
/// How deep the bottom-edge grab strip is, measured up from the lane's bottom.
///
/// **7 px, and drawn.** It was 3 px and invisible, on the operator's original
/// instruction that "a desperate user will find the resize if they need it" —
/// and then the operator could not find it: *"the size increase of the lyrics
/// cue bar doesn't seem to work"*, followed by *"I guess we do need a bit of a
/// stronger border there for resizing"* (2026-08-06). Both halves of that were
/// the same defect. A 3 px target is itself the pixel-aiming the taller lane
/// exists to remove, and an affordance that only appears once the pointer is
/// already on it cannot be found by a pointer that never lands there.
///
/// It still bounds the block band at both ends ([`Self::lyric_lane`] insets by
/// it top and bottom), so no cue is ever drawn in the strip and a resize press
/// can never be a press on a cue.
const LANE_RESIZE_GRAB_PIXELS: f32 = 7.0;

/// Breathing room above the blocks — the oracle's inset, and only at the top.
///
/// The bottom's inset is [`LANE_RESIZE_GRAB_PIXELS`] instead, so growing the
/// grab strip does not silently cost the blocks twice what it costs once.
const LANE_BLOCK_TOP_INSET: f32 = 3.0;
/// 5 px, and the 5 is not a taste decision — see [`CHROME_ABOVE_PANEL`].
pub const LANE_GAP: f32 = 5.0;

/// Shortest row a cue's own text may be typed into (LX2).
///
/// Below this the line has no room to sit off the block's border and reads as a
/// smear rather than as words. A three-deep stack in a 50 px lane gives each row
/// about 13 px, so the fan-out silently drops back to plain colour — which is
/// correct: at that height the colour and the tooltip are the legible channels.
const LANE_TEXT_MIN_ROW_PIXELS: f32 = 16.0;
/// Narrowest block that gets text at all.
///
/// Under this, everything drawn would be inside the fade, so the block would
/// carry a grey smudge that says nothing and hides its own fill.
const LANE_TEXT_MIN_BLOCK_PIXELS: f32 = 26.0;
/// How far back from the block's edge the text fades out.
///
/// The operator asked for a fade rather than a cut or an ellipsis: an ellipsis
/// costs three characters of the little text a block can hold, and a hard cut
/// mid-glyph reads as a rendering fault rather than as "there is more".
const LANE_TEXT_FADE_PIXELS: f32 = 12.0;

/// The band the zoom readout, the nudge-key hint and the Zoom out button take,
/// **below** the cue lane (LX1).
///
/// It used to sit between the waveform and the lane, which put a row of chrome
/// through the middle of what is really one three-lane timeline — scene plan,
/// waveform, lyric cues — and made the lyric lane read as a separate widget
/// glued underneath. Operator request, 2026-08-06. The number is unchanged from
/// what `timeline_strip` reserved above the lane, so the band's total chrome is
/// the same to the pixel and the assertion below still holds.
pub const ZOOM_ROW_HEIGHT: f32 = 28.0;

/// What a requested lane height becomes (LX1-d): the lane's own bounds.
///
/// Since WX-5d the band has one height in every view and the editor lives in
/// the column, so there is no form below the lane to protect and no band
/// budget (`lyrics_editor_layout`'s, retired) to solve against. The band
/// still caps the drawn lane at its slack where it is drawn
/// (`band_lyric_lane`); this only bounds what a drag may ask for.
fn clamp_lane_height(requested: f32, _window_height: f32) -> f32 {
    if !requested.is_finite() {
        return LANE_HEIGHT;
    }
    requested.clamp(LANE_HEIGHT_MIN, LANE_HEIGHT_MAX)
}

/// The tallest lane a drag may ask for: [`LANE_HEIGHT_MAX`]. A function so
/// the grip's "can it grow?" reads the same bound the clamp uses.
fn lane_height_ceiling(_window_height: f32) -> f32 {
    LANE_HEIGHT_MAX
}

/// Widget id namespaces for this panel: aliases of the shared table's entries.
///
/// These were local literals while the fan-out was live, sitting "outside the
/// range that module uses" — a range that then grew. That is exactly how the
/// manual event row's two literals ended up aliasing `EXPORT` and `SEEK` and
/// eating their presses (EX1). These four never collided, but they were true by
/// luck, and the module doc on [`widgets::id`] is now the rule.
mod ns {
    use super::widgets;

    /// The pane toggles and the document buttons.
    pub const ACTIONS: u32 = widgets::id::LYRICS_ACTIONS;
    /// The editing form: the two time rows and Apply/Discard/Delete.
    pub const FORM: u32 = widgets::id::LYRICS_FORM;
    /// The caption pane's choices, anchor grid, swatches and face buttons.
    pub const CAPTION: u32 = widgets::id::LYRICS_CAPTION;
    /// One per cue, keyed by cue id, so a row keeps its id across a scroll.
    pub const ROWS: u32 = widgets::id::LYRICS_ROWS;
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
    /// The tune editor's own controls (UX0-C14).
    pub const TUNE_CLAMP: u32 = 104;
    pub const TUNE_RESET: u32 = 105;
    pub const TUNE_CURVE_PREVIOUS: u32 = 106;
    pub const TUNE_CURVE_NEXT: u32 = 107;
    /// The two anchor pairs' four sliders, in low-in/low-out/high-in/high-out
    /// order.
    pub const TUNE_SLIDERS: u32 = 112;
}

/// What each [`CueOrigin`] is drawn in (LX1-d).
///
/// The oracle has one amber for every block (`lyrics_editor_ui.c:998`) because
/// it has one kind of cue. Since a document can now hold four, one amber is a
/// lane that cannot answer the question the operator opened this work with:
/// after an assist run, which of these did a machine place and which did I?
///
/// Not in `theme::rgba` because that is a shared file; the request to move them
/// there is in the report.
///
/// # How they were chosen, with the numbers
///
/// **One colour per origin, the same in both appearances** (WX-5b). Since the
/// re-look the code is carried by a 3 px stripe and a 1 px top line rather than
/// by a tinted block, so a colour has to stand off every surface those marks
/// touch: the lane (`ui_raised`), the lane trough beside it, and the three block
/// fills (`ui_cue_block`, `ui_cue_now`, `ui_selected`), in the dark palette
/// *and* the light one. That is ten surfaces from near-black to white, and a
/// colour clears `AA_COMPONENT` (3.0) on all of them only if its
/// relative luminance sits in a band around 0.19 — so each origin kept its hue
/// and saturation and moved its value into that band. The test at the bottom of
/// this file recomputes every pair rather than trusting this table:
///
/// | origin | was | now | worst surface | ratio |
/// | --- | --- | --- | --- | --- |
/// | `UserApplied` | `#1F7A4D` | `#238A57` | dark *now* fill | 3.48 |
/// | `InferredCertain` | `#2563C7` | `#2C75EB` | light trough | 3.48 |
/// | `InferredAmbiguous` | `#D07A00` | `#B16800` | light trough | 3.48 |
/// | `Potential` | `#7C7C86` | unchanged | light trough | 3.32 |
///
/// The old green and blue fell under 3:1 on the dark block and the warm *now*
/// fill (2.97 and 2.65 for the blue), and the old amber under it on the light
/// block (2.81): each had been measured against the white lane only, which
/// stopped being the background when the lane went dark and the blocks opaque.
///
/// The **semantics** are the operator's: green reads as settled, blue as a
/// machine that is sure, amber as check-this, and `Potential` is deliberately
/// the only achromatic one — a proposal is not content, and a grey block reads
/// as scaffolding rather than as a line with an unusual colour. It also carries
/// [`hatch`], so the one state that means "this is a guess, not a placement" is
/// distinguished by texture as well as hue and survives a colour-blind reading.
const fn origin_color(origin: CueOrigin) -> Color {
    match origin {
        CueOrigin::UserApplied => Color::new(0x23, 0x8A, 0x57, 255),
        CueOrigin::InferredCertain => Color::new(0x2C, 0x75, 0xEB, 255),
        CueOrigin::InferredAmbiguous => Color::new(0xB1, 0x68, 0x00, 255),
        CueOrigin::Potential => Color::new(0x7C, 0x7C, 0x86, 255),
    }
}

/// Width of the provenance stripe down a cue block's start (WX-5b).
const ORIGIN_STRIPE_PIXELS: f32 = 3.0;

/// A cue block's fill (WX-5b, the mockup's `.ly`): neutral, so the words carry
/// the block. The line on screen takes the warm *now* tint and a selected block
/// the selection tint (with its outline, drawn by the caller); then the fill
/// darkens 20 % per stack step (LX1-d), so a fanned cluster reads as rows,
/// brightest on top.
///
/// No hover tint: `ui_hover` sits within a few levels of `ui_cue_block` in both
/// appearances, so it would be a state nobody could see. A hovered block says so
/// with its grab bars, its tooltip and the cursor, all of which are legible.
fn cue_block_fill(now: bool, selected: bool, shade_steps: usize) -> Color {
    let base = if now {
        color::ui_cue_now()
    } else if selected {
        color::ui_selected()
    } else {
        color::ui_cue_block()
    };
    shade(base, shade_multiplier(shade_steps))
}

/// The colour a block's provenance marks are drawn in on `fill`: the origin's
/// colour, or that colour darkened with its stack row, whichever stands further
/// off the fill.
///
/// The choice is what keeps a deep row's marks legible in both appearances. On
/// the dark lane the fill darkens toward black with the row, so darkening the
/// mark as well would sink it into the fill; on the light lane the fill darkens
/// from near-white toward the mark, and darkening the mark keeps the gap. Row 0
/// is never darkened, so the colour the legend shows is the colour on the lane.
fn origin_mark(origin: CueOrigin, fill: Color, shade_steps: usize) -> Color {
    let base = origin_color(origin);
    let shaded = shade(base, shade_multiplier(shade_steps));
    let fill = packed_rgba(fill);
    if contrast::ratio(packed_rgba(shaded), fill) > contrast::ratio(packed_rgba(base), fill) {
        shaded
    } else {
        base
    }
}

/// The words' colour on a block: muted, ink on the line on screen, and whichever
/// of the palette's text colours (or plain black or white) still reads where a
/// deep stack row has darkened the fill past them.
fn cue_word_ink(now: bool, fill: Color) -> Color {
    let fill_packed = packed_rgba(fill);
    let preferred = if now {
        color::ui_ink()
    } else {
        color::ui_muted()
    };
    for candidate in [preferred, color::ui_ink()] {
        if contrast::ratio(packed_rgba(candidate), fill_packed) >= contrast::AA_TEXT {
            return candidate;
        }
    }
    widgets::contrasting_ink(fill)
}

/// `0xRRGGBBAA`, for [`contrast::ratio`].
const fn packed_rgba(color: Color) -> u32 {
    u32::from_be_bytes([color.r, color.g, color.b, color.a])
}

/// The four origins in the order the legend and the report line list them.
///
/// Canonical order, so a capture's legend and its `origins=` line can be read
/// against each other without translating.
const ORIGIN_ORDER: [CueOrigin; 4] = [
    CueOrigin::UserApplied,
    CueOrigin::InferredCertain,
    CueOrigin::InferredAmbiguous,
    CueOrigin::Potential,
];

/// The legend's word for one origin.
///
/// [`CueOrigin::token`] rather than [`CueOrigin::label`]: the labels are
/// sentences ("AI inferred (ambiguous)") and four of them do not fit across the
/// cue list's column at 960 px. The tokens are the persisted vocabulary, they
/// are what the report line prints, and the full label is one hover away in the
/// lane's tooltip.
const fn origin_legend_word(origin: CueOrigin) -> &'static str {
    origin.token()
}

fn lyric_lane_hover_allowed(lane: UiRect, pointer: Vector2, drag_active: bool) -> bool {
    !drag_active && lane.contains_point(pointer.x, pointer.y)
}

/// Whether the lane's Ctrl+C/X/V may fire (LX1-d).
///
/// Pure and named, because "which key combinations are refused while a field has
/// focus" is exactly the kind of condition that reads as obviously right and is
/// wrong at the one state nobody tried. All three arms are refusals: a focused
/// cue field owns Ctrl+V for text, and the caption and font panes draw no lane
/// at all, so a shortcut there would act on a selection that is not on screen.
const fn lyric_clipboard_keys_allowed(
    style_pane: bool,
    font_pane: bool,
    text_focused: bool,
) -> bool {
    !style_pane && !font_pane && !text_focused
}

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
    /// A committed edge drag in the lane (`lyrics_retime`, `:337`), and every
    /// stamp a tap run produces (UX0-C03).
    Retime {
        id: u64,
        start_seconds: f64,
        end_seconds: f64,
    },
    /// Break one cue in two at the playhead (`lyrics_split`, `lyrics.c:416-452`).
    ///
    /// Ported and tested since Agent B's band and unreachable from the interface
    /// until UX0-B05: splitting a long imported line is the commonest subtitle
    /// edit there is, and without it the only route was retyping both halves.
    Split {
        id: u64,
        split_seconds: f64,
        left_text: String,
        right_text: String,
    },
    /// Join two canonically adjacent cues (`lyrics_merge`, `lyrics.c:454-487`).
    Merge {
        first_id: u64,
        second_id: u64,
        separator: String,
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
                    // A cue the editor's own form created; the human typed the
                    // text and chose the window (LX1).
                    origin: CueOrigin::UserApplied,
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
            LyricsEdit::Split {
                id,
                split_seconds,
                left_text,
                right_text,
            } => track
                .lyrics
                .split(id, split_seconds, &left_text, &right_text)
                .map(Some),
            LyricsEdit::Merge {
                first_id,
                second_id,
                separator,
            } => track
                .lyrics
                .merge(first_id, second_id, &separator)
                .map(|()| None),
            LyricsEdit::SetCaptionStyle(style) => {
                track.caption_style = *style;
                Ok(None)
            }
        }
    }

    /// Whether this edit changes the cue document, and so belongs on the undo
    /// stack (UX0-B03).
    ///
    /// `SetCaptionStyle` is the whole reason this predicate exists: it travels
    /// down the same channel, it is the highest-frequency edit in the panel by
    /// far — every frame of a slider drag is one — and it does not touch a cue.
    /// Recording it would flood a 64-deep stack in under a second of dragging
    /// and push every real cue edit out of reach.
    #[must_use]
    pub fn touches_cues(&self) -> bool {
        !matches!(self, LyricsEdit::SetCaptionStyle(_))
    }

    /// The name this edit goes on the undo stack under, in the imperative the
    /// notice reads back: "Undid: Move 3 cues".
    #[must_use]
    pub fn history_label(&self) -> String {
        match self {
            LyricsEdit::Insert { .. } => "Add cue".to_owned(),
            LyricsEdit::Update { .. } => "Edit cue".to_owned(),
            LyricsEdit::Delete { .. } => "Delete cue".to_owned(),
            LyricsEdit::ShiftMany { ids, .. } => match ids.len() {
                1 => "Move cue".to_owned(),
                count => format!("Move {count} cues"),
            },
            LyricsEdit::Retime { .. } => "Retime cue".to_owned(),
            LyricsEdit::Split { .. } => "Split cue".to_owned(),
            LyricsEdit::Merge { .. } => "Merge cues".to_owned(),
            LyricsEdit::SetCaptionStyle(_) => "Caption style".to_owned(),
        }
    }
}

/// The name one drained batch goes on the undo stack under.
///
/// A batch is one user action — one frame's worth of pushes — so it earns one
/// snapshot and one label. A cut of five cues is five `Delete`s and one step,
/// because five presses of Ctrl+Z to reverse one press of Ctrl+X is not undo.
///
/// Returns `None` when the batch touched no cue, which is the caption-style
/// case and must not consume a stack slot.
#[must_use]
fn batch_history_label(edits: &[LyricsEdit]) -> Option<String> {
    let mut cue_edits = edits.iter().filter(|edit| edit.touches_cues());
    let first = cue_edits.next()?;
    let remaining = cue_edits.count();
    Some(match remaining {
        0 => first.history_label(),
        // A tap is exactly two retimes — the line it closes and the line it
        // opens — and naming that pair "2 edits" would make a run of taps read
        // as a wall of identical anonymous steps.
        1 if matches!(first, LyricsEdit::Retime { .. }) => "Stamp cue".to_owned(),
        count => format!("{} edits", count + 1),
    })
}

/// Which way a pending history step goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryStep {
    Undo,
    Redo,
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
    /// bar; `10..=16` are the effects form's sliders (10-12 glow, 13 sweep,
    /// 14-16 the backing tuners, now drawn on the style form).
    style_drag: u8,
    /// The free colour picker, closed unless a custom swatch opened it.
    picker: CaptionPicker,
    /// Which drive's tuning editor claims the effects form's left column
    /// (UX0-C14), or `None`. Mutually exclusive with the picker — they occupy
    /// the same space, and a control painted under another claims the press
    /// first.
    tune_target: Option<TuneTarget>,
    lane_selection: LyricLaneSelection,
    lane_drag: LaneDrag,
    /// The lane's current height in logical pixels (LX1-d).
    ///
    /// The editor's own copy of `UiPreferences::lyric_lane_height`, synced at
    /// the top of every draw. It has to live here because
    /// [`Self::timeline_height`] is asked how tall the band should be *before*
    /// the panel draws, and that method is handed nothing but the window — the
    /// preference belongs to the shell and the shell asks the editor.
    lane_height: f32,
    /// What the lane was **actually** drawn at last frame.
    ///
    /// Differs from [`Self::lane_height`] only when the band the shell handed
    /// this panel is shorter than the one it asked for, which a persisted
    /// timeline split can do. Kept apart so the request survives a frame drawn
    /// under a short band instead of being written down to it — and so the
    /// report line states the drawn number, which is the one a capture shows.
    lane_drawn: f32,
    /// Whether the bottom-edge drag has the pointer.
    lane_resizing: bool,
    /// Whether the grip should draw in its emphasised form this frame. Set by
    /// [`Shell::lane_resize_gesture`], read by [`Shell::draw_lane_resize_grip`],
    /// because the two run either side of the lane's own fill.
    lane_resize_hovered: bool,
    /// Copy/cut/paste, session-only (LX1-d). Never persisted: see
    /// [`LyricClipboard`]'s module comment.
    clipboard: LyricClipboard,
    /// Rows the last drawn frame's overlap fan-out needed, for the report line.
    stack_rows: usize,
    /// Cue counts per origin in [`ORIGIN_ORDER`], for the report line.
    origin_counts: [usize; ORIGIN_ORDER.len()],
    /// Whether the lane put a tooltip on screen last frame, for the report line.
    ///
    /// A tip is the one surface here a capture can photograph but a log cannot
    /// otherwise claim, and the requirement it has to meet — that it never
    /// covers the lane it explains — is a geometric one. So the report says
    /// whether there was one, and the gate measures where it landed.
    tooltip_visible: bool,
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

    // -- the timing loop (UX0-B02/B03/B04/B05, UX0-C03) --------------------
    /// Undo/redo for this track's cue document.
    ///
    /// Held here rather than on the [`Track`] so the whole feature stays inside
    /// this panel's ownership, and cleared by [`Self::enter_track`] for the same
    /// reason [`Self::owner_slot`] exists: cue ids restart at 1 in every
    /// document, so one track's history restores another track's cues.
    ///
    /// The cost of that decision, stated rather than hidden: switching tracks
    /// and switching back loses the history. A per-slot map would keep it, and
    /// is the obvious follow-up if anyone misses it.
    history: LyricHistory,
    /// A history step the panel asked for and `main.rs` has not run yet.
    ///
    /// A request rather than a direct call for the same reason every edit here
    /// is deferred: the panel draws inside a begin/end pair and does not hold
    /// `&mut Track`.
    history_request: Option<HistoryStep>,
    /// The play-and-tap run, armed or not.
    tap: LyricTap,
    /// The instant a tap run last stamped, for the readout, so the status line
    /// can show *where* the last press landed rather than only how many landed.
    tap_last_seconds: Option<f64>,
    /// A calibration the user just changed, waiting to be written to the
    /// per-user preference file (PXF-2).
    ///
    /// Deferred like [`Self::seek_request`], and for the same structural
    /// reason: `[`/`]` are read by `lyric_timing_keys`, which is handed no
    /// command vector, and the store is a side effect only the shell can have.
    tap_offset_request: Option<f64>,
    /// A seek the panel wants the transport to make (UX0-B02).
    ///
    /// Selecting a cue moves the playhead to its start, which is the single
    /// change that turns the cue list from a spreadsheet into a transport: the
    /// C's row click selects and never seeks, so checking a line meant reading
    /// its timecode and scrubbing to it by hand.
    seek_request: Option<f64>,
    /// Which time field is being typed into, and the buffer, or `None`
    /// (UX0-B04).
    ///
    /// The readout has always shown milliseconds and never accepted them. This
    /// is a `TextField` rather than a raw string so the caret, the selection and
    /// the clipboard behave the way the cue field does.
    time_edit: Option<TimeEdit>,
    /// Hold-to-repeat state for whichever nudge button is down.
    nudge_repeat: HoldRepeat,
    /// What the column view drew last frame, and its pane's scroll (WX-5d).
    column: column::ColumnEvidence,
    /// A family list `--ui-probe fonts=PATH` named, waiting for the shell to
    /// hand it to the font browser: this editor sees the probe, the shell owns
    /// the browser.
    probe_catalogue: Option<PathBuf>,
}

/// The authored form fields recovery must preserve without applying them to the
/// canonical lyric document (CX-3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LyricRecoveryDraft {
    pub owner_slot: usize,
    pub selected_id: u64,
    pub is_new: bool,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

/// A time readout being typed into (UX0-B04).
struct TimeEdit {
    /// `true` for START, `false` for END.
    is_start: bool,
    field: TextField,
}

/// Hold-to-repeat for the nudge buttons (UX0-B04).
///
/// Counted in **frames** rather than seconds, which is the one decision here
/// worth defending. A wall clock would give a repeat rate that is the same on
/// every machine, which is the usual argument — but this repository's evidence
/// is a headless capture run at a fixed frame budget under Xvfb, where
/// `GetTime` measures how long the software renderer took and nothing a user
/// would recognise. A frame count makes "hold the button for 30 frames and the
/// value moves 4 steps" a thing a probe can assert.
///
/// At the app's normal 60 Hz the numbers below are a 0.33 s delay and about 17
/// steps a second, which is the ladder every spinner control uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HoldRepeat {
    /// The widget currently held, or `0`.
    id: u64,
    frames: u32,
}

impl HoldRepeat {
    /// Frames a button must be held before it starts repeating.
    const DELAY_FRAMES: u32 = 20;
    /// Frames between repeats once it has started.
    const INTERVAL_FRAMES: u32 = 4;

    /// Advances the state for one frame and reports whether `id` should fire an
    /// extra step now.
    ///
    /// Never fires on the frame the press begins: that one belongs to
    /// `clicked`, and firing both would move the value two steps for one tap.
    fn tick(&mut self, id: u64, pressed: bool) -> bool {
        if !pressed {
            if self.id == id {
                *self = Self::default();
            }
            return false;
        }
        if self.id != id {
            *self = Self { id, frames: 0 };
            return false;
        }
        self.frames += 1;
        self.frames > Self::DELAY_FRAMES
            && (self.frames - Self::DELAY_FRAMES) % Self::INTERVAL_FRAMES == 0
    }
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
        self.text.is_focused() || self.time_edit.is_some()
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
            tune_target: None,
            lane_selection: LyricLaneSelection::new(),
            lane_drag: LaneDrag::default(),
            lane_height: LANE_HEIGHT,
            lane_drawn: LANE_HEIGHT,
            lane_resizing: false,
            lane_resize_hovered: false,
            clipboard: LyricClipboard::default(),
            stack_rows: 1,
            origin_counts: [0; ORIGIN_ORDER.len()],
            tooltip_visible: false,
            cue_count: 0,
            owner_slot: None,
            pending: Vec::new(),
            authored: AuthoredEvidence::default(),
            history: LyricHistory::new(),
            history_request: None,
            tap: LyricTap::default(),
            tap_last_seconds: None,
            tap_offset_request: None,
            seek_request: None,
            time_edit: None,
            nudge_repeat: HoldRepeat::default(),
            column: column::ColumnEvidence::default(),
            probe_catalogue: None,
        }
    }

    /// The workspace slot this draft belongs to, or `None` before any track has
    /// been entered (review 1.3).
    #[must_use]
    pub fn draft_owner(&self) -> Option<usize> {
        self.owner_slot
    }

    #[must_use]
    pub(crate) fn recovery_draft(&self) -> Option<LyricRecoveryDraft> {
        Some(LyricRecoveryDraft {
            owner_slot: self.owner_slot?,
            selected_id: self.selected_id,
            is_new: self.draft_new,
            start_seconds: self.draft_start,
            end_seconds: self.draft_end,
            text: self.text.edit.text().to_string(),
        })
    }

    pub(crate) fn restore_recovery_draft(&mut self, draft: LyricRecoveryDraft) {
        self.owner_slot = Some(draft.owner_slot);
        self.selected_id = draft.selected_id;
        self.draft_new = draft.is_new;
        self.draft_start = draft.start_seconds;
        self.draft_end = draft.end_seconds;
        self.text.bind(&draft.text);
        self.text.set_focused(false);
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
        // Both of these hold cue ids from the document being left, and both
        // would resolve — wrongly — against the one being entered. The history
        // is the more serious of the two: restoring it would replace this
        // track's whole document with another track's.
        self.history.clear();
        self.history_request = None;
        self.tap.disarm();
        self.tap_last_seconds = None;
        self.seek_request = None;
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

    // -----------------------------------------------------------------------
    // The timing loop (UX0-B02/B03/B04/B05, UX0-C03)
    // -----------------------------------------------------------------------

    /// Binds the form to a cue **and asks the transport to go there** (UX0-B02).
    ///
    /// The C's row click selects and never seeks (`lyrics_editor_ui.c:1255`), so
    /// checking a line meant reading its timecode off the row and scrubbing to it
    /// by hand — 60 lines is 120 of those. Seeking to the cue's start rather than
    /// its middle is deliberate: the start is the thing being judged, and landing
    /// there means pressing play immediately answers "is this on the beat".
    pub fn select_and_seek(&mut self, document: &LyricsDocument, id: u64) {
        let Some(start) = document.find(id).map(|cue| cue.start_seconds) else {
            return;
        };
        self.select_single(document, id);
        self.seek_request = Some(start);
    }

    /// Steps the selection to the neighbouring cue in canonical order and seeks
    /// there, for the Up/Down keys.
    ///
    /// With nothing selected it takes the cue under the playhead, so pressing
    /// Down on a fresh panel starts where the user is listening rather than at
    /// the top of a 200-line document.
    pub fn step_selection(&mut self, document: &LyricsDocument, forward: bool, playhead: f64) {
        if document.is_empty() {
            return;
        }
        let current = document.index_of(self.selected_id).or_else(|| {
            document
                .cues()
                .iter()
                .position(|cue| cue.start_seconds <= playhead && playhead < cue.end_seconds)
        });
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => document.len() - 1,
            // Saturating at both ends rather than wrapping: a list that jumps
            // from the last line to the first reads as a lost keypress.
            (Some(index), true) => (index + 1).min(document.len() - 1),
            (Some(index), false) => index.saturating_sub(1),
        };
        let id = document.cues()[next].id;
        self.select_and_seek(document, id);
    }

    /// Takes the seek the panel asked for, if any.
    pub fn take_seek_request(&mut self) -> Option<f64> {
        self.seek_request.take()
    }

    /// Records one drained batch on the undo stack, before it is applied.
    ///
    /// `before` is the document as it stands with the batch still pending, which
    /// is the state Ctrl+Z must return to.
    pub fn record_history(&mut self, edits: &[LyricsEdit], before: &LyricsDocument) {
        if let Some(label) = batch_history_label(edits) {
            self.history.record(label, before);
        }
    }

    /// Records a snapshot under a caller-chosen label.
    ///
    /// For the changes that do not travel as a [`LyricsEdit`] — today that is
    /// the TSV import, which replaces the whole document at once (D3). It is
    /// still one undo step, and it has to be one: an import over hand-timed work
    /// is the single most expensive accident this panel can have.
    pub fn record_history_label(&mut self, label: &str, before: &LyricsDocument) {
        self.history.record(label, before);
    }

    /// Drops every cue id the editor is holding, after something replaced the
    /// whole document underneath it.
    ///
    /// The draft, the lane selection and any armed run all address cues by id,
    /// and an import allocates a fresh set — so all three would resolve against
    /// cues that are not the ones they were bound to.
    pub fn enter_document_change(&mut self) {
        self.clear_draft();
        self.lane_selection.clear();
        self.tap.disarm();
        self.time_edit = None;
        self.list_first = 0;
    }

    /// Asks for an undo or a redo on the next drain.
    pub fn request_history(&mut self, step: HistoryStep) {
        self.history_request = Some(step);
    }

    /// Runs a requested history step against `document`.
    ///
    /// Returns `None` when nothing was requested or there was nothing to do in
    /// that direction, so the caller can say so rather than leaving a keypress
    /// unanswered.
    pub fn run_history_step(
        &mut self,
        document: &mut LyricsDocument,
    ) -> Option<(HistoryStep, Result<String, String>)> {
        let step = self.history_request.take()?;
        let outcome = match step {
            HistoryStep::Undo => self.history.undo(document),
            HistoryStep::Redo => self.history.redo(document),
        }?;
        // A restored document invalidates every cue id the panel is holding: an
        // undone insert takes its cue with it, and a bound form would then edit
        // an id that is not there.
        self.lane_selection.prune(document);
        if document.find(self.selected_id).is_none() {
            self.clear_draft();
        } else {
            self.select(document, self.selected_id);
        }
        // A run armed over an order the undo just rewrote would stamp the wrong
        // lines. Ending it is the honest answer, and the status line says so.
        self.tap.disarm();
        Some((step, outcome.map_err(|failure| failure.to_string())))
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    #[must_use]
    pub fn tap_is_armed(&self) -> bool {
        self.tap.is_armed()
    }

    /// Arms or disarms the tap run, reporting what happened for the notice.
    ///
    /// Arming blurs the cue field, and that exclusion is the whole answer to the
    /// defect review 2 names: `begin_new` focuses the field, a focused field
    /// stands down every global key including Space, and the natural
    /// add-type-play-tap loop broke at the tap. Tap mode and text entry are
    /// mutually exclusive states rather than two things fighting over the
    /// keyboard, so there is no key that means two things at once.
    pub fn toggle_tap(&mut self, document: &LyricsDocument, playhead: f64) -> bool {
        if self.tap.is_armed() {
            self.tap.disarm();
            return false;
        }
        self.text.set_focused(false);
        self.time_edit = None;
        self.tap_last_seconds = None;
        self.tap.arm(document, playhead)
    }

    /// One tap at the playhead. Enqueues the retimes it asks for.
    pub fn tap(&mut self, document: &LyricsDocument, at_seconds: f64) -> Result<usize, TapRefusal> {
        let stamp = self.tap.tap(document, at_seconds)?;
        let opened = stamp.opened_id;
        let stamped: Vec<_> = stamp.retimes.clone();
        for (id, start_seconds, end_seconds) in stamped {
            self.push(LyricsEdit::Retime {
                id,
                start_seconds,
                end_seconds,
            });
        }
        self.tap_last_seconds = Some(at_seconds + self.tap.offset_seconds());
        // Bind the form to the line just placed, so the readout and the list
        // follow the run without a click. Not `select_and_seek` — seeking to the
        // cue you just stamped would stop the music the run is riding on.
        if opened != 0 {
            self.select_single(document, opened);
            // Bound to what the cue is *about to be*, not what it still is. The
            // retime is pending for another frame, so binding from the document
            // would leave the draft holding the pre-stamp span — and a draft
            // that differs from the stored cue is a dirty one, so every tap
            // would arm the "Finish the lyric edit first" guard and the next
            // press would be refused. Caught by the first headless capture: the
            // report line read `draft dirty` after a clean run of four taps.
            if let Some((_, start, end)) = stamp
                .retimes
                .iter()
                .find(|(id, _, _)| *id == opened)
                .copied()
            {
                self.draft_start = start;
                self.draft_end = end;
            }
        }
        Ok(stamp.remaining)
    }

    pub fn adjust_tap_offset(&mut self, steps: i32) {
        self.tap.adjust_offset(steps);
        // Requested on every press rather than on the run ending: a calibration
        // is usually dialled in *during* playback and the run may end by the
        // user quitting, which is exactly the launch that must remember it. The
        // saturating case still requests, because it is one file write for a key
        // a hand can only press so fast, not sixty a second of a drag.
        self.tap_offset_request = Some(self.tap.offset_seconds());
    }

    /// Restores a calibration measured in an earlier session (PXF-2).
    ///
    /// The offset is per-user, not per-project, so this is fed from
    /// `UiPreferences` rather than from the track. `None` leaves
    /// [`LyricTap`]'s own default in place — "never calibrated" is a different
    /// fact from "calibrated to zero", and only the first should follow a later
    /// change to that default.
    pub fn set_tap_offset(&mut self, seconds: Option<f64>) {
        if let Some(seconds) = seconds {
            self.tap.set_offset_seconds(seconds);
        }
    }

    /// A calibration the shell has not written to the preference file yet.
    pub fn take_tap_offset_request(&mut self) -> Option<f64> {
        self.tap_offset_request.take()
    }

    #[must_use]
    pub fn tap_offset_seconds(&self) -> f64 {
        self.tap.offset_seconds()
    }

    /// The tap run's status line, or `None` when no run is armed.
    #[must_use]
    fn tap_status(&self) -> Option<String> {
        if !self.tap.is_armed() {
            return None;
        }
        let offset_ms = (self.tap.offset_seconds() * 1000.0).round() as i64;
        let target = match self.tap.target_id() {
            Some(_) => format!("line {} of {}", self.tap.position(), self.tap.total()),
            // The N+1st press, which places the last line's out point. Naming it
            // is what stops a user thinking the run already ended.
            None => "the last line's end".to_owned(),
        };
        Some(format!(
            "TAP  Enter stamps {target}  |  {} placed  |  offset {offset_ms:+} ms ([ / ])  |  Esc ends",
            self.tap.stamped()
        ))
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
        if let Some(browser) = probe.font_browser.as_ref() {
            // The browser is a pane *inside* the style pane, and its own Back
            // button returns there (`lyrics_editor_ui.c:1165-1174`).
            self.style_pane = true;
            self.font_pane = true;
            if let crate::cli::FontBrowserProbe::Catalogue(path) = browser {
                self.probe_catalogue = Some(path.clone());
            }
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
        if let Some(target) = probe.caption_tune {
            // The tune editor is a disclosure inside the effects form, and it
            // shares the left column with the picker — the probe mirrors the
            // opener buttons: effects pane on, picker closed.
            self.style_pane = true;
            self.font_pane = false;
            self.effects_pane = true;
            honoured &= probe.caption_picker.is_none();
            self.picker.target = None;
            self.tune_target = Some(match target {
                CaptionTuneProbe::Pulse => TuneTarget::Pulse,
                CaptionTuneProbe::Hue => TuneTarget::Hue,
            });
        }
        if let Some(path) = probe.lyrics_reference_path.as_ref() {
            track.lyrics_reference_path = Some(PathBuf::from(path));
        }
        // The timing loop's probe (UX0-C03). Xvfb has no keyboard any more than
        // it has a wheel, so without this the whole feature is unphotographable:
        // an armed run and a disarmed one differ by one 11 px line of hint text,
        // and a stamp that landed looks exactly like a stamp that was refused.
        //
        // Deliberately drives the same entry points the keys do, rather than
        // writing the cue spans directly — a probe that reproduced the
        // arithmetic instead of exercising it would photograph a picture of the
        // feature working while the feature was broken.
        if let Some(taps) = probe.lyric_taps {
            let playhead = probe.seek_seconds.unwrap_or(0.0);
            if self.toggle_tap(&track.lyrics, playhead) {
                for step in 0..taps {
                    // Spread across the track so no two stamps land inside the
                    // minimum gap, which would refuse and make the count a lie.
                    let at = playhead + f64::from(step) * LYRIC_PROBE_TAP_SPACING_SECONDS;
                    if self.tap(&track.lyrics, at).is_err() {
                        honoured = false;
                        break;
                    }
                }
            } else {
                honoured = false;
            }
        }
        if probe.lyric_undo {
            // Requested rather than run: the history executes in `main.rs`,
            // against the same drain the interactive path uses, so the probe
            // cannot accidentally test a second implementation.
            //
            // Deliberately **not** gated on `can_undo()`. This runs before the
            // frame loop, so the taps above are still pending and the stack is
            // still empty — checking here would refuse every combined
            // `lyric-tap=N,lyric-undo=1` run, which is the only combination
            // worth photographing. `main.rs` drains, records and applies first,
            // and the step lands in the same frame. What actually happened is
            // in the `lyrics:` report line's `history u/r` pair, which is where
            // the gate asserts it.
            self.request_history(HistoryStep::Undo);
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
        // LX1-d. Everything the lane can now draw that a picture cannot be
        // graded on: which provenance the blocks carry, whether the fan-out
        // actually fanned, how tall the lane ended up, whether a copy landed,
        // and whether a tip was on screen when the shutter fired. The rule this
        // follows is the one three unwired `Option`s cost two bands:
        // a surface that can draw without its data must say which one it did.
        let origins = ORIGIN_ORDER
            .iter()
            .zip(self.origin_counts)
            .map(|(origin, count)| format!("{}={count}", origin.token()))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "{} cues, pane {}, selected {}, owner {}, lane {}, draft {}, shadowed {}, \
             face={} field={} missing={} picker={} tune={}, origins {}, stack rows {}, \
             lane height {:.0}, clipboard {}, tip {}, {}",
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
            // The tune editor claims the same column (UX0-C14), with the same
            // ambiguity and the same cure.
            self.tune_target.map_or("none", |target| target.token()),
            origins,
            // `1` means every cluster stayed flat, which is what the lane looked
            // like before the fan-out existed — so a capture that photographs an
            // overlapping document and still says `stack rows 1` is a capture of
            // the feature not running.
            self.stack_rows,
            self.lane_drawn,
            self.clipboard.len(),
            if self.tooltip_visible { "on" } else { "off" },
            self.describe_timing(),
        )
    }

    /// The timing loop's own evidence (UX0-B02/B03/B04/B05, UX0-C03).
    ///
    /// Every field here separates two pictures a capture cannot tell apart. An
    /// armed tap run and a disarmed one differ only by one line of hint text at
    /// 11 px; a stamp that landed and a stamp that was refused both leave the
    /// music playing and the panel looking identical; and an undo stack that is
    /// recording and one that is silently dropping every batch produce exactly
    /// the same frame until the moment someone presses Ctrl+Z and finds nothing
    /// there. `history 0/0` on a frame that has just been edited is the failure
    /// this line exists to make visible.
    fn describe_timing(&self) -> String {
        let tap = if self.tap.is_armed() {
            format!(
                "armed {}/{} stamped {} offset {:+.0}ms",
                self.tap.position(),
                self.tap.total(),
                self.tap.stamped(),
                self.tap_offset_seconds() * 1000.0
            )
        } else {
            "off".to_owned()
        };
        format!(
            "tap {tap}, last stamp {}, history {}/{}, typing {}",
            self.tap_last_seconds
                .map_or_else(|| "none".to_owned(), widgets::format_timestamp),
            self.history.undo_depth(),
            self.history.redo_depth(),
            match &self.time_edit {
                None if self.text.is_focused() => "cue-text",
                None => "none",
                Some(edit) if edit.is_start => "start-time",
                Some(_) => "end-time",
            }
        )
    }
}

impl Shell {
    /// The `lyrics column:` report line (WX-5d §5): pane, card, visible rows
    /// with their rects, current and selected cue, follow, the sample caption
    /// and the footer's controls. **The Lyrics builder's**; empty until built,
    /// and an empty line is not printed.
    #[must_use]
    pub(crate) fn describe_lyrics_column(&self, _workspace: &Workspace) -> String {
        if !self.lyrics_column_shown() {
            return String::new();
        }
        self.lyrics.column.lyrics_line.clone()
    }

    /// The `caption style:` report line (WX-5d §5): the Style pane's groups,
    /// face chips, swatches and place, each with its rect. **The Lyrics
    /// builder's**; empty until built.
    #[must_use]
    pub(crate) fn describe_caption_style(&self, _workspace: &Workspace) -> String {
        if !self.lyrics_column_shown() {
            return String::new();
        }
        self.lyrics.column.style_line.clone()
    }

    /// Whether the Lyrics view is in the column: the switch is on and the
    /// view is Lyrics. Read by the report lines, which are printed at
    /// shutdown, so a view left mid-run cannot report what it drew before.
    fn lyrics_column_shown(&self) -> bool {
        self.column_view() == super::super::shell::ColumnView::Lyrics
    }

    /// The cue the preview shows as a sample caption this frame (§3.2): the
    /// nearest displayable cue, while Style, Effects or the font browser is
    /// open in the column and no cue is live at `seconds`. `None` otherwise,
    /// and always `None` with the switch off.
    #[must_use]
    pub(crate) fn sample_cue_id(&self, workspace: &Workspace, seconds: f64) -> Option<u64> {
        self.sample_cue_for(self.lyrics.pane(), workspace, seconds)
    }

    /// [`Self::sample_cue_id`] for a pane named by the caller: the column draws
    /// with the editor taken out of `self`, so it passes its own.
    fn sample_cue_for(&self, pane: LyricsPane, workspace: &Workspace, seconds: f64) -> Option<u64> {
        if !self.lyrics_column_shown() || !pane.shapes_caption() {
            return None;
        }
        let track = workspace.current()?;
        // Liveness by the renderer's own rule (`at_time`), so the sample can
        // never claim a frame the real line is drawn on.
        if track.lyrics.at_time(seconds).is_some() {
            return None;
        }
        let displayable: Vec<&lyrics::LyricCue> = track
            .lyrics
            .cues()
            .iter()
            .filter(|cue| cue.origin.is_displayable())
            .collect();
        let spans: Vec<(f64, f64)> = displayable
            .iter()
            .map(|cue| (cue.start_seconds, cue.end_seconds))
            .collect();
        let index = musializer_core::ui::lyrics_column_layout::sample_cue(true, &spans, seconds)?;
        Some(displayable[index].id)
    }

    /// [`Self::sample_cue_id`] as the cue the scene renderer draws, for the
    /// preview only: `main.rs` hands it to `SceneRenderer::sample_caption`
    /// before the preview draw, and the export path never does.
    #[must_use]
    pub(crate) fn sample_caption(
        &self,
        workspace: &Workspace,
        seconds: f64,
    ) -> Option<musializer_core::scene::LyricCue> {
        let id = self.sample_cue_id(workspace, seconds)?;
        let track = workspace.current()?;
        let cue = track.lyrics.find(id)?;
        Some(musializer_core::scene::LyricCue {
            id: cue.id,
            start_seconds: cue.start_seconds,
            end_seconds: cue.end_seconds,
            text: cue.text.clone(),
            voice: track.lyrics.voice_of(cue.id).map(|voice| voice.style),
        })
    }

    /// The lyrics editor, in the timeline strip's place.
    ///
    /// Returns the y the zoom row should draw at — the bottom of the cue lane
    /// (LX1). `Shell::open_panel` passes it back to `timeline_strip`, so the
    /// readout follows whichever lane actually ended up last instead of being
    /// pinned above the panel by an assumption.
    pub(crate) fn lyrics_panel(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        // `fonts=PATH`: nothing else hands the browser its local family list,
        // so without this only the consent body could ever be photographed.
        if let Some(path) = self.lyrics.probe_catalogue.take() {
            match self.font_browser.load_catalogue_from_file(&path) {
                // The consent the browsing body needs, as `allow_network`'s
                // own comment intends for a probe. It starts no request: only
                // Download does, and no probe presses it.
                Ok(()) => self.font_browser.allow_network(),
                Err(error) => eprintln!("FONTS: --ui-probe fonts={}: {error}", path.display()),
            }
        }
        // WX-5d: the band keeps only the cue lane, exactly as every other
        // view has it, and the editor draws in the column's rect.
        let zoom_row_y = self.band_lyric_lane(d, input, content, strip, commands, true);
        let column = self.column_rect();
        let mut editor = std::mem::take(&mut self.lyrics);
        self.lyrics_column(d, input, &mut editor, column, commands);
        if let Some(seconds) = editor.take_seek_request() {
            commands.push(ShellCommand::Seek(seconds));
        }
        self.lyrics = editor;
        zoom_row_y
    }

    /// The cue lane alone, with the editor closed (D4).
    ///
    /// The lane is where lyric *timing* lives, and timing is judged against the
    /// waveform and the scene plan — which stay on screen when the editor does
    /// not. Closing the editor to see more preview used to take the cue blocks
    /// with it, so the commonest question in this whole workflow ("does that
    /// line land on that transient?") could only be asked in the one layout that
    /// hides most of the picture.
    ///
    /// **It costs no band height.** The `None`/`Tune` band is
    /// `DEFAULT_TIMELINE_HEIGHT + SCENE_SECTION_HEIGHT` (before WX-5c also the
    /// 66 px manual event row), and
    /// the 180 of that spends 56 on the waveform strip and 28 on the zoom row —
    /// so the lane draws in slack that was already reserved and already empty.
    /// That is deliberate rather than lucky: growing the band would move
    /// `workspace_layout`'s sidebar guarantee at 720p, which is the arithmetic
    /// that already forced the manual event row out of the lyrics band, and
    /// UX0-C17 records that kind of change as one that queues rather than
    /// sneaks in. If the slack is ever spent, this draws nothing and the zoom
    /// row goes back where it was.
    ///
    /// Fully interactive, not a picture of the lane. It is the same
    /// [`Self::lyric_lane`] the editor draws, so there is no second
    /// implementation to diverge — and every gesture it offers is reversible now
    /// that Ctrl+Z exists.
    pub(crate) fn closed_lyric_lane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        self.band_lyric_lane(d, input, content, strip, commands, false)
    }

    /// [`Self::closed_lyric_lane`], and the lane under the column's editor
    /// (`editor_open`): then the card's fields keep the keyboard, because they
    /// are on screen, and a draft carried in from another track is dropped
    /// out loud, as the band's editor does.
    fn band_lyric_lane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        content: UiRect,
        strip: UiRect,
        commands: &mut Vec<ShellCommand>,
        editor_open: bool,
    ) -> f32 {
        let below_strip = strip.y + strip.height;
        let Some(track) = input.workspace.current() else {
            return below_strip;
        };
        // What is left after the zoom row, which still has to fit under
        // whatever this draws.
        let available = content.y + content.height - below_strip - LANE_GAP * 2.0 - ZOOM_ROW_HEIGHT;
        if available < ORACLE_LANE_HEIGHT {
            return below_strip;
        }
        let mut editor = std::mem::take(&mut self.lyrics);
        // The same binding the panel does, and for the same reason: an edit this
        // lane pushes is stamped with the editor's owner slot, and an unbound
        // editor would stamp it `None` and have it refused at the drain.
        if editor.enter_track(input.workspace.current_index()) && editor_open {
            self.notify(
                Severity::Warning,
                "Unsaved lyric draft was dropped",
                "The new cue belonged to the track that was open before this one.",
            );
        }
        // Neither text surface is on screen here, so neither may keep the
        // keyboard. This is the UX0-A02 stranded-state shape one field over: a
        // time readout left mid-edit when the panel closed would make
        // `lyric_lane_keys` stand down for the rest of the session, and the
        // lane's own keys would silently stop working with nothing on screen
        // to explain why. Under the column's editor they are on screen.
        if !editor_open {
            editor.text.set_focused(false);
            editor.time_edit = None;
        }
        editor.cue_count = track.lyrics.len();
        editor.lane_selection.prune(&track.lyrics);
        // Capped at the slack rather than at the window, so this never grows the
        // band. A user who dragged the lane to 66 px in the editor gets 66 here
        // when it fits and less when it does not, instead of the panel below
        // being pushed off the bottom.
        editor.lane_height = self
            .ui_preferences
            .lyric_lane_height
            .unwrap_or(LANE_HEIGHT)
            .clamp(ORACLE_LANE_HEIGHT, available);
        editor.lane_drawn = editor.lane_height;

        let lane = UiRect::new(
            strip.x,
            below_strip + LANE_GAP,
            strip.width,
            editor.lane_drawn,
        );
        // The keys come with the lane. Ctrl+Z on a cue you can see and cannot
        // reach any other way is the whole point, and `lyric_lane_keys` already
        // refuses everything that needs the form.
        self.load_tap_calibration(&mut editor);
        self.lyric_lane_keys(d, input, &mut editor, track);
        self.store_tap_calibration(&mut editor, commands);
        self.lyric_lane(d, input, &mut editor, track, lane, commands);
        if let Some(seconds) = editor.take_seek_request() {
            commands.push(ShellCommand::Seek(seconds));
        }
        self.lyrics = editor;
        lane.y + lane.height
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
        commands: &mut Vec<ShellCommand>,
    ) {
        if lane.is_empty() {
            return;
        }
        self.timeline_pan_gesture(d, input, lane);
        // The resize claims the pointer before the cue gesture sees it, because
        // its strip is inside the lane and a press there must never also be a
        // press on the block above it.
        let resizing = self.lane_resize_gesture(d, input, editor, lane, commands);
        if !resizing {
            self.lane_gesture(d, input.ui_scale, editor, track, lane);
        }

        // The wheel zooms the shared view from this lane too (operator request,
        // 2026-08-06). Three lanes draw the same time axis and it was arbitrary
        // that only the PCM strip answered the wheel — the lane a user is aiming
        // at a 2 s cue in is precisely the one they want to zoom from.
        //
        // `Shell::request_timeline_zoom` takes the first claim of the frame and
        // applies it on the *next* one, so two lanes cannot compound one notch
        // and all three consume the new view together. Strictly inside the lane
        // rect, which is what keeps it from stealing the cue list's scroll.
        //
        // Declined mid-gesture: moving the axis under a drag would retime a cue
        // by an amount the hand never asked for.
        let pointer = input.ui_scale.mouse(d);
        if !resizing && !editor.lane_drag.active && lane.contains_point(pointer.x, pointer.y) {
            self.request_timeline_zoom(
                self.wheel_delta(d),
                pointer.x,
                lane.x,
                lane.width,
                input.duration_seconds,
            );
        }

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

        // Where overlapping cues go (LX1-d). Built for the whole document rather
        // than per pair, so a cue's row cannot change because of a cue it does
        // not touch — see `lyric_lane_stack`'s three rules.
        let stack = LyricStack::build(&track.lyrics);
        let rows = stack.rows();
        editor.stack_rows = rows;
        editor.origin_counts = ORIGIN_ORDER.map(|origin| {
            track
                .lyrics
                .cues()
                .iter()
                .filter(|cue| cue.origin == origin)
                .count()
        });
        // The band the rows tile. Asymmetric on purpose since the grab strip grew
        // to 7 px: the top keeps the oracle's 3 px breathing room, and only the
        // bottom gives up the whole strip, because every pixel spent above the
        // blocks is click area bought back for nothing. No block is ever drawn in
        // the strip, which is what lets a resize press never be a press on a cue.
        let content_y = lane.y + LANE_BLOCK_TOP_INSET;
        let content_height =
            (lane.height - LANE_BLOCK_TOP_INSET - LANE_RESIZE_GRAB_PIXELS).max(1.0);
        let mut tip: Option<(UiRect, String)> = None;
        // The line on screen at the playhead (WX-5b, the mockup's `.ly.now`):
        // the display resolver's answer rather than every block the playhead
        // crosses, so a proposal parked under it, or a line a later one talks
        // over, is not dressed as the line the frame is showing.
        let now_id = track
            .lyrics
            .at_time(self.timeline_playhead_seconds(input.time_seconds))
            .map(|cue| cue.id);

        for (index, cue) in track.lyrics.cues().iter().enumerate() {
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
            let slot = stack.slot(index);
            let (row_offset, row_height) = row_geometry(content_height, rows, slot.row);
            // The part of this row nothing is drawn over, which is where a label
            // belongs. Rows are nested rather than tiled — every row runs to the
            // bottom of the band and the next one starts partway down it — so
            // centring text in `row_height` puts three labels within a few
            // pixels of each other and they collide. The next row's offset is
            // exactly where this one stops being visible.
            let label_height = if slot.row + 1 < stack.cluster_rows(index) {
                row_geometry(content_height, rows, slot.row + 1).0 - row_offset
            } else {
                row_height
            };
            // Clipped to the lane by the same geometry the hit test reads. A
            // boundary which scrolled off-screen is therefore never recreated
            // as a handle at the lane border.
            let block = UiRect::new(
                geometry.left as f32,
                content_y + row_offset,
                geometry.width() as f32,
                row_height,
            );

            let in_selection = editor.lane_selection.contains(cue.id);
            let hovered = hover.is_some_and(|hit| hit.id == cue.id);
            let now = now_id == Some(cue.id);
            // WX-5b: a neutral block, the words on it, and provenance as a
            // stripe and a top line — the mockup's calm row of words. Before it
            // every block was its origin colour at 38 % over the lane with a
            // full-strength outline, and a track of user-applied lines read as
            // a green barcode in which the words were the quietest thing.
            //
            // The fill darkens with the stack row (LX1-d), so a fanned cluster
            // still reads as rows, brightest on top.
            let fill = cue_block_fill(now, in_selection, slot.shade_steps);
            // A one-pixel seam at the block's end, so two cues that abut read
            // as two lines rather than one long one — the mockup's
            // `border-right`. Only where the end is on screen and the block can
            // spare it; the hit test is unchanged either way.
            let body = UiRect::new(
                block.x,
                block.y,
                if geometry.end_visible && block.width >= 4.0 {
                    block.width - 1.0
                } else {
                    block.width
                },
                block.height,
            );
            widgets::fill(d, body, fill);
            let mark = origin_mark(cue.origin, fill, slot.shade_steps);
            // A proposal is hatched as well as greyed, so the one state that
            // means "not a placement" is legible without colour vision at all.
            //
            // This cannot be confused with the shadow hatching below even though
            // both are 45° strokes, because the two can never appear on the same
            // block: `LyricsDocument::cue_shadow` returns `None` for a cue that
            // is not displayable and skips one as a shadower, so a `Potential`
            // block is never in `shadows` and never carries the warning wash.
            // The pitch differs anyway, at twice the shadow's spacing.
            if cue.origin == CueOrigin::Potential {
                hatch_spaced(d, body, fade(mark, 0.7), HATCH_SPACING * 2.0);
            }
            // Selection is an outline and a tint (WX-5a), drawn before the
            // origin marks so the stripe and the top line stay the origin's
            // colour on a selected block too.
            if in_selection {
                d.draw_rectangle_lines_ex(widgets::rectangle(body), 1.0, color::ui_selected_edge());
            }
            // A span the preview and the export will never show this cue in is
            // greyed and hatched, so two blocks in one colour stop meaning the
            // same thing (review 1.14).
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
            // move several cues at once, and one colour cannot say both. Before
            // the origin marks, so the stripe keeps its three columns.
            if cue.id == editor.selected_id {
                d.draw_rectangle_lines_ex(
                    widgets::rectangle(UiRect::new(
                        body.x + 1.0,
                        body.y + 1.0,
                        body.width - 2.0,
                        body.height - 2.0,
                    )),
                    1.0,
                    color::ui_ink(),
                );
            }
            // Provenance (LX1-d): a 1 px line along the top and a stripe down
            // the start, in the origin's colour. Both are opaque, which is what
            // lets the gate count them by exact colour.
            //
            // The top line is drawn on every block and is what carries the code
            // at every zoom. The stripe marks where the cue *starts*, so it is
            // drawn only where the start is on screen, and only on a block wide
            // enough to carry words: on a whole-track view most cues are a few
            // pixels wide, and a 3 px bar on each made a 10 px block a third
            // origin colour — the barcode this re-look exists to remove.
            widgets::fill(d, UiRect::new(body.x, body.y, body.width, 1.0), mark);
            if geometry.start_visible && block.width >= LANE_TEXT_MIN_BLOCK_PIXELS {
                widgets::fill(
                    d,
                    UiRect::new(
                        body.x,
                        body.y,
                        ORIGIN_STRIPE_PIXELS.min(body.width),
                        body.height,
                    ),
                    mark,
                );
            }
            // The cue's own words, inside the block (operator request,
            // 2026-08-06). The lane is 2.25x the oracle's height now, and the
            // space bought for the click target is also space a block can use to
            // say what it is — which is the difference between a row of coloured
            // rectangles and a lane you can read without hovering every cue.
            //
            // Typed through the **authored** face, not the chrome bank. This is
            // a cue's text, and `Faces::ui()` is Latin-only — routing it there is
            // exactly UX0-A05, where Greek cue rows drew as `?` for weeks.
            //
            // The left inset clears the start grab bar so a handle is never
            // drawn over a letter, and falls back to a bare 3 px when the start
            // scrolled off the side, because there is no handle there to clear.
            let label = cue.text.trim();
            let inset = if geometry.start_visible {
                LYRIC_LANE_EDGE_GRAB_PIXELS as f32 + 2.0
            } else {
                3.0
            };
            let label_width = block.width - inset - 3.0;
            if !label.is_empty()
                && label_height >= LANE_TEXT_MIN_ROW_PIXELS
                && block.width >= LANE_TEXT_MIN_BLOCK_PIXELS
                && label_width > 0.0
            {
                let size = metric::UI_FONT_CAPTION.min(label_height - 6.0).max(9.0);
                // Muted serif words, ink on the line on screen (the mockup's
                // `.ly` and `.ly.now`), checked against the opaque fill they sit
                // on: a stack row four shade steps down is a different
                // background from row 0.
                let ink = cue_word_ink(now, fill);
                widgets::draw_text_faded(
                    d,
                    &input.fonts.authored(),
                    label,
                    block.x + inset,
                    block.y + (label_height - size) * 0.5,
                    label_width,
                    LANE_TEXT_FADE_PIXELS.min(label_width * 0.5),
                    size,
                    ink,
                );
            }
            // **Both** grab bars, on any hovered block wide enough to have them,
            // at the width the hit test actually claims (operator feedback,
            // 2026-08-06: *"the hover to drag begin and ending needs to be a bit
            // more visible"*, after finding them only by zooming in until a cue
            // was wide enough to stumble onto one).
            //
            // The old version drew a 2 px line and only once the pointer was
            // already inside the 5 px zone, so finding a handle required
            // standing on it first. That is the same "you have to be on it to
            // learn it is there" defect the lane's own resize had, and it is
            // worse here because a block is a target the user is already aiming
            // at — the affordance has to be legible from the body of the block,
            // not from the 5 px they are trying to find.
            //
            // The geometry is the hit test's own — `true_left`/`true_right`, and
            // its two visibility flags — not the drawn block's, so a handle can
            // never be painted where a press would not take it, and a boundary
            // that scrolled off the side is offered by neither.
            let offers_handles = hovered
                && !editor.lane_drag.active
                && geometry.true_width() >= LYRIC_LANE_EDGE_MIN_BLOCK_PIXELS;
            if offers_handles {
                let grab = LYRIC_LANE_EDGE_GRAB_PIXELS as f32;
                let held = hover.map(|hit| hit.zone);
                for (present, edge, bar_x) in [
                    (
                        geometry.start_visible,
                        LyricLaneZone::StartEdge,
                        geometry.true_left as f32,
                    ),
                    (
                        geometry.end_visible,
                        LyricLaneZone::EndEdge,
                        geometry.true_right as f32 - grab,
                    ),
                ] {
                    if !present {
                        continue;
                    }
                    let live = held == Some(edge);
                    // The band, which is the honest picture of the grab zone,
                    // and then the boundary column itself — so "this is the edge
                    // you are about to move" is a hard line rather than the
                    // middle of a soft one.
                    widgets::fill(
                        d,
                        UiRect::new(bar_x, block.y + 1.0, grab, (block.height - 2.0).max(1.0)),
                        fade(color::ui_ink(), if live { 0.60 } else { 0.32 }),
                    );
                    let rule_x = if edge == LyricLaneZone::StartEdge {
                        bar_x
                    } else {
                        bar_x + grab - 2.0
                    };
                    widgets::fill(
                        d,
                        UiRect::new(rule_x, block.y, 2.0, block.height),
                        fade(color::ui_ink(), if live { 1.0 } else { 0.55 }),
                    );
                    if live {
                        // The conventional signal, and the one this lane could
                        // not give until `Shell::request_cursor` existed.
                        self.request_cursor(MouseCursor::MOUSE_CURSOR_RESIZE_EW);
                    }
                }
            }
            if hovered && !editor.lane_drag.active {
                // Anchored to the **whole lane**, not to the block. That is the
                // entire tooltip requirement expressed as geometry rather than
                // as a rule to remember: `widgets::tooltip_box` puts the tip
                // clear of its anchor's vertical extent, so an anchor spanning
                // the lane cannot produce a tip that covers a neighbouring cue.
                // Horizontally it still centres on the block, so it points at
                // the thing it describes.
                let anchor = UiRect::new(block.x, lane.y, block.width, lane.height);
                tip = Some((anchor, lane_tooltip_text(cue)));
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

        // Last in the lane, and nowhere else. Drawn here rather than deferred
        // through `Widgets::hint` for two reasons that are both about this
        // surface specifically:
        //
        // - **No dwell.** The operator asked for a tip that appears the instant
        //   the pointer enters a block. `hint`'s delay is shared state that
        //   every other control in the application depends on, and a lane that
        //   zeroed it would take the strobe with it. Drawing locally means the
        //   tip exists exactly on the frames the pointer is over a block and on
        //   no others, which is also the "not sticky" requirement, for free.
        // - **The face.** The tip carries the cue's *own words*, and
        //   `widgets::draw_tooltip` types through the Latin-only chrome bank —
        //   the same defect that drew Greek cue rows as `?` for weeks (UX0-A05).
        //   The authored atlas is glyph-complete and is what the rows use.
        //
        // Z-order is safe in the one direction it needs to be: the waveform
        // strip is drawn *before* the open panel, and `tooltip_box` prefers
        // above, so the tip lands on top of the strip rather than under it.
        // The resize grip, over the lane's own fill and under the tip. This is
        // where it has to be: `lane_resize_gesture` runs before the fill and
        // anything it drew was painted over, which is why the affordance has
        // never appeared in a single frame.
        // "Resizable" is whether this window affords a *range*, not whether the
        // lane is currently below the ceiling. A lane already dragged to the
        // maximum can still be dragged back down, and hiding the handle there
        // would make the gesture one-way.
        let ceiling = lane_height_ceiling(input.window.1);
        self.draw_lane_resize_grip(d, editor, lane, ceiling > LANE_HEIGHT.min(ceiling) + 0.5);

        editor.tooltip_visible = false;
        if let Some((anchor, text)) = tip {
            self.draw_lane_tooltip(d, input, anchor, &text);
            editor.tooltip_visible = true;
        }
    }

    /// The cue lane's own tooltip plate, typed through the authored face.
    fn draw_lane_tooltip(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        anchor: UiRect,
        text: &str,
    ) {
        let authored = input.fonts.authored();
        let size = metric::UI_FONT_CAPTION;
        // Bounded before it is measured: a 512-character cue would otherwise ask
        // for a plate wider than the window, and `tooltip_box` would clamp the
        // box while the text kept going.
        let budget = (input.window.0 * 0.5).max(160.0);
        let fitted = authored.ellipsize(text, budget, size, 0.0);
        let width = authored.measure_text(&fitted, size, 0.0).x;
        let boundary = widgets::tooltip_box(anchor, width, input.window);
        if boundary.is_empty() {
            return;
        }
        widgets::tooltip_plate(d, boundary);
        let (text_x, text_y) = widgets::tooltip_text_origin(boundary);
        authored.draw_text(
            d,
            &fitted,
            Vector2::new(text_x, text_y),
            size,
            0.0,
            color::ui_tooltip_ink(),
        );
    }

    /// The lane's resize handle, drawn **after** everything else in the lane.
    ///
    /// Always visible, not only under the pointer. The operator's original
    /// instruction was that no border be drawn here — "a desperate user will find
    /// the resize if they need it" — and then the operator could not find it and
    /// asked for "a bit of a stronger border there for resizing" (2026-08-06).
    /// Both requests are honoured by what this draws: two short grip lines
    /// centred on the edge, which read as a handle rather than as a fourth lane
    /// border, and which grow and darken while the pointer is on them or a drag
    /// is live — so the state is legible without depending on a cursor shape the
    /// window manager may decline to change.
    fn draw_lane_resize_grip(
        &self,
        d: &mut RaylibDrawHandle<'_>,
        editor: &LyricEditor,
        lane: UiRect,
        resizable: bool,
    ) {
        // A handle that cannot move is worse than none: it invites a drag that
        // does nothing. On a window too short to seat more lane there is nothing
        // to grab, and the lane says so by not offering.
        if !resizable || lane.width < 48.0 {
            return;
        }
        let emphasised = editor.lane_resize_hovered;
        let width = if emphasised { 56.0f32 } else { 36.0 }.min(lane.width);
        let tint = if emphasised {
            color::ui_ink()
        } else {
            color::ui_muted()
        };
        for (index, offset) in [5.0f32, 3.0].into_iter().enumerate() {
            widgets::fill(
                d,
                UiRect::new(
                    lane.x + (lane.width - width) * 0.5,
                    lane.y + lane.height - offset,
                    width,
                    1.0,
                ),
                widgets::alpha(tint, if index == 0 { 0.9 } else { 0.6 }),
            );
        }
    }

    /// The lane's bottom-edge resize (LX1-d). Returns whether it owns the
    /// pointer this frame, so the cue gesture can stand down.
    ///
    /// The strip is the lane's bottom [`LANE_RESIZE_GRAB_PIXELS`], which is
    /// exactly the band no block is drawn in, so a resize press is never also a
    /// press on a cue and the two gestures cannot both be started.
    ///
    /// The drawn affordance is [`Self::draw_lane_resize_grip`]'s, because
    /// anything painted from here is covered by the lane's own fill on the next
    /// line. What this adds is the cursor, which was unavailable until
    /// `Shell::request_cursor` existed: `Shell::splitters` runs after every panel
    /// and used to set the cursor unconditionally back to `MOUSE_CURSOR_DEFAULT`,
    /// so a shape asked for here was overwritten in the same frame.
    fn lane_resize_gesture(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        lane: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> bool {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        let mouse = input.ui_scale.mouse(d);
        let strip = UiRect::new(
            lane.x,
            lane.y + lane.height - LANE_RESIZE_GRAB_PIXELS,
            lane.width,
            LANE_RESIZE_GRAB_PIXELS,
        );
        let hovered = strip.contains_point(mouse.x, mouse.y);
        // Recorded rather than drawn here. This function runs *before* the lane
        // is filled — it has to, because the gesture decides whether the lane is
        // even the height it is about to be drawn at — so anything it painted
        // would be covered by `widgets::fill(d, lane, ..)` on the next line. That
        // is why the original hover dimple was never visible in any frame, hover
        // or not, and why the operator reported the resize as simply not working.
        editor.lane_resize_hovered = hovered || editor.lane_resizing;
        if editor.lane_resize_hovered {
            self.request_cursor(MouseCursor::MOUSE_CURSOR_RESIZE_NS);
        }

        if editor.lane_resizing {
            if d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
                editor.lane_height = clamp_lane_height(mouse.y - lane.y, input.window.1);
                self.ui_preferences.lyric_lane_height = Some(editor.lane_height);
            } else {
                editor.lane_resizing = false;
                // Written once, on release. The preference file is replaced
                // atomically, and doing that on every frame of a drag would be
                // sixty rewrites a second of a file the user may be editing.
                commands.push(ShellCommand::SaveUiPreferences(self.ui_preferences));
            }
            return true;
        }

        if hovered && d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) {
            editor.lane_resizing = true;
            return true;
        }
        false
    }

    /// Copy, cut and paste over the lane selection (LX1-d).
    ///
    /// **Guarded on the text field, not on the pointer.** `Ctrl+V` inside the
    /// cue field means paste *text* — the form's own hint row says so — and the
    /// field reads raylib's keyboard directly, so this handler has to decline
    /// while it has focus rather than expect to win. The style, effects and font
    /// panes are excluded too: there is no lane on screen in any of them, and a
    /// shortcut that acts on an invisible selection is the interface doing
    /// something the user cannot see.
    /// Hands the editor the calibration this user measured last time (PXF-2).
    ///
    /// Run before the timing keys on every frame that has a lane, rather than
    /// once at startup, because the editor is reachable from two draw paths and
    /// a load that only one of them performed would leave the offset depending
    /// on which panel the user opened first.
    fn load_tap_calibration(&mut self, editor: &mut LyricEditor) {
        editor.set_tap_offset(self.ui_preferences.lyric_tap_offset.map(f64::from));
    }

    /// Writes a calibration the user just changed back to the preference file.
    ///
    /// Immediately, not on the run ending: the offset is dialled in while the
    /// track plays and the session that follows may end with the window being
    /// closed, which is precisely the launch that has to remember it. One
    /// atomic replace per keypress is affordable in a way the lane drag's sixty
    /// a second was not.
    fn store_tap_calibration(
        &mut self,
        editor: &mut LyricEditor,
        commands: &mut Vec<ShellCommand>,
    ) {
        let Some(offset) = editor.take_tap_offset_request() else {
            return;
        };
        self.ui_preferences.lyric_tap_offset = Some(offset as f32);
        commands.push(ShellCommand::SaveUiPreferences(self.ui_preferences));
    }

    fn lyric_lane_keys(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
    ) {
        use raylib::consts::KeyboardKey as Key;

        if !lyric_clipboard_keys_allowed(
            editor.style_pane,
            editor.font_pane,
            editor.text.is_focused() || editor.time_edit.is_some(),
        ) {
            return;
        }

        // The timing loop's own bindings, ahead of the clipboard's, because the
        // one key a tap run needs must never be reachable only through a
        // modifier the other hand is already holding.
        self.lyric_timing_keys(d, input, editor, track);

        if !(d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL)) {
            return;
        }
        let copy = d.is_key_pressed(Key::KEY_C);
        let cut = d.is_key_pressed(Key::KEY_X);
        let paste = d.is_key_pressed(Key::KEY_V);

        if copy || cut {
            let ids = editor.lane_selection.ids().to_vec();
            let Some(clipboard) = LyricClipboard::copy(&track.lyrics, &ids) else {
                // A silent copy is indistinguishable from a broken one, and so
                // is a silent refusal.
                self.notify(
                    Severity::Warning,
                    "Nothing to copy",
                    "Select one or more cues in the lane first.",
                );
                return;
            };
            let count = clipboard.len();
            editor.clipboard = clipboard;
            if cut {
                for id in ids {
                    editor.push(LyricsEdit::Delete { id });
                }
                editor.clear_draft();
                editor.lane_selection.clear();
            }
            self.notify(
                Severity::Success,
                if cut { "Cues cut" } else { "Cues copied" },
                &format!("{count} cue(s) on the clipboard. Ctrl+V pastes at the playhead."),
            );
            return;
        }

        if !paste {
            return;
        }
        let at = self.timeline_playhead_seconds(input.time_seconds);
        let pasted = match editor.clipboard.paste_at(&track.lyrics, at) {
            Ok(pasted) => pasted,
            Err(refusal) => {
                self.notify(Severity::Warning, "Paste refused", &refusal.to_string());
                return;
            }
        };
        // Dry run first, against a copy of the document. `LyricsEdit::apply`
        // runs the model's validating insert one cue at a time and `main.rs`
        // stops at the first error, which would leave half a paste behind with
        // no undo. Rehearsing it here makes the whole paste an all-or-nothing
        // operation without giving this panel a way to write to the model.
        let mut rehearsal = track.lyrics.clone();
        for cue in &pasted {
            if let Err(error) = rehearsal.insert(cue.clone()) {
                self.notify(
                    Severity::Warning,
                    "Paste refused",
                    &format!("Nothing was pasted: {error}"),
                );
                return;
            }
        }
        let count = pasted.len();
        for cue in pasted {
            editor.push(LyricsEdit::Insert {
                start_seconds: cue.start_seconds,
                end_seconds: cue.end_seconds,
                text: cue.text,
            });
        }
        self.notify(
            Severity::Success,
            "Cues pasted",
            &format!("{count} cue(s) at {}.", widgets::format_timestamp(at)),
        );
    }

    /// The timing loop's keyboard (UX0-B02/B03/B04/B05, UX0-C03).
    ///
    /// **Every binding here is a key the shell does not already own**, and that
    /// constraint chose most of them. The shell reads the keyboard before any
    /// panel draws (`Shell::keyboard`), and its globals are unconditional once no
    /// text field has focus: `T` opens the inspector, `M` mutes, `F` goes
    /// fullscreen, `H` toggles the readout, `S` saves, `Space` plays, `Tab`
    /// cycles the scene, and the arrows seek. A panel key that shadowed one of
    /// those would fire *both*, because the panel runs second and neither side
    /// consumes the press.
    ///
    /// So the tap key is **Enter**. It is the only unbound key that is large,
    /// central and reachable without looking — which for a control the user
    /// presses in time with music is not a nicety. `Ctrl+Enter` arms the run, and
    /// cannot collide with the form's own `Ctrl+Enter` because that one requires
    /// the cue field to have focus and an armed run requires it not to.
    fn lyric_timing_keys(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
    ) {
        use raylib::consts::KeyboardKey as Key;

        let control = d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL);
        let shift = d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT);
        let playhead = self.timeline_playhead_seconds(input.time_seconds);
        let enter = d.is_key_pressed(Key::KEY_ENTER) || d.is_key_pressed(Key::KEY_KP_ENTER);

        // -- the tap run -----------------------------------------------------
        if enter && control {
            self.toggle_tap_run(editor, track, playhead);
            return;
        }
        if enter {
            self.tap_once(editor, track, playhead);
            return;
        }
        if editor.tap_is_armed() {
            if d.is_key_pressed(Key::KEY_ESCAPE) {
                editor.toggle_tap(&track.lyrics, playhead);
                self.notify(
                    Severity::Info,
                    "Tap run ended",
                    "Ctrl+Enter starts another one from the playhead.",
                );
                return;
            }
            // The calibration keys, live during a run so a user who hears
            // themselves landing late can fix it without stopping.
            let steps = i32::from(d.is_key_pressed(Key::KEY_RIGHT_BRACKET))
                - i32::from(d.is_key_pressed(Key::KEY_LEFT_BRACKET));
            if steps != 0 {
                editor.adjust_tap_offset(steps);
            }
        }

        // -- undo and redo ---------------------------------------------------
        if control && (d.is_key_pressed(Key::KEY_Z) || d.is_key_pressed(Key::KEY_Y)) {
            let step = if shift || d.is_key_pressed(Key::KEY_Y) {
                HistoryStep::Redo
            } else {
                HistoryStep::Undo
            };
            let available = match step {
                HistoryStep::Undo => editor.can_undo(),
                HistoryStep::Redo => editor.can_redo(),
            };
            if available {
                editor.request_history(step);
            } else {
                // A shortcut that silently does nothing is indistinguishable
                // from one that is not bound, and this repository has a rule
                // about that.
                self.notify(
                    Severity::Info,
                    match step {
                        HistoryStep::Undo => "Nothing to undo",
                        HistoryStep::Redo => "Nothing to redo",
                    },
                    "The cue history starts when this track becomes current and holds 64 steps.",
                );
            }
            return;
        }

        // -- split and merge (UX0-B05) ---------------------------------------
        if control && d.is_key_pressed(Key::KEY_B) {
            self.split_selected(editor, track, playhead);
            return;
        }
        if control && d.is_key_pressed(Key::KEY_J) {
            self.merge_selected(editor, track);
            return;
        }

        // -- safe deletion (UX0-B03) -----------------------------------------
        //
        // Bound at all only because undo exists: the review's objection to the
        // Delete *button* was that it was one unconfirmed click with no way
        // back, and a key is one unconfirmed keystroke. The answer is the same
        // for both, and it is Ctrl+Z rather than a confirmation dialog — a
        // reversible destructive action does not need to be slow.
        if d.is_key_pressed(Key::KEY_DELETE) && editor.selected_id != 0 && !editor.draft_new {
            let id = editor.selected_id;
            editor.push(LyricsEdit::Delete { id });
            editor.clear_draft();
            self.notify(
                Severity::Info,
                "Cue deleted",
                "Ctrl+Z puts it back, with its id and its provenance.",
            );
            return;
        }

        // -- selection and nudging -------------------------------------------
        if d.is_key_pressed(Key::KEY_DOWN) || d.is_key_pressed(Key::KEY_UP) {
            let forward = d.is_key_pressed(Key::KEY_DOWN);
            editor.step_selection(&track.lyrics, forward, playhead);
            return;
        }
        let nudge = i32::from(
            d.is_key_pressed(Key::KEY_PERIOD) || d.is_key_pressed_repeat(Key::KEY_PERIOD),
        ) - i32::from(
            d.is_key_pressed(Key::KEY_COMMA) || d.is_key_pressed_repeat(Key::KEY_COMMA),
        );
        if nudge != 0 && !editor.lane_selection.is_empty() {
            let step = lyric_lane_edit::cue_nudge_step_seconds(control, shift) * f64::from(nudge);
            let delta = lyric_lane_edit::clamp_move(&track.lyrics, &editor.lane_selection, step);
            if delta != 0.0 {
                editor.push(LyricsEdit::ShiftMany {
                    ids: editor.lane_selection.ids().to_vec(),
                    delta_seconds: delta,
                });
            }
        }
    }

    /// Arms or ends a tap run, and says which (UX0-C03).
    fn toggle_tap_run(&mut self, editor: &mut LyricEditor, track: &Track, playhead: f64) {
        if editor.tap_is_armed() {
            editor.toggle_tap(&track.lyrics, playhead);
            self.notify(
                Severity::Info,
                "Tap run ended",
                "Ctrl+Enter starts another one from the playhead.",
            );
            return;
        }
        if !self.allow_context_change(editor, track) {
            return;
        }
        if editor.toggle_tap(&track.lyrics, playhead) {
            self.notify(
                Severity::Success,
                "Tap mode armed",
                "Play, then press Enter on each line. Every press closes the line before it.",
            );
        } else {
            // The one refusal a user meets by accident, so it names the way out
            // rather than only the problem.
            self.notify(
                Severity::Warning,
                "Nothing to stamp",
                "Tapping places the times of lines that already exist. Add cues, or import a .lyrics.tsv.",
            );
        }
    }

    /// One tap press.
    fn tap_once(&mut self, editor: &mut LyricEditor, track: &Track, playhead: f64) {
        match editor.tap(&track.lyrics, playhead) {
            Ok(0) => self.notify(
                Severity::Success,
                "Tap run finished",
                "Every line has a start, and the last one has an end.",
            ),
            Ok(_) => {}
            // Not armed is not a refusal worth a notice: Enter is a key people
            // press for many reasons, and answering every one of them with a
            // toast would make the panel nag.
            Err(TapRefusal::NotArmed) => {}
            Err(refusal) => self.notify(
                Severity::Warning,
                "That tap did not land",
                &refusal.to_string(),
            ),
        }
    }

    /// Breaks the selected cue at the playhead (UX0-B05).
    ///
    /// The text is split at the word boundary nearest the seam's position within
    /// the cue, which is a guess — but it is the guess a subtitle editor makes,
    /// and it is right for the case this exists for: one imported line covering
    /// two sung phrases. A wrong guess costs one edit in a field that is already
    /// open; retyping both halves is what it replaces.
    fn split_selected(&mut self, editor: &mut LyricEditor, track: &Track, playhead: f64) {
        let Some(cue) = track.lyrics.find(editor.selected_id) else {
            self.notify(
                Severity::Warning,
                "No cue to split",
                "Select a line in the lane or the list first.",
            );
            return;
        };
        if playhead <= cue.start_seconds + LYRIC_MIN_CUE_SECONDS
            || playhead >= cue.end_seconds - LYRIC_MIN_CUE_SECONDS
        {
            self.notify(
                Severity::Warning,
                "The playhead is not inside that cue",
                "Move it into the line, at least 20 ms from either end, and split there.",
            );
            return;
        }
        let fraction = (playhead - cue.start_seconds) / (cue.end_seconds - cue.start_seconds);
        let (left, right) = split_text_at_fraction(&cue.text, fraction);
        editor.push(LyricsEdit::Split {
            id: cue.id,
            split_seconds: playhead,
            left_text: left,
            right_text: right,
        });
        self.notify(
            Severity::Success,
            "Cue split",
            "The words were divided at the nearest space. Ctrl+Z undoes it.",
        );
    }

    /// Joins the selected cue with the one after it (UX0-B05).
    fn merge_selected(&mut self, editor: &mut LyricEditor, track: &Track) {
        let Some(index) = track.lyrics.index_of(editor.selected_id) else {
            self.notify(
                Severity::Warning,
                "No cue to merge",
                "Select a line in the lane or the list first.",
            );
            return;
        };
        let Some(next) = track.lyrics.cues().get(index + 1) else {
            self.notify(
                Severity::Warning,
                "Nothing to merge with",
                "The selected line is the last one in the document.",
            );
            return;
        };
        editor.push(LyricsEdit::Merge {
            first_id: track.lyrics.cues()[index].id,
            second_id: next.id,
            separator: " ".to_owned(),
        });
        self.notify(
            Severity::Success,
            "Cues merged",
            "The two lines are one, spanning both. Ctrl+Z undoes it.",
        );
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

    // -----------------------------------------------------------------------
    // The editing form
    // -----------------------------------------------------------------------

    /// The `MM:SS.mmm` readout, as a field a user can type into (UX0-B04).
    ///
    /// Returns a new value only when a typed time was **committed**, never on
    /// every keystroke. Typing `0` on the way to `01:23` would otherwise clamp
    /// the cue to the top of the track and take the rest of the string with it.
    ///
    /// Three ways out, and they are deliberately not the same: Enter commits or
    /// complains, Escape abandons, and clicking away commits only if what is
    /// there parses. A refused string typed at Enter is *kept in the field* so
    /// the user can see it and fix it — a field that empties itself when you get
    /// one character wrong is the most annoying possible answer.
    #[allow(
        clippy::too_many_arguments,
        reason = "one rectangle, two faces, the draft it writes and the id namespace it draws in"
    )]
    fn time_readout(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        authored: musializer_runtime::font::AuthoredText<'_>,
        editor: &mut LyricEditor,
        boundary: UiRect,
        is_start: bool,
        value: f64,
        id_base: u32,
        (display_size, text_dy, text_dx, edit_size): (f32, f32, f32, f32),
    ) -> Option<f64> {
        use raylib::consts::KeyboardKey as Key;

        let id = widgets::widget_id(ns::FORM, id_base + 3);
        let editing = editor
            .time_edit
            .as_ref()
            .is_some_and(|edit| edit.is_start == is_start);
        if !editing {
            let state = self.widgets.button(d, id, boundary);
            if state.hovered {
                widgets::fill(d, boundary, color::ui_hover());
            }
            widgets::draw_text(
                d,
                font,
                &widgets::format_timestamp(value),
                boundary.x + text_dx,
                boundary.y + text_dy,
                display_size,
                color::ui_ink(),
            );
            self.widgets.hint(
                d,
                state,
                id,
                boundary,
                "Click to type an exact time, as MM:SS.mmm or in seconds",
            );
            if state.clicked {
                let mut field = TextField::new(TextRules::ascii_query(16));
                field.bind(&widgets::format_timestamp(value));
                field.set_focused(true);
                field.edit.select_all();
                // A tap run and a text field cannot both own the keyboard, and
                // the run is the one with a mode indicator, so it stands down.
                editor.tap.disarm();
                editor.time_edit = Some(TimeEdit { is_start, field });
            }
            return None;
        }

        let edit = editor.time_edit.as_mut()?;
        // Drawn through the authored face because `TextField::draw_with_face`
        // takes only that one. A timecode is ASCII digits, a colon and a full
        // stop, so the chrome bank would serve it identically — this is an API
        // shape rather than a UX0-A05 decision.
        edit.field
            .draw_with_face(d, authored, boundary, edit_size, "MM:SS.mmm", true);
        let commit = d.is_key_pressed(Key::KEY_ENTER) || d.is_key_pressed(Key::KEY_KP_ENTER);
        let abandon = d.is_key_pressed(Key::KEY_ESCAPE);
        // Clicking away is neither of those, and it is the case worth thinking
        // about: discarding a finished `01:23.456` because the user reached for
        // the lane instead of pressing Enter loses work silently, and applying a
        // half-typed `01:2` would move the cue somewhere nobody asked for. So a
        // blur commits **only if it parses**, and otherwise drops it without a
        // notice — the user is already looking somewhere else, and a toast about
        // a field they have left is nagging.
        let blurred = !edit.field.is_focused();
        if !(commit || abandon || blurred) {
            return None;
        }
        let typed = edit.field.edit.text().to_owned();
        if abandon {
            editor.time_edit = None;
            return None;
        }
        let parsed = lyric_lane_edit::parse_cue_timestamp(&typed);
        if parsed.is_some() || blurred {
            editor.time_edit = None;
            return parsed;
        }
        // Enter on something unparseable. The string stays in the field so the
        // user can see what they typed and fix it: a field that empties itself
        // when you get one character wrong is the most annoying possible answer.
        self.notify(
            Severity::Warning,
            "That is not a time",
            "Type it as MM:SS.mmm — 01:23.456 — or as plain seconds. Escape leaves it alone.",
        );
        None
    }

    // -----------------------------------------------------------------------
    // The caption typography pane
    // -----------------------------------------------------------------------

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
            color::ui_ink(),
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

    /// The drive tuning editor (UX0-C14), in the effects form's left column:
    /// heading with Clamp and Reset, the live meter, the quiet/loud anchor
    /// pairs and the curve stepper — the Tune inspector's route editor, built
    /// from the same [`mapping_editor`] pieces, writing a [`DriveTuning`]
    /// straight into the style instead of staging a route draft.
    fn caption_tune_editor(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        layout: &CaptionTuneLayout,
        target: TuneTarget,
        style: &mut CaptionStyle,
    ) -> bool {
        let font = input.fonts.ui();
        let mut tuning = target.read(style);
        let mut changed = false;
        let drive = target.drive(style);
        let drive_label = DRIVE_LABELS[index_of(EffectDrive::ALL, drive)];

        widgets::draw_text(
            d,
            font,
            &format!("{} \u{b7} {}", target.heading(), drive_label),
            layout.heading.x,
            layout.heading.y,
            12.0,
            color::ui_muted(),
        );
        let clamp_id = widgets::widget_id(ns::CAPTION, caption_id::TUNE_CLAMP);
        let state = self.widgets.text_button(
            d,
            font,
            clamp_id,
            layout.clamp,
            "Clamp",
            tuning.clamp,
            ButtonStyle::Neutral,
            Some(metric::UI_FONT_CAPTION),
        );
        if state.clicked {
            tuning.clamp = !tuning.clamp;
            changed = true;
        }
        self.widgets.hint(
            d,
            state,
            clamp_id,
            layout.clamp,
            "Hold the drive inside the quiet/loud window; off lets the curve extrapolate past it",
        );
        let reset_id = widgets::widget_id(ns::CAPTION, caption_id::TUNE_RESET);
        let state = self.widgets.text_button(
            d,
            font,
            reset_id,
            layout.reset,
            "Reset",
            false,
            ButtonStyle::Neutral,
            Some(metric::UI_FONT_CAPTION),
        );
        if state.clicked && !tuning.is_default() {
            tuning = DriveTuning::default();
            changed = true;
        }
        self.widgets.hint(
            d,
            state,
            reset_id,
            layout.reset,
            "Back to the identity mapping: 0..1 in, 0..1 out, linear",
        );

        // The live line and meter: the drive's value this frame, and what the
        // tuning maps it to — the same numbers the overlay's resolver uses.
        //
        // Deliberately "DRIVE", not "LIVE RMS": the scene Tune editor's meter
        // shows the *raw* source, while a caption drive is already shaped
        // (RMS is square-rooted, Beat is cubed). Two meters carrying the same
        // label on different scales would quietly disagree side by side; the
        // heading above names the source, and this line names the thing it
        // actually measures.
        let live = f64::from(drive_value(drive, &input.effect_inputs));
        let arrow = mapping_editor::arrow(font.all_loaded());
        widgets::draw_text(
            d,
            font,
            &format!("DRIVE {live:.3} {arrow} {:.2}", tuning.apply(live)),
            layout.meter_caption.x,
            layout.meter_caption.y,
            12.0,
            color::ui_muted(),
        );
        mapping_editor::meter(
            d,
            layout.meter,
            mapping_editor::window_position(tuning.input_min, tuning.input_max, live),
        );

        // The two anchors, at the route editor's stride. The minimum window
        // width keeps `input_max > input_min` true — the same reason a route
        // refuses a degenerate window, enforced here at the slider instead of
        // at a failed save.
        const WINDOW_MINIMUM: f64 = 0.01;
        for (high, y) in [(false, layout.quiet_y), (true, layout.loud_y)] {
            let name = drive_anchor_label(drive, high);
            let (anchor_input, anchor_output) = if high {
                (tuning.input_max, tuning.output_max)
            } else {
                (tuning.input_min, tuning.output_min)
            };
            let caption = format!("{name}  {anchor_input:.2} {arrow} {anchor_output:.2}");
            let slot = caption_id::TUNE_SLIDERS + if high { 2 } else { 0 };
            let edit = mapping_editor::anchor_pair(
                d,
                &mut self.widgets,
                font,
                layout.x,
                y,
                layout.width,
                &mapping_editor::AnchorPair {
                    caption: &caption,
                    input_fraction: anchor_input as f32,
                    output_fraction: anchor_output as f32,
                    input_id: widgets::widget_id(ns::CAPTION, slot),
                    output_id: widgets::widget_id(ns::CAPTION, slot + 1),
                    arrow_ok: font.all_loaded(),
                },
            );
            if let Some(fraction) = edit.input {
                let fraction = f64::from(fraction);
                if high {
                    tuning.input_max = fraction.max(tuning.input_min + WINDOW_MINIMUM).min(1.0);
                } else {
                    tuning.input_min = fraction.min(tuning.input_max - WINDOW_MINIMUM).max(0.0);
                }
                changed = true;
            }
            if let Some(fraction) = edit.output {
                if high {
                    tuning.output_max = f64::from(fraction);
                } else {
                    tuning.output_min = f64::from(fraction);
                }
                changed = true;
            }
        }

        if let Some(curve) = mapping_editor::curve_stepper(
            d,
            &mut self.widgets,
            font,
            layout.x,
            layout.curve_y,
            layout.width,
            widgets::widget_id(ns::CAPTION, caption_id::TUNE_CURVE_PREVIOUS),
            widgets::widget_id(ns::CAPTION, caption_id::TUNE_CURVE_NEXT),
            tuning.curve,
        ) {
            tuning.curve = curve;
            changed = true;
        }

        if changed {
            target.write(style, tuning);
        }
        changed
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

/// Which drive's tuning the effects form's editor is open on (UX0-C14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuneTarget {
    Pulse,
    Hue,
}

impl TuneTarget {
    /// For the report line and the `--ui-probe` key.
    const fn token(self) -> &'static str {
        match self {
            Self::Pulse => "pulse",
            Self::Hue => "hue",
        }
    }

    /// The editor's heading names the drive it maps, because opening it
    /// replaces the controls that were in that space.
    const fn heading(self) -> &'static str {
        match self {
            Self::Pulse => "PULSE TUNING",
            Self::Hue => "HUE TUNING",
        }
    }

    const fn drive(self, style: &CaptionStyle) -> EffectDrive {
        match self {
            Self::Pulse => style.effects.glow_pulse,
            Self::Hue => style.effects.glow_hue_drive,
        }
    }

    const fn read(self, style: &CaptionStyle) -> DriveTuning {
        match self {
            Self::Pulse => style.effects.pulse_tuning,
            Self::Hue => style.effects.hue_tuning,
        }
    }

    fn write(self, style: &mut CaptionStyle, tuning: DriveTuning) {
        match self {
            Self::Pulse => style.effects.pulse_tuning = tuning,
            Self::Hue => style.effects.hue_tuning = tuning,
        }
    }
}

/// The two ends of a drive's axis, for the anchor captions. The route editor's
/// `anchor_label` answers this for `AnalysisSource`; the drives have their own
/// axes ([`EffectDrive`] is not that enum), so the wording lives here.
const fn drive_anchor_label(drive: EffectDrive, high: bool) -> &'static str {
    match drive {
        EffectDrive::Rms | EffectDrive::Bass => {
            if high {
                "Loud"
            } else {
                "Quiet"
            }
        }
        EffectDrive::Flux => {
            if high {
                "Busy"
            } else {
                "Calm"
            }
        }
        EffectDrive::Beat => {
            if high {
                "Beat peak"
            } else {
                "Beat tail"
            }
        }
        EffectDrive::Time => {
            if high {
                "Cycle peak"
            } else {
                "Cycle low"
            }
        }
        EffectDrive::None => {
            if high {
                "High"
            } else {
                "Low"
            }
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

/// The open tune editor's rectangles, all inside the effects form's left
/// column (UX0-C14): heading row with Clamp/Reset, live meter, two anchor
/// pairs at the route editor's 40 px stride, and the curve stepper.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CaptionTuneLayout {
    heading: UiRect,
    clamp: UiRect,
    reset: UiRect,
    meter_caption: UiRect,
    meter: UiRect,
    /// The anchor pairs' caption baselines; each pair is 40 px tall
    /// ([`mapping_editor::anchor_pair`]).
    quiet_y: f32,
    loud_y: f32,
    curve_y: f32,
    x: f32,
    width: f32,
}

/// The drive rows' labels, in [`EffectDrive::ALL`] order.
const DRIVE_LABELS: [&str; 6] = ["Off", "RMS", "Bass", "Beat", "Flux", "Time"];

/// Position of `value` in a `named_enum!`'s `ALL`, which is the C enum's order
/// and therefore the index a choice row uses.
fn index_of<T: PartialEq + Copy>(all: &[T], value: T) -> usize {
    all.iter().position(|entry| *entry == value).unwrap_or(0)
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
/// Divides a cue's words at the space nearest `fraction` through the line
/// (UX0-B05).
///
/// Character *offsets* rather than time: a split at 40 % of a cue's duration
/// almost never falls at 40 % of its text, but the words are what the user is
/// reading when they choose the seam, and a proportional guess at a word
/// boundary is the one every subtitle editor makes.
///
/// Never returns an empty half, because `lyrics::validate_text` refuses empty
/// text and a split that produced one would be refused by the model after the
/// user had already committed to it. A single word with no space in it therefore
/// splits inside itself, at a character boundary, rather than failing.
fn split_text_at_fraction(text: &str, fraction: f64) -> (String, String) {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let characters: Vec<char> = text.chars().collect();
    if characters.len() < 2 {
        // Nothing to divide. Both halves get the whole line, which is a visible,
        // fixable outcome; an empty half is a refusal after the fact.
        return (text.to_owned(), text.to_owned());
    }
    let wanted = ((characters.len() as f64) * fraction).round() as usize;
    let wanted = wanted.clamp(1, characters.len() - 1);
    // The space closest to the wanted offset, searched outward in both
    // directions at once so neither end is preferred.
    let seam = (0..characters.len())
        .filter(|index| {
            characters[*index].is_whitespace() && *index > 0 && *index + 1 < characters.len()
        })
        .min_by_key(|index| index.abs_diff(wanted))
        .map_or(wanted, |index| index);
    let left: String = characters[..seam].iter().collect();
    let right: String = characters[seam..].iter().collect();
    let (left, right) = (left.trim().to_owned(), right.trim().to_owned());
    if left.is_empty() || right.is_empty() {
        let left: String = characters[..wanted].iter().collect();
        let right: String = characters[wanted..].iter().collect();
        return (left, right);
    }
    (left, right)
}

/// Which timing controls fit beside Apply/Discard/Delete, at a given width.
///
/// A **ladder of whole configurations whose widths strictly decrease**, not a
/// greedy fill, and the difference is the lesson `core::ui::transport_bar`
/// already paid for: greedy placement made the transport's mute button vanish
/// and then reappear as the window narrowed, because dropping the volume slider
/// freed enough room for it. The property that makes a resize feel stable is
/// monotonicity in width, and a ladder has it by construction rather than by a
/// lucky ordering. `the_timing_control_ladder_never_brings_a_control_back` is
/// the assertion.
///
/// Nothing here is the only route to its action — every one of these has a key,
/// and the hint row names them — so shedding a control costs discoverability
/// rather than capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimingControl {
    Tap,
    Split,
    Merge,
    Undo,
    Redo,
}

impl TimingControl {
    const fn label(self) -> &'static str {
        match self {
            Self::Tap => "Tap",
            Self::Split => "Split",
            Self::Merge => "Merge",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Tap => "Ctrl+Enter",
            Self::Split => "Ctrl+B",
            Self::Merge => "Ctrl+J",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Shift+Z",
        }
    }

    /// Index inside [`ns::ACTIONS`]. Indices 0 and 1 are the Style toggle and
    /// Add cue, so these start at 2 — a namespace of their own is not needed
    /// (`widgets::id`'s budget is 4096 indices per namespace) and not minting
    /// one is the safer of the two options.
    const fn widget_index(self) -> u32 {
        match self {
            Self::Tap => 2,
            Self::Split => 3,
            Self::Merge => 4,
            Self::Undo => 5,
            Self::Redo => 6,
        }
    }
}

/// Seconds between the taps `--ui-probe lyric-tap=` synthesizes.
///
/// Comfortably past [`LYRIC_MIN_CUE_SECONDS`], so a probe's stamps never trip
/// the double-keypress refusal and turn the requested count into a lie. Half a
/// second is also roughly a real lyric's line rate, which keeps the resulting
/// document plausible enough to photograph.
const LYRIC_PROBE_TAP_SPACING_SECONDS: f64 = 0.5;

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

/// One line of a lane tooltip: the cue's words, its provenance, its span.
///
/// A function rather than an inline `format!` so the order is assertable — the
/// text comes first because it is what identifies the cue to the person reading,
/// and the label second because that is the question the colour code raised.
fn lane_tooltip_text(cue: &LyricCue) -> String {
    format!(
        "{}  ·  {}  ·  {} – {}",
        cue.text,
        cue.origin.label(),
        widgets::format_timestamp(cue.start_seconds),
        widgets::format_timestamp(cue.end_seconds)
    )
}

/// Multiplies a colour's channels, for [`shade_multiplier`]'s stack darkening.
fn shade(color: Color, multiplier: f32) -> Color {
    let scale = |channel: u8| (f32::from(channel) * multiplier.clamp(0.0, 1.0)) as u8;
    Color::new(scale(color.r), scale(color.g), scale(color.b), color.a)
}

/// Spacing between the hatch strokes that mark a shadowed span.
const HATCH_SPACING: f32 = 5.0;

/// Diagonal hatching inside `rect`, clipped by construction (review 1.14).
fn hatch(d: &mut RaylibDrawHandle<'_>, rect: UiRect, tint: Color) {
    hatch_spaced(d, rect, tint, HATCH_SPACING);
}

/// [`hatch`] at an explicit pitch (LX1-d).
///
/// The pitch is a parameter rather than a second function because a `Potential`
/// block wants the same marks further apart — sparse reads as "provisional",
/// dense as "something is wrong here" — and two implementations of a clipped
/// diagonal would be two places to get the clipping wrong.
///
/// No scissor: each stroke is a segment of the 45° line through the rect's bottom
/// edge, and the parameter range is solved for the rect rather than drawn long
/// and cut. A lane block is a few pixels wide on a long track, and starting a
/// scissor per block would be a state change per cue per frame.
fn hatch_spaced(d: &mut RaylibDrawHandle<'_>, rect: UiRect, tint: Color, spacing: f32) {
    if rect.width <= 0.0 || rect.height <= 0.0 || !spacing.is_finite() || spacing <= 0.0 {
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
        origin += spacing;
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
    use musializer_core::ui::contrast;

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
                    origin: Default::default(),
                })
                .expect("the fixture cues are valid");
        }
        document
    }

    // -----------------------------------------------------------------------
    // LX1-d: the colour code, the resize, the tooltip anchor, the shortcuts
    // -----------------------------------------------------------------------

    fn packed(color: Color) -> u32 {
        packed_rgba(color)
    }

    use super::super::super::theme::{self, palette, Palette, UiTheme};

    const APPEARANCES: [(&str, UiTheme, &Palette); 2] = [
        ("dark", UiTheme::BlackAmber, &palette::DARK),
        ("light", UiTheme::LightBlue, &palette::LIGHT),
    ];

    /// Every surface a provenance mark touches in one appearance: the lane and
    /// the trough beside it, and the three fills a block can take.
    fn mark_surfaces(palette: &Palette) -> [(&'static str, u32); 5] {
        [
            ("ui_raised", palette.ui_raised),
            ("ui_lane_trough", palette.ui_lane_trough),
            ("ui_cue_block", palette.ui_cue_block),
            ("ui_cue_now", palette.ui_cue_now),
            ("ui_selected", palette.ui_selected),
        ]
    }

    #[test]
    fn every_origin_colour_is_legible_against_the_lane_it_is_drawn_on() {
        // The stripe and the top line carry the code, so each has to clear the
        // bar for a non-text component against every surface it can touch, in
        // both appearances. The worst ratios in `origin_color`'s table are
        // recomputed here rather than trusted: a colour written into a doc
        // comment and never measured is exactly the amber severity label that
        // shipped at 3.96:1, and this table's first version measured the white
        // lane only and went illegible when the lane went dark.
        let expected = [
            (CueOrigin::UserApplied, 3.48f64),
            (CueOrigin::InferredCertain, 3.48),
            (CueOrigin::InferredAmbiguous, 3.48),
            (CueOrigin::Potential, 3.32),
        ];
        let mut measured_pairs = 0;
        for (origin, documented) in expected {
            let colour = packed(origin_color(origin));
            let mut worst = f64::INFINITY;
            for (appearance, _, palette) in APPEARANCES {
                for (surface, value) in mark_surfaces(palette) {
                    let ratio = contrast::ratio(colour, value);
                    assert!(
                        ratio >= contrast::AA_COMPONENT,
                        "{} is {ratio:.2}:1 on the {appearance} {surface}",
                        origin.token()
                    );
                    worst = worst.min(ratio);
                    measured_pairs += 1;
                }
                // The playhead crosses every block, and the accent means now:
                // no origin may be mistaken for it.
                assert!(
                    crate::ui::scene_colors::tests::reads_apart(colour, palette.accent),
                    "{} reads as the {appearance} accent",
                    origin.token()
                );
            }
            assert!(
                (worst - documented).abs() < 0.01,
                "{} measures {worst:.2} at worst, the table says {documented:.2}",
                origin.token()
            );
        }
        assert_eq!(measured_pairs, 4 * 10);
        // And four distinct colours, which is the whole point of the code.
        for (index, first) in ORIGIN_ORDER.iter().enumerate() {
            for second in &ORIGIN_ORDER[index + 1..] {
                assert_ne!(
                    packed(origin_color(*first)),
                    packed(origin_color(*second)),
                    "{} and {} share a colour",
                    first.token(),
                    second.token()
                );
            }
        }
        // `Potential` is the only achromatic one, which is the non-hue half of
        // its distinction; the hatch is the other half.
        let grey = origin_color(CueOrigin::Potential);
        assert_eq!(grey.r, grey.g, "the proposal swatch must read as grey");
        for origin in [
            CueOrigin::UserApplied,
            CueOrigin::InferredCertain,
            CueOrigin::InferredAmbiguous,
        ] {
            let color = origin_color(origin);
            assert!(
                color.r != color.g || color.g != color.b,
                "{} must carry a hue",
                origin.token()
            );
        }
    }

    /// The negative control for the sweep above, kept as the measurements it
    /// came from: the pre-WX-5b colours were chosen against the white lane, and
    /// two of them fail on the opaque blocks this lane draws now.
    #[test]
    fn the_white_lane_colours_would_fail_on_the_blocks() {
        let old_certain = 0x2563_C7FFu32;
        let old_ambiguous = 0xD07A_00FFu32;
        assert!(contrast::ratio(old_certain, palette::DARK.ui_cue_block) < contrast::AA_COMPONENT);
        assert!(contrast::ratio(old_certain, palette::DARK.ui_cue_now) < contrast::AA_COMPONENT);
        assert!(
            contrast::ratio(old_ambiguous, palette::LIGHT.ui_cue_block) < contrast::AA_COMPONENT
        );
        // And both still passed the one surface the old test measured.
        assert!(contrast::ratio(old_certain, 0xFFFF_FFFF) >= contrast::AA_COMPONENT);
        assert!(contrast::ratio(old_ambiguous, 0xFFFF_FFFF) >= contrast::AA_COMPONENT);
    }

    #[test]
    fn a_shaded_row_stays_the_colour_it_started_as() {
        // A fanned cluster (LX1-d) darkens each row's fill 20 % per step, and
        // the marks follow `origin_mark`'s rule. What has to hold at every depth
        // in both appearances: each row is darker than the one above it, so the
        // rows read as rows, and the marks still stand off the row's own fill.
        //
        // The bar is 3:1 for rows 0 and 1 everywhere and for every row on the
        // dark lane, where a darker fill only widens the gap. On the light lane
        // a deeper row darkens from near-white toward the marks themselves, and
        // the measured floor is 2.0:1 at the fifth row of a five-deep stack —
        // rows a 50 px lane makes 8 px tall, whose cues the tooltip names.
        for (appearance, theme_choice, _) in APPEARANCES {
            theme::set_current(theme_choice);
            for now in [false, true] {
                let mut above: Option<f64> = None;
                for steps in 0..=musializer_core::ui::lyric_lane_stack::LYRIC_STACK_MAX_SHADE_STEPS
                {
                    let fill = cue_block_fill(now, false, steps);
                    let luminance = contrast::relative_luminance(packed(fill));
                    if let Some(previous) = above {
                        assert!(
                            luminance < previous,
                            "{appearance} row {steps} is not darker"
                        );
                    }
                    above = Some(luminance);
                    for origin in ORIGIN_ORDER {
                        let mark = origin_mark(origin, fill, steps);
                        let ratio = contrast::ratio(packed(mark), packed(fill));
                        let bar = if steps <= 1 || appearance == "dark" {
                            contrast::AA_COMPONENT
                        } else {
                            2.0
                        };
                        assert!(
                            ratio >= bar,
                            "{appearance} {} row {steps}: {ratio:.2}:1",
                            origin.token()
                        );
                        // Alpha is a selection state, never a stack depth.
                        assert_eq!(mark.a, 255);
                    }
                }
            }
            for origin in ORIGIN_ORDER {
                let base = origin_color(origin);
                assert_eq!(shade(base, 1.0), base, "step 0 must not move the colour");
                // Row 0's marks are the legend's colour, in both appearances.
                assert_eq!(
                    origin_mark(origin, cue_block_fill(false, false, 0), 0),
                    base
                );
            }
        }
    }

    #[test]
    fn a_blocks_words_read_on_whatever_fill_it_has() {
        // Muted words on a resting block and ink on the line on screen are the
        // look (the mockup's `.ly` and `.ly.now`); body-text contrast is the
        // floor at every state and stack depth, where the fallback takes over.
        for (appearance, theme_choice, palette) in APPEARANCES {
            theme::set_current(theme_choice);
            assert_eq!(
                packed(cue_word_ink(false, cue_block_fill(false, false, 0))),
                palette.ui_muted,
                "{appearance}: a resting block's words are muted"
            );
            assert_eq!(
                packed(cue_word_ink(true, cue_block_fill(true, false, 0))),
                palette.ui_ink,
                "{appearance}: the line on screen is in ink"
            );
            for now in [false, true] {
                for selected in [false, true] {
                    for steps in
                        0..=musializer_core::ui::lyric_lane_stack::LYRIC_STACK_MAX_SHADE_STEPS
                    {
                        let fill = cue_block_fill(now, selected, steps);
                        let ink = cue_word_ink(now, fill);
                        let ratio = contrast::ratio(packed(ink), packed(fill));
                        assert!(
                            ratio >= contrast::AA_TEXT,
                            "{appearance} now={now} selected={selected} row {steps}: {ratio:.2}:1"
                        );
                    }
                }
            }
            // The line on screen is a different fill from a resting block and
            // from a selected one, or *now* would be a word colour and nothing
            // else.
            let resting = cue_block_fill(false, false, 0);
            assert_ne!(cue_block_fill(true, false, 0), resting, "{appearance}");
            assert_ne!(
                cue_block_fill(true, false, 0),
                cue_block_fill(false, true, 0),
                "{appearance}"
            );
        }
    }

    #[test]
    fn a_lane_tooltip_never_covers_the_lane_it_explains() {
        // The operator's hard requirement, and the reason the anchor is the lane
        // rather than the block: a tip drawn over the lane hides the very blocks
        // the user is scanning. Asserted as a rectangle intersection over every
        // lane height, every block position and both window sizes the gate
        // captures, because "it looked fine in the screenshot" is what this kind
        // of geometry survives.
        for window in [(960.0f32, 640.0f32), (1280.0, 720.0), (1920.0, 1080.0)] {
            for lane_height in [LANE_HEIGHT, 44.0, LANE_HEIGHT_MAX] {
                let lane = UiRect::new(20.0, window.1 - 300.0, window.0 - 40.0, lane_height);
                let mut block_x = lane.x;
                while block_x < lane.x + lane.width {
                    let anchor = UiRect::new(block_x, lane.y, 40.0, lane.height);
                    for text_width in [40.0f32, 220.0, 900.0] {
                        let tip = widgets::tooltip_box(anchor, text_width, window);
                        assert!(
                            tip.intersect(lane).is_empty(),
                            "a {text_width}px tip at x={block_x} over a {lane_height}px lane \
                             landed on {tip:?} and overlaps the lane {lane:?}"
                        );
                        // And it is on screen, or it is not a tooltip.
                        assert!(tip.y >= 0.0 && tip.y + tip.height <= window.1);
                        assert!(tip.x >= 0.0 && tip.x + tip.width <= window.0 + 0.01);
                    }
                    block_x += 37.0;
                }
            }
        }
    }

    #[test]
    fn the_tooltip_says_what_the_block_could_not() {
        let cue = LyricCue {
            id: 3,
            start_seconds: 61.5,
            end_seconds: 64.25,
            text: "Καλημέρα κόσμε".to_string(),
            origin: CueOrigin::InferredAmbiguous,
        };
        let text = lane_tooltip_text(&cue);
        assert!(text.starts_with("Καλημέρα κόσμε"), "{text}");
        assert!(
            text.contains(CueOrigin::InferredAmbiguous.label()),
            "{text}"
        );
        assert!(text.contains(&widgets::format_timestamp(61.5)), "{text}");
        assert!(text.contains(&widgets::format_timestamp(64.25)), "{text}");
    }

    #[test]
    fn the_clipboard_shortcuts_stand_down_where_they_would_be_wrong() {
        // Ctrl+V in the cue field means paste *text* — the form's own hint row
        // says so — and the field reads raylib's keyboard directly, so the lane
        // has to decline rather than expect to win the race.
        assert!(lyric_clipboard_keys_allowed(false, false, false));
        assert!(!lyric_clipboard_keys_allowed(false, false, true));
        // And there is no lane on screen in the caption or font panes, so a
        // shortcut there would act on a selection the user cannot see.
        assert!(!lyric_clipboard_keys_allowed(true, false, false));
        assert!(!lyric_clipboard_keys_allowed(false, true, false));
        assert!(!lyric_clipboard_keys_allowed(true, true, true));
    }

    #[test]
    fn the_report_line_carries_everything_a_capture_is_graded_on() {
        let mut editor = LyricEditor::new();
        let mut track = test_track("provenance", 0);
        for (start, origin) in [
            (1.0, CueOrigin::UserApplied),
            (1.5, CueOrigin::InferredCertain),
            (1.8, CueOrigin::InferredAmbiguous),
            (2.0, CueOrigin::Potential),
        ] {
            track
                .lyrics
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: start + 3.0,
                    text: format!("{origin:?}"),
                    origin,
                })
                .expect("a valid fixture cue");
        }
        editor.enter_track(Some(0));
        // The draw writes these; the report only prints them, so the test sets
        // them the way a frame would rather than pretending `describe` derives
        // them — that is the point of recording them during the draw.
        let stack = LyricStack::build(&track.lyrics);
        editor.stack_rows = stack.rows();
        editor.origin_counts = ORIGIN_ORDER.map(|origin| {
            track
                .lyrics
                .cues()
                .iter()
                .filter(|cue| cue.origin == origin)
                .count()
        });
        editor.lane_drawn = 44.0;

        let line = editor.describe(Some(&track));
        assert!(
            line.contains("origins user=1 certain=1 ambiguous=1 potential=1"),
            "{line}"
        );
        assert!(line.contains("stack rows 4"), "{line}");
        assert!(line.contains("lane height 44"), "{line}");
        assert!(line.contains("clipboard 0"), "{line}");
        assert!(line.contains("tip off"), "{line}");

        // A capture that photographs an overlapping document and still says
        // `stack rows 1` is a capture of the feature not running, so the flat
        // case has to say something different.
        let flat = test_track("flat", 3);
        editor.stack_rows = LyricStack::build(&flat.lyrics).rows();
        editor.origin_counts = [3, 0, 0, 0];
        let line = editor.describe(Some(&flat));
        assert!(line.contains("stack rows 1"), "{line}");
        assert!(
            line.contains("origins user=3 certain=0 ambiguous=0 potential=0"),
            "{line}"
        );
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
                origin: Default::default(),
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
            // +3 is the typed-time readout (UX0-B04). It is a *button* until it
            // is clicked, so it claims a press like any other control and needs
            // an id of its own.
            form_id::START_TIME + 3,
            form_id::END_TIME,
            form_id::END_TIME + 1,
            form_id::END_TIME + 2,
            form_id::END_TIME + 3,
        ];
        let control_count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), control_count);
    }

    /// The other half of the same check, one namespace over.
    ///
    /// `ns::ACTIONS` held two controls for its whole life and now holds nine.
    /// Enumerated individually rather than swept, because a collision test over
    /// a hand-maintained *subset* is precisely what let `EXPORT` and `SEEK` eat
    /// the export panel's SIZE row while three collision tests stayed green.
    #[test]
    fn every_lyrics_action_control_has_a_distinct_widget_id() {
        let mut ids = vec![
            0, // the Style pane toggle
            1, // Add cue
            TimingControl::Tap.widget_index(),
            TimingControl::Split.widget_index(),
            TimingControl::Merge.widget_index(),
            TimingControl::Undo.widget_index(),
            TimingControl::Redo.widget_index(),
            7, // Export (D3)
            8, // Import (D3)
        ];
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "two ACTIONS controls share an index");
    }

    #[test]
    fn a_split_never_produces_a_half_the_model_would_refuse() {
        // `lyrics::validate_text` refuses empty text, so an empty half is a
        // refusal *after* the user committed to the edit.
        for text in [
            "one two three four",
            "a b",
            "no-spaces-at-all-in-this-line",
            "x",
            "",
            "  leading and trailing  ",
            "Ελληνικά και ρωσικά",
        ] {
            for step in 0..=10 {
                let (left, right) = split_text_at_fraction(text, f64::from(step) / 10.0);
                if text.chars().count() >= 2 {
                    assert!(!left.is_empty(), "{text:?} at {step} gave an empty left");
                    assert!(!right.is_empty(), "{text:?} at {step} gave an empty right");
                }
            }
        }
        // A non-finite fraction is a bug upstream, not a panic here.
        let (left, right) = split_text_at_fraction("one two", f64::NAN);
        assert_eq!((left.as_str(), right.as_str()), ("one", "two"));
    }

    #[test]
    fn a_split_prefers_the_word_boundary_nearest_the_seam() {
        assert_eq!(
            split_text_at_fraction("hold me closer tiny dancer", 0.5),
            ("hold me closer".to_owned(), "tiny dancer".to_owned())
        );
        assert_eq!(
            split_text_at_fraction("hold me closer tiny dancer", 0.05),
            ("hold".to_owned(), "me closer tiny dancer".to_owned())
        );
        assert_eq!(
            split_text_at_fraction("hold me closer tiny dancer", 0.95),
            ("hold me closer tiny".to_owned(), "dancer".to_owned())
        );
    }

    #[test]
    fn a_typed_time_round_trips_through_the_forms_own_readout() {
        // The parser is in `core` and the formatter is in this crate, so nothing
        // makes them agree except this. It is the assertion that stops the split
        // drifting — a formatter change that this test does not fail is a
        // formatter change that quietly breaks typing.
        for seconds in [
            0.0, 0.001, 0.999, 1.0, 59.999, 60.0, 83.456, 599.999, 3600.0,
        ] {
            let printed = widgets::format_timestamp(seconds);
            let parsed = lyric_lane_edit::parse_cue_timestamp(&printed).unwrap_or_else(|| {
                panic!("{printed:?} came out of the formatter and will not parse")
            });
            assert!(
                (parsed - seconds).abs() < 1e-6,
                "{seconds} printed as {printed} and read back as {parsed}"
            );
        }
    }

    #[test]
    fn one_user_action_earns_exactly_one_undo_step() {
        // A cut of five cues is five `Delete`s in one frame. Five presses of
        // Ctrl+Z to reverse one press of Ctrl+X is not undo.
        let cut: Vec<_> = (1..=5).map(|id| LyricsEdit::Delete { id }).collect();
        assert_eq!(batch_history_label(&cut).as_deref(), Some("5 edits"));
        assert_eq!(
            batch_history_label(&[LyricsEdit::ShiftMany {
                ids: vec![1, 2, 3],
                delta_seconds: 0.5
            }])
            .as_deref(),
            Some("Move 3 cues")
        );
        // A tap is exactly two retimes — the line it closes and the line it
        // opens — and a run of them must not read as a wall of "2 edits".
        let tap = [
            LyricsEdit::Retime {
                id: 1,
                start_seconds: 0.0,
                end_seconds: 1.0,
            },
            LyricsEdit::Retime {
                id: 2,
                start_seconds: 1.0,
                end_seconds: 2.0,
            },
        ];
        assert_eq!(batch_history_label(&tap).as_deref(), Some("Stamp cue"));
        // A caption-style batch must not consume a stack slot: it is the
        // highest-frequency edit in the panel — every frame of a slider drag is
        // one — and it does not touch a cue.
        assert_eq!(
            batch_history_label(&[LyricsEdit::SetCaptionStyle(Box::default())]),
            None
        );
        // Mixed: the cue edit still earns the step, and names it.
        let mixed = [
            LyricsEdit::SetCaptionStyle(Box::default()),
            LyricsEdit::Delete { id: 3 },
        ];
        assert_eq!(batch_history_label(&mixed).as_deref(), Some("Delete cue"));
    }

    #[test]
    fn hold_repeat_fires_once_on_press_and_then_at_a_steady_rate() {
        let mut repeat = HoldRepeat::default();
        // The first frame belongs to `clicked`; firing here as well would move
        // the value two steps for one tap.
        assert!(!repeat.tick(7, true));
        let mut fires = 0;
        for _ in 0..HoldRepeat::DELAY_FRAMES {
            if repeat.tick(7, true) {
                fires += 1;
            }
        }
        assert_eq!(fires, 0, "it repeated before the delay elapsed");
        for _ in 0..HoldRepeat::INTERVAL_FRAMES * 4 {
            if repeat.tick(7, true) {
                fires += 1;
            }
        }
        assert_eq!(fires, 4);
        // Releasing resets, and moving to another button does not inherit the
        // held frames.
        assert!(!repeat.tick(7, false));
        assert!(!repeat.tick(9, true));
        assert_eq!(repeat, HoldRepeat { id: 9, frames: 0 });
    }

    #[test]
    fn a_time_field_stands_the_global_shortcuts_down_as_firmly_as_the_cue_field() {
        // The shell asks exactly one question — `is_typing` — before deciding
        // whether Space plays and `M` mutes. A second text surface in this panel
        // that did not answer it would re-create UX0-A06 one field over: typing
        // `1:23` into a time readout would seek the track and toggle the readout.
        let mut editor = LyricEditor::new();
        assert!(!editor.is_typing());
        editor.time_edit = Some(TimeEdit {
            is_start: true,
            field: TextField::new(TextRules::ascii_query(16)),
        });
        assert!(editor.is_typing(), "a typed time must silence the globals");
        // And the panel-local lane shortcuts stand down too, or Ctrl+V would
        // paste cues while the user meant to paste a timecode.
        assert!(!lyric_clipboard_keys_allowed(false, false, true));
    }

    #[test]
    fn arming_a_tap_run_blurs_the_cue_field_so_no_key_means_two_things() {
        // The defect review 2 names: `begin_new` focuses the field, a focused
        // field stands down every transport key including Space, and the natural
        // add-type-play-tap loop broke at the tap. Tap mode and text entry are
        // mutually exclusive *states*, which is what makes it unstateable rather
        // than guarded against.
        let mut document = LyricsDocument::new(60.0).unwrap();
        for start in [1.0, 3.0, 5.0] {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: start + 1.0,
                    text: "a line".into(),
                    origin: CueOrigin::UserApplied,
                })
                .unwrap();
        }
        let mut editor = LyricEditor::new();
        editor.begin_new(&document, 0.0);
        assert!(editor.is_typing(), "a new cue focuses the field");
        assert!(editor.toggle_tap(&document, 0.0));
        assert!(
            !editor.is_typing(),
            "arming a run must take the keyboard back from the field"
        );
        assert!(editor.tap_is_armed());
    }

    #[test]
    fn selecting_a_cue_asks_the_transport_to_go_there() {
        let mut document = LyricsDocument::new(60.0).unwrap();
        for start in [1.0, 3.0, 5.0] {
            document
                .insert(LyricCue {
                    id: 0,
                    start_seconds: start,
                    end_seconds: start + 1.0,
                    text: "a line".into(),
                    origin: CueOrigin::UserApplied,
                })
                .unwrap();
        }
        let mut editor = LyricEditor::new();
        assert_eq!(editor.take_seek_request(), None);
        editor.select_and_seek(&document, 2);
        assert_eq!(editor.take_seek_request(), Some(3.0));
        // Drained, not read: a seek left behind would be emitted again next
        // frame and pin the playhead to the selected cue forever.
        assert_eq!(editor.take_seek_request(), None);

        // Down and Up walk the list and seek with it, saturating rather than
        // wrapping — a list that jumps from the last line to the first reads as
        // a lost keypress.
        editor.step_selection(&document, true, 0.0);
        assert_eq!(editor.take_seek_request(), Some(5.0));
        editor.step_selection(&document, true, 0.0);
        assert_eq!(editor.take_seek_request(), Some(5.0));
        editor.step_selection(&document, false, 0.0);
        assert_eq!(editor.take_seek_request(), Some(3.0));

        // With nothing bound it starts from the cue under the playhead rather
        // than from the top of the document.
        let mut fresh = LyricEditor::new();
        fresh.step_selection(&document, true, 3.5);
        assert_eq!(fresh.take_seek_request(), Some(5.0));
    }

    #[test]
    fn a_closed_editor_cannot_strand_a_text_field_holding_the_keyboard() {
        // UX0-A02's shape, one field over. Neither text surface is drawn with
        // the editor closed, so neither may keep the keyboard — a time readout
        // left mid-edit would make `lyric_lane_keys` stand down for the rest of
        // the session, and the closed lane's keys would stop working with
        // nothing on screen to explain it.
        //
        // Asserted through the predicate the guard actually reads rather than
        // through a draw, because the draw needs a GPU and the guard does not.
        let mut editor = LyricEditor::new();
        editor.time_edit = Some(TimeEdit {
            is_start: false,
            field: TextField::new(TextRules::ascii_query(16)),
        });
        editor.text.set_focused(true);
        assert!(editor.is_typing());

        // What `closed_lyric_lane` does before it reaches the gesture code.
        editor.text.set_focused(false);
        editor.time_edit = None;
        assert!(!editor.is_typing());
        assert!(lyric_clipboard_keys_allowed(
            false,
            false,
            editor.is_typing()
        ));
    }

    #[test]
    fn entering_another_track_drops_the_history_that_would_restore_its_cues() {
        // Cue ids restart at 1 in every document, so one track's snapshot
        // restores another track's cues wholesale — a far worse version of the
        // defect `owner_slot` exists for, because it replaces the document
        // rather than one cue in it.
        let mut document = LyricsDocument::new(60.0).unwrap();
        document
            .insert(LyricCue {
                id: 0,
                start_seconds: 1.0,
                end_seconds: 2.0,
                text: "a line".into(),
                origin: CueOrigin::UserApplied,
            })
            .unwrap();
        let mut editor = LyricEditor::new();
        editor.enter_track(Some(0));
        editor.record_history_label("Move", &document);
        assert!(editor.can_undo());
        assert!(editor.toggle_tap(&document, 0.0));

        editor.enter_track(Some(1));
        assert!(!editor.can_undo(), "a history survived a track switch");
        assert!(!editor.tap_is_armed(), "a tap run survived a track switch");
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
                origin: Default::default(),
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
                    origin: Default::default(),
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
                origin: Default::default(),
            })
            .expect("valid");
        document
            .insert(LyricCue {
                id: 0,
                start_seconds: 42.1,
                end_seconds: 46.0,
                text: "the one that wins".to_string(),
                origin: Default::default(),
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
                    origin: Default::default(),
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

    /// UX0-C14: the tune probe opens the effects form's editor, the report line
    /// says so, and the editor and the picker cannot both hold the left column.
    #[test]
    fn the_tune_editor_is_reachable_and_excludes_the_picker() {
        let mut editor = LyricEditor::new();
        let mut track = test_track("/tmp/tune.wav", 0);
        let probe = UiProbe {
            caption_tune: Some(CaptionTuneProbe::Pulse),
            ..UiProbe::default()
        };
        assert!(editor.apply_probe(&probe, &mut track));
        assert!(editor.style_pane && editor.effects_pane);
        assert_eq!(editor.tune_target, Some(TuneTarget::Pulse));
        assert_eq!(editor.picker.target, None);
        let line = editor.describe(Some(&track));
        assert!(line.contains("tune=pulse"), "{line:?}");
        assert!(line.contains("picker=none"), "{line:?}");

        // Both at once is a dishonoured probe: the two claim the same column,
        // and a capture that photographed one while asserting the other would
        // pass on the wrong frame.
        let both = UiProbe {
            caption_tune: Some(CaptionTuneProbe::Hue),
            caption_picker: Some(CaptionPickerProbe::Glow),
            ..UiProbe::default()
        };
        let mut fresh = LyricEditor::new();
        assert!(!fresh.apply_probe(&both, &mut track));
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
