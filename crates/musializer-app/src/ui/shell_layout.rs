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

use super::theme::metric;

/// Timeline height with no panel open (`plug.c:7669`).
pub const DEFAULT_TIMELINE_HEIGHT: f32 = 180.0;

/// Preview height the export panel refuses to squeeze below (`plug.c:7691-7694`).
pub const EXPORT_PANEL_TIMELINE_HEIGHT: f32 = 330.0;

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
        let widths = ShellWidths::layout(window_width, inspector_open)
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
}
