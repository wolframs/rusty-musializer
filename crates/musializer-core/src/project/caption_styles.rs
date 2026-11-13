//! Named caption styles and the title lane (post-legacy, MV2, 2026-11-13).
//!
//! Before this a project had exactly one caption style, which a director hit
//! within minutes twice over: a bottom caption collides with whatever a scene
//! keeps at the bottom (Clawd's cats, Spectrum's bars), and a requested title
//! card had to be a lyric line pushed through Cadence.
//!
//! - A **named style** states only what differs from the project's base
//!   [`CaptionStyle`] and inherits the rest. A lyric line picks one, a section
//!   of the plan may name one as the default for the lines that *begin* inside
//!   it. They stack: the section's members are written over the base and the
//!   line's over those, so the line's choice wins over the section's and the
//!   section's over the base, member by member. A voice still colours a line
//!   on top of whatever style it has.
//! - A **title card** is a timed entry of its own lane: a title line and, under
//!   it, an artist line (a display name and an optional `@handle`). The shared
//!   caption path draws it after every scene, Cadence and Stage included, so a
//!   title no longer needs a scene. Nothing adds one by default (operator,
//!   2026-10-12: no title card unless one is asked for, and one that is asked
//!   for carries the artist).
//!
//! Pure data and resolution; the drawing is the application's.

use super::model::{caption, caption_fx, CaptionAnchor, CaptionBox, CaptionFace, CaptionStyle};

/// Most named styles a project may hold.
pub const CAPTION_STYLE_CAPACITY: usize = 16;
/// Longest style name, in bytes — the voice rule, so one grammar names both.
pub const STYLE_NAME_MAX_BYTES: usize = 32;
/// Most title cards a project may hold.
pub const TITLE_CAPACITY: usize = 16;
/// Longest title, artist or handle text, in bytes: the `.musi` metadata
/// capacity the project's own title and artist already have.
pub const TITLE_TEXT_MAX_BYTES: usize = super::model::capacity::NAME;
/// Longest `@handle`, in bytes.
pub const HANDLE_MAX_BYTES: usize = 64;
/// How long a title card stays up when its entry does not say.
pub const DEFAULT_TITLE_SECONDS: f64 = 5.0;
/// The shortest title card: under this the entrance cannot finish.
pub const MIN_TITLE_SECONDS: f64 = 0.5;

/// A style's name, held inline so a plan section (which is `Copy`) can carry
/// one. `[A-Za-z0-9_-]`, 1..=32 bytes, the same rule a voice's name has.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StyleName {
    bytes: [u8; STYLE_NAME_MAX_BYTES],
    len: u8,
}

impl StyleName {
    /// `None` for a name outside the rule.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        if !Self::is_valid(name) {
            return None;
        }
        let mut bytes = [0u8; STYLE_NAME_MAX_BYTES];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Some(Self {
            bytes,
            len: name.len() as u8,
        })
    }

    #[must_use]
    pub fn is_valid(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= STYLE_NAME_MAX_BYTES
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        // Only ASCII is ever stored (see `new`).
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

impl core::fmt::Debug for StyleName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl core::fmt::Display for StyleName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a named style changes in the caption effects. Every member optional;
/// the drive tuning blocks are not overridable (they belong to the base).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectsOverride {
    pub glow_strength: Option<f64>,
    pub glow_radius: Option<f64>,
    pub glow_rgba: Option<u32>,
    pub glow_pulse: Option<super::model::EffectDrive>,
    pub glow_pulse_depth: Option<f64>,
    pub glow_hue_drive: Option<super::model::EffectDrive>,
    pub glow_hue_range: Option<f64>,
    pub shadow_blur: Option<f64>,
    pub shadow_opacity: Option<f64>,
    pub plate_roundness: Option<f64>,
    pub entrance_seconds: Option<f64>,
    pub exit_seconds: Option<f64>,
}

/// What a named style changes. Every member optional: absent is "the base's".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaptionOverride {
    /// Never [`CaptionFace::Imported`]: the imported face is the base's one
    /// bundled asset, so a style can keep it (by not naming a face) but not
    /// name it.
    pub face: Option<CaptionFace>,
    pub box_style: Option<CaptionBox>,
    pub anchor: Option<CaptionAnchor>,
    pub size_scale: Option<f64>,
    pub margin_scale: Option<f64>,
    pub width_scale: Option<f64>,
    pub text_rgba: Option<u32>,
    pub box_rgba: Option<u32>,
    pub effects: EffectsOverride,
}

fn within(value: Option<f64>, minimum: f64, maximum: f64) -> bool {
    value.is_none_or(|value| value.is_finite() && value >= minimum && value <= maximum)
}

impl CaptionOverride {
    /// Every stated member inside the bounds the base's own member has.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let fx = &self.effects;
        self.face != Some(CaptionFace::Imported)
            && within(
                self.size_scale,
                caption::SIZE_MINIMUM,
                caption::SIZE_MAXIMUM,
            )
            && within(
                self.margin_scale,
                caption::MARGIN_MINIMUM,
                caption::MARGIN_MAXIMUM,
            )
            && within(
                self.width_scale,
                caption::WIDTH_MINIMUM,
                caption::WIDTH_MAXIMUM,
            )
            && within(
                fx.glow_strength,
                caption_fx::GLOW_STRENGTH_MINIMUM,
                caption_fx::GLOW_STRENGTH_MAXIMUM,
            )
            && within(
                fx.glow_radius,
                caption_fx::GLOW_RADIUS_MINIMUM,
                caption_fx::GLOW_RADIUS_MAXIMUM,
            )
            && within(
                fx.glow_pulse_depth,
                caption_fx::PULSE_DEPTH_MINIMUM,
                caption_fx::PULSE_DEPTH_MAXIMUM,
            )
            && within(
                fx.glow_hue_range,
                caption_fx::HUE_RANGE_MINIMUM,
                caption_fx::HUE_RANGE_MAXIMUM,
            )
            && within(
                fx.shadow_blur,
                caption_fx::SHADOW_BLUR_MINIMUM,
                caption_fx::SHADOW_BLUR_MAXIMUM,
            )
            && within(
                fx.shadow_opacity,
                caption_fx::SHADOW_OPACITY_MINIMUM,
                caption_fx::SHADOW_OPACITY_MAXIMUM,
            )
            && within(
                fx.plate_roundness,
                caption_fx::PLATE_ROUNDNESS_MINIMUM,
                caption_fx::PLATE_ROUNDNESS_MAXIMUM,
            )
            && within(
                fx.entrance_seconds,
                caption_fx::ENTRANCE_MINIMUM,
                caption_fx::ENTRANCE_MAXIMUM,
            )
            && within(
                fx.exit_seconds,
                caption_fx::ENTRANCE_MINIMUM,
                caption_fx::ENTRANCE_MAXIMUM,
            )
    }

    /// Whether the style states nothing at all. Legal — a name can be declared
    /// before it is designed — and drawn exactly as the base.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The members it states, for an interface that says what a style does.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(face) = self.face {
            parts.push(face.canonical_name().replace('_', " "));
        }
        if let Some(anchor) = self.anchor {
            parts.push(anchor.canonical_name().replace('_', " "));
        }
        if let Some(size) = self.size_scale {
            parts.push(format!("size {size:.3}"));
        }
        if let Some(backing) = self.box_style {
            parts.push(format!("{} backing", backing.canonical_name()));
        }
        if self.margin_scale.is_some() || self.width_scale.is_some() {
            parts.push("measure".into());
        }
        if self.text_rgba.is_some() {
            parts.push("ink".into());
        }
        if self.box_rgba.is_some() {
            parts.push("backing colour".into());
        }
        if self.effects != EffectsOverride::default() {
            parts.push("effects".into());
        }
        if parts.is_empty() {
            "as the base".into()
        } else {
            parts.join(", ")
        }
    }

    /// `base` with this style's members written over it. The result carries no
    /// table of its own: it is a style to draw with, not one to save.
    #[must_use]
    pub fn apply(&self, base: &CaptionStyle) -> CaptionStyle {
        let mut style = CaptionStyle {
            styles: Vec::new(),
            ..base.clone()
        };
        if let Some(face) = self.face {
            style.face = face;
            // The bundled face stays with the base; a style that names a
            // built-in face draws in it.
            style.font = None;
        }
        if let Some(value) = self.box_style {
            style.box_style = value;
        }
        if let Some(value) = self.anchor {
            style.anchor = value;
        }
        if let Some(value) = self.size_scale {
            style.size_scale = value;
        }
        if let Some(value) = self.margin_scale {
            style.margin_scale = value;
        }
        if let Some(value) = self.width_scale {
            style.width_scale = value;
        }
        if let Some(value) = self.text_rgba {
            style.text_rgba = value;
        }
        if let Some(value) = self.box_rgba {
            style.box_rgba = value;
        }
        let over = &self.effects;
        let fx = &mut style.effects;
        let set = |slot: &mut f64, value: Option<f64>| {
            if let Some(value) = value {
                *slot = value;
            }
        };
        set(&mut fx.glow_strength, over.glow_strength);
        set(&mut fx.glow_radius, over.glow_radius);
        set(&mut fx.glow_pulse_depth, over.glow_pulse_depth);
        set(&mut fx.glow_hue_range, over.glow_hue_range);
        set(&mut fx.shadow_blur, over.shadow_blur);
        set(&mut fx.shadow_opacity, over.shadow_opacity);
        set(&mut fx.plate_roundness, over.plate_roundness);
        set(&mut fx.entrance_seconds, over.entrance_seconds);
        set(&mut fx.exit_seconds, over.exit_seconds);
        if let Some(value) = over.glow_rgba {
            fx.glow_rgba = value;
        }
        if let Some(value) = over.glow_pulse {
            fx.glow_pulse = value;
        }
        if let Some(value) = over.glow_hue_drive {
            fx.glow_hue_drive = value;
        }
        style
    }
}

/// One entry of the table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NamedCaptionStyle {
    pub name: StyleName,
    pub style: CaptionOverride,
}

/// The table's rule: bounded, names unique, every entry valid.
#[must_use]
pub fn table_is_valid(table: &[NamedCaptionStyle]) -> bool {
    table.len() <= CAPTION_STYLE_CAPACITY
        && table.iter().enumerate().all(|(index, entry)| {
            entry.style.is_valid() && !table[..index].iter().any(|other| other.name == entry.name)
        })
}

/// The named style `name` in `table`.
#[must_use]
pub fn find(table: &[NamedCaptionStyle], name: StyleName) -> Option<&CaptionOverride> {
    table
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| &entry.style)
}

/// The style a lyric line is drawn in: the base, then the named style its
/// section names written over it, then the line's own written over that. So
/// the line's members win over the section's and the section's over the
/// base, and a member neither states is the base's: a shouted line in a
/// section that moved its captions to the top stays at the top. (Found on the
/// first contact sheet: with the line's style *replacing* the section's, the
/// shout dropped back to the bottom among Clawd's cats, and every such line
/// would have needed a second style that repeated the section's anchor.)
///
/// A name the table does not hold — reachable only through an in-memory
/// edit, since every loader refuses one — is skipped rather than drawing
/// nothing. Borrowed when no named style applies, so the common case clones
/// nothing. The second value names what applied, section first: `top+shout`.
#[must_use]
pub fn resolve_line<'a>(
    base: &'a CaptionStyle,
    own: Option<StyleName>,
    section: Option<StyleName>,
) -> (std::borrow::Cow<'a, CaptionStyle>, Option<String>) {
    let found = |name: Option<StyleName>| {
        name.and_then(|name| find(&base.styles, name).map(|style| (name, style)))
    };
    let applied: Vec<(StyleName, &CaptionOverride)> =
        [found(section), found(own)].into_iter().flatten().collect();
    if applied.is_empty() {
        return (std::borrow::Cow::Borrowed(base), None);
    }
    let mut style = base.clone();
    let label = applied
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join("+");
    for (_, over) in applied {
        style = over.apply(&style);
    }
    (std::borrow::Cow::Owned(style), Some(label))
}

/// One title card: a title line, and under it who made the music.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleCard {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub title: String,
    /// The display name. `None` takes the project's artist (WX-5c's
    /// `metadata.artist`) when the card is drawn.
    pub artist: Option<String>,
    /// `@name`, drawn after the artist.
    pub handle: Option<String>,
    /// A named style written over the title look (see [`title_look`]).
    pub style: Option<StyleName>,
}

/// Title, artist and handle text: 1..=128 bytes, no control characters.
#[must_use]
pub fn title_text_is_valid(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= TITLE_TEXT_MAX_BYTES
        && !text.bytes().any(|byte| byte < 0x20 || byte == 0x7F)
}

/// `@` and then 1..=63 bytes with no whitespace or control characters.
#[must_use]
pub fn handle_is_valid(handle: &str) -> bool {
    handle.len() >= 2
        && handle.len() <= HANDLE_MAX_BYTES
        && handle.starts_with('@')
        && !handle.bytes().any(|byte| byte <= 0x20 || byte == 0x7F)
        && !handle[1..].contains('@')
}

impl TitleCard {
    #[must_use]
    pub fn is_valid(&self, duration_seconds: f64) -> bool {
        self.start_seconds.is_finite()
            && self.end_seconds.is_finite()
            && self.start_seconds >= 0.0
            && self.end_seconds > self.start_seconds
            && self.end_seconds <= duration_seconds
            && title_text_is_valid(&self.title)
            && self.artist.as_deref().is_none_or(title_text_is_valid)
            && self.handle.as_deref().is_none_or(handle_is_valid)
    }

    /// Whether the card is on screen at `time_seconds`: half-open, like a cue.
    #[must_use]
    pub fn covers(&self, time_seconds: f64) -> bool {
        self.start_seconds <= time_seconds && time_seconds < self.end_seconds
    }
}

/// The lane's rule: bounded, each valid, in order and never overlapping (one
/// card at a time — two stacked cards read as a layout fault), every style
/// name in `table`.
#[must_use]
pub fn titles_are_valid(
    titles: &[TitleCard],
    duration_seconds: f64,
    table: &[NamedCaptionStyle],
) -> bool {
    titles.len() <= TITLE_CAPACITY
        && titles.iter().enumerate().all(|(index, card)| {
            card.is_valid(duration_seconds)
                && (index == 0 || titles[index - 1].end_seconds <= card.start_seconds + 1.0e-9)
                && card.style.is_none_or(|name| find(table, name).is_some())
        })
}

/// The card on screen at `time_seconds`.
#[must_use]
pub fn title_at(titles: &[TitleCard], time_seconds: f64) -> Option<&TitleCard> {
    titles.iter().find(|card| card.covers(time_seconds))
}

/// How much smaller the artist line is than the title line.
pub const TITLE_ARTIST_RATIO: f32 = 0.30;

/// The built-in look of a title card, before any named style.
///
/// The face, ink and imported font are the project's own, so the card is set
/// in the type the video's captions use; the layout is the card's: centred in
/// the frame, the title at a little over twice a caption's size, no plate but
/// a soft shadow and a faint glow so it holds over any scene, arriving over
/// most of a second and leaving a little faster. A named style's members are
/// written over this, so `{ "anchor": "top_left" }` moves a card and keeps
/// everything else that makes it one.
#[must_use]
pub fn title_look(base: &CaptionStyle, style: Option<&CaptionOverride>) -> CaptionStyle {
    let mut look = CaptionStyle {
        styles: Vec::new(),
        anchor: CaptionAnchor::MiddleCenter,
        box_style: CaptionBox::Shadow,
        box_rgba: 0x0000_00D0,
        size_scale: 0.104,
        margin_scale: 0.09,
        width_scale: 0.80,
        ..base.clone()
    };
    let fx = &mut look.effects;
    *fx = super::model::CaptionEffects {
        glow_strength: 0.32,
        glow_radius: 0.42,
        glow_rgba: 0xFFE7_C2FF,
        shadow_blur: 0.32,
        shadow_opacity: 0.9,
        entrance_seconds: 1.1,
        exit_seconds: 0.8,
        ..super::model::CaptionEffects::default()
    };
    match style {
        Some(style) => style.apply(&look),
        None => look,
    }
}

/// The artist line a card draws: its own artist, else the project's, and the
/// handle after it. `None` when neither names an artist — the case a report
/// must say out loud, because the operator's rule is that a requested card
/// carries the artist.
#[must_use]
pub fn artist_line(card: &TitleCard, project_artist: Option<&str>) -> Option<String> {
    let name = card
        .artist
        .as_deref()
        .or(project_artist.filter(|artist| !artist.is_empty()));
    match (name, card.handle.as_deref()) {
        (Some(name), Some(handle)) => Some(format!("{name}  ·  {handle}")),
        (Some(name), None) => Some(name.to_owned()),
        (None, Some(handle)) => Some(handle.to_owned()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> StyleName {
        StyleName::new(text).unwrap()
    }

    #[test]
    fn a_style_name_is_the_voice_rule_and_round_trips() {
        assert_eq!(name("top-left_2").as_str(), "top-left_2");
        assert!(StyleName::new("").is_none());
        assert!(StyleName::new("a b").is_none());
        assert!(StyleName::new(&"x".repeat(33)).is_none());
        assert!(StyleName::new(&"x".repeat(32)).is_some());
    }

    #[test]
    fn a_named_style_changes_only_what_it_states() {
        let base = CaptionStyle {
            text_rgba: 0x1122_33FF,
            ..CaptionStyle::default()
        };
        let top = CaptionOverride {
            anchor: Some(CaptionAnchor::TopCenter),
            effects: EffectsOverride {
                glow_strength: Some(0.5),
                ..EffectsOverride::default()
            },
            ..CaptionOverride::default()
        };
        let drawn = top.apply(&base);
        assert_eq!(drawn.anchor, CaptionAnchor::TopCenter);
        assert_eq!(drawn.effects.glow_strength, 0.5);
        // Everything else is the base's.
        assert_eq!(drawn.text_rgba, 0x1122_33FF);
        assert_eq!(drawn.size_scale, base.size_scale);
        assert_eq!(drawn.box_style, base.box_style);
        assert_eq!(drawn.effects.glow_radius, base.effects.glow_radius);
    }

    #[test]
    fn the_lines_own_style_is_written_over_its_sections_over_the_base() {
        let mut base = CaptionStyle::default();
        base.styles = vec![
            NamedCaptionStyle {
                name: name("top"),
                style: CaptionOverride {
                    anchor: Some(CaptionAnchor::TopCenter),
                    size_scale: Some(0.05),
                    ..CaptionOverride::default()
                },
            },
            NamedCaptionStyle {
                name: name("big"),
                style: CaptionOverride {
                    size_scale: Some(0.09),
                    ..CaptionOverride::default()
                },
            },
        ];
        let (drawn, used) = resolve_line(&base, Some(name("big")), Some(name("top")));
        assert_eq!(used.as_deref(), Some("top+big"));
        assert_eq!(drawn.size_scale, 0.09, "the line's member wins");
        assert_eq!(
            drawn.anchor,
            CaptionAnchor::TopCenter,
            "a member the line does not state is its section's"
        );
        assert_eq!(
            drawn.box_style, base.box_style,
            "and one neither states the base's"
        );
        let (drawn, used) = resolve_line(&base, None, Some(name("top")));
        assert_eq!(used.as_deref(), Some("top"));
        assert_eq!(drawn.anchor, CaptionAnchor::TopCenter);
        let (drawn, used) = resolve_line(&base, Some(name("big")), None);
        assert_eq!(used.as_deref(), Some("big"));
        assert_eq!(drawn.anchor, CaptionAnchor::BottomCenter);
        let (drawn, used) = resolve_line(&base, None, None);
        assert_eq!(used, None);
        assert!(matches!(drawn, std::borrow::Cow::Borrowed(_)));
        // An unknown name is skipped, never drawn as nothing.
        let (drawn, used) = resolve_line(&base, Some(name("gone")), Some(name("top")));
        assert_eq!(used.as_deref(), Some("top"));
        assert_eq!(drawn.anchor, CaptionAnchor::TopCenter);
        assert!(
            drawn.styles.is_empty(),
            "a style to draw with carries no table"
        );
    }

    #[test]
    fn the_table_is_bounded_unique_and_never_names_the_imported_face() {
        let entry = |text: &str| NamedCaptionStyle {
            name: name(text),
            style: CaptionOverride::default(),
        };
        assert!(table_is_valid(&[entry("a"), entry("b")]));
        assert!(!table_is_valid(&[entry("a"), entry("a")]));
        let too_many: Vec<_> = (0..=CAPTION_STYLE_CAPACITY)
            .map(|index| entry(&format!("s{index}")))
            .collect();
        assert!(!table_is_valid(&too_many));
        let imported = NamedCaptionStyle {
            name: name("x"),
            style: CaptionOverride {
                face: Some(CaptionFace::Imported),
                ..CaptionOverride::default()
            },
        };
        assert!(!table_is_valid(&[imported]));
        let oversize = NamedCaptionStyle {
            name: name("x"),
            style: CaptionOverride {
                size_scale: Some(caption::SIZE_MAXIMUM + 0.01),
                ..CaptionOverride::default()
            },
        };
        assert!(!table_is_valid(&[oversize]));
    }

    fn card(start: f64, end: f64) -> TitleCard {
        TitleCard {
            start_seconds: start,
            end_seconds: end,
            title: "Door, No Room".into(),
            artist: None,
            handle: Some("@chaoscatfamiliar".into()),
            style: None,
        }
    }

    #[test]
    fn titles_are_ordered_never_overlap_and_name_known_styles() {
        assert!(titles_are_valid(
            &[card(0.0, 5.0), card(5.0, 8.0)],
            10.0,
            &[]
        ));
        assert!(!titles_are_valid(
            &[card(0.0, 5.0), card(4.0, 8.0)],
            10.0,
            &[]
        ));
        assert!(!titles_are_valid(&[card(0.0, 11.0)], 10.0, &[]));
        let mut styled = card(0.0, 5.0);
        styled.style = Some(name("card"));
        assert!(!titles_are_valid(std::slice::from_ref(&styled), 10.0, &[]));
        assert!(titles_are_valid(
            &[styled],
            10.0,
            &[NamedCaptionStyle {
                name: name("card"),
                style: CaptionOverride::default()
            }]
        ));
        assert!(title_at(&[card(1.0, 5.0)], 0.5).is_none());
        assert!(title_at(&[card(1.0, 5.0)], 1.0).is_some());
        assert!(title_at(&[card(1.0, 5.0)], 5.0).is_none());
    }

    #[test]
    fn a_handle_starts_with_one_at_and_has_no_spaces() {
        assert!(handle_is_valid("@chaoscatfamiliar"));
        assert!(!handle_is_valid("chaoscatfamiliar"));
        assert!(!handle_is_valid("@"));
        assert!(!handle_is_valid("@chaos cat"));
        assert!(!handle_is_valid("@a@b"));
    }

    #[test]
    fn the_artist_line_falls_back_to_the_project_and_says_when_there_is_none() {
        let mut entry = card(0.0, 5.0);
        assert_eq!(
            artist_line(&entry, Some("Chaos Cat Familiar")).as_deref(),
            Some("Chaos Cat Familiar  ·  @chaoscatfamiliar")
        );
        entry.artist = Some("Someone Else".into());
        assert_eq!(
            artist_line(&entry, Some("Chaos Cat Familiar")).as_deref(),
            Some("Someone Else  ·  @chaoscatfamiliar")
        );
        entry.artist = None;
        entry.handle = None;
        assert_eq!(artist_line(&entry, Some("")), None);
        assert_eq!(artist_line(&entry, None), None);
    }

    #[test]
    fn a_title_look_is_the_projects_type_in_the_cards_layout() {
        let base = CaptionStyle {
            face: CaptionFace::SpaceGrotesk,
            text_rgba: 0xEEDD_CCFF,
            anchor: CaptionAnchor::TopRight,
            ..CaptionStyle::default()
        };
        let look = title_look(&base, None);
        assert_eq!(look.face, CaptionFace::SpaceGrotesk);
        assert_eq!(look.text_rgba, 0xEEDD_CCFF);
        assert_eq!(look.anchor, CaptionAnchor::MiddleCenter);
        assert!(look.size_scale > 2.0 * base.size_scale);
        assert!(look.effects.entrance_seconds > 0.0 && look.effects.exit_seconds > 0.0);
        let moved = title_look(
            &base,
            Some(&CaptionOverride {
                anchor: Some(CaptionAnchor::BottomLeft),
                ..CaptionOverride::default()
            }),
        );
        assert_eq!(moved.anchor, CaptionAnchor::BottomLeft);
        assert_eq!(moved.size_scale, look.size_scale);
    }
}
