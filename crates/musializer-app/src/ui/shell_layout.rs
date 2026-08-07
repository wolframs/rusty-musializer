//! Where the workspace's regions go.
//!
//! Raylib-free on purpose, and tested here rather than photographed: this is the
//! module where the two rules the C repository already paid for actually bite.
//!
//! 1. **A panel that reserves height it never draws steals it from the scene
//!    preview.** So [`WorkspaceFrame::layout`] takes the timeline height as an
//!    argument rather than deciding it from a guess about which panel is open.
//!    The caller computes it from the panel's own content policy — the same
//!    policy the panel draws with — so the reservation and the drawing cannot
//!    drift.
//! 2. **A panel's minimum size is measured against the panel the minimum
//!    supported window (960x640) actually produces.** The tests below assert
//!    against that window rather than against a threshold somebody guessed.
//!
//! Ported from `scene_settings_ui_layout` (`../musializer/src/scene_settings.c:491-520`)
//! and the frame assembly in `plug.c:7623-7760`.
//!
//! ## Ownership note
//!
//! `scene_settings_ui_layout` lives in `scene_settings.c` in the C, which the
//! ownership map assigns to the shared-contracts bundle — but it is pure
//! workspace geometry with no bearing on the settings *contract*, and nothing
//! outside the shell reads it. It landed app-side rather than being wedged into
//! `core::scene::settings`, which Agents C, D and F all compile against.

use musializer_core::ui::workspace_layout::{TracksPanelMode, UiRect, WorkspaceSidebar};

use super::preferences::recent;
use super::theme::metric;

/// Timeline height with no panel open (`plug.c:7669`).
pub const DEFAULT_TIMELINE_HEIGHT: f32 = 180.0;

/// Timeline height the export panel asks for by default.
///
/// The C uses 330 (`plug.c:7691-7694`), but its band spends nothing on a manual
/// event row or a scene-plan lane — this rewrite's does — so 330 here leaves the
/// panel's boundary 21 px short of its own content and it would draw its
/// too-small notice at the default size (review 1.4, UX0-A04). Derived from the
/// panel's single source of truth so the two can never drift apart again; the
/// value works out to 351.
pub const EXPORT_PANEL_TIMELINE_HEIGHT: f32 = super::panels::export::EXPORT_MIN_BAND_HEIGHT
    - super::panels::events::EVENT_ROW_HEIGHT
    - super::panels::scene_timeline::SCENE_SECTION_HEIGHT;

/// Narrowest window `scene_settings_ui_layout` will lay out
/// (`scene_settings.c:494`). Below this it fails, and the C then keeps the
/// fallback it initialized the struct with (`plug.c:7573-7577`).
pub const MINIMUM_LAYOUT_WIDTH: f32 = 640.0;

/// Horizontal split of the window: the inspector on the right, the workspace to
/// its left, and the tracks rail inside the workspace
/// (`Scene_Settings_Ui_Layout`, `scene_settings.h:82-86`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellWidths {
    /// Zero when the tuning inspector is closed.
    pub inspector_width: f32,
    /// Everything that is not the inspector.
    pub workspace_width: f32,
    /// The tracks/scenes rail on the left of the workspace.
    pub tracks_width: f32,
}

/// User-sized split positions in logical UI units. `None` keeps the existing
/// content-aware automatic policy exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutOverrides {
    pub inspector_width: Option<f32>,
    pub tracks_width: Option<f32>,
}

impl ShellWidths {
    /// The C's fallback when the layout cannot be computed: the whole window is
    /// workspace and the rail is its fixed 320 px (`plug.c:7573-7577`).
    #[must_use]
    pub fn fallback(window_width: f32) -> Self {
        Self {
            inspector_width: 0.0,
            workspace_width: window_width,
            tracks_width: 320.0,
        }
    }

    /// Port of `scene_settings_ui_layout` (`scene_settings.c:491-520`).
    ///
    /// Returns `None` for a non-finite width or one under
    /// [`MINIMUM_LAYOUT_WIDTH`], matching the C's `false`; callers use
    /// [`ShellWidths::fallback`] then, as `plug.c` does.
    #[must_use]
    // The sequential cap-then-floor `if`s are the C's, and they are not
    // interchangeable with `clamp`: `clamp` panics when max < min and propagates
    // NaN, where these saturate. The order also matters here — the 620 px preview
    // rule runs *between* the cap and the 240 px floor, so folding either pair
    // into a `clamp` would change which constraint wins.
    #[allow(clippy::manual_clamp)]
    pub fn layout(window_width: f32, inspector_open: bool) -> Option<Self> {
        if !window_width.is_finite() || window_width < MINIMUM_LAYOUT_WIDTH {
            return None;
        }
        let mut inspector_width = 0.0f32;
        if inspector_open {
            inspector_width = window_width * 0.265;
            if inspector_width < 280.0 {
                inspector_width = 280.0;
            }
            if inspector_width > 340.0 {
                inspector_width = 340.0;
            }
            // The preview keeps 620 px before the inspector gets its share.
            if window_width - inspector_width < 620.0 {
                inspector_width = window_width - 620.0;
            }
            if inspector_width < 240.0 {
                inspector_width = 240.0;
            }
        }
        let workspace_width = window_width - inspector_width;
        let mut tracks_width = if inspector_open {
            workspace_width * 0.25
        } else {
            320.0
        };
        if tracks_width < 240.0 {
            tracks_width = 240.0;
        }
        if tracks_width > 320.0 {
            tracks_width = 320.0;
        }
        // At very narrow widths keep the visualizer as the primary surface while
        // leaving a compact — rather than vanished — project rail
        // (`scene_settings.c:509-514`).
        let minimum_preview = window_width * 0.30;
        if inspector_open && workspace_width - tracks_width < minimum_preview {
            tracks_width = 168.0f32.max(workspace_width - minimum_preview);
        }
        Some(Self {
            inspector_width,
            workspace_width,
            tracks_width,
        })
    }

    /// Automatic layout plus optional user splits. The automatic path returns
    /// [`Self::layout`] unchanged so scale 1.0 remains the frozen baseline.
    #[must_use]
    pub fn layout_with_overrides(
        window_width: f32,
        inspector_open: bool,
        overrides: LayoutOverrides,
    ) -> Option<Self> {
        let mut widths = Self::layout(window_width, inspector_open)?;
        if inspector_open {
            if let Some(requested) = overrides.inspector_width.filter(|v| v.is_finite()) {
                let mut inspector = requested.clamp(240.0, 520.0);
                if window_width - inspector < 620.0 {
                    inspector = window_width - 620.0;
                }
                if inspector < 240.0 {
                    inspector = 240.0;
                }
                widths.inspector_width = inspector;
                widths.workspace_width = (window_width - inspector).max(0.0);
            }
        }
        if let Some(requested) = overrides.tracks_width.filter(|v| v.is_finite()) {
            let preview_floor = 480.0f32.max(widths.workspace_width * 0.30);
            let maximum = (widths.workspace_width - preview_floor).max(168.0);
            widths.tracks_width = requested.clamp(168.0, 520.0).min(maximum);
        }
        Some(widths)
    }
}

/// Every region of one workspace frame, in window coordinates.
///
/// Regions with nothing to draw are empty rects rather than `Option`s, because
/// every consumer already has to ask "is there room?" and
/// [`UiRect::is_empty`] is that question.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkspaceFrame {
    pub widths: ShellWidths,
    /// Right-hand tuning inspector; empty when closed.
    pub inspector: UiRect,
    /// The scene preview. This is what every other region takes from.
    pub preview: UiRect,
    /// Transport toolbar, directly beneath the preview (`plug.c:7755-7760`).
    pub toolbar: UiRect,
    /// Tracks panel; empty when [`TracksPanelMode::Hidden`].
    pub tracks: UiRect,
    pub scenes: UiRect,
    pub timeline: UiRect,
    pub tracks_mode: TracksPanelMode,
}

impl WorkspaceFrame {
    /// Assembles the frame (`plug.c:7623-7760`).
    ///
    /// `timeline_height` is the caller's, deliberately: see the module comment.
    /// It is clamped so the preview cannot go negative, because a preview with
    /// negative height is the shape the C's "reserved height it never drew"
    /// defects took.
    #[must_use]
    pub fn layout(
        window_width: f32,
        window_height: f32,
        inspector_open: bool,
        track_count: usize,
        timeline_height: f32,
    ) -> Self {
        Self::layout_with_overrides(
            window_width,
            window_height,
            inspector_open,
            track_count,
            timeline_height,
            LayoutOverrides::default(),
        )
    }

    #[must_use]
    pub fn layout_with_overrides(
        window_width: f32,
        window_height: f32,
        inspector_open: bool,
        track_count: usize,
        timeline_height: f32,
        overrides: LayoutOverrides,
    ) -> Self {
        let widths = ShellWidths::layout_with_overrides(window_width, inspector_open, overrides)
            .unwrap_or_else(|| ShellWidths::fallback(window_width));
        let toolbar_height = metric::HUD_BUTTON_SIZE;

        // A non-finite request is treated as the default rather than propagated:
        // NaN here would poison every rect in the frame.
        let requested = if timeline_height.is_finite() && timeline_height >= 0.0 {
            timeline_height
        } else {
            DEFAULT_TIMELINE_HEIGHT
        };
        // The preview never goes negative. If the request leaves no room the
        // timeline gives the space back — a panel is worth less than the surface
        // the whole application exists to show.
        let timeline_height = requested.min((window_height - toolbar_height).max(0.0));
        let preview_height = (window_height - timeline_height - toolbar_height).max(0.0);

        let preview = UiRect::new(
            widths.tracks_width,
            0.0,
            (widths.workspace_width - widths.tracks_width).max(0.0),
            preview_height,
        );
        let toolbar = UiRect::new(preview.x, preview.height, preview.width, toolbar_height);

        // The sidebar split used to be inline arithmetic that could allocate the
        // tracks panel zero height while it drew and registered its actions
        // anyway, stealing scene-tile clicks (`workspace_layout.h:7-19`).
        let sidebar_height = window_height - timeline_height;
        let sidebar = WorkspaceSidebar::layout(widths.tracks_width, sidebar_height, track_count);
        let (tracks, scenes, tracks_mode) = match sidebar {
            Some(sidebar) => (sidebar.tracks, sidebar.scenes, sidebar.tracks_mode),
            // The C's own fallback (`plug.c:7725-7730`): no tracks panel, and the
            // browser gets whatever is left.
            None => (
                UiRect::new(0.0, 0.0, widths.tracks_width, 0.0),
                UiRect::new(0.0, 0.0, widths.tracks_width, sidebar_height.max(0.0)),
                TracksPanelMode::Hidden,
            ),
        };

        Self {
            widths,
            inspector: UiRect::new(
                widths.workspace_width,
                0.0,
                widths.inspector_width,
                if inspector_open { window_height } else { 0.0 },
            ),
            preview,
            toolbar,
            tracks,
            scenes,
            timeline: UiRect::new(
                0.0,
                window_height - timeline_height,
                widths.workspace_width,
                timeline_height,
            ),
            tracks_mode,
        }
    }

    /// The full-screen frame: the preview takes everything and the toolbar
    /// overlays its bottom edge (`plug.c:7624-7666`).
    #[must_use]
    pub fn fullscreen(window_width: f32, window_height: f32, hud_visible: bool) -> Self {
        let toolbar_height = metric::HUD_BUTTON_SIZE;
        let preview_height = if hud_visible {
            (window_height - toolbar_height).max(0.0)
        } else {
            window_height
        };
        let empty = UiRect::new(0.0, 0.0, 0.0, 0.0);
        Self {
            widths: ShellWidths {
                inspector_width: 0.0,
                workspace_width: window_width,
                tracks_width: 0.0,
            },
            inspector: empty,
            preview: UiRect::new(0.0, 0.0, window_width, preview_height),
            toolbar: if hud_visible {
                UiRect::new(0.0, preview_height, window_width, toolbar_height)
            } else {
                empty
            },
            tracks: empty,
            scenes: empty,
            timeline: empty,
            tracks_mode: TracksPanelMode::Hidden,
        }
    }

    /// Timeline height for an open export panel (`plug.c:7689-7695`).
    #[must_use]
    pub fn export_timeline_height(window_height: f32) -> f32 {
        let toolbar_height = metric::HUD_BUTTON_SIZE;
        let ceiling = window_height - toolbar_height - DEFAULT_TIMELINE_HEIGHT;
        if EXPORT_PANEL_TIMELINE_HEIGHT > ceiling {
            150.0f32.max(ceiling)
        } else {
            EXPORT_PANEL_TIMELINE_HEIGHT
        }
    }

    /// The width the Assist panel's layout is measured against
    /// (`plug.c:7674`: `workspace_width - 12.0f`).
    ///
    /// **Owner: Agent J.** It is the workspace width, not the window width, so an
    /// open inspector really does narrow the panel — which at the 960 px minimum
    /// is what drops the mode grid to two rows and makes the panel taller. That
    /// is the case worth reproducing rather than approximating with a constant.
    #[must_use]
    pub fn assist_panel_width(window_width: f32, inspector_open: bool) -> f32 {
        let widths = ShellWidths::layout(window_width, inspector_open)
            .unwrap_or_else(|| ShellWidths::fallback(window_width));
        (widths.workspace_width - 12.0).max(0.0)
    }
}

/// Where the welcome screen's pieces go (`plug.c:7769-7830`).
///
/// The C computes these inline while it draws. They are pulled out here for the
/// same reason as everything else in this file: the interesting failures are at
/// the small end, where `Open audio` — the one control on the screen that
/// matters — can be pushed off the bottom edge by a window the application still
/// permits. That is assertable, and a screenshot at 1280x720 would never show it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WelcomeFrame {
    /// The masthead rule, at a fixed y (`plug.c:7771`).
    pub header_rule_y: f32,
    /// The vertical rule at 72% of the width (`plug.c:7772`).
    pub column_rule_x: f32,
    /// `MUSIALIZER`, top left.
    pub masthead: UiRect,
    /// The oversized `01`, right of the column rule.
    pub step_number: UiRect,
    /// Left edge and first baseline of the body column
    /// (`fmaxf(48, w*0.10)`, `fmaxf(120, h*0.20)`).
    pub body: UiRect,
    pub open_audio: UiRect,
    pub open_project: UiRect,
    /// `or drop audio anywhere in this window`.
    pub drop_hint: UiRect,
    /// The three numbered steps, left to right.
    pub steps: [UiRect; 3],
    /// The rule above the steps.
    pub steps_rule: UiRect,
    /// The supported-format strip along the bottom.
    pub formats: UiRect,
    /// `RECENT` and the rule under it, in the right-hand column (UX0-C06).
    pub recent_header: UiRect,
    /// Row seats for the recent list, top to bottom. Always [`recent::CAPACITY`]
    /// of them; [`Self::recent_visible`] says how many the window can hold, so a
    /// caller never has to re-derive the arithmetic to know where to stop.
    pub recent_rows: [UiRect; recent::CAPACITY],
    /// How many of [`Self::recent_rows`] fit above the format strip.
    pub recent_visible: usize,
}

impl WelcomeFrame {
    /// Button size at `plug.c:7789`, and the 10 px gap at `:7802`.
    const BUTTON: (f32, f32) = (176.0, 44.0);
    const BUTTON_GAP: f32 = 10.0;

    /// The recent column's own metrics (UX0-C06).
    ///
    /// A row is two lines — the project's name, then its age and folder — so 44
    /// is `15 + 4 + 12` of text inside 6 px of padding, and the 6 px gap is what
    /// keeps two rows from reading as one block of four lines.
    const RECENT_ROW: f32 = 44.0;
    const RECENT_GAP: f32 = 6.0;
    /// Distance from the column rule to the list's text.
    const RECENT_GUTTER: f32 = 24.0;
    /// Below this the column is not a list, it is a truncation. At the minimum
    /// supported window (960) the column is 212 px, so this is headroom rather
    /// than a limit anything reaches — but a caller that shrinks
    /// `column_rule_x` should get an empty seat rather than overlapping text.
    const RECENT_MINIMUM_WIDTH: f32 = 150.0;

    #[must_use]
    pub fn layout(window_width: f32, window_height: f32) -> Self {
        let w = window_width;
        let h = window_height;
        let left = 48.0f32.max(w * 0.10);
        let top = 120.0f32.max(h * 0.20);

        let open_audio = UiRect::new(left, top + 158.0, Self::BUTTON.0, Self::BUTTON.1);
        let open_project = UiRect::new(
            open_audio.x + open_audio.width + Self::BUTTON_GAP,
            open_audio.y,
            Self::BUTTON.0,
            Self::BUTTON.1,
        );

        // `fminf(250, (w*0.60)/3)`: the columns stop spreading at 250 px so the
        // three captions stay a group rather than drifting apart on a wide window.
        let step_stride = 250.0f32.min((w * 0.60) / 3.0);
        let steps_y = top + 250.0;
        let steps = [0usize, 1, 2].map(|index| {
            UiRect::new(
                left + index as f32 * step_stride,
                steps_y,
                step_stride,
                // Number at `steps_y`, caption 40 px below it at 15 px.
                55.0,
            )
        });

        // The recent column: everything right of the 72 % rule and below the
        // oversized step number, which is the only region on this screen the C
        // leaves empty. Bounded above by the number's own baseline box and below
        // by the format strip, so it can never print through either.
        let column_rule_x = w * 0.72;
        let step_number = UiRect::new(w - 150.0, 82.0, 150.0, 84.0);
        let formats = UiRect::new(32.0, h - 48.0, w - 64.0, 14.0);
        let recent_x = column_rule_x + Self::RECENT_GUTTER;
        let recent_width = (w - 32.0 - recent_x).max(0.0);
        let recent_header = UiRect::new(
            recent_x,
            step_number.y + step_number.height + 28.0,
            recent_width,
            14.0,
        );
        let rows_top = recent_header.y + 26.0;
        let rows_bottom = formats.y - 16.0;
        let stride = Self::RECENT_ROW + Self::RECENT_GAP;
        let recent_visible = if recent_width < Self::RECENT_MINIMUM_WIDTH {
            0
        } else {
            // `+ RECENT_GAP` because the last row needs no gap after it, and
            // without that the column loses a whole row to a 6 px remainder.
            (((rows_bottom - rows_top + Self::RECENT_GAP) / stride)
                .floor()
                .max(0.0) as usize)
                .min(recent::CAPACITY)
        };
        let recent_rows = std::array::from_fn(|index| {
            UiRect::new(
                recent_x,
                rows_top + index as f32 * stride,
                recent_width,
                Self::RECENT_ROW,
            )
        });

        Self {
            header_rule_y: 72.0,
            column_rule_x,
            masthead: UiRect::new(32.0, 30.0, w - 64.0, 24.0),
            step_number,
            body: UiRect::new(left, top, (w * 0.72 - left).max(0.0), 112.0 + 17.0),
            open_audio,
            open_project,
            drop_hint: UiRect::new(
                open_audio.x,
                open_audio.y + open_audio.height + 14.0,
                w - open_audio.x - 32.0,
                15.0,
            ),
            steps,
            steps_rule: UiRect::new(left, steps_y - 16.0, (w * 0.66 - left).max(0.0), 1.0),
            formats,
            recent_header,
            recent_rows,
            recent_visible,
        }
    }

    /// Whether everything this screen needs is inside the window.
    ///
    /// Reported rather than enforced, because the answer at the sizes the
    /// application permits should be "yes" — and if it ever is not, the caller
    /// dropping the step captions is a better outcome than drawing them off the
    /// edge.
    #[must_use]
    pub fn fits(&self, window_height: f32) -> bool {
        let lowest = self.steps[0].y + self.steps[0].height;
        lowest <= self.formats.y && self.formats.y + self.formats.height <= window_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimum supported window (`musializer.c:354`). Every "does it fit"
    /// assertion below is measured against what *this* produces.
    const MIN: (f32, f32) = (960.0, 640.0);

    #[test]
    fn a_closed_inspector_gives_the_workspace_the_whole_window() {
        let widths = ShellWidths::layout(1280.0, false).unwrap();
        assert_eq!(widths.inspector_width, 0.0);
        assert_eq!(widths.workspace_width, 1280.0);
        assert_eq!(widths.tracks_width, 320.0);
    }

    #[test]
    fn the_inspector_stays_inside_its_bounds() {
        // 0.265 of the width, floored at 280 and capped at 340
        // (`scene_settings.c:496-499`).
        assert_eq!(
            ShellWidths::layout(1000.0, true).unwrap().inspector_width,
            280.0
        );
        let mid = ShellWidths::layout(1200.0, true).unwrap().inspector_width;
        assert!((mid - 318.0).abs() < 0.01, "got {mid}");
        assert_eq!(
            ShellWidths::layout(2560.0, true).unwrap().inspector_width,
            340.0
        );
    }

    #[test]
    fn the_inspector_yields_so_the_preview_keeps_620_pixels() {
        // At 900 the 620 px rule would push the inspector to 280, which is also
        // its floor (`scene_settings.c:500-503`).
        let widths = ShellWidths::layout(900.0, true).unwrap();
        assert_eq!(widths.inspector_width, 280.0);
        // The 240 floor is what stops the inspector vanishing on a narrow window.
        let widths = ShellWidths::layout(700.0, true).unwrap();
        assert_eq!(widths.inspector_width, 240.0);
    }

    #[test]
    fn an_unusable_width_falls_back_the_way_the_c_does() {
        assert_eq!(ShellWidths::layout(639.0, false), None);
        assert_eq!(ShellWidths::layout(f32::NAN, false), None);
        let fallback = ShellWidths::fallback(400.0);
        assert_eq!(fallback.workspace_width, 400.0);
        assert_eq!(fallback.tracks_width, 320.0);
        // The frame still produces something drawable rather than propagating.
        let frame = WorkspaceFrame::layout(400.0, 400.0, false, 0, DEFAULT_TIMELINE_HEIGHT);
        assert_eq!(frame.widths, fallback);
    }

    #[test]
    fn the_regions_tile_the_window_without_overlapping_the_preview() {
        let frame = WorkspaceFrame::layout(1280.0, 720.0, false, 3, DEFAULT_TIMELINE_HEIGHT);
        assert!(!frame.preview.is_empty());
        assert!(!frame.toolbar.is_empty());
        assert!(!frame.timeline.is_empty());
        // The toolbar sits directly under the preview, sharing an edge. Touching
        // edges are not an overlap (`workspace_layout.c:28-33`), which is exactly
        // why that predicate is written the way it is.
        assert_eq!(frame.toolbar.y, frame.preview.y + frame.preview.height);
        assert!(!frame.preview.overlaps(frame.toolbar));
        assert!(!frame.preview.overlaps(frame.timeline));
        assert!(!frame.toolbar.overlaps(frame.timeline));
        // Preview + toolbar + timeline account for the full height.
        let total = frame.preview.height + frame.toolbar.height + frame.timeline.height;
        assert!((total - 720.0).abs() < 0.01, "got {total}");
    }

    #[test]
    fn empty_overrides_preserve_the_automatic_layout_exactly() {
        for (width, inspector) in [(960.0, false), (960.0, true), (1920.0, true)] {
            assert_eq!(
                ShellWidths::layout_with_overrides(width, inspector, LayoutOverrides::default()),
                ShellWidths::layout(width, inspector)
            );
        }
    }

    #[test]
    fn user_splits_are_logical_sizes_and_keep_the_preview_primary() {
        let frame = WorkspaceFrame::layout_with_overrides(
            1920.0,
            1080.0,
            true,
            3,
            330.0,
            LayoutOverrides {
                inspector_width: Some(440.0),
                tracks_width: Some(400.0),
            },
        );
        assert_eq!(frame.widths.inspector_width, 440.0);
        assert_eq!(frame.widths.tracks_width, 400.0);
        assert_eq!(frame.preview.width, 1080.0);
        assert_eq!(frame.timeline.height, 330.0);

        let constrained = ShellWidths::layout_with_overrides(
            960.0,
            true,
            LayoutOverrides {
                inspector_width: Some(520.0),
                tracks_width: Some(520.0),
            },
        )
        .unwrap();
        assert!(constrained.workspace_width - constrained.tracks_width >= 288.0);
        assert!(constrained.inspector_width >= 240.0);
    }

    #[test]
    fn the_sidebar_and_the_preview_do_not_overlap() {
        let frame = WorkspaceFrame::layout(1280.0, 720.0, false, 3, DEFAULT_TIMELINE_HEIGHT);
        assert!(!frame.tracks.overlaps(frame.scenes));
        // The rail is left of the preview; the preview starts where it ends.
        assert_eq!(frame.preview.x, frame.widths.tracks_width);
        assert!(frame.tracks.width <= frame.widths.tracks_width);
        assert!(frame.scenes.width <= frame.widths.tracks_width);
    }

    #[test]
    fn the_minimum_window_still_leaves_a_visible_preview() {
        // Rule 2: measured against what 960x640 actually produces.
        let frame = WorkspaceFrame::layout(MIN.0, MIN.1, false, 0, DEFAULT_TIMELINE_HEIGHT);
        // 640 - 180 - 50 = 410.
        assert_eq!(frame.preview.height, 410.0);
        assert_eq!(frame.preview.width, 960.0 - 320.0);
        assert!(!frame.preview.is_empty());
    }

    #[test]
    fn the_minimum_window_with_the_inspector_open_still_leaves_a_preview() {
        let frame = WorkspaceFrame::layout(MIN.0, MIN.1, true, 0, DEFAULT_TIMELINE_HEIGHT);
        assert!(!frame.inspector.is_empty());
        assert!(
            frame.preview.width >= 300.0,
            "preview collapsed to {} px at the minimum window",
            frame.preview.width
        );
        assert!(!frame.preview.overlaps(frame.inspector));
        // The inspector begins where the workspace ends.
        assert_eq!(frame.inspector.x, frame.widths.workspace_width);
    }

    #[test]
    fn an_over_tall_timeline_request_gives_the_space_back_instead_of_inverting() {
        // The failure mode this guards: a panel asking for more than the window
        // has used to produce a negative-height preview, which raylib happily
        // scissors into nothing while the panel drew off the bottom edge.
        let frame = WorkspaceFrame::layout(1280.0, 640.0, false, 0, 5_000.0);
        assert_eq!(frame.preview.height, 0.0);
        assert!(frame.preview.is_empty());
        assert!(frame.timeline.height <= 640.0);
        assert!(frame.timeline.y >= 0.0);
    }

    #[test]
    fn a_non_finite_timeline_request_falls_back_to_the_default() {
        for bad in [f32::NAN, f32::INFINITY, -1.0] {
            let frame = WorkspaceFrame::layout(1280.0, 720.0, false, 0, bad);
            assert_eq!(frame.timeline.height, DEFAULT_TIMELINE_HEIGHT, "{bad}");
        }
    }

    #[test]
    fn a_hidden_tracks_panel_yields_its_height_to_the_browser() {
        // The containment promise: when the panel cannot host its action row it
        // is not drawn at all, and the space goes to the browser rather than
        // becoming invisible buttons that steal clicks.
        let frame = WorkspaceFrame::layout(1280.0, 400.0, false, 0, 320.0);
        if frame.tracks_mode == TracksPanelMode::Hidden {
            assert!(frame.tracks.is_empty());
            assert_eq!(frame.tracks.height, 0.0);
        }
        // Whatever the mode, the action row it claims must fit inside it.
        if let Some((top, height)) = frame.tracks_mode.action_row() {
            let row = UiRect::new(
                frame.tracks.x + 8.0,
                frame.tracks.y + top,
                frame.tracks.width - 16.0,
                height,
            );
            assert!(
                frame.tracks.contains(row),
                "mode {:?} claims a row at +{top} h{height} that does not fit {:?}",
                frame.tracks_mode,
                frame.tracks
            );
        }
    }

    #[test]
    fn the_action_row_fits_at_every_height_from_the_minimum_window_upward() {
        // A sweep rather than three spot checks, because the mode ladder's
        // boundaries are exactly where a reserved-but-undrawn row appears.
        for height in 640..=1440 {
            let frame =
                WorkspaceFrame::layout(1280.0, height as f32, false, 4, DEFAULT_TIMELINE_HEIGHT);
            if let Some((top, row_height)) = frame.tracks_mode.action_row() {
                let row = UiRect::new(
                    frame.tracks.x,
                    frame.tracks.y + top,
                    frame.tracks.width,
                    row_height,
                );
                assert!(
                    frame.tracks.contains(row),
                    "height {height}: mode {:?} row does not fit",
                    frame.tracks_mode
                );
            } else {
                assert!(
                    frame.tracks.is_empty(),
                    "height {height}: hidden but non-empty"
                );
            }
            assert!(frame.preview.height >= 0.0);
        }
    }

    #[test]
    fn fullscreen_gives_the_preview_the_window_and_overlays_the_hud() {
        let frame = WorkspaceFrame::fullscreen(1280.0, 720.0, false);
        assert_eq!(frame.preview.height, 720.0);
        assert!(frame.toolbar.is_empty());
        assert!(frame.timeline.is_empty());

        let frame = WorkspaceFrame::fullscreen(1280.0, 720.0, true);
        assert_eq!(frame.preview.height, 670.0);
        assert_eq!(frame.toolbar.y, 670.0);
        assert!(!frame.preview.overlaps(frame.toolbar));
    }

    #[test]
    fn the_export_panel_yields_before_the_preview_does() {
        // Tall window: the panel gets what it asked for.
        assert_eq!(
            WorkspaceFrame::export_timeline_height(1080.0),
            EXPORT_PANEL_TIMELINE_HEIGHT
        );
        // 640 - 50 - 180 = 410, still above 330.
        assert_eq!(
            WorkspaceFrame::export_timeline_height(640.0),
            EXPORT_PANEL_TIMELINE_HEIGHT
        );
        // A short window floors at 150 rather than pushing the preview negative.
        assert_eq!(WorkspaceFrame::export_timeline_height(300.0), 150.0);
    }

    #[test]
    fn the_assist_panel_is_measured_against_the_workspace_not_the_window() {
        // `workspace_width - 12` (`plug.c:7674`). An open inspector narrows it,
        // and at the 960 px minimum that is what pushes the mode grid to two rows
        // — the case a constant would have hidden.
        use musializer_core::ui::assist_ui_state::{self, AssistPanelContent};

        let wide = WorkspaceFrame::assist_panel_width(1280.0, false);
        let narrow = WorkspaceFrame::assist_panel_width(960.0, true);
        assert!(narrow < wide);
        assert_eq!(
            assist_ui_state::ui_layout(wide, AssistPanelContent::Ready, false).mode_rows,
            1
        );
        assert_eq!(
            assist_ui_state::ui_layout(narrow, AssistPanelContent::Ready, false).mode_rows,
            2,
            "the inspector at the minimum window is what makes the grid wrap"
        );
        // A degenerate window still answers with a non-negative width rather
        // than poisoning the layout with a negative one.
        assert!(WorkspaceFrame::assist_panel_width(1.0, true) >= 0.0);
    }

    #[test]
    fn the_welcome_screen_seats_its_call_to_action_at_the_smallest_permitted_window() {
        // GLFW clamps a smaller request up to 960x640 (`cli::MIN_WINDOW`), so that
        // is the real floor and the one worth asserting. The failure this catches
        // is the interesting one: `top` is `max(120, h*0.20)` and the buttons sit
        // 158 px below it, so a short window pushes the only control on the screen
        // toward the format strip rather than shrinking anything.
        for (w, h) in [(960.0f32, 640.0f32), (1280.0, 720.0), (1920.0, 1080.0)] {
            let frame = WelcomeFrame::layout(w, h);
            let window = UiRect::new(0.0, 0.0, w, h);
            assert!(
                window.contains(frame.open_audio),
                "{w}x{h}: Open audio is outside the window"
            );
            assert!(
                window.contains(frame.open_project),
                "{w}x{h}: Open project is outside the window"
            );
            assert!(
                !frame.open_audio.overlaps(frame.open_project),
                "{w}x{h}: the two buttons overlap"
            );
            assert!(frame.fits(h), "{w}x{h}: the screen does not fit");
        }
    }

    #[test]
    fn the_recent_column_seats_whole_rows_inside_the_spare_region() {
        // UX0-C06. Pinned numbers rather than "it is somewhere on the right",
        // because a range assertion is satisfied by a wrong formula — the lesson
        // the `layout` differential harness exists to record. These are what the
        // arithmetic above produces at the three windows the gate photographs.
        for (w, h, expected_rows) in [
            (960.0f32, 640.0f32, 7usize),
            (1280.0, 720.0, 8),
            (1920.0, 1080.0, 8),
        ] {
            let frame = WelcomeFrame::layout(w, h);
            assert_eq!(
                frame.recent_visible, expected_rows,
                "{w}x{h}: wrong number of recent seats"
            );
            let window = UiRect::new(0.0, 0.0, w, h);
            for (index, row) in frame.recent_rows[..frame.recent_visible].iter().enumerate() {
                assert!(window.contains(*row), "{w}x{h}: row {index} is off screen");
                // The two things this column must never do: print through the
                // oversized step number above it, or through the format strip
                // below it. Both are drawn unconditionally, so an overlap is not
                // a near miss, it is two strings in the same pixels.
                assert!(
                    !row.overlaps(frame.step_number),
                    "{w}x{h}: row {index} runs into the step number"
                );
                assert!(
                    !row.overlaps(frame.formats),
                    "{w}x{h}: row {index} runs into the format strip"
                );
                assert!(
                    row.x > frame.column_rule_x,
                    "{w}x{h}: row {index} crosses the column rule"
                );
            }
            assert!(!frame.recent_header.overlaps(frame.step_number));
            assert!(!frame.recent_header.overlaps(frame.recent_rows[0]));
            // The list is one column, so consecutive seats may touch but never
            // overlap — the property that keeps a click landing on one row.
            for pair in frame.recent_rows[..frame.recent_visible].windows(2) {
                assert!(!pair[0].overlaps(pair[1]));
                assert!(pair[1].y > pair[0].y);
            }
        }
    }

    #[test]
    fn a_column_too_narrow_to_be_a_list_seats_none_rather_than_a_truncated_one() {
        // Unreachable through the window minimum — 960 gives 212 px — but the
        // seat count has to be honest for a caller that narrows the rule, because
        // the alternative is a name and a folder printing through each other and
        // still photographing as text.
        let frame = WelcomeFrame::layout(400.0, 720.0);
        assert_eq!(frame.recent_visible, 0);
        let short = WelcomeFrame::layout(1280.0, 320.0);
        assert_eq!(
            short.recent_visible, 0,
            "a window with no room between the number and the strip seats nothing"
        );
    }

    #[test]
    fn a_window_too_short_for_the_steps_says_so_rather_than_drawing_off_the_edge() {
        // Not reachable through the window minimum, but reachable through
        // `--size` before GLFW clamps, and through a fullscreen probe. `fits`
        // exists so the caller can drop the captions instead of painting them
        // over the format strip.
        let frame = WelcomeFrame::layout(960.0, 420.0);
        assert!(!frame.fits(420.0));
    }

    #[test]
    fn the_step_columns_stop_spreading_on_a_wide_window() {
        // `fminf(250, (w*0.60)/3)`. Without the cap the three captions drift to
        // the far side of a 4K window and stop reading as one sequence.
        let wide = WelcomeFrame::layout(3840.0, 2160.0);
        let stride = wide.steps[1].x - wide.steps[0].x;
        assert_eq!(stride, 250.0);
        // And below the cap it is proportional, so a narrow window packs them.
        let narrow = WelcomeFrame::layout(960.0, 640.0);
        assert!((narrow.steps[1].x - narrow.steps[0].x) < 250.0);
    }
}
