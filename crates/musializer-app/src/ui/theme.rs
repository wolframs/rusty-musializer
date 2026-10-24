//! The workspace palette, type scale and metrics: the one source of truth for
//! how the chrome looks, read by the raylib widgets and by egui alike (WX-5a).
//!
//! Two appearances run on **one set of tokens**. [`Palette`] names every role
//! the chrome draws with, and [`palette::DARK`] and [`palette::LIGHT`] fill it
//! in; a widget never asks "which theme is this?", it asks for a role. The rule
//! the C kept (`ui_palette.h` is raylib-free packed `0xRRGGBBAA`, so a test can
//! check the exact numbers the application draws with) holds here too: **do not
//! draw chrome with a colour that is not a field of [`Palette`] or a constant in
//! [`rgba`]**, or it becomes invisible to the contrast sweep in this module's
//! tests.
//!
//! Three rules came with the mockup the operator approved on 2026-10-21
//! (`docs/WORKSTATION.md`, WX-0), and every token below answers to them:
//!
//! - **One accent, and it means *now*.** The playhead, the section or line under
//!   it, and the primary action (Render). Before WX-5a the same amber marked the
//!   track, the selected scene, Auto, the waveform, every slider, every panel
//!   heading and every focus ring, so nothing stood out. [`Palette::accent`] has
//!   no other job; there are separate roles for everything it used to do.
//! - **Selection is an outline and a tint, never a slab.** [`Palette::ui_selected`]
//!   is a surface one step up, [`Palette::ui_selected_edge`] the outline that
//!   makes it a state rather than a hover. Neither is the accent.
//! - **A filled amount is warm grey.** Slider fills, meters, the played part of
//!   the waveform and the scrub bar are [`Palette::ui_level`]: the mockup's
//!   "played" tone, so "how much" reads as one thing everywhere.

/// Workstation appearance; never part of an exported scene or music project.
///
/// The serialized names are the ones every `ui.json` this application has
/// written carries, so they stay. What they *mean* moved with WX-5a: `black-amber`
/// is the dark mockup look and the default, `light-blue` the light appearance on
/// the same tokens. Both names still describe their looks — the dark accent is
/// amber and the light one blue — which is why they were kept rather than
/// renamed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiTheme {
    LightBlue,
    /// The default since WX-5a (operator, 2026-10-24: "the dark mockup look
    /// becomes the default; the light appearance is kept").
    #[default]
    BlackAmber,
}
impl UiTheme {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LightBlue => "Light",
            Self::BlackAmber => "Dark",
        }
    }
    /// The persisted names, plus `dark` and `light` as plain aliases for
    /// `--ui-theme`. The aliases are additive: every name the flag accepted
    /// before still means what it meant.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "light-blue" | "light" => Some(Self::LightBlue),
            "black-amber" | "dark" => Some(Self::BlackAmber),
            _ => None,
        }
    }
    #[must_use]
    pub const fn palette(self) -> &'static Palette {
        match self {
            Self::LightBlue => &palette::LIGHT,
            Self::BlackAmber => &palette::DARK,
        }
    }
}

// Widgets call parameterless palette functions. The UI thread's appearance is a
// thread-local so independent headless UI tests cannot change each other's look;
// model and renderer state never read it.
thread_local! { static CURRENT_THEME: std::cell::Cell<UiTheme> = const { std::cell::Cell::new(UiTheme::BlackAmber) }; }
pub fn set_current(theme: UiTheme) {
    CURRENT_THEME.set(theme);
}
pub fn current() -> UiTheme {
    CURRENT_THEME.get()
}
/// The palette the UI thread is drawing with.
#[must_use]
pub fn palette() -> &'static Palette {
    current().palette()
}

/// Every role the chrome draws with, as packed `0xRRGGBBAA`.
///
/// A struct rather than a `match` on role names, which is what this replaced:
/// a role missing from one appearance was a silent fallback to the other's
/// value, and the dark appearance's egui visuals had drifted from its raylib
/// chrome because nothing made them read the same numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// *Now*: the playhead, the current section and line, the primary action.
    pub accent: u32,
    /// Text and icons on an [`Self::accent`] fill.
    pub on_accent: u32,
    /// Panels: the tracks sidebar, the timeline band, the inspector, dialogs.
    pub ui_surface: u32,
    /// One step up from the surface: list rows, lane fills, label plates.
    pub ui_raised: u32,
    /// A button at rest.
    pub ui_control: u32,
    /// A button's outline. Quiet on purpose — the mockup's buttons are told
    /// apart by their fill and their label, and a loud box on every control is
    /// what made the old dark appearance read as a form.
    pub ui_control_edge: u32,
    /// The surface under the pointer.
    pub ui_hover: u32,
    /// A selected item's tint.
    pub ui_selected: u32,
    /// A selected item's outline, a focused field's ring and the active
    /// splitter. Clears the 3:1 non-text bar against the surfaces, so a
    /// selection is perceivable without the accent.
    pub ui_selected_edge: u32,
    pub ui_tooltip_surface: u32,
    pub ui_tooltip_ink: u32,
    pub ui_tooltip_edge: u32,
    pub ui_ink: u32,
    pub ui_muted: u32,
    /// Disabled labels, which WCAG 1.4.3 exempts.
    pub ui_disabled: u32,
    /// Structural dividers and lane outlines.
    pub ui_rule: u32,
    pub ui_danger: u32,
    pub ui_warning: u32,
    pub ui_success: u32,
    /// The groove the timed lanes sit in (LX1-e).
    ///
    /// The timeline band is several lanes over one time axis, and the unifier is
    /// this one tone painted into every gap *between* lanes, bounded by the group
    /// frame, so the lanes read as rows of one table rather than as boxes that
    /// happen to be stacked. Never a colour text is drawn on, and always an
    /// **exact** fill distinct from [`Self::ui_surface`] and [`Self::ui_raised`]:
    /// the gate's seam detector matches it with tolerance zero (AGENTS.md, "The
    /// headless gate runs on the GPU").
    pub ui_lane_trough: u32,
    /// A filled amount: slider and meter fills, the scrub bar, the played part
    /// of the waveform. A *warm* grey, and the waveform ahead of the playhead
    /// a *cool* one, in both appearances: the mockup's "grey, warming where it
    /// has played", and the reason `tools/timeline_tick_plate.py` can tell an
    /// envelope pixel from a neutral glyph or rule — the tint is the classifier.
    pub ui_level: u32,
    /// The waveform ahead of the playhead.
    pub ui_waveform: u32,
    /// Selected text inside a text field.
    pub ui_text_highlight: u32,
    /// A lyric cue block's fill in the cue lane (WX-5b, the mockup's `.ly`):
    /// neutral, so the words carry the block and the origin colour is a stripe
    /// rather than a slab.
    pub ui_cue_block: u32,
    /// The cue block under the playhead: a warm tint, not the accent (WX-5b,
    /// the mockup's `.ly.now`). The accent is the playhead crossing it.
    pub ui_cue_now: u32,
    /// The hairline at a cut *inside* a plan-lane run (WX-5b). Opaque, so the
    /// gate can count it by exact colour (AGENTS.md: opaque fills are the one
    /// thing every renderer writes bit-exactly), and a colour nothing else in
    /// the lane uses.
    pub ui_run_cut: u32,
}

/// The two appearances.
pub mod palette {
    use super::Palette;

    /// The mockup's dark workstation (`tools/wx_mockup.py`'s `:root` block:
    /// `--s1 #111317`, `--s2 #171a1f`, `--s3 #1e2228`, `--line #252a32`,
    /// `--text #e8e5dd`, `--acc #f0b347`). Two values are lifted from the
    /// mockup rather than copied: muted (`#8d919a` there) clears 4.5:1 on the
    /// selected tint only at `#9296a0`, and the mockup's `--faint #5b606a`
    /// reads 2.9:1, so it serves only as a disabled label at `#636873`.
    #[rustfmt::skip]
    pub const DARK: Palette = Palette {
        accent:             0xF0B3_47FF,
        on_accent:          0x1A12_05FF,
        ui_surface:         0x1113_17FF,
        ui_raised:          0x171A_1FFF,
        ui_control:         0x171A_1FFF,
        ui_control_edge:    0x2E33_3CFF,
        ui_hover:           0x1D21_27FF,
        ui_selected:        0x2226_2DFF,
        ui_selected_edge:   0x747A_85FF,
        ui_tooltip_surface: 0x262A_31FF,
        ui_tooltip_ink:     0xE8E5_DDFF,
        ui_tooltip_edge:    0x3A40_4AFF,
        ui_ink:             0xE8E5_DDFF,
        ui_muted:           0x9296_A0FF,
        ui_disabled:        0x6368_73FF,
        ui_rule:            0x252A_32FF,
        ui_danger:          0xF270_7FFF,
        ui_warning:         0xEB85_5EFF,
        ui_success:         0x7DB3_8CFF,
        ui_lane_trough:     0x0A0B_0DFF,
        ui_level:           0x9A98_90FF,
        ui_waveform:        0x3D42_4BFF,
        ui_text_highlight:  0x3442_58FF,
        ui_cue_block:       0x2024_2AFF,
        ui_cue_now:         0x2B26_18FF,
        ui_run_cut:         0x5A5E_66FF,
    };

    /// The light appearance: the pre-WX-5a light/blue palette moved onto the
    /// same roles. Hover and selection lost their blue cast — a blue tint on a
    /// selected row was the accent in disguise — and the warning and success
    /// greens were darkened a step so they clear 4.5:1 on the selected tint too.
    #[rustfmt::skip]
    pub const LIGHT: Palette = Palette {
        accent:             0x002F_A7FF,
        on_accent:          0xFFFF_FFFF,
        ui_surface:         0xF7F7_F8FF,
        ui_raised:          0xFFFF_FFFF,
        ui_control:         0xFFFF_FFFF,
        ui_control_edge:    0xD2D2_D6FF,
        ui_hover:           0xEFEF_F2FF,
        ui_selected:        0xE8E8_EDFF,
        ui_selected_edge:   0x7575_7DFF,
        ui_tooltip_surface: 0x1414_14FF,
        ui_tooltip_ink:     0xFFFF_FFFF,
        ui_tooltip_edge:    0x3E3E_3EFF,
        ui_ink:             0x1414_14FF,
        ui_muted:           0x6262_67FF,
        ui_disabled:        0x8C8C_92FF,
        ui_rule:            0xD2D2_D6FF,
        ui_danger:          0xC628_28FF,
        ui_warning:         0x9758_00FF,
        ui_success:         0x166E_47FF,
        ui_lane_trough:     0xE6E6_EAFF,
        ui_level:           0x8A7F_6EFF,
        ui_waveform:        0xBDC1_CBFF,
        ui_text_highlight:  0xD6DE_F5FF,
        ui_cue_block:       0xEEEE_F2FF,
        ui_cue_now:         0xF6EC_D8FF,
        ui_run_cut:         0xFBFA_F7FF,
    };
}

/// What a palette entry is drawn *on*, so the contrast sweep walks every pair
/// rather than the pairs somebody remembered (UX0-D04).
///
/// Declaring the surface on the entry means a colour cannot be added to a
/// palette's [`Palette::entries`] without answering "on what?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// Text on the chrome: panels, rows, controls at rest, under the pointer
    /// and selected. All five, because a label does not change colour when its
    /// row is hovered or selected.
    Chrome,
    /// Text on the accent fill of the primary action.
    OnAccent,
    /// Text on a tooltip.
    Tooltip,
    /// Text on the notice tray's card, over the preview.
    DarkOverlay,
    /// A non-text component that has to be perceivable (WCAG 1.4.11, 3:1)
    /// against the panels: the selection outline, the accent's playhead.
    Graphic,
    /// Not text: a fill, a rule, or a background other colours are measured
    /// against. No contrast requirement of its own.
    Fill,
    /// Text that WCAG 1.4.3 exempts. Only disabled controls qualify, and the
    /// exemption is the reason a disabled label is allowed to be quiet.
    ExemptText,
}

impl Palette {
    /// Every role and the surface it sits on.
    ///
    /// Read only by the tests below. That is the point of it: a list the suite
    /// walks is how a colour added without a contrast check gets caught.
    #[allow(
        dead_code,
        reason = "the palette audit in this module's tests is its only reader"
    )]
    pub const fn entries(&self) -> [(&'static str, u32, Surface); 26] {
        [
            ("accent", self.accent, Surface::Chrome),
            ("on_accent", self.on_accent, Surface::OnAccent),
            ("ui_surface", self.ui_surface, Surface::Fill),
            ("ui_raised", self.ui_raised, Surface::Fill),
            ("ui_control", self.ui_control, Surface::Fill),
            ("ui_control_edge", self.ui_control_edge, Surface::Fill),
            ("ui_hover", self.ui_hover, Surface::Fill),
            ("ui_selected", self.ui_selected, Surface::Fill),
            ("ui_selected_edge", self.ui_selected_edge, Surface::Graphic),
            ("ui_tooltip_surface", self.ui_tooltip_surface, Surface::Fill),
            ("ui_tooltip_ink", self.ui_tooltip_ink, Surface::Tooltip),
            ("ui_tooltip_edge", self.ui_tooltip_edge, Surface::Fill),
            ("ui_ink", self.ui_ink, Surface::Chrome),
            ("ui_muted", self.ui_muted, Surface::Chrome),
            ("ui_disabled", self.ui_disabled, Surface::ExemptText),
            ("ui_rule", self.ui_rule, Surface::Fill),
            ("ui_danger", self.ui_danger, Surface::Chrome),
            ("ui_warning", self.ui_warning, Surface::Chrome),
            ("ui_success", self.ui_success, Surface::Chrome),
            ("ui_lane_trough", self.ui_lane_trough, Surface::Fill),
            ("ui_level", self.ui_level, Surface::Graphic),
            ("ui_waveform", self.ui_waveform, Surface::Fill),
            ("ui_text_highlight", self.ui_text_highlight, Surface::Fill),
            ("ui_cue_block", self.ui_cue_block, Surface::Fill),
            ("ui_cue_now", self.ui_cue_now, Surface::Fill),
            ("ui_run_cut", self.ui_run_cut, Surface::Fill),
        ]
    }

    /// The backgrounds an entry of this kind must clear its bar against, and
    /// the bar.
    #[allow(
        dead_code,
        reason = "the palette audit in this module's tests is its only reader"
    )]
    pub fn backgrounds(&self, surface: Surface) -> (Vec<(&'static str, u32)>, f64) {
        use musializer_core::ui::contrast::{AA_COMPONENT, AA_TEXT};
        match surface {
            Surface::Chrome => (
                vec![
                    ("ui_surface", self.ui_surface),
                    ("ui_raised", self.ui_raised),
                    ("ui_control", self.ui_control),
                    ("ui_hover", self.ui_hover),
                    ("ui_selected", self.ui_selected),
                ],
                AA_TEXT,
            ),
            Surface::OnAccent => (vec![("accent", self.accent)], AA_TEXT),
            Surface::Tooltip => (
                vec![("ui_tooltip_surface", self.ui_tooltip_surface)],
                AA_TEXT,
            ),
            Surface::DarkOverlay => (
                vec![("ui_overlay_surface", rgba::UI_OVERLAY_SURFACE)],
                AA_TEXT,
            ),
            Surface::Graphic => (
                vec![
                    ("ui_surface", self.ui_surface),
                    ("ui_raised", self.ui_raised),
                ],
                AA_COMPONENT,
            ),
            Surface::Fill | Surface::ExemptText => (Vec::new(), 0.0),
        }
    }
}

/// Values that do not change with the appearance: the scene's own clear, the
/// frame around the preview, and the notice tray, which floats over video in
/// both appearances.
pub mod rgba {
    pub const BACKGROUND: u32 = 0x1515_15FF;
    pub const WHITE: u32 = 0xFFFF_FFFF;

    /// The margin around a preview framed to a non-panel-shaped export (EX2).
    ///
    /// Distinctly darker than [`BACKGROUND`], which is the scene's own clear, so
    /// the letter- or pillar-box reads as "your video ends here" rather than as
    /// a scene that failed to fill its panel. Not black either: at a tall aspect
    /// the surround is most of the panel, and a pure-black field beside a
    /// near-black scene would make the frame edge invisible, which is the one
    /// thing this colour exists to draw.
    pub const PREVIEW_SURROUND: u32 = 0x0A0A_0BFF;

    /// The one-pixel rule that says where the exported frame ends (EX2).
    ///
    /// The surround alone is not enough and a capture proved it: Pentagram
    /// Orbits pillarboxed at 9:16 is near-black scene against near-black
    /// surround, the seam is invisible, and the picture then says nothing about
    /// the framing it exists to show. Bright enough to read against both, dim
    /// enough not to compete with the scene.
    pub const PREVIEW_FRAME_EDGE: u32 = 0x4A4A_52FF;

    /// The notice tray's card, floating over the scene preview.
    ///
    /// **Opaque, and that is the fix rather than an aesthetic choice**
    /// (review 1.11, UX0-A11). It was `rgba(20, 22, 28, 232)`, so the colour the
    /// user actually read was the card composited over whatever the scene was
    /// drawing that frame — which means no contrast ratio measured against it was
    /// true, and the sweep in this module could not have measured it in the first
    /// place: the opacity assertion exists precisely because
    /// [`musializer_core::ui::contrast`] treats every colour as fully composited.
    pub const UI_OVERLAY_SURFACE: u32 = 0x1416_1CFF;
    /// A notice's title, on [`UI_OVERLAY_SURFACE`]. raylib's `RAYWHITE`, promoted
    /// into the palette so the sweep can see it.
    pub const UI_OVERLAY_INK: u32 = 0xF5F5_F5FF;
    /// A notice's detail text: the dark card's equivalent of a muted label.
    pub const UI_OVERLAY_MUTED: u32 = 0xB4BE_CDFF;

    /// The four severity labels on the notice card (review 1.11, UX0-A11).
    ///
    /// The tray used to draw the *light chrome* colours here, which are chosen
    /// to sit on a near-white panel. On the near-black card the old blue accent
    /// measured **1.69:1** and the light danger **3.19:1**, both below the 4.5:1
    /// body-text minimum, so an error and an info were indistinguishable at a
    /// glance. These are the dark-surface variants: same hues, lifted until they
    /// clear AA against the card.
    /// `the_light_chrome_severities_would_fail_the_dark_sweep` keeps the old
    /// numbers as a negative control.
    ///
    /// Since WX-5a the tray draws an INFO notice in [`UI_OVERLAY_MUTED`]
    /// instead: in the light appearance this blue sits six degrees of hue from
    /// the accent. The protocol card and its pennants still use it.
    pub const NOTICE_INFO_ON_DARK: u32 = 0x8AB4_F8FF;
    pub const NOTICE_SUCCESS_ON_DARK: u32 = 0x6FDB_A0FF;
    /// Orange rather than the amber it was (`0xF2B441`) since WX-5a: that amber
    /// sat two degrees of hue from the dark accent, which means *now*.
    pub const NOTICE_WARNING_ON_DARK: u32 = 0xF392_6BFF;
    pub const NOTICE_ERROR_ON_DARK: u32 = 0xFF6B_6BFF;

    /// The dim laid over the workspace behind a modal dialog. Translucent by
    /// design, so it is not a palette entry: nothing is ever measured against it.
    pub const SCRIM: u32 = 0x0A0B_0EBE;

    /// The theme-independent entries, for the sweep.
    #[allow(
        dead_code,
        reason = "the palette audit in this module's tests is its only reader"
    )]
    pub const ALL: [(&str, u32, super::Surface); 9] = [
        ("background", BACKGROUND, super::Surface::Fill),
        ("white", WHITE, super::Surface::Fill),
        (
            "ui_overlay_surface",
            UI_OVERLAY_SURFACE,
            super::Surface::Fill,
        ),
        (
            "ui_overlay_ink",
            UI_OVERLAY_INK,
            super::Surface::DarkOverlay,
        ),
        (
            "ui_overlay_muted",
            UI_OVERLAY_MUTED,
            super::Surface::DarkOverlay,
        ),
        (
            "notice_info_on_dark",
            NOTICE_INFO_ON_DARK,
            super::Surface::DarkOverlay,
        ),
        (
            "notice_success_on_dark",
            NOTICE_SUCCESS_ON_DARK,
            super::Surface::DarkOverlay,
        ),
        (
            "notice_warning_on_dark",
            NOTICE_WARNING_ON_DARK,
            super::Surface::DarkOverlay,
        ),
        (
            "notice_error_on_dark",
            NOTICE_ERROR_ON_DARK,
            super::Surface::DarkOverlay,
        ),
    ];
}

/// The palette as raylib colours (`../musializer/src/ui_theme.h:17-38`).
pub mod color {
    use raylib::prelude::Color;

    macro_rules! themed {
        ($($name:ident;)*) => {
            $(
                #[must_use]
                pub fn $name() -> Color {
                    Color::get_color(super::palette().$name)
                }
            )*
        };
    }
    macro_rules! fixed {
        ($($name:ident = $source:ident;)*) => {
            $(
                #[must_use]
                pub fn $name() -> Color {
                    Color::get_color(super::rgba::$source)
                }
            )*
        };
    }

    themed! {
        accent;
        on_accent;
        ui_surface;
        ui_raised;
        ui_control;
        ui_control_edge;
        ui_hover;
        ui_selected;
        ui_selected_edge;
        ui_tooltip_surface;
        ui_tooltip_ink;
        ui_tooltip_edge;
        ui_ink;
        ui_muted;
        ui_disabled;
        ui_rule;
        ui_danger;
        ui_warning;
        ui_success;
        ui_lane_trough;
        ui_level;
        ui_waveform;
        ui_text_highlight;
        ui_cue_block;
        ui_cue_now;
        ui_run_cut;
    }

    fixed! {
        background = BACKGROUND;
        white = WHITE;
        preview_surround = PREVIEW_SURROUND;
        preview_frame_edge = PREVIEW_FRAME_EDGE;
        ui_overlay_surface = UI_OVERLAY_SURFACE;
        ui_overlay_ink = UI_OVERLAY_INK;
        ui_overlay_muted = UI_OVERLAY_MUTED;
        notice_info_on_dark = NOTICE_INFO_ON_DARK;
        notice_success_on_dark = NOTICE_SUCCESS_ON_DARK;
        notice_warning_on_dark = NOTICE_WARNING_ON_DARK;
        notice_error_on_dark = NOTICE_ERROR_ON_DARK;
        scrim = SCRIM;
    }
}

/// The same palette for egui, so the dock, the Editor and the welcome picker
/// draw with the numbers the raylib chrome draws with.
pub mod color32 {
    use egui::Color32;

    /// Unpacks `0xRRGGBBAA`.
    #[must_use]
    pub fn of(value: u32) -> Color32 {
        let [r, g, b, a] = value.to_be_bytes();
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }

    macro_rules! themed {
        ($($name:ident;)*) => {
            $(
                #[must_use]
                pub fn $name() -> Color32 {
                    of(super::palette().$name)
                }
            )*
        };
    }

    themed! {
        ui_ink;
        ui_danger;
        ui_warning;
        ui_success;
    }
}

/// HUD and control metrics (`../musializer/src/ui_theme.h:40-58`), and the
/// **type scale** both toolkits draw with (WX-5a).
///
/// One scale, in logical pixels — which is what egui calls points, since the
/// egui pass runs at the UI scale's pixels-per-point — so a label in the dock
/// and a label in the raylib chrome come out the same size in the same face.
/// Before WX-5a the raylib chrome used 19/16/13/15 and egui 20/15/15/12, and
/// the dark appearance drew all of egui in monospace.
pub mod metric {
    /// Toolbar height: the C's `toolbar_height = HUD_BUTTON_SIZE`
    /// (`plug.c:7623`).
    pub const HUD_BUTTON_SIZE: f32 = 50.0;
    /// Panel headings. Was 19: an uppercase heading that size in the accent was
    /// the loudest thing in every panel.
    pub const UI_FONT_HEADER: f32 = 16.0;
    /// Body text and labels. Was 16; the mockup's body is 13 at 1600 px.
    pub const UI_FONT_LABEL: f32 = 14.0;
    /// Secondary text. Unchanged at 13 because the timeline band reserves it
    /// (`panels::events::EVENT_ROW_HEIGHT`), and moving the band moves every
    /// lane the gate measures for a one-pixel difference nobody would see.
    pub const UI_FONT_CAPTION: f32 = 13.0;
    /// Tabular readouts: the timecode, a tuning value.
    pub const UI_FONT_VALUE: f32 = 14.0;
    /// Lane and section heads, drawn uppercase and tracked, as in the mockup's
    /// `PLAN · 70 SECTIONS IN 21 RUNS`.
    pub const UI_FONT_MICRO: f32 = 11.0;
    /// The ceiling for a fitted button row. `row_typography::base_font_size`
    /// lets a tall box grow its label to 22, which is how a scene tile came to
    /// say "Phosphor Dream" at 27 physical pixels; chrome text is never larger
    /// than a heading.
    pub const UI_FONT_ROW_MAX: f32 = UI_FONT_HEADER;
    /// A track row's name, which is the user's own words and so is drawn in the
    /// authored face (Alegreya, review 1.5) rather than the interface face.
    /// Alegreya is a small-eyed serif: at [`UI_FONT_LABEL`] its capitals stood
    /// 9 px tall in a 1920x1080 capture, beside action buttons whose 16 px
    /// Space Grotesk stands 11, and the one thing in the sidebar that names the
    /// work read as its smallest print. This is the optical match for the
    /// buttons above it, not a larger size.
    ///
    /// 16 since WX-5b (it was 18): `AuthoredText` now applies Alegreya's own
    /// optical size (1.154, measured from its tables) to every authored draw, so
    /// 18 corrected twice and drew the name 11 % large. 16 x 1.154 lands its
    /// capitals at 8.6 logical px beside the buttons' 8.8.
    pub const UI_FONT_AUTHORED_ROW: f32 = 16.0;
    /// The corner radius of a control and a dialog (WX-5a), as in the mockup.
    pub const UI_RADIUS: f32 = 4.0;
    pub const UI_PANEL_PADDING: f32 = 10.0;
    pub const UI_CONTROL_GAP: f32 = 8.0;
    pub const UI_BUTTON_HEIGHT: f32 = 36.0;

    // The timed-lane system (LX1-e). The timeline band stacks the scene plan
    // lane, the waveform strip and the lyric cue lane over one time axis, and
    // each was styled by a different agent at a different time: the scene lane
    // sat 4 px under its controls row, the waveform shared an edge with it (so
    // two 1 px rules drew as one 2 px line), and the cue lane sat 5 px lower
    // with a top rule and no box. Three names, used by all three lanes, so the
    // next lane cannot invent a fourth set of numbers.
    //
    // `LANE_GAP` is 5 rather than 6 because `panels::lyrics` already spends
    // exactly 5 between the waveform and the cue lane, and its
    // `LYRIC_EDITOR_TIMELINE_CHROME` assertion is what forbids that band from
    // growing — `shell.rs` keeps a `const _: () = assert!` pinning the two
    // together.

    /// Every timed lane's outline, and the group frame around all of them.
    pub const LANE_BORDER: f32 = 1.0;
    /// The vertical gap between two adjacent timed lanes, and between the
    /// scene-plan controls row and the first lane.
    pub const LANE_GAP: f32 = 5.0;
    /// The playhead's stroke. One marker crosses the whole group, so this is
    /// the only width any timed lane's playhead is drawn at.
    pub const LANE_PLAYHEAD_WIDTH: f32 = 2.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::ui::contrast;

    const BOTH: [(&str, &Palette); 2] = [("dark", &palette::DARK), ("light", &palette::LIGHT)];

    #[test]
    fn the_default_appearance_is_the_dark_mockup_and_old_names_still_load() {
        assert_eq!(UiTheme::default(), UiTheme::BlackAmber);
        assert_eq!(UiTheme::default().palette(), &palette::DARK);
        for (name, theme) in [
            ("black-amber", UiTheme::BlackAmber),
            ("light-blue", UiTheme::LightBlue),
            ("dark", UiTheme::BlackAmber),
            ("light", UiTheme::LightBlue),
        ] {
            assert_eq!(UiTheme::parse(name), Some(theme), "{name}");
        }
        // What `ui.json` carries is unchanged, so a file written by any earlier
        // build means what it meant.
        assert_eq!(
            serde_json::to_string(&UiTheme::BlackAmber).unwrap(),
            "\"black-amber\""
        );
        assert_eq!(
            serde_json::from_str::<UiTheme>("\"light-blue\"").unwrap(),
            UiTheme::LightBlue
        );
    }

    #[test]
    fn every_palette_entry_is_opaque() {
        // The ratio functions ignore alpha because they describe fully
        // composited colours; a translucent palette entry would silently be
        // measured as if it were opaque.
        for (appearance, palette) in BOTH {
            for (name, value, _) in palette.entries() {
                assert_eq!(value & 0xFF, 0xFF, "{appearance} {name} is not opaque");
            }
        }
        for (name, value, _) in rgba::ALL {
            assert_eq!(value & 0xFF, 0xFF, "{name} is not opaque");
        }
    }

    /// The sweep UX0-D04 asks for: every entry against every surface it
    /// declares, in both appearances.
    ///
    /// It walks entries nobody thought to name, which is how the notice tray's
    /// severity labels went unmeasured while INFO sat at 1.69:1 on the card.
    #[test]
    fn every_entry_clears_its_bar_on_the_surfaces_it_declares() {
        let mut measured = 0;
        for (appearance, palette) in BOTH {
            let entries = palette.entries();
            for (name, value, surface) in entries.iter().copied().chain(rgba::ALL) {
                let (backgrounds, bar) = palette.backgrounds(surface);
                for (background_name, background) in backgrounds {
                    let ratio = contrast::ratio(value, background);
                    assert!(
                        ratio >= bar,
                        "{appearance}: {name} on {background_name} reads at {ratio:.2}:1, below {bar:.1}:1"
                    );
                    measured += 1;
                }
            }
        }
        // A sweep that measured nothing would pass silently, which is the failure
        // mode of every table-driven test that lost its table.
        assert!(measured >= 80, "the sweep only checked {measured} pairs");
    }

    /// The accent means *now*, so it must be the one colour nothing else in the
    /// palette can be mistaken for: no other role may equal it, and the roles
    /// that took over its old jobs (selection, level, warning) must not be it in
    /// disguise.
    #[test]
    fn the_accent_is_worn_by_nothing_else() {
        for (appearance, palette) in BOTH {
            for (name, value, _) in palette.entries() {
                if name == "accent" {
                    continue;
                }
                assert_ne!(value, palette.accent, "{appearance}: {name} is the accent");
            }
            // The notice card's severities are coloured text over the preview
            // in both appearances, so none of them may read as the accent.
            for (name, value, surface) in rgba::ALL {
                if surface != Surface::DarkOverlay {
                    continue;
                }
                assert!(
                    reads_apart(value, palette.accent),
                    "{appearance}: {name} reads as the accent"
                );
            }
            for (name, value) in [
                ("ui_selected_edge", palette.ui_selected_edge),
                ("ui_selected", palette.ui_selected),
                ("ui_level", palette.ui_level),
                ("ui_warning", palette.ui_warning),
            ] {
                // Distinct enough to be told apart side by side, not merely
                // unequal: the old dark warning *was* the accent.
                assert!(
                    reads_apart(value, palette.accent),
                    "{appearance}: {name} reads as the accent"
                );
            }
        }
    }

    /// The negative control for the test above, kept as the measurement it came
    /// from: the pre-WX-5a dark appearance drew warnings in its accent.
    #[test]
    fn the_old_dark_warning_would_fail_the_accent_rule() {
        let (old_accent, old_warning) = (0xFFB8_00FFu32, 0xFFB8_00FFu32);
        assert!(!reads_apart(old_warning, old_accent));
        // And the notice card's old warning amber against today's accent.
        assert!(!reads_apart(0xF2B4_41FF, palette::DARK.accent));
    }

    /// Two colours a glance can tell apart: a clear step of lightness, or a
    /// clear step of hue. Neither alone is required — a pale blue info label is
    /// not the deep blue accent — but one of them is.
    fn reads_apart(a: u32, b: u32) -> bool {
        contrast::ratio(a, b) >= 1.4 || hue_distance(a, b) >= 20.0
    }

    fn hue_distance(a: u32, b: u32) -> f64 {
        let hue = |value: u32| {
            let [r, g, b, _] = value.to_be_bytes();
            let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let delta = max - min;
            // A near-grey has no hue worth comparing: a bluish grey is not "the
            // blue accent" any more than a white is.
            if max == 0.0 || delta / max < 0.2 {
                return None;
            }
            let h = if max == r {
                ((g - b) / delta).rem_euclid(6.0)
            } else if max == g {
                (b - r) / delta + 2.0
            } else {
                (r - g) / delta + 4.0
            };
            Some(h * 60.0)
        };
        match (hue(a), hue(b)) {
            (Some(x), Some(y)) => {
                let d = (x - y).abs();
                d.min(360.0 - d)
            }
            // A grey against a colour is as far apart as hues get.
            _ => 180.0,
        }
    }

    /// Content colours keep their own tables (an event type is not chrome),
    /// but none of them may be mistaken for *now*: the semantic marker was the
    /// C's amber until WX-5a, four degrees of hue from the dark accent.
    #[test]
    fn no_event_type_wears_the_accent() {
        use musializer_core::scene::events::{EventType, UNKNOWN_EVENT_RGBA};
        let kinds = [
            EventType::Lyric.rgba(),
            EventType::Semantic.rgba(),
            EventType::Cue.rgba(),
            EventType::Custom.rgba(),
            UNKNOWN_EVENT_RGBA,
        ];
        for (appearance, palette) in BOTH {
            for kind in kinds {
                assert!(
                    reads_apart(kind, palette.accent),
                    "{appearance}: event colour {kind:08X} reads as the accent"
                );
            }
        }
        // The negative control: the C's semantic amber fails it.
        assert!(!reads_apart(0xF2BE_42FF, palette::DARK.accent));
    }

    #[test]
    fn the_lane_trough_is_an_exact_fill_distinct_from_both_lane_surfaces() {
        // The gate's seam detector matches the trough with tolerance zero, so
        // the only property it needs is inequality — but it needs it in both
        // appearances, or a light capture and a dark one would disagree about
        // where the seams are.
        for (appearance, palette) in BOTH {
            assert_ne!(palette.ui_lane_trough, palette.ui_surface, "{appearance}");
            assert_ne!(palette.ui_lane_trough, palette.ui_raised, "{appearance}");
            assert_ne!(palette.ui_lane_trough, palette.ui_rule, "{appearance}");
        }
    }

    #[test]
    fn hover_and_selection_are_different_states() {
        // A selected row under the pointer and an unselected one must not look
        // the same; the outline carries the rest.
        for (appearance, palette) in BOTH {
            assert_ne!(palette.ui_hover, palette.ui_selected, "{appearance}");
            assert_ne!(palette.ui_hover, palette.ui_control, "{appearance}");
        }
    }

    /// The negative control, kept as the measurement it came from.
    ///
    /// These are the exact colours the tray drew before UX0-A11 — light chrome
    /// colours on a near-black card. If somebody "simplifies" the dark variants
    /// back to the palette they were copied from, this is what says no.
    #[test]
    fn the_light_chrome_severities_would_fail_the_dark_sweep() {
        for (name, value) in [
            ("old light accent (was INFO)", 0x002F_A7FFu32),
            ("ui_danger (was ERROR)", palette::LIGHT.ui_danger),
            ("old ui_success (was DONE)", 0x1879_4EFF),
            ("old ui_warning (was WARNING)", 0x9E5D_00FF),
        ] {
            let ratio = contrast::ratio(value, rgba::UI_OVERLAY_SURFACE);
            assert!(
                ratio < contrast::AA_TEXT,
                "{name} now reads {ratio:.2}:1 on the card; the control is stale"
            );
        }
        // And the replacements clear it, by a margin rather than by a hair.
        for (name, value) in [
            ("info", rgba::NOTICE_INFO_ON_DARK),
            ("success", rgba::NOTICE_SUCCESS_ON_DARK),
            ("warning", rgba::NOTICE_WARNING_ON_DARK),
            ("error", rgba::NOTICE_ERROR_ON_DARK),
        ] {
            let ratio = contrast::ratio(value, rgba::UI_OVERLAY_SURFACE);
            assert!(
                ratio >= 6.0,
                "{name} reads {ratio:.2}:1, too close to the line"
            );
        }
    }

    /// The negative control for the widened chrome sweep. Before WX-5a a chrome
    /// label was measured against the panel and the row only; this grey clears
    /// both and fails on the hover and selected tints, which is exactly what a
    /// label on a selected row is drawn on. The old two-surface sweep passed it.
    #[test]
    fn the_sweep_would_catch_text_that_fails_on_the_selected_tint() {
        let failing = 0x6E6E_72FFu32;
        let light = &palette::LIGHT;
        assert!(contrast::ratio(failing, light.ui_surface) >= contrast::AA_TEXT);
        assert!(contrast::ratio(failing, light.ui_raised) >= contrast::AA_TEXT);
        let (backgrounds, bar) = light.backgrounds(Surface::Chrome);
        let caught: Vec<_> = backgrounds
            .iter()
            .filter(|(_, background)| contrast::ratio(failing, *background) < bar)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(caught, ["ui_hover", "ui_selected"]);
    }
}
