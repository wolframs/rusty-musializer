//! The top bar (WX-5c, `docs/WX5C_DESIGN.md` §4): the mark, the project
//! title and its status, the mode tabs, and Still / Contact sheet / Render.
//!
//! **Owner: the Top bar agent.** This file is the seam the 5c split was cut
//! along (§8): the rail's agent owns the frame and `ui/shell.rs`, this file
//! owns what is drawn inside [`WorkspaceFrame::top_bar`]. Everything here
//! reads the shell through `pub(crate)` state and methods; what it needs that
//! does not exist yet is asked of the rail's agent rather than added there.
//!
//! # What it is for
//!
//! The first thing the user sees, and the one row that answers "what am I
//! working on, is it safe, and how do I get it out". So it is quiet: one
//! surface, one rule under it, the title as the anchor, everything else in
//! the muted tone — and **Render…** as the one invitation, the only accent
//! fill on the workspace (WX-5a's rule that the accent means *now* and the
//! primary action).
//!
//! # The pieces, left to right
//!
//! - the mark, the mockup's conic swatch, drawn as a Gouraud fan
//!   ([`draw_mark`]);
//! - the title (the project's own, else the file's stem) and, when the
//!   project names one, the artist (`metadata.artist`, schema v2). Both are
//!   the user's words: drawn in the interface face as the mockup sets them
//!   when the chrome bank covers every character, else in the authored face,
//!   which covers the user's alphabet.
//!   An empty artist gets a quiet Add artist invitation. Clicking opens an
//!   inline draft; Enter commits, Escape or clicking away cancels. The artist
//!   is never guessed from a file name;
//! - the info line, `5:22 · 1080p 16:9 · 30 fps`, from the track's duration
//!   and its render config;
//! - the save state as a dot and a word, all five [`SaveState`]s, coloured
//!   by [`save_state_tint`] so the top bar and the tracks header cannot
//!   disagree about which states ask for attention;
//! - the mode tabs, **views onto `UiPanel` and the inspector, never a copy**
//!   ([`Shell::mode_tab_selected`]); selected is the outline and tint every
//!   selection wears, never the accent;
//! - Still (`ExportStill`, the same command as the Export panel's Save still,
//!   kept in both places on purpose), Contact sheet (`ExportContactSheet`)
//!   and Render… (`StartRender`, [`ButtonStyle::Primary`]).
//!
//! # Narrow windows
//!
//! [`TopBarLayout::plan`] picks one whole composition from a ladder whose
//! widths strictly decrease: the info line goes first (the duration is on the
//! transport and the format in the Export panel), then the artist slot
//! shortens to 72 px (remaining editable), then
//! "Contact sheet" becomes "Sheet", then the save word leaves its dot (the
//! tooltip keeps it), and at each rung the tabs try the bar's centre before
//! packing beside the title. The property its test sweeps is the transport
//! row's: **monotonic in width**, so nothing shed at one width comes back at a
//! narrower one. Only at the last rung does the title ellipsize.
//!
//! [`WorkspaceFrame::top_bar`]: super::shell_layout::WorkspaceFrame::top_bar
//! [`SaveState`]: crate::workspace::SaveState

use musializer_core::timing::render_export::{Aspect, RenderExportConfig, Resolution};
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::draw;
use musializer_runtime::process::ffmpeg::ffmpeg_available;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::shell::{save_state_tint, ModeTab, Shell, ShellCommand, ShellInput};
use super::theme::{color, metric};
use super::widgets::{self, ButtonState, ButtonStyle};
use crate::workspace::{SaveState, Track};

/// Inset from the bar's right end.
const BAR_PADDING: f32 = 12.0;
/// The mark's edge, as in the mockup (`.mark{width:22px}`).
const MARK_SIZE: f32 = 22.0;
/// Inset from the bar's left end: the mark centred over the rail below it, so
/// the mark reads as the rail's head and the left edge as one column rather
/// than two pieces 3 px apart (critic pass, WX-5c).
const MARK_INSET: f32 = (musializer_core::ui::workspace_layout::RAIL_WIDTH - MARK_SIZE) * 0.5;
/// Between the mark and the title.
const MARK_GAP: f32 = 10.0;
/// Between the title and the artist, which read as one name.
const NAME_GAP: f32 = 8.0;
/// Before the info line and before the save state: a beat, not a column.
const META_GAP: f32 = 12.0;
/// The save dot's diameter and the space after it.
const DOT_SIZE: f32 = 7.0;
const DOT_GAP: f32 = 6.0;
/// Every control on the bar: tabs and actions alike.
const CONTROL_HEIGHT: f32 = 30.0;
/// A tab's label inset, and the tab group's own inset around its tabs.
const TAB_PADDING: f32 = 12.0;
const GROUP_INSET: f32 = 3.0;
/// An action's label inset, and the space between actions.
const ACTION_PADDING: f32 = 14.0;
const ACTION_GAP: f32 = 8.0;
/// The least space between the three clusters.
const CLUSTER_GAP: f32 = 16.0;
/// Below this a title is not a title; at the last rung the bar keeps at least
/// this much of it and lets the tabs pack tighter instead.
const MIN_TITLE: f32 = 48.0;

/// Widget slots in `widgets::id::TOP_BAR`. Tabs by what they are (their
/// position in `ModeTab::ALL`), actions after them, so a rung change never
/// hands one control's claimed press to another.
const TAB_SLOT: u32 = 0;
const STILL_SLOT: u32 = 10;
const SHEET_SLOT: u32 = 11;
const RENDER_SLOT: u32 = 12;
const ARTIST_SLOT: u32 = 22;

/// The info line as a press (WX-5d): it opens the Render view.
const FORMAT_SLOT: u32 = 13;

/// What the top bar drew in its last frame, kept for [`Shell::describe_top_bar`].
///
/// Lives on [`Shell`] as `top_bar_state`, and is this file's to grow: a field
/// added here needs no edit to `ui/shell.rs`. Reset to the default on the
/// welcome screen, which has no top bar.
#[derive(Clone, Debug, Default)]
pub(crate) struct TopBarState {
    /// The rect the bar was drawn into, `None` when it had no room.
    pub drawn: Option<UiRect>,
    /// The words it drew, and where.
    pub title: String,
    pub artist: Option<String>,
    pub artist_editing: bool,
    pub artist_draft: Option<ArtistDraft>,
    pub artist_event: String,
    pub artist_probe: Option<(Option<String>, ArtistAction)>,
    pub info: String,
    pub info_shown: bool,
    pub save: Option<SaveState>,
    pub save_reason: Option<String>,
    pub layout: Option<TopBarLayout>,
    /// Whether Render… was live (it is disabled, with its reason, without
    /// FFmpeg on PATH).
    pub render_live: bool,
    /// Whether the Render view was open (WX-5d): Render… wore the selection
    /// look instead of the accent.
    pub render_view: bool,
    /// The UI scale the rects are in, so a script can turn them into pixels.
    pub scale: f32,
}

/// A draft is bound to the audio identity; switching tracks discards it.
#[derive(Clone, Debug)]
pub(crate) struct ArtistDraft {
    track: std::path::PathBuf,
    field: super::text_input::TextField,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtistAction {
    Draft,
    Commit,
    Cancel,
    Clear,
}

/// One metadata edit. Empty text clears the extension, so schema_version()
/// returns v1 again; no persisted schema flag or format change is involved.
pub(crate) fn commit_artist(track: &mut Track, text: &str, now: f64) -> bool {
    let artist = text.trim();
    if artist.len() > musializer_core::project::model::capacity::NAME
        || artist.chars().any(char::is_control)
    {
        return false;
    }
    let previous = track
        .project_metadata
        .as_ref()
        .map_or("", |m| m.artist.as_str());
    if previous == artist {
        return false;
    }
    let title = project_title(track);
    let project_id = format!(
        "track-{}",
        &track.audio_sha256[..track.audio_sha256.len().min(16)]
    );
    let metadata =
        track
            .project_metadata
            .get_or_insert_with(|| musializer_core::project::model::Metadata {
                title,
                project_id,
                ..Default::default()
            });
    metadata.artist = artist.to_owned();
    track.mark_dirty(now);
    true
}

/// The natural widths of everything the bar might draw, measured by the
/// caller in the faces that will draw them. A separate value so the layout
/// is arithmetic a test can sweep without a window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TopBarMeasures {
    pub title: f32,
    /// The artist text, or the quiet Add artist invitation, or the draft.
    pub artist: f32,
    pub info: f32,
    pub save_word: f32,
    /// One per tab drawn: five on the band layout, four with the column
    /// (WX-5d, Export left for Render…).
    pub tabs: Vec<f32>,
    /// Still, Contact sheet, Render….
    pub actions: [f32; 3],
    /// "Sheet", the narrow label of the second action.
    pub sheet_short: f32,
}

/// One rung of the ladder: what is shed, cumulatively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Shed {
    Nothing,
    Info,
    Artist,
    SheetWord,
    SaveWord,
    /// Tighter insets on every control and between the clusters: the bar
    /// beside an open inspector at the 960 minimum, ~650 logical wide.
    Compact,
}

impl Shed {
    const LADDER: [Shed; 6] = [
        Shed::Nothing,
        Shed::Info,
        Shed::Artist,
        Shed::SheetWord,
        Shed::SaveWord,
        Shed::Compact,
    ];

    /// A tab's label inset, an action's, and the gap between clusters.
    fn insets(self) -> (f32, f32, f32) {
        if self >= Shed::Compact {
            (7.0, 9.0, 12.0)
        } else {
            (TAB_PADDING, ACTION_PADDING, CLUSTER_GAP)
        }
    }

    fn name(self) -> &'static str {
        match self {
            Shed::Nothing => "full",
            Shed::Info => "no-info",
            Shed::Artist => "no-artist",
            Shed::SheetWord => "short-sheet",
            Shed::SaveWord => "dot-only",
            Shed::Compact => "compact",
        }
    }
}

/// Where everything goes, in the bar's own (logical) coordinates.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TopBarLayout {
    pub shed: Shed,
    /// Tabs at the bar's centre, or packed after the title.
    pub centred: bool,
    /// The index on the ladder (rung * 2 + packed), which the monotonicity
    /// test reads.
    pub rung: usize,
    pub mark: UiRect,
    /// The title's box; narrower than its text only at the last rung.
    pub title: UiRect,
    pub artist: Option<UiRect>,
    pub info: Option<UiRect>,
    pub dot: UiRect,
    pub save_word: Option<UiRect>,
    pub group: UiRect,
    pub tabs: Vec<UiRect>,
    pub actions: [UiRect; 3],
    pub sheet_short: bool,
}

impl TopBarLayout {
    /// The richest composition that fits `bar` (see the module comment).
    #[must_use]
    pub(crate) fn plan(bar: UiRect, m: &TopBarMeasures) -> Self {
        let mut rung = 0;
        for shed in Shed::LADDER {
            for centred in [true, false] {
                if let Some(layout) = Self::place(bar, m, shed, centred, rung, false) {
                    return layout;
                }
                rung += 1;
            }
        }
        // The last rung, with the title giving way. Always succeeds: what
        // does not fit is drawn clipped rather than not at all, because a
        // window this narrow is below the minimum `effective_scale` allows.
        Self::place(bar, m, Shed::Compact, false, rung, true)
            .expect("the squeezed rung always places")
    }

    fn place(
        bar: UiRect,
        m: &TopBarMeasures,
        shed: Shed,
        centred: bool,
        rung: usize,
        squeeze: bool,
    ) -> Option<Self> {
        let show_info = shed < Shed::Info && m.info > 0.0;
        // Editing must remain reachable even at the minimum window. The
        // artist rung now shortens the slot rather than removing it.
        let show_artist = m.artist > 0.0;
        let artist_width = if shed >= Shed::Artist {
            m.artist.min(72.0)
        } else {
            m.artist.min(160.0)
        };
        let sheet_short = shed >= Shed::SheetWord;
        let show_word = shed < Shed::SaveWord;
        let (tab_padding, action_padding, cluster_gap) = shed.insets();

        let tabs_width = GROUP_INSET * 2.0
            + m.tabs
                .iter()
                .map(|label| label + tab_padding * 2.0)
                .sum::<f32>();
        let action_labels = [
            m.actions[0],
            if sheet_short {
                m.sheet_short
            } else {
                m.actions[1]
            },
            m.actions[2],
        ];
        let action_widths = action_labels.map(|label| label + action_padding * 2.0);
        let actions_width = action_widths.iter().sum::<f32>() + ACTION_GAP * 2.0;

        // Everything left of the tabs but the title.
        let left_fixed = MARK_INSET
            + MARK_SIZE
            + MARK_GAP
            + if show_artist {
                NAME_GAP + artist_width
            } else {
                0.0
            }
            + if show_info { META_GAP + m.info } else { 0.0 }
            + META_GAP
            + DOT_SIZE
            + if show_word {
                DOT_GAP + m.save_word
            } else {
                0.0
            };
        let right_edge = bar.x + bar.width - BAR_PADDING;
        let actions_x = right_edge - actions_width;

        let title_width = if squeeze {
            let room = actions_x - cluster_gap - tabs_width - cluster_gap - (bar.x + left_fixed);
            m.title.min(room.max(MIN_TITLE.min(m.title)))
        } else {
            m.title
        };
        let left_end = bar.x + left_fixed + title_width;

        let group_x = if centred {
            let x = bar.x + (bar.width - tabs_width) * 0.5;
            if x < left_end + cluster_gap || x + tabs_width + cluster_gap > actions_x {
                return None;
            }
            x
        } else {
            let x = left_end + cluster_gap;
            if !squeeze && x + tabs_width + cluster_gap > actions_x {
                return None;
            }
            x
        };

        let control_y = bar.y + (bar.height - CONTROL_HEIGHT) * 0.5;
        let line = |x: f32, width: f32| UiRect::new(x, bar.y, width, bar.height);
        let mut x = bar.x + MARK_INSET;
        let mark = UiRect::new(
            x,
            bar.y + (bar.height - MARK_SIZE) * 0.5,
            MARK_SIZE,
            MARK_SIZE,
        );
        x += MARK_SIZE + MARK_GAP;
        let title = line(x, title_width);
        x += title_width;
        let artist = show_artist.then(|| {
            let rect = line(x + NAME_GAP, artist_width);
            x += NAME_GAP + artist_width;
            rect
        });
        let info = show_info.then(|| {
            let rect = line(x + META_GAP, m.info);
            x += META_GAP + m.info;
            rect
        });
        x += META_GAP;
        let dot = UiRect::new(x, bar.y + (bar.height - DOT_SIZE) * 0.5, DOT_SIZE, DOT_SIZE);
        x += DOT_SIZE;
        let save_word = show_word.then(|| line(x + DOT_GAP, m.save_word));

        let group = UiRect::new(group_x, control_y, tabs_width, CONTROL_HEIGHT);
        let mut tabs = vec![UiRect::default(); m.tabs.len()];
        let mut tab_x = group_x + GROUP_INSET;
        for (rect, label) in tabs.iter_mut().zip(&m.tabs) {
            let width = label + tab_padding * 2.0;
            *rect = UiRect::new(
                tab_x,
                control_y + GROUP_INSET,
                width,
                CONTROL_HEIGHT - GROUP_INSET * 2.0,
            );
            tab_x += width;
        }

        let mut actions = [UiRect::default(); 3];
        let mut action_x = actions_x;
        for (rect, width) in actions.iter_mut().zip(action_widths) {
            *rect = UiRect::new(action_x, control_y, width, CONTROL_HEIGHT);
            action_x += width + ACTION_GAP;
        }

        Some(Self {
            shed,
            centred,
            rung,
            mark,
            title,
            artist,
            info,
            dot,
            save_word,
            group,
            tabs,
            actions,
            sheet_short,
        })
    }
}

/// The project's title: its own, else the `.musi`'s stem, else the audio
/// file's. Never empty while a track is open.
fn project_title(track: &Track) -> String {
    track
        .project_metadata
        .as_ref()
        .map(|metadata| metadata.title.trim())
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .or_else(|| stem(track.project_path.as_deref()))
        .or_else(|| stem(Some(&track.file_path)))
        .unwrap_or_else(|| "Untitled".to_owned())
}

fn stem(path: Option<&std::path::Path>) -> Option<String> {
    path?
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.trim().is_empty())
}

/// The artist when the project names one, `None` otherwise — never a guess.
fn project_artist(track: &Track) -> Option<String> {
    track
        .project_metadata
        .as_ref()
        .map(|metadata| metadata.artist.trim())
        .filter(|artist| !artist.is_empty())
        .map(str::to_owned)
}

/// `5:22 · 1080p 16:9 · 30 fps`: the length, the output's rung and shape when
/// it is a preset, its geometry otherwise, and the frame rate.
pub(crate) fn info_line(duration_seconds: f64, config: &RenderExportConfig) -> String {
    format!(
        "{} \u{00b7} {} \u{00b7} {} fps",
        duration_label(duration_seconds),
        format_label(config.width, config.height),
        config.fps
    )
}

/// Whole minutes and seconds, floored: a 5:22.52 track is "5:22", as the
/// transport's `5:22.52` would round it, not "5:23". Hours when there are any.
fn duration_label(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds.floor() as u64
    } else {
        0
    };
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// `1080p 16:9` for a preset geometry (named by its short edge, EX2), the
/// bare `1234x568` for anything else — never the nearest preset, which would
/// tell the user their export is something it is not.
fn format_label(width: u32, height: u32) -> String {
    match (
        Aspect::of(width, height),
        Resolution::of_short_edge(width.min(height)),
    ) {
        (Some(aspect), Some(rung)) => format!("{} {}", rung.name(), aspect.name()),
        _ => format!("{width}x{height}"),
    }
}

/// The innermost clause of a save error, as the tracks header shortens it:
/// "Permission denied", not the path that preceded it.
fn save_reason_summary(reason: &str) -> String {
    let trimmed = reason.trim().trim_end_matches('.');
    trimmed
        .rsplit(": ")
        .next()
        .filter(|clause| !clause.trim().is_empty())
        .unwrap_or(trimmed)
        .trim()
        .to_owned()
}

/// The save dot's colour: the word's tint, except that Saved is the success
/// green the mockup's dot wears — the one calm state that has earned a colour
/// of its own — while its word stays muted.
fn save_dot_colour(state: SaveState) -> Color {
    match state {
        SaveState::Saved => color::ui_success(),
        other => save_state_tint(other),
    }
}

/// What the save state's tooltip says: the state in a sentence, and what to
/// do about it.
fn save_tip(track: &Track, state: SaveState) -> String {
    let path = track
        .project_path
        .as_ref()
        .map(|path| path.display().to_string());
    match state {
        SaveState::Saved => format!("Saved to {}", path.unwrap_or_default()),
        SaveState::Unsaved => format!(
            "Working changes, autosaved shortly to {} [Ctrl+S saves now]",
            path.unwrap_or_default()
        ),
        SaveState::NoProjectFile => {
            "No project file yet: Save As keeps this work [Ctrl+Shift+S]".to_owned()
        }
        SaveState::UnfiledWork => {
            "Unfiled work: nothing has saved it. Save As [Ctrl+Shift+S]".to_owned()
        }
        SaveState::Failed => format!(
            "Save failed: {}. Autosave waits for the next edit [Ctrl+S retries]",
            track
                .project_save_error
                .as_deref()
                .unwrap_or("the last write did not complete")
        ),
    }
}

fn tab_tip(tab: ModeTab) -> &'static str {
    match tab {
        ModeTab::Direct => "Direct: the preview and the plan, no panel open",
        ModeTab::Lyrics => "Lyrics: the lines and their timing",
        ModeTab::Look => "Look: the tuning inspector [T]",
        ModeTab::Assist => "Assist: analysis and lyric timing",
    }
}

/// The mockup's mark: a 22 px rounded square under a conic gradient
/// (`conic-gradient(from 200deg, #6d6496, #f0b347, #4f6f8c, #6d6496)`).
///
/// A fan of thin triangles from the centre, each carrying the gradient's
/// colour at its own angle at all three corners' angle positions, so the
/// interpolation runs *around* the centre as a conic gradient does rather
/// than across it. The warm stop is lifted off the accent on purpose: the
/// accent means *now*, and the census would rightly count a mark in it.
fn draw_mark(d: &mut RaylibDrawHandle<'_>, rect: UiRect) {
    const STOPS: [(f32, [u8; 3]); 4] = [
        (0.0, [0x6d, 0x64, 0x96]),
        (1.0 / 3.0, [0xc9, 0x93, 0x52]),
        (2.0 / 3.0, [0x4f, 0x6f, 0x8c]),
        (1.0, [0x6d, 0x64, 0x96]),
    ];
    const SEGMENTS: usize = 48;
    const RADIUS: f32 = 6.0;
    let at = |turn: f32| -> Color {
        let turn = turn.rem_euclid(1.0);
        let index = STOPS
            .windows(2)
            .position(|pair| turn <= pair[1].0)
            .unwrap_or(STOPS.len() - 2);
        let (from, to) = (STOPS[index], STOPS[index + 1]);
        let t = (turn - from.0) / (to.0 - from.0);
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        // The mockup's `opacity: .9`, over whichever surface the bar is.
        Color::new(
            mix(from.1[0], to.1[0]),
            mix(from.1[1], to.1[1]),
            mix(from.1[2], to.1[2]),
            230,
        )
    };
    let centre = Vector2::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let half = rect.width * 0.5;
    // The point on the rounded square's outline in the direction `angle`
    // (radians, clockwise from up, as CSS measures a conic gradient).
    let outline = |angle: f32| -> Vector2 {
        let (dx, dy) = (angle.sin(), -angle.cos());
        // Out to the square's edge, then pulled in at the corners onto the arc.
        let reach = half / dx.abs().max(dy.abs());
        let mut point = Vector2::new(dx * reach, dy * reach);
        let inner = half - RADIUS;
        if point.x.abs() > inner && point.y.abs() > inner {
            let corner = Vector2::new(inner * point.x.signum(), inner * point.y.signum());
            // Where the ray from the centre meets the corner's circle.
            let (cx, cy) = (corner.x, corner.y);
            let b = dx * cx + dy * cy;
            let c = cx * cx + cy * cy - RADIUS * RADIUS;
            let t = b + (b * b - c).max(0.0).sqrt();
            point = Vector2::new(dx * t, dy * t);
        }
        Vector2::new(centre.x + point.x, centre.y + point.y)
    };
    let from = 200.0_f32.to_radians();
    let mut mesh = Vec::with_capacity(SEGMENTS * 3);
    for segment in 0..SEGMENTS {
        let a0 = segment as f32 / SEGMENTS as f32;
        let a1 = (segment + 1) as f32 / SEGMENTS as f32;
        let mid = at((a0 + a1) * 0.5);
        let p0 = outline(from + a0 * std::f32::consts::TAU);
        let p1 = outline(from + a1 * std::f32::consts::TAU);
        // Counter-clockwise on screen (y down): centre, the later angle, the
        // earlier one — a clockwise triangle is culled and draws nothing.
        mesh.push((centre, mid));
        mesh.push((p1, at(a1)));
        mesh.push((p0, at(a0)));
    }
    draw::shaded_triangles(d, &mesh);
}

impl Shell {
    /// Draws the top bar into `rect`. Called by [`Shell::draw`] after the
    /// preview chrome, outside fullscreen.
    pub(crate) fn top_bar(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        rect: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        self.top_bar_state.drawn = None;
        self.top_bar_state.layout = None;
        if rect.is_empty() {
            self.cancel_artist_edit();
            return;
        }
        let Some(track) = input.workspace.current() else {
            self.cancel_artist_edit();
            return;
        };
        if self.assist_settings.is_open()
            || self
                .top_bar_state
                .artist_draft
                .as_ref()
                .is_some_and(|draft| draft.track != track.file_path)
        {
            self.cancel_artist_edit();
        }

        widgets::fill(d, rect, color::ui_surface());
        d.draw_line_ex(
            Vector2::new(rect.x, rect.y + rect.height - 0.5),
            Vector2::new(rect.x + rect.width, rect.y + rect.height - 0.5),
            1.0,
            color::ui_rule(),
        );

        let ui = input.fonts.ui();
        let title = project_title(track);
        let artist = project_artist(track);
        // The mockup sets the name in the interface face, and so does this —
        // whenever the chrome bank can draw every character of it. A title or
        // artist the bank cannot draw (it is Latin-only) goes through the
        // authored face instead, which covers the user's alphabet; the bar
        // never draws a `?` for a character in a name.
        let chrome = musializer_runtime::font::AuthoredText::from_ui(ui);
        let draft_text = self
            .top_bar_state
            .artist_draft
            .as_ref()
            .map_or("", |draft| draft.field.edit.text());
        let names = format!("{title}{}{draft_text}", artist.as_deref().unwrap_or(""));
        let authored = if chrome.missing(&names) == 0 {
            chrome
        } else {
            input.fonts.authored()
        };
        let info = info_line(track.duration_seconds, &track.render_config);
        let state = track.save_state();
        let reason = (state == SaveState::Failed)
            .then(|| track.project_save_error.as_deref().map(save_reason_summary))
            .flatten();
        let save_word = match &reason {
            Some(reason) => format!("{} ({reason})", state.label()),
            None => state.label().to_owned(),
        };

        const TITLE_SIZE: f32 = metric::UI_FONT_HEADER;
        const ARTIST_SIZE: f32 = metric::UI_FONT_LABEL;
        const META_SIZE: f32 = metric::UI_FONT_CAPTION;
        const TAB_SIZE: f32 = metric::UI_FONT_LABEL;
        const ACTION_SIZE: f32 = metric::UI_FONT_LABEL;
        let action_labels = ["Still", "Contact sheet", "Render\u{2026}"];
        // WX-5d: with the column, four tabs; Export is Render…'s view.
        let tab_list: &[ModeTab] = &ModeTab::ALL;
        let measures = TopBarMeasures {
            title: authored.measure_text(&title, TITLE_SIZE, 0.0).x,
            artist: if self.top_bar_state.artist_editing {
                160.0
            } else {
                authored
                    .measure_text(artist.as_deref().unwrap_or("Add artist"), ARTIST_SIZE, 0.0)
                    .x
                    .clamp(64.0, 160.0)
            },
            info: widgets::measure_tabular(ui, &info, META_SIZE),
            save_word: widgets::measure(ui, &save_word, META_SIZE),
            tabs: tab_list
                .iter()
                .map(|tab| widgets::measure(ui, tab.label(), TAB_SIZE))
                .collect(),
            actions: action_labels.map(|label| widgets::measure(ui, label, ACTION_SIZE)),
            sheet_short: widgets::measure(ui, "Sheet", ACTION_SIZE),
        };
        let layout = TopBarLayout::plan(rect, &measures);
        let pointer = self.widgets.pointer(d);
        let hover = |area: UiRect| ButtonState {
            hovered: area.contains_point(pointer.x, pointer.y),
            ..ButtonState::default()
        };
        let text_y = |size: f32| rect.y + (rect.height - size) * 0.5;

        // The name: mark, title, artist.
        draw_mark(d, layout.mark);
        let shown_title = authored.ellipsize(&title, layout.title.width + 0.5, TITLE_SIZE, 0.0);
        authored.draw_text(
            d,
            &shown_title,
            Vector2::new(layout.title.x, text_y(TITLE_SIZE)),
            TITLE_SIZE,
            0.0,
            color::ui_ink(),
        );
        let title_tip = match &track.project_path {
            Some(path) => format!("{title}: {}", path.display()),
            None => format!("{title}: not saved as a project yet"),
        };
        let title_id = widgets::widget_id(widgets::id::TOP_BAR, 20);
        self.widgets
            .hint(d, hover(layout.title), title_id, layout.title, &title_tip);
        if let Some(area) = layout.artist {
            let area = UiRect::new(
                area.x,
                rect.y + (rect.height - CONTROL_HEIGHT) * 0.5,
                area.width,
                CONTROL_HEIGHT,
            );
            let id = widgets::widget_id(widgets::id::TOP_BAR, ARTIST_SLOT);
            let button = self.widgets.button(d, id, area);
            if button.clicked && !self.top_bar_state.artist_editing {
                let mut field =
                    super::text_input::TextField::new(musializer_core::ui::text_edit::TextRules {
                        max_bytes: musializer_core::project::model::capacity::NAME,
                        ascii_only: false,
                    });
                field.bind(artist.as_deref().unwrap_or(""));
                field.set_focused(true);
                field.edit.select_all();
                self.top_bar_state.artist_draft = Some(ArtistDraft {
                    track: track.file_path.clone(),
                    field,
                });
                self.top_bar_state.artist_editing = true;
                self.top_bar_state.artist_event = "opened".into();
            }
            if self.top_bar_state.artist_editing {
                let mut action = if d.is_key_pressed(raylib::consts::KeyboardKey::KEY_ESCAPE) {
                    ArtistAction::Cancel
                } else if d.is_key_pressed(raylib::consts::KeyboardKey::KEY_ENTER)
                    || d.is_key_pressed(raylib::consts::KeyboardKey::KEY_KP_ENTER)
                {
                    ArtistAction::Commit
                } else {
                    ArtistAction::Draft
                };
                if let Some((text, probe_action)) = self.top_bar_state.artist_probe.take() {
                    if let Some(draft) = self.top_bar_state.artist_draft.as_mut() {
                        if let Some(text) = text {
                            let _ = draft.field.edit.set_text(&text);
                        }
                        if probe_action == ArtistAction::Clear {
                            draft.field.edit.clear();
                        }
                    }
                    action = probe_action;
                }
                if let Some(draft) = self.top_bar_state.artist_draft.as_mut() {
                    draft
                        .field
                        .draw_with_face(d, authored, area, ARTIST_SIZE, "Artist", true);
                    // Clicking away cancels the draft. Enter is the only commit.
                    if !draft.field.is_focused() && action == ArtistAction::Draft {
                        action = ArtistAction::Cancel;
                    }
                }
                self.finish_artist_edit(action, commands);
            } else {
                if button.hovered {
                    widgets::control_box(d, area, color::ui_hover(), color::ui_hover(), 1.0);
                }
                let label = artist.as_deref().unwrap_or("Add artist");
                let shown = authored.ellipsize(label, area.width, ARTIST_SIZE, 0.0);
                authored.draw_text(
                    d,
                    &shown,
                    Vector2::new(area.x, text_y(ARTIST_SIZE)),
                    ARTIST_SIZE,
                    0.0,
                    color::ui_muted(),
                );
            }
            self.widgets.hint(
                d,
                button,
                id,
                area,
                "Artist: click to edit · Enter saves · Escape cancels · empty clears",
            );
        }
        // The info line: tabular, so the duration does not jitter between
        // two tracks of the same length.
        if let Some(area) = layout.info {
            widgets::draw_text_tabular(
                d,
                ui,
                &info,
                area.x,
                text_y(META_SIZE),
                META_SIZE,
                color::ui_muted(),
            );
        }
        // The save state.
        d.draw_circle_v(
            Vector2::new(
                layout.dot.x + layout.dot.width * 0.5,
                layout.dot.y + layout.dot.height * 0.5,
            ),
            DOT_SIZE * 0.5,
            save_dot_colour(state),
        );
        if let Some(area) = layout.save_word {
            widgets::draw_text(
                d,
                ui,
                &save_word,
                area.x,
                text_y(META_SIZE),
                META_SIZE,
                save_state_tint(state),
            );
        }
        let save_area = layout.save_word.map_or(layout.dot, |word| {
            UiRect::new(
                layout.dot.x,
                rect.y,
                word.x + word.width - layout.dot.x,
                rect.height,
            )
        });
        let save_id = widgets::widget_id(widgets::id::TOP_BAR, 21);
        self.widgets.hint(
            d,
            hover(save_area),
            save_id,
            save_area,
            &save_tip(track, state),
        );

        // The tabs: a quiet group, the selected one outlined and tinted.
        widgets::control_box(
            d,
            layout.group,
            color::ui_raised(),
            color::ui_control_edge(),
            1.0,
        );
        for (tab, &area) in tab_list.iter().copied().zip(&layout.tabs) {
            // By what the tab is, so the four-tab bar keeps every tab's id.
            let index = ModeTab::ALL
                .iter()
                .position(|candidate| *candidate == tab)
                .unwrap_or(0);
            let id = widgets::widget_id(widgets::id::TOP_BAR, TAB_SLOT + index as u32);
            let button = self.widgets.button(d, id, area);
            let selected = self.mode_tab_selected(tab);
            let look = widgets::button_look(ButtonStyle::Neutral, selected, button.hovered);
            if selected || button.hovered {
                widgets::control_box(
                    d,
                    area,
                    look.fill,
                    if selected { look.edge } else { look.fill },
                    1.0,
                );
            }
            let ink = if selected || button.hovered {
                color::ui_ink()
            } else {
                color::ui_muted()
            };
            let width = widgets::measure(ui, tab.label(), TAB_SIZE);
            widgets::draw_text(
                d,
                ui,
                tab.label(),
                area.x + (area.width - width) * 0.5,
                area.y + (area.height - TAB_SIZE) * 0.5,
                TAB_SIZE,
                ink,
            );
            // The Direct tab's dot while another view shows (WX-5d §2): the
            // dock's news, never the accent.
            if tab == ModeTab::Direct && !selected {
                if let Some(tone) = self.column_report.badge.tone() {
                    d.draw_circle_v(
                        Vector2::new(area.x + area.width - 5.0, area.y + 7.0),
                        3.0,
                        tone.color(),
                    );
                }
            }
            self.widgets.hint(d, button, id, area, tab_tip(tab));
            if button.clicked {
                self.activate_mode_tab(tab, input.workspace, commands);
            }
        }

        // The actions. Render… is the one accent fill on the workspace.
        let sheet_label = if layout.sheet_short {
            "Sheet"
        } else {
            action_labels[1]
        };
        let still_id = widgets::widget_id(widgets::id::TOP_BAR, STILL_SLOT);
        let still = self.widgets.text_button(
            d,
            ui,
            still_id,
            layout.actions[0],
            action_labels[0],
            false,
            ButtonStyle::Neutral,
            Some(ACTION_SIZE),
        );
        self.widgets.hint(
            d,
            still,
            still_id,
            layout.actions[0],
            "Still: the frame at the playhead as a PNG, through the export renderer",
        );
        if still.clicked {
            commands.push(ShellCommand::ExportStill);
        }
        let sheet_id = widgets::widget_id(widgets::id::TOP_BAR, SHEET_SLOT);
        let sheet = self.widgets.text_button(
            d,
            ui,
            sheet_id,
            layout.actions[1],
            sheet_label,
            false,
            ButtonStyle::Neutral,
            Some(ACTION_SIZE),
        );
        let sheet_tip = if track.scene_switches.enabled && !track.scene_switches.is_empty() {
            format!(
                "Contact sheet: one frame per section ({}) as one PNG",
                track.scene_switches.len()
            )
        } else {
            format!(
                "Contact sheet: {} frames across the track as one PNG",
                musializer_core::render::contact_sheet::DEFAULT_TILES
            )
        };
        self.widgets
            .hint(d, sheet, sheet_id, layout.actions[1], &sheet_tip);
        if sheet.clicked {
            commands.push(ShellCommand::ExportContactSheet);
        }
        // The Export panel's own rule: a render needs FFmpeg, and a button that
        // cannot press says why rather than vanishing.
        let render_live = ffmpeg_available();
        let render_id = widgets::widget_id(widgets::id::TOP_BAR, RENDER_SLOT);
        let render_view = self.column_view() == super::shell::ColumnView::Render;
        {
            // WX-5d §3.5: Render… opens the Render view, whose commit renders.
            // While the view is open the button wears the selection look and
            // the commit carries the accent, so the frame still has one.
            let render = self.widgets.text_button(
                d,
                ui,
                render_id,
                layout.actions[2],
                action_labels[2],
                render_view,
                if render_view {
                    ButtonStyle::Neutral
                } else {
                    ButtonStyle::Primary
                },
                Some(ACTION_SIZE),
            );
            self.widgets.hint(
                d,
                render,
                render_id,
                layout.actions[2],
                if render_view {
                    "Close the render settings"
                } else {
                    "Render: size, shape, clip and quality, then choose a file"
                },
            );
            if render.clicked {
                self.toggle_column_view(
                    super::shell::ColumnView::Render,
                    input.workspace,
                    commands,
                );
            }
            // The format text opens the same view (§3.5): it names what the
            // view sets.
            if let Some(area) = layout.info {
                let format_id = widgets::widget_id(widgets::id::TOP_BAR, FORMAT_SLOT);
                let format = self.widgets.button(d, format_id, area);
                self.widgets.hint(
                    d,
                    format,
                    format_id,
                    area,
                    "Size, shape and frame rate: open the render settings",
                );
                if format.clicked && !render_view {
                    self.toggle_column_view(
                        super::shell::ColumnView::Render,
                        input.workspace,
                        commands,
                    );
                }
            }
        }

        let artist_editing = self.top_bar_state.artist_editing;
        let artist_draft = self.top_bar_state.artist_draft.take();
        let artist_event = std::mem::take(&mut self.top_bar_state.artist_event);
        let artist_probe = self.top_bar_state.artist_probe.take();
        self.top_bar_state = TopBarState {
            artist_editing,
            artist_draft,
            artist_event,
            artist_probe,
            drawn: Some(rect),
            title,
            artist,
            info,
            info_shown: layout.info.is_some(),
            save: Some(state),
            save_reason: reason,
            layout: Some(layout),
            render_live,
            render_view,
            scale: input.ui_scale.value(),
        };
    }

    pub(crate) fn cancel_artist_edit(&mut self) {
        self.top_bar_state.artist_editing = false;
        self.top_bar_state.artist_draft = None;
        self.top_bar_state.artist_probe = None;
    }

    /// Shared by real Enter/Escape and the headless input seam.
    fn finish_artist_edit(&mut self, action: ArtistAction, commands: &mut Vec<ShellCommand>) {
        if action == ArtistAction::Draft {
            return;
        }
        if let Some(draft) = self.top_bar_state.artist_draft.take() {
            if action != ArtistAction::Cancel {
                commands.push(ShellCommand::SetArtist {
                    track: draft.track,
                    artist: draft.field.edit.text().trim().to_owned(),
                });
            }
            self.top_bar_state.artist_event = if action == ArtistAction::Cancel {
                "cancelled"
            } else {
                "committed"
            }
            .into();
        }
        self.top_bar_state.artist_editing = false;
    }

    /// Keyboard ownership is queried before egui gathers and drains text.
    pub fn artist_editing(&self) -> bool {
        self.top_bar_state.artist_editing && !self.fullscreen && !self.assist_settings.is_open()
    }

    /// Probe input still requires a real tile-slot press to open the draft.
    pub fn probe_artist(&mut self, text: Option<String>, action: ArtistAction) {
        if self.artist_editing() {
            self.top_bar_state.artist_probe = Some((text, action));
        } else {
            self.top_bar_state.artist_event = "probe-refused-no-focus".into();
        }
    }

    /// The `top bar:` report line (§4): title, artist, info, save state, tabs
    /// with the selected ones starred, actions, and every rect in logical
    /// units (`scale=` turns them into pixels). `none` while the bar has had
    /// no room; `hidden` in fullscreen, which hides it.
    #[must_use]
    pub fn describe_top_bar(&self) -> String {
        if self.fullscreen {
            return "hidden (fullscreen)".to_owned();
        }
        let state = &self.top_bar_state;
        let (Some(rect), Some(layout)) = (state.drawn, state.layout.as_ref()) else {
            return "none".to_owned();
        };
        let at = |area: UiRect| {
            format!(
                "{:.0},{:.0},{:.0},{:.0}",
                area.x, area.y, area.width, area.height
            )
        };
        let tab_list: &[ModeTab] = &ModeTab::ALL;
        let tabs: Vec<String> = tab_list
            .iter()
            .copied()
            .zip(layout.tabs.iter().copied())
            .map(|(tab, area)| {
                format!(
                    "{}{}@{}",
                    tab.name(),
                    if self.mode_tab_selected(tab) { "*" } else { "" },
                    at(area)
                )
            })
            .collect();
        let save = state.save.map_or("none", SaveState::token);
        let save_rect = layout.save_word.map_or_else(|| "none".to_owned(), at);
        format!(
            "rect={} scale={:.2} title=\"{}\" artist={} info={} save={}{} word={} dot={} \
             layout={}{} tabs={} actions=still@{},{}@{},render{}@{}",
            at(rect),
            state.scale,
            state.title,
            // What was drawn: a project's artist the bar had no room for is
            // `hidden`, which is a different fact from having none.
            match (&state.artist, layout.artist) {
                (None, _) => "none".to_owned(),
                (Some(artist), Some(_)) => format!("\"{artist}\""),
                (Some(_), None) => "hidden".to_owned(),
            },
            if state.info_shown {
                format!("\"{}\"", state.info)
            } else {
                "hidden".to_owned()
            },
            save,
            state
                .save_reason
                .as_ref()
                .map_or_else(String::new, |reason| format!("({reason})")),
            save_rect,
            at(layout.dot),
            layout.shed.name(),
            if layout.centred { "" } else { ",packed" },
            tabs.join(";"),
            at(layout.actions[0]),
            if layout.sheet_short {
                "sheet-short"
            } else {
                "sheet"
            },
            at(layout.actions[1]),
            // Render… opens the Render view whether or not FFmpeg is on PATH:
            // the view's commit is what says it cannot render.
            if state.render_view { "(view)" } else { "" },
            at(layout.actions[2]),
        ) + &format!(
            " format={} artist-slot={} artist-editing={} artist-draft={:?} artist-event={} ffmpeg={}",
            layout.info.map_or_else(|| "hidden".to_owned(), at),
            layout.artist.map_or_else(
                || "none".to_owned(),
                |area| format!(
                    "artist@{}",
                    at(UiRect::new(
                        area.x,
                        rect.y + (rect.height - CONTROL_HEIGHT) * 0.5,
                        area.width,
                        CONTROL_HEIGHT
                    ))
                )
            ),
            state.artist_editing,
            state
                .artist_draft
                .as_ref()
                .map(|draft| draft.field.edit.text()),
            if state.artist_event.is_empty() {
                "none"
            } else {
                &state.artist_event
            },
            state.render_live
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plausible widths at 100 %: Space Grotesk 14 for the controls, Alegreya
    /// for the names.
    fn measures(artist: bool) -> TopBarMeasures {
        TopBarMeasures {
            title: 112.0,
            artist: if artist { 126.0 } else { 0.0 },
            info: 168.0,
            save_word: 38.0,
            tabs: vec![44.0, 46.0, 32.0, 46.0],
            actions: [32.0, 92.0, 62.0],
            sheet_short: 38.0,
        }
    }

    fn bar(width: f32) -> UiRect {
        UiRect::new(0.0, 0.0, width, 46.0)
    }

    fn overlaps(a: UiRect, b: UiRect) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width
    }

    /// Nothing shed at one width comes back at a narrower one: the rung only
    /// ever climbs as the bar narrows, with or without an artist.
    #[test]
    fn the_ladder_is_monotonic_in_width() {
        for artist in [true, false] {
            let m = measures(artist);
            let mut previous = 0;
            let mut width = 2400.0;
            while width >= 560.0 {
                let layout = TopBarLayout::plan(bar(width), &m);
                assert!(
                    layout.rung >= previous,
                    "at {width} the rung went back from {previous} to {}",
                    layout.rung
                );
                previous = layout.rung;
                width -= 1.0;
            }
        }
    }

    /// At every width from the 960 minimum down to a bar beside an open
    /// inspector, the three clusters never touch, and every control keeps its
    /// full height inside the bar.
    #[test]
    fn the_clusters_never_overlap() {
        let m = measures(true);
        let mut width = 2560.0;
        while width >= 960.0 {
            let layout = TopBarLayout::plan(bar(width), &m);
            let left = UiRect::new(
                0.0,
                0.0,
                layout
                    .save_word
                    .map_or(layout.dot.x + layout.dot.width, |word| word.x + word.width),
                46.0,
            );
            assert!(!overlaps(left, layout.group), "left meets tabs at {width}");
            for action in layout.actions {
                assert!(
                    !overlaps(layout.group, action),
                    "tabs meet actions at {width}"
                );
                assert!(action.x + action.width <= width - BAR_PADDING + 0.01);
                assert_eq!(action.height, CONTROL_HEIGHT);
            }
            width -= 7.0;
        }
    }

    /// The design's frame: the mockup's 1600 shows everything with the tabs
    /// centred, 1280 still shows everything, and the 960 minimum keeps the
    /// title whole and sheds from the info line.
    #[test]
    fn wide_bars_show_everything_and_the_minimum_sheds_the_info_first() {
        let m = measures(true);
        let wide = TopBarLayout::plan(bar(1600.0), &m);
        assert_eq!((wide.shed, wide.centred), (Shed::Nothing, true));
        assert!(wide.artist.is_some() && wide.info.is_some() && wide.save_word.is_some());
        let centre = wide.group.x + wide.group.width * 0.5;
        assert!((centre - 800.0).abs() < 0.01);
        assert_eq!(TopBarLayout::plan(bar(1280.0), &m).shed, Shed::Nothing);

        let minimum = TopBarLayout::plan(bar(960.0), &m);
        assert!(minimum.shed >= Shed::Info);
        assert_eq!(minimum.title.width, m.title);
        assert!(minimum.save_word.is_some(), "the save word survives at 960");
    }

    /// Below every rung the title gives way, but never below `MIN_TITLE`.
    #[test]
    fn only_the_last_rung_squeezes_the_title() {
        let mut m = measures(true);
        m.title = 600.0;
        let layout = TopBarLayout::plan(bar(960.0), &m);
        assert_eq!(layout.shed, Shed::Compact);
        assert!(layout.title.width < m.title);
        assert!(layout.title.width >= MIN_TITLE);
        assert!(layout.group.x >= layout.title.x + layout.title.width + 12.0 - 0.01);
    }

    #[test]
    fn the_info_line_names_the_preset_and_never_rounds_up() {
        let mut config = RenderExportConfig::default();
        (config.width, config.height, config.fps) = (1920, 1080, 30);
        assert_eq!(
            info_line(322.52, &config),
            "5:22 \u{b7} 1080p 16:9 \u{b7} 30 fps"
        );
        (config.width, config.height, config.fps) = (1080, 1920, 60);
        assert_eq!(
            info_line(59.999, &config),
            "0:59 \u{b7} 1080p 9:16 \u{b7} 60 fps"
        );
        (config.width, config.height) = (1234, 568);
        assert_eq!(
            info_line(3725.0, &config),
            "1:02:05 \u{b7} 1234x568 \u{b7} 60 fps"
        );
    }

    #[test]
    fn a_save_failure_is_shortened_to_its_cause() {
        assert_eq!(
            save_reason_summary("could not write /x/y.musi.tmp: Permission denied (os error 13)."),
            "Permission denied (os error 13)"
        );
        assert_eq!(save_reason_summary("disk full"), "disk full");
    }

    /// The artist is the project's own or nothing: an empty or blank one is
    /// no artist, and nothing is derived from a file name.
    #[test]
    fn the_artist_is_never_guessed() {
        let mut track = Track::new(
            std::path::PathBuf::from("/music/Chaos Cat Familiar - Door, No Room.mp3"),
            60.0,
            musializer_core::scene::SceneId::Spectrum,
            7,
        )
        .unwrap();
        assert_eq!(project_artist(&track), None);
        assert_eq!(project_title(&track), "Chaos Cat Familiar - Door, No Room");
        track.project_metadata = Some(musializer_core::project::model::Metadata {
            title: "Door, No Room".into(),
            artist: "  ".into(),
            ..Default::default()
        });
        assert_eq!(project_artist(&track), None);
        assert_eq!(project_title(&track), "Door, No Room");
        track.project_metadata.as_mut().unwrap().artist = "Chaos Cat Familiar".into();
        assert_eq!(
            project_artist(&track).as_deref(),
            Some("Chaos Cat Familiar")
        );
    }
    #[test]
    fn inline_artist_clear_returns_the_original_v1_bytes() {
        let mut track = Track::new(
            "fixture.wav".into(),
            20.0,
            musializer_core::scene::SceneId::Spectrum,
            7,
        )
        .unwrap();
        track.audio_sha256 = "a".repeat(64);
        let bytes = |track: &Track| {
            let project =
                crate::project::build_project(track, "fixture.wav", None, None, None, 44100, 1)
                    .unwrap();
            (
                project.schema_version(),
                musializer_core::project::io::serialize(&project).unwrap(),
            )
        };
        let original = bytes(&track);
        assert_eq!(original.0, 1);
        assert!(commit_artist(&mut track, "  Chaos Cat Familiar  ", 1.0));
        assert_eq!(bytes(&track).0, 2);
        assert_eq!(
            track.project_metadata.as_ref().unwrap().artist,
            "Chaos Cat Familiar"
        );
        assert!(commit_artist(&mut track, "", 2.0));
        assert_eq!(bytes(&track), original);
        assert!(!commit_artist(&mut track, "", 3.0));
        assert!(!commit_artist(&mut track, &"x".repeat(129), 4.0));
        assert_eq!(bytes(&track), original);
    }

    #[test]
    fn artist_commit_and_cancel_use_the_same_draft_without_a_window() {
        let mut shell = Shell::new();
        let draft = || {
            let mut field = super::super::text_input::TextField::new(
                musializer_core::ui::text_edit::TextRules {
                    max_bytes: 128,
                    ascii_only: false,
                },
            );
            field.bind("Space Mono F M T");
            field.set_focused(true);
            ArtistDraft {
                track: "fixture.wav".into(),
                field,
            }
        };
        let mut commands = Vec::new();
        shell.top_bar_state.artist_draft = Some(draft());
        shell.top_bar_state.artist_editing = true;
        shell.finish_artist_edit(ArtistAction::Cancel, &mut commands);
        assert!(commands.is_empty());
        assert!(!shell.artist_editing());
        shell.top_bar_state.artist_draft = Some(draft());
        shell.top_bar_state.artist_editing = true;
        shell.finish_artist_edit(ArtistAction::Commit, &mut commands);
        assert_eq!(
            commands,
            vec![ShellCommand::SetArtist {
                track: "fixture.wav".into(),
                artist: "Space Mono F M T".into()
            }]
        );
        assert!(!shell.artist_editing());
    }
}
