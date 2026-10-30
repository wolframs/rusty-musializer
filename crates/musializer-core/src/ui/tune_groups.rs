//! Look's parameter groups (WX-5d §3.3): TYPE, MOTION, COLOUR, LIGHT, SHAPE,
//! SCENE, and the Look column's geometry.
//!
//! **Owner: the Look & Assist builder.** A table from each descriptor key to a
//! group, beside the descriptors rather than inside them, so
//! `scene/settings.rs` and `differential_settings.sh` stay untouched. An
//! exhaustive test over `scene::settings::descriptors()` for every scene keeps
//! a new parameter from landing ungrouped.
//!
//! The groups answer "where would I look for this" for someone tuning a video,
//! not "how is it implemented":
//!
//! - **TYPE** is the words on screen: their size, spacing, halo, reveal, the
//!   lyric line a scene draws.
//! - **MOTION** is how the picture moves and how hard it answers the music:
//!   speeds, drift, sway, beat and onset responses.
//! - **COLOUR** is hue, saturation, palettes and colour splits.
//! - **LIGHT** is glow, bloom, light, shine and the finish laid over the
//!   picture: grain, scanlines, CRT, glitch.
//! - **SHAPE** is geometry: sizes, counts, densities, widths and depths.
//! - **SCENE** is the scene's staging choices: which field, solid or layout,
//!   where the camera sits, who is in the picture.
//!
//! Within a group the rows keep the descriptor order, so the table decides
//! only *where* a parameter goes, never the order a scene's author chose.
//!
//! The second half of the file is [`LookLayout`], every pixel the Look column
//! draws, as arithmetic with exact tests (the WX-5d rule: no number in the
//! drawing code that a test cannot pin).

use crate::scene::{settings, SceneId};
use crate::ui::workspace_layout::UiRect;

/// One of Look's six parameter groups, in the order the column lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Type,
    Motion,
    Colour,
    Light,
    Shape,
    Scene,
}

impl Group {
    /// Every group, in drawing order.
    pub const ALL: [Group; 6] = [
        Group::Type,
        Group::Motion,
        Group::Colour,
        Group::Light,
        Group::Shape,
        Group::Scene,
    ];

    /// The section head's word, drawn as a micro head (uppercased there).
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Group::Type => "Type",
            Group::Motion => "Motion",
            Group::Colour => "Colour",
            Group::Light => "Light",
            Group::Shape => "Shape",
            Group::Scene => "Scene",
        }
    }

    /// The lowercase token the `tune layout:` report line prints.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Group::Type => "type",
            Group::Motion => "motion",
            Group::Colour => "colour",
            Group::Light => "light",
            Group::Shape => "shape",
            Group::Scene => "scene",
        }
    }
}

use Group::{Colour as C, Light as L, Motion as M, Scene as S, Shape as H, Type as T};

/// The six pixel-cat scenes share one descriptor shape (`settings.pixel-*.`),
/// so they share one row of the table.
#[rustfmt::skip]
const PIXEL: &[(&str, Group)] = &[
    ("palette", C), ("hue", C), ("crt", L), ("response", M), ("shake", M),
    ("pace", T), ("pops", M), ("glitch", L), ("pixels", H), ("motion", M),
];

/// Every descriptor's group, keyed by the key's scene part and its last part
/// (`settings.<scene>.<suffix>`). Checked column by column by the exhaustive
/// test below, so it is skipped by rustfmt (AGENTS.md trap).
#[rustfmt::skip]
const TABLE: &[(&str, &[(&str, Group)])] = &[
    ("spectrum", &[("amplitude", M), ("trail", H), ("saturation", C), ("glow_softness", L),
                   ("hue_swing", C), ("core_glow", L), ("bar_taper", H), ("reflection", L)]),
    ("pulse", &[("scale", H), ("rings", H), ("motion", M), ("arc", H), ("weight", H),
                ("petals", H), ("hue", C), ("glow", L)]),
    ("orbital", &[("motion", M), ("radius", H), ("depth", H), ("nodes", H), ("links", H),
                  ("tilt", S), ("hue", C), ("reactivity", M), ("sway", M)]),
    ("ascii", &[("motion", M), ("cycling", M), ("scanlines", L), ("split", C), ("gain", L),
                ("tint", C)]),
    ("atlas", &[("height", H), ("width", H), ("depth", H), ("camera", S), ("contours", H),
                ("color", C), ("speed", M), ("wireframe", S), ("detail", S), ("hue_motion", C),
                ("orbit", M), ("zoom", S)]),
    ("terrarium", &[("motion", M), ("growth", H), ("particles", H), ("sim_speed", M),
                    ("creature_speed", M), ("glass_opacity", L), ("density", H),
                    ("creature_glow", L)]),
    ("constellation", &[("motion", M), ("scale", H), ("glow", L), ("event_duration", M),
                        ("event_reach", M), ("hue_swing", C), ("density", H), ("web", L)]),
    ("cadence", &[("scale", T), ("swarm", M), ("focus", M), ("beat", M), ("glow", L),
                  ("spacing", T), ("hue_swing", C), ("hue", C), ("saturation", C), ("halo", T),
                  ("reveal", T), ("ribbons", L)]),
    ("loom", &[("density", H), ("weight", H), ("complexity", H), ("edge", H), ("saturation", C),
               ("motion", M), ("glints", L)]),
    ("pentagram", &[("motion", M), ("nest", H), ("orbits", H), ("glow", L), ("chords", H),
                    ("hue", C), ("pulse", M), ("zoom", S)]),
    ("phosphor", &[("field", S), ("motion", M), ("breathe", M), ("density", H), ("ramp", S),
                   ("bloom", L), ("aberration", C), ("scanlines", L), ("hue", C),
                   ("reactivity", M), ("dwell", S), ("titles", T)]),
    ("clawd", &[("petals", M), ("bounce", M), ("spin", M), ("mood", S), ("cats", S), ("show", S),
                ("terminal", T), ("hue", C), ("daylight", L), ("ink", H), ("glow", L),
                ("wiggle", M)]),
    ("knot", &[("size", H), ("response", M), ("pulse", M), ("drift", M), ("flow", L), ("hue", C),
               ("atmosphere", L), ("polish", L), ("bloom", L), ("fringe", C), ("grain", L)]),
    ("stage", &[("layout", S), ("hue", C), ("saturation", C), ("scale", T), ("bounce", M),
                ("tilt", T), ("light", L), ("accent", T), ("grain", L), ("pace", T),
                ("familiar", S)]),
    ("threshold", &[("distance", S), ("open", H), ("light", L), ("haze", L), ("beyond", S),
                    ("presences", H), ("motes", H), ("mist", L), ("hue", C), ("sway", M),
                    ("reflection", L)]),
    ("fluid", &[("flow", M), ("curl", M), ("radius", H), ("lifetime", M), ("response", M),
                ("hue", C), ("glow", L), ("relief", L)]),
    ("chrome", &[("flow", M), ("scale", H), ("ripple", H), ("response", M), ("hue", C),
                 ("shine", L), ("contrast", L), ("grain", L)]),
    ("aurora", &[("flow", M), ("height", H), ("folds", H), ("response", M), ("hue", C),
                 ("glow", L), ("stars", H), ("spread", H)]),
    ("metaballs", &[("flow", M), ("count", H), ("size", H), ("response", M), ("hue", C),
                    ("metal", L), ("glow", L), ("orbit", M)]),
    ("tunnel", &[("speed", M), ("twist", H), ("sides", H), ("response", M), ("hue", C),
                 ("glow", L), ("depth", H), ("width", H)]),
    ("voronoi", &[("flow", M), ("cells", H), ("edges", H), ("response", M), ("hue", C),
                  ("glow", L), ("drift", M), ("facet", L)]),
    ("iridescent", &[("flow", M), ("scale", H), ("ripple", H), ("response", M), ("hue", C),
                     ("shimmer", L), ("light", L), ("spread", H)]),
    ("pixel-boot", PIXEL),
    ("pixel-shell", PIXEL),
    ("pixel-run", PIXEL),
    ("pixel-quartet", PIXEL),
    ("pixel-chapel", PIXEL),
    ("pixel-rave", PIXEL),
    ("glyphs", &[("solid", S), ("words", T), ("spin", M), ("size", H), ("density", H),
                 ("stroke", H), ("glow", L), ("ghosts", L), ("flicker", M), ("hue", C),
                 ("saturation", C), ("response", M)]),
];

/// The group a descriptor key belongs to, or `None` for a key the table does
/// not name (which the exhaustive test makes unreachable for a real key).
#[must_use]
pub fn group_of(key: &str) -> Option<Group> {
    let rest = key.strip_prefix("settings.")?;
    let (scene, suffix) = rest.rsplit_once('.')?;
    TABLE
        .iter()
        .find(|(name, _)| *name == scene)?
        .1
        .iter()
        .find(|(name, _)| *name == suffix)
        .map(|(_, group)| *group)
}

/// One group of a scene's parameters: its descriptor indices, in descriptor
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupRows {
    pub group: Group,
    pub indices: Vec<usize>,
}

/// A scene's parameters in Look's groups: groups in [`Group::ALL`] order,
/// empty groups left out, descriptor order within each. A key the table does
/// not name lands in [`Group::Scene`] rather than vanishing — a parameter the
/// user cannot reach is worse than one in the wrong group, and the test fails
/// either way.
#[must_use]
pub fn groups(scene: SceneId) -> Vec<GroupRows> {
    let descriptors = settings::descriptors(scene);
    Group::ALL
        .iter()
        .filter_map(|&group| {
            let indices: Vec<usize> = descriptors
                .iter()
                .enumerate()
                .filter(|(_, descriptor)| group_of(descriptor.key).unwrap_or(Group::Scene) == group)
                .map(|(index, _)| index)
                .collect();
            (!indices.is_empty()).then_some(GroupRows { group, indices })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The Look column's geometry.
//
// Everything below is the approved mockup's in numbers. The app measures text and passes widths in; nothing here draws.
// ---------------------------------------------------------------------------

/// Left and right inset of everything in the column's body (`.body`'s 14 px,
/// the head's left padding).
pub const LOOK_PADDING: f32 = 14.0;
/// The scene name, 20 px type on a 26 px line.
pub const LOOK_TITLE_HEIGHT: f32 = 26.0;
/// The scope sentence under it.
pub const LOOK_SCOPE_HEIGHT: f32 = 18.0;
const LOOK_TITLE_TOP: f32 = 12.0;
const LOOK_HEADER_BOTTOM_GAP: f32 = 12.0;
/// The fixed block above the scrolling list: name and scope.
pub const LOOK_HEADER_HEIGHT: f32 =
    LOOK_TITLE_TOP + LOOK_TITLE_HEIGHT + LOOK_SCOPE_HEIGHT + LOOK_HEADER_BOTTOM_GAP;
/// The footer pinned under the list: `Reset scene…` and the hint.
pub const LOOK_FOOTER_HEIGHT: f32 = 40.0;
const LOOK_FOOTER_BUTTON_HEIGHT: f32 = 26.0;
/// The two explore tiles.
pub const EXPLORE_TILE_HEIGHT: f32 = 62.0;
const EXPLORE_TILE_GAP: f32 = 8.0;
/// The audition card that replaces the tiles while a trial runs.
pub const AUDITION_CARD_HEIGHT: f32 = 118.0;
const AUDITION_PADDING: f32 = 12.0;
const AUDITION_LINE_HEIGHT: f32 = 16.0;
const AUDITION_BUTTON_HEIGHT: f32 = 30.0;
const AUDITION_KEEP_WIDTH: f32 = 58.0;
const AUDITION_REVERT_WIDTH: f32 = 66.0;
const AUDITION_GAP: f32 = 8.0;
/// A route draft kept for another scene or track (Edit draft / Discard draft).
pub const DRAFT_BAR_HEIGHT: f32 = 58.0;
/// The section head's height, as `widgets::SECTION_HEAD_HEIGHT` draws it.
pub const SECTION_HEAD_HEIGHT: f32 = 18.0;
/// A parameter row, as `widgets::SLIDER_ROW_HEIGHT` draws it.
pub const ROW_HEIGHT: f32 = 30.0;
/// A saved-look chip.
pub const CHIP_HEIGHT: f32 = 28.0;
const CHIP_GAP: f32 = 6.0;
const CHIP_ROW_GAP: f32 = 6.0;
/// Between a section head and what it heads.
const HEAD_GAP: f32 = 6.0;
/// Between two blocks in the list (explore, saved looks, each group).
const BLOCK_GAP: f32 = 16.0;
/// Between a row and the route editor opened under it, and after the editor.
const EDITOR_GAP_ABOVE: f32 = 4.0;
const EDITOR_GAP_BELOW: f32 = 8.0;
const LIST_TOP_PADDING: f32 = 2.0;
const LIST_BOTTOM_PADDING: f32 = 12.0;
/// The list's scrollbar: a thin bar inside the right padding, so it never
/// overlaps a route button.
pub const LOOK_SCROLLBAR_WIDTH: f32 = 4.0;
const LOOK_SCROLLBAR_INSET: f32 = 5.0;
const PRESET_ACTION_GAP: f32 = 4.0;

/// What the Look column has to place this frame. Text widths are measured by
/// the caller in the faces it draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct LookSpec {
    pub pixel_look: bool,
    /// Rows per drawn group, in drawing order (from [`groups`]).
    pub groups: Vec<(Group, usize)>,
    /// A route draft for another scene or track is waiting (its bar draws first).
    pub draft_bar: bool,
    /// An audition runs: the card replaces the two tiles.
    pub audition: bool,
    /// The saved-look chips' widths, label plus padding.
    pub chip_widths: Vec<f32>,
    /// `+ Save this look`'s width.
    pub save_width: f32,
    /// The empty library's sentence, drawn after the save chip; `0` for none.
    pub empty_hint_width: f32,
    /// The section head's trailing actions (Update, Delete), right-aligned;
    /// empty for none.
    pub preset_action_widths: Vec<f32>,
    /// The open route editor: the row it opens under (index into the flat row
    /// order, groups concatenated) and its height.
    pub editor: Option<(usize, f32)>,
}

/// The explore block: two tiles, or the audition card that replaces them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExploreLayout {
    Tiles { nudge: UiRect, surprise: UiRect },
    Audition(AuditionLayout),
}

/// The audition card (`TRYING SURPRISE 3 · 5 VALUES MOVED`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuditionLayout {
    pub card: UiRect,
    /// The micro line: label left, the moved count right-aligned in it.
    pub line: UiRect,
    /// The A · before / B · this switch's boundary (a segmented control).
    pub compare: UiRect,
    pub revert: UiRect,
    pub keep: UiRect,
    /// `Nudge again` and `Another`.
    pub again: UiRect,
    pub another: UiRect,
}

/// One group: its head and its rows, `(descriptor index, row)` pairs.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupLayout {
    pub group: Group,
    pub head: UiRect,
    /// `(position in the flat row order, row rect)`.
    pub rows: Vec<(usize, UiRect)>,
}

/// Everything the Look column draws below its head, in window coordinates
/// with the scroll offset already applied to the list.
#[derive(Clone, Debug, PartialEq)]
pub struct LookLayout {
    pub pixel: Option<UiRect>,
    pub title: UiRect,
    pub scope: UiRect,
    /// The scrolling list's window; the list is clipped to it.
    pub viewport: UiRect,
    pub scrollbar: UiRect,
    /// The list's full height, and how far it can scroll.
    pub content_height: f32,
    pub max_offset: f32,
    /// The offset these rects were laid out at, after clamping.
    pub offset: f32,
    pub draft_bar: Option<UiRect>,
    pub explore: ExploreLayout,
    pub presets_head: UiRect,
    /// Update and Delete, in [`LookSpec::preset_action_widths`] order.
    pub preset_actions: Vec<UiRect>,
    pub chips: Vec<UiRect>,
    pub save: UiRect,
    pub empty_hint: Option<UiRect>,
    pub groups: Vec<GroupLayout>,
    /// The open route editor's card.
    pub editor: Option<UiRect>,
    pub footer: UiRect,
    pub reset: UiRect,
    pub footer_hint: UiRect,
}

impl LookLayout {
    /// Every row in flat order, `(position, rect)`.
    pub fn rows(&self) -> impl Iterator<Item = (usize, UiRect)> + '_ {
        self.groups
            .iter()
            .flat_map(|group| group.rows.iter().copied())
    }
}

/// Flows chips left to right, wrapping at `width`; returns the rects relative
/// to `(x, y)` and the height used.
fn flow(x: f32, y: f32, width: f32, widths: &[f32]) -> (Vec<UiRect>, f32) {
    let mut rects = Vec::with_capacity(widths.len());
    let mut cursor = x;
    let mut top = y;
    for &chip in widths {
        let chip = chip.max(0.0).min(width.max(0.0));
        if cursor > x && cursor + chip > x + width {
            cursor = x;
            top += CHIP_HEIGHT + CHIP_ROW_GAP;
        }
        rects.push(UiRect::new(cursor, top, chip, CHIP_HEIGHT));
        cursor += chip + CHIP_GAP;
    }
    let height = if rects.is_empty() {
        0.0
    } else {
        top + CHIP_HEIGHT - y
    };
    (rects, height)
}

/// The Look column below its head. `body` is the column minus its head;
/// `offset` the list's requested scroll, clamped here.
#[must_use]
pub fn look_layout(body: UiRect, spec: &LookSpec, offset: f32) -> LookLayout {
    let x = body.x + LOOK_PADDING;
    let width = (body.width - LOOK_PADDING * 2.0).max(0.0);
    let title = UiRect::new(x, body.y + LOOK_TITLE_TOP, width, LOOK_TITLE_HEIGHT);
    let scope = UiRect::new(x, title.y + title.height, width, LOOK_SCOPE_HEIGHT);
    let footer_height = LOOK_FOOTER_HEIGHT.min(body.height.max(0.0));
    let footer = UiRect::new(
        body.x,
        body.y + body.height - footer_height,
        body.width.max(0.0),
        footer_height,
    );
    let viewport_top = (body.y + LOOK_HEADER_HEIGHT).min(footer.y);
    let viewport = UiRect::new(
        body.x,
        viewport_top,
        body.width.max(0.0),
        (footer.y - viewport_top).max(0.0),
    );

    // The list, laid out from y = 0 and shifted once its height is known.
    let mut y = LIST_TOP_PADDING;
    let draft_bar = spec.draft_bar.then(|| {
        let rect = UiRect::new(x, y, width, DRAFT_BAR_HEIGHT);
        y += DRAFT_BAR_HEIGHT + BLOCK_GAP;
        rect
    });

    let explore = if spec.audition {
        let card = UiRect::new(x, y, width, AUDITION_CARD_HEIGHT);
        let inner_x = x + AUDITION_PADDING;
        let inner_width = (width - AUDITION_PADDING * 2.0).max(0.0);
        let line = UiRect::new(
            inner_x,
            y + AUDITION_PADDING,
            inner_width,
            AUDITION_LINE_HEIGHT,
        );
        let second = line.y + line.height + 10.0;
        let keep = UiRect::new(
            inner_x + inner_width - AUDITION_KEEP_WIDTH,
            second,
            AUDITION_KEEP_WIDTH,
            AUDITION_BUTTON_HEIGHT,
        );
        let revert = UiRect::new(
            keep.x - AUDITION_GAP - AUDITION_REVERT_WIDTH,
            second,
            AUDITION_REVERT_WIDTH,
            AUDITION_BUTTON_HEIGHT,
        );
        let compare = UiRect::new(
            inner_x,
            second,
            (revert.x - AUDITION_GAP - inner_x).max(0.0),
            AUDITION_BUTTON_HEIGHT,
        );
        let third = second + AUDITION_BUTTON_HEIGHT + AUDITION_GAP;
        let half = ((inner_width - AUDITION_GAP) * 0.5).floor().max(0.0);
        let again = UiRect::new(inner_x, third, half, AUDITION_BUTTON_HEIGHT);
        let another = UiRect::new(
            inner_x + half + AUDITION_GAP,
            third,
            (inner_width - half - AUDITION_GAP).max(0.0),
            AUDITION_BUTTON_HEIGHT,
        );
        y += AUDITION_CARD_HEIGHT + BLOCK_GAP;
        ExploreLayout::Audition(AuditionLayout {
            card,
            line,
            compare,
            revert,
            keep,
            again,
            another,
        })
    } else {
        let half = ((width - EXPLORE_TILE_GAP) * 0.5).floor().max(0.0);
        let nudge = UiRect::new(x, y, half, EXPLORE_TILE_HEIGHT);
        let surprise = UiRect::new(
            x + half + EXPLORE_TILE_GAP,
            y,
            (width - half - EXPLORE_TILE_GAP).max(0.0),
            EXPLORE_TILE_HEIGHT,
        );
        y += EXPLORE_TILE_HEIGHT + BLOCK_GAP;
        ExploreLayout::Tiles { nudge, surprise }
    };

    // Saved looks: the head with its trailing actions, then the chip flow.
    let presets_head = UiRect::new(x, y, width, SECTION_HEAD_HEIGHT);
    let mut preset_actions = Vec::with_capacity(spec.preset_action_widths.len());
    let mut right = x + width;
    for &action in spec.preset_action_widths.iter().rev() {
        let action = action.max(0.0);
        right -= action;
        preset_actions.push(UiRect::new(
            right,
            y - 4.0,
            action,
            SECTION_HEAD_HEIGHT + 8.0,
        ));
        right -= PRESET_ACTION_GAP;
    }
    preset_actions.reverse();
    y += SECTION_HEAD_HEIGHT + HEAD_GAP;
    let mut widths = spec.chip_widths.clone();
    widths.push(spec.save_width);
    let (mut chips, chips_height) = flow(x, y, width, &widths);
    let save = chips.pop().unwrap_or(UiRect::new(x, y, 0.0, 0.0));
    // The sentence sits beside the save chip when it fits, under it when not.
    let mut block_height = chips_height;
    let empty_hint = (spec.empty_hint_width > 0.0).then(|| {
        let beside = save.x + save.width + CHIP_GAP * 2.0;
        if beside + spec.empty_hint_width <= x + width {
            UiRect::new(beside, save.y, spec.empty_hint_width, CHIP_HEIGHT)
        } else {
            let hint = UiRect::new(
                x,
                save.y + CHIP_HEIGHT + CHIP_ROW_GAP,
                spec.empty_hint_width.min(width),
                SECTION_HEAD_HEIGHT,
            );
            block_height = hint.y + hint.height - y;
            hint
        }
    });
    y += block_height + BLOCK_GAP;

    // The groups.
    let mut groups = Vec::with_capacity(spec.groups.len());
    let mut editor = None;
    let mut position = 0usize;
    for (index, &(group, count)) in spec.groups.iter().enumerate() {
        if index > 0 {
            y += BLOCK_GAP;
        }
        let head = UiRect::new(x, y, width, SECTION_HEAD_HEIGHT);
        y += SECTION_HEAD_HEIGHT + HEAD_GAP;
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            rows.push((position, UiRect::new(x, y, width, ROW_HEIGHT)));
            y += ROW_HEIGHT;
            if let Some((open, height)) = spec.editor {
                if open == position {
                    y += EDITOR_GAP_ABOVE;
                    editor = Some(UiRect::new(x, y, width, height.max(0.0)));
                    y += height.max(0.0) + EDITOR_GAP_BELOW;
                }
            }
            position += 1;
        }
        groups.push(GroupLayout { group, head, rows });
    }
    let pixel = spec.pixel_look.then(|| {
        y += BLOCK_GAP;
        let area = UiRect::new(x, y, width, super::pixel_look::HEIGHT);
        y += super::pixel_look::HEIGHT;
        area
    });

    let content_height = y + LIST_BOTTOM_PADDING;
    let max_offset = (content_height - viewport.height).max(0.0);
    let offset = if offset.is_finite() {
        offset.clamp(0.0, max_offset)
    } else {
        0.0
    };
    let shift = viewport.y - offset;
    let moved = |rect: UiRect| UiRect::new(rect.x, rect.y + shift, rect.width, rect.height);

    let explore = match explore {
        ExploreLayout::Tiles { nudge, surprise } => ExploreLayout::Tiles {
            nudge: moved(nudge),
            surprise: moved(surprise),
        },
        ExploreLayout::Audition(card) => ExploreLayout::Audition(AuditionLayout {
            card: moved(card.card),
            line: moved(card.line),
            compare: moved(card.compare),
            revert: moved(card.revert),
            keep: moved(card.keep),
            again: moved(card.again),
            another: moved(card.another),
        }),
    };
    let groups = groups
        .into_iter()
        .map(|group| GroupLayout {
            group: group.group,
            head: moved(group.head),
            rows: group
                .rows
                .into_iter()
                .map(|(position, row)| (position, moved(row)))
                .collect(),
        })
        .collect();

    let scrollbar = if max_offset > 0.0 && viewport.height > 0.0 {
        let track = viewport.height;
        let thumb = (track * track / content_height).max(24.0).min(track);
        let top = (track - thumb) * offset / max_offset;
        UiRect::new(
            viewport.x + viewport.width - LOOK_SCROLLBAR_INSET - LOOK_SCROLLBAR_WIDTH,
            viewport.y + top,
            LOOK_SCROLLBAR_WIDTH,
            thumb,
        )
    } else {
        UiRect::new(viewport.x + viewport.width, viewport.y, 0.0, 0.0)
    };

    let reset = UiRect::new(
        x - 6.0,
        (footer.y + (footer.height - LOOK_FOOTER_BUTTON_HEIGHT) * 0.5).round(),
        112.0f32.min(width),
        LOOK_FOOTER_BUTTON_HEIGHT,
    );
    let hint_x = reset.x + reset.width + 8.0;
    let footer_hint = UiRect::new(
        hint_x,
        reset.y,
        (x + width - hint_x).max(0.0),
        LOOK_FOOTER_BUTTON_HEIGHT,
    );

    LookLayout {
        pixel: pixel.map(moved),
        title,
        scope,
        viewport,
        scrollbar,
        content_height,
        max_offset,
        offset,
        draft_bar: draft_bar.map(moved),
        explore,
        presets_head: moved(presets_head),
        preset_actions: preset_actions.into_iter().map(moved).collect(),
        chips: chips.into_iter().map(moved).collect(),
        save: moved(save),
        empty_hint: empty_hint.map(moved),
        groups,
        editor: editor.map(moved),
        footer,
        reset,
        footer_hint,
    }
}

/// The scroll offset that brings `target` (in the layout's coordinates, as
/// [`look_layout`] returned it at `layout.offset`) fully into the viewport,
/// preferring its top when it is taller than the viewport. Used when a route
/// editor opens below the fold.
#[must_use]
pub fn reveal_offset(layout: &LookLayout, target: UiRect) -> f32 {
    let top = target.y - layout.viewport.y + layout.offset;
    let bottom = top + target.height;
    let mut offset = layout.offset;
    if bottom > offset + layout.viewport.height {
        offset = bottom - layout.viewport.height;
    }
    if top < offset {
        offset = top;
    }
    offset.clamp(0.0, layout.max_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_descriptor_of_every_scene_lands_in_exactly_one_group() {
        let mut total = 0;
        for scene in SceneId::ALL {
            let descriptors = settings::descriptors(scene);
            for descriptor in descriptors {
                assert!(
                    group_of(descriptor.key).is_some(),
                    "{} has no Look group: add it to tune_groups::TABLE",
                    descriptor.key
                );
            }
            let drawn: Vec<usize> = groups(scene)
                .into_iter()
                .flat_map(|rows| rows.indices)
                .collect();
            let mut sorted = drawn.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (0..descriptors.len()).collect::<Vec<_>>(),
                "{scene:?}: every row exactly once"
            );
            total += descriptors.len();
        }
        // The count the design was written against (§3.3): 29 scenes, 271
        // descriptors. A new scene or parameter moves it, and must move it
        // with its table row.
        assert_eq!(SceneId::ALL.len(), 29);
        assert_eq!(total, 271);
    }

    #[test]
    fn the_table_names_no_key_that_does_not_exist_and_none_twice() {
        let mut seen = std::collections::HashSet::new();
        for (scene, rows) in TABLE {
            for (suffix, _) in *rows {
                let key = format!("settings.{scene}.{suffix}");
                assert!(seen.insert(key.clone()), "{key} is in the table twice");
                assert!(
                    settings::descriptor_by_key(&key).is_some(),
                    "{key} is in the table but is no descriptor"
                );
            }
        }
        assert_eq!(seen.len(), 271);
    }

    #[test]
    fn cadence_groups_as_the_mockup_draws_it() {
        let groups = groups(SceneId::Cadence);
        let named: Vec<(Group, Vec<&str>)> = groups
            .iter()
            .map(|rows| {
                (
                    rows.group,
                    rows.indices
                        .iter()
                        .map(|&index| settings::descriptors(SceneId::Cadence)[index].label)
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            named,
            vec![
                (
                    Group::Type,
                    vec!["Type scale", "Letter spacing", "Type halo", "Reveal style"]
                ),
                (
                    Group::Motion,
                    vec!["Swarm spread", "Focus speed", "Beat breathing"]
                ),
                (Group::Colour, vec!["Semantic hue swing", "Hue", "Colour"]),
                (Group::Light, vec!["Particle glow", "Backdrop ribbons"]),
            ]
        );
    }

    #[test]
    fn group_of_refuses_malformed_and_unknown_keys() {
        assert_eq!(group_of("settings.cadence.scale"), Some(Group::Type));
        assert_eq!(group_of("settings.pixel-rave.crt"), Some(Group::Light));
        assert_eq!(group_of("cadence.scale"), None);
        assert_eq!(group_of("settings.cadence"), None);
        assert_eq!(group_of("settings.nowhere.scale"), None);
        assert_eq!(group_of("settings.cadence.nothing"), None);
    }

    fn cadence_spec() -> LookSpec {
        LookSpec {
            pixel_look: false,
            groups: vec![
                (Group::Type, 4),
                (Group::Motion, 3),
                (Group::Colour, 3),
                (Group::Light, 2),
            ],
            draft_bar: false,
            audition: false,
            chip_widths: vec![],
            save_width: 120.0,
            empty_hint_width: 240.0,
            preset_action_widths: vec![],
            editor: None,
        }
    }

    /// The 1920x1080 column's body: x 1460, below the 46 px top bar and the
    /// 40 px head, 460 wide.
    const BODY_1920: UiRect = UiRect::new(1460.0, 86.0, 460.0, 994.0);

    #[test]
    fn pixel_filter_extends_the_list_without_moving_parameters() {
        let spec = LookSpec {
            pixel_look: true,
            ..cadence_spec()
        };
        let layout = look_layout(BODY_1920, &spec, 0.0);
        assert_eq!(layout.pixel, Some(UiRect::new(1474.0, 822.0, 432.0, 290.0)));
        assert_eq!(layout.content_height, 970.0);
        assert_eq!(layout.max_offset, 84.0);
        assert_eq!(
            layout.groups,
            look_layout(BODY_1920, &cadence_spec(), 0.0).groups
        );
    }

    #[test]
    fn the_1920_column_places_every_block_exactly() {
        let layout = look_layout(BODY_1920, &cadence_spec(), 0.0);
        assert_eq!(layout.title, UiRect::new(1474.0, 98.0, 432.0, 26.0));
        assert_eq!(layout.scope, UiRect::new(1474.0, 124.0, 432.0, 18.0));
        assert_eq!(layout.viewport, UiRect::new(1460.0, 154.0, 460.0, 886.0));
        assert_eq!(layout.footer, UiRect::new(1460.0, 1040.0, 460.0, 40.0));
        assert_eq!(layout.reset, UiRect::new(1468.0, 1047.0, 112.0, 26.0));
        assert_eq!(layout.footer_hint, UiRect::new(1588.0, 1047.0, 318.0, 26.0));
        assert_eq!(
            layout.explore,
            ExploreLayout::Tiles {
                nudge: UiRect::new(1474.0, 156.0, 212.0, 62.0),
                surprise: UiRect::new(1694.0, 156.0, 212.0, 62.0),
            }
        );
        assert_eq!(layout.presets_head, UiRect::new(1474.0, 234.0, 432.0, 18.0));
        assert_eq!(layout.save, UiRect::new(1474.0, 258.0, 120.0, 28.0));
        assert_eq!(
            layout.empty_hint,
            Some(UiRect::new(1606.0, 258.0, 240.0, 28.0))
        );
        let heads: Vec<UiRect> = layout.groups.iter().map(|group| group.head).collect();
        assert_eq!(
            heads,
            vec![
                UiRect::new(1474.0, 302.0, 432.0, 18.0),
                UiRect::new(1474.0, 462.0, 432.0, 18.0),
                UiRect::new(1474.0, 592.0, 432.0, 18.0),
                UiRect::new(1474.0, 722.0, 432.0, 18.0),
            ]
        );
        let rows: Vec<(usize, UiRect)> = layout.rows().collect();
        assert_eq!(rows.len(), 12);
        assert_eq!(rows[0], (0, UiRect::new(1474.0, 326.0, 432.0, 30.0)));
        assert_eq!(rows[3], (3, UiRect::new(1474.0, 416.0, 432.0, 30.0)));
        assert_eq!(rows[4], (4, UiRect::new(1474.0, 486.0, 432.0, 30.0)));
        assert_eq!(rows[11], (11, UiRect::new(1474.0, 776.0, 432.0, 30.0)));
        // Twelve rows and four heads fit: nothing scrolls at 1920x1080.
        assert_eq!(layout.content_height, 664.0);
        assert_eq!(layout.max_offset, 0.0);
        assert!(layout.scrollbar.is_empty());
    }

    #[test]
    fn an_audition_replaces_the_tiles_with_its_card() {
        let spec = LookSpec {
            audition: true,
            ..cadence_spec()
        };
        let layout = look_layout(BODY_1920, &spec, 0.0);
        let ExploreLayout::Audition(card) = layout.explore else {
            panic!("no audition card");
        };
        assert_eq!(card.card, UiRect::new(1474.0, 156.0, 432.0, 118.0));
        assert_eq!(card.line, UiRect::new(1486.0, 168.0, 408.0, 16.0));
        assert_eq!(card.keep, UiRect::new(1836.0, 194.0, 58.0, 30.0));
        assert_eq!(card.revert, UiRect::new(1762.0, 194.0, 66.0, 30.0));
        assert_eq!(card.compare, UiRect::new(1486.0, 194.0, 268.0, 30.0));
        assert_eq!(card.again, UiRect::new(1486.0, 232.0, 200.0, 30.0));
        assert_eq!(card.another, UiRect::new(1694.0, 232.0, 200.0, 30.0));
        // Everything below moved by the card's extra 56 px.
        assert_eq!(layout.presets_head.y, 234.0 + 56.0);
        // The card stays inside its column and clear of the scrollbar's gutter.
        assert!(card.card.x + card.card.width <= BODY_1920.x + BODY_1920.width - 14.0);
    }

    #[test]
    fn the_route_editor_opens_under_its_row_and_pushes_the_rest_down() {
        let spec = LookSpec {
            editor: Some((6, 354.0)),
            ..cadence_spec()
        };
        let layout = look_layout(BODY_1920, &spec, 0.0);
        let rows: Vec<(usize, UiRect)> = layout.rows().collect();
        assert_eq!(rows[6].1, UiRect::new(1474.0, 546.0, 432.0, 30.0));
        assert_eq!(
            layout.editor,
            Some(UiRect::new(1474.0, 580.0, 432.0, 354.0))
        );
        assert_eq!(rows[7].1.y, 592.0 + 4.0 + 354.0 + 8.0 + 24.0);
        // Now it scrolls, and the editor can be brought fully into view.
        assert!(layout.max_offset > 0.0);
        let offset = reveal_offset(&layout, layout.editor.unwrap());
        let shown = look_layout(BODY_1920, &spec, offset);
        let editor = shown.editor.unwrap();
        assert!(editor.y >= shown.viewport.y);
        assert!(editor.y + editor.height <= shown.viewport.y + shown.viewport.height);
    }

    #[test]
    fn the_offset_is_clamped_and_moves_only_the_list() {
        let spec = LookSpec {
            editor: Some((0, 354.0)),
            ..cadence_spec()
        };
        let small = UiRect::new(962.0, 86.0, 318.0, 554.0);
        let top = look_layout(small, &spec, -50.0);
        assert_eq!(top.offset, 0.0);
        let bottom = look_layout(small, &spec, 1.0e9);
        assert_eq!(bottom.offset, bottom.max_offset);
        assert_eq!(bottom.title, top.title);
        assert_eq!(bottom.footer, top.footer);
        assert_eq!(
            bottom.presets_head.y,
            top.presets_head.y - bottom.max_offset
        );
        // At the bottom the last row ends exactly 12 px above the footer.
        let last = bottom.rows().last().unwrap().1;
        assert_eq!(last.y + last.height + 12.0, bottom.footer.y);
        assert_eq!(look_layout(small, &spec, f32::NAN).offset, 0.0);
        // The scrollbar thumb runs the viewport's full travel.
        assert_eq!(top.scrollbar.y, top.viewport.y);
        assert_eq!(
            bottom.scrollbar.y + bottom.scrollbar.height,
            bottom.viewport.y + bottom.viewport.height
        );
        assert_eq!(top.scrollbar.x, 962.0 + 318.0 - 9.0);
    }

    #[test]
    fn chips_wrap_and_the_actions_sit_at_the_heads_right_end() {
        let spec = LookSpec {
            chip_widths: vec![90.0, 90.0, 90.0, 90.0],
            save_width: 120.0,
            empty_hint_width: 0.0,
            preset_action_widths: vec![54.0, 52.0],
            ..cadence_spec()
        };
        // 330 wide: three 90s fit with their 6 px gaps (282), the fourth wraps.
        let body = UiRect::new(0.0, 0.0, 358.0, 900.0);
        let layout = look_layout(body, &spec, 0.0);
        let head = layout.presets_head;
        assert_eq!(
            layout.chips[0],
            UiRect::new(14.0, head.y + 24.0, 90.0, 28.0)
        );
        assert_eq!(
            layout.chips[2],
            UiRect::new(206.0, head.y + 24.0, 90.0, 28.0)
        );
        assert_eq!(
            layout.chips[3],
            UiRect::new(14.0, head.y + 58.0, 90.0, 28.0)
        );
        assert_eq!(layout.save, UiRect::new(110.0, head.y + 58.0, 120.0, 28.0));
        assert_eq!(layout.empty_hint, None);
        assert_eq!(
            layout.preset_actions,
            vec![
                UiRect::new(344.0 - 52.0 - 4.0 - 54.0, head.y - 4.0, 54.0, 26.0),
                UiRect::new(344.0 - 52.0, head.y - 4.0, 52.0, 26.0),
            ]
        );
        // The first group starts one block gap below the second chip row.
        assert_eq!(layout.groups[0].head.y, head.y + 58.0 + 28.0 + 16.0);
    }

    #[test]
    fn an_empty_hint_that_does_not_fit_beside_the_chip_goes_under_it() {
        let spec = LookSpec {
            empty_hint_width: 260.0,
            ..cadence_spec()
        };
        let body = UiRect::new(0.0, 0.0, 320.0, 900.0);
        let layout = look_layout(body, &spec, 0.0);
        let save = layout.save;
        assert_eq!(
            layout.empty_hint,
            Some(UiRect::new(14.0, save.y + 34.0, 260.0, 18.0))
        );
        assert_eq!(layout.groups[0].head.y, save.y + 34.0 + 18.0 + 16.0);
    }

    #[test]
    fn a_waiting_route_draft_takes_the_top_of_the_list() {
        let spec = LookSpec {
            draft_bar: true,
            ..cadence_spec()
        };
        let layout = look_layout(BODY_1920, &spec, 0.0);
        assert_eq!(
            layout.draft_bar,
            Some(UiRect::new(1474.0, 156.0, 432.0, 58.0))
        );
        let ExploreLayout::Tiles { nudge, .. } = layout.explore else {
            panic!("tiles");
        };
        assert_eq!(nudge.y, 156.0 + 58.0 + 16.0);
    }
}
