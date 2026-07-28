//! One file per bottom panel and per inspector pane.
//!
//! **This split exists for the fan-out, not for tidiness.** Six agents fill six
//! surfaces at once, and in session 1 the thing that prevented collisions was
//! pre-creating every file with its `pub mod` line already registered, so no
//! agent ever edits a `mod.rs`. The same rule applies here and to
//! `super::shell`, `super::widgets` and `super::theme`: an agent that needs a
//! new widget or colour **requests** it rather than adding it, because those are
//! the files every agent touches.
//!
//! Each panel is an `impl Shell` block in its own module. Rust allows inherent
//! impls to be split across modules of the same crate, so a panel gets a normal
//! method with access to shell state without anything being made public.
//!
//! # The seams
//!
//! Three surfaces nest inside another agent's, and the call sites are defined
//! here rather than negotiated later:
//!
//! | Surface | Owner | Called from |
//! | --- | --- | --- |
//! | the route editor row | G | [`tune`]'s per-setting row loop |
//! | the font browser pane | K | [`lyrics`]'s three-pane editor |
//! | the manual event row | L | [`events`], from the timeline strip |

pub mod assist;
pub mod events;
pub mod export;
pub mod fonts;
pub mod lyrics;
pub mod tune;

use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::Face;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle};

use super::theme::{color, metric};
use super::widgets;

/// Names a surface that is not built yet, in the space it would occupy.
///
/// A blank region is indistinguishable from a broken one, and the panel that
/// says "not built yet" is the one a reviewer can act on. Shared so every
/// unfilled surface says it the same way — and so that deleting the last call to
/// it is a visible, checkable event.
///
/// The geometry is not incidental. The box starts **below** the timeline strip
/// (`strip.y + strip.height + 28.0`), because the panel and the strip share one
/// band and the extra height an open panel asks for is the space underneath. A
/// first attempt at this split drew into `content` directly and printed the
/// stub over the timeline's ticks — which a capture showed and no test would
/// have.
pub fn stub(
    d: &mut RaylibDrawHandle<'_>,
    font: &Face,
    content: UiRect,
    strip: UiRect,
    title: &str,
    detail: &str,
) {
    let top = strip.y + strip.height + 28.0;
    let area = UiRect::new(
        content.x + metric::UI_PANEL_PADDING,
        top,
        content.width - metric::UI_PANEL_PADDING * 2.0,
        (content.y + content.height - top - metric::UI_PANEL_PADDING).max(0.0),
    );
    if area.is_empty() {
        return;
    }
    widgets::fill(d, area, color::ui_raised());
    d.draw_rectangle_lines_ex(widgets::rectangle(area), 1.0, color::ui_warning());
    widgets::draw_text(
        d,
        font,
        title,
        area.x + 8.0,
        area.y + 8.0,
        metric::UI_FONT_CAPTION,
        color::ui_warning(),
    );
    widgets::draw_text(
        d,
        font,
        "not built yet",
        area.x + 8.0,
        area.y + 26.0,
        metric::UI_FONT_LABEL,
        color::ui_ink(),
    );
    if area.height > 62.0 {
        widgets::draw_text(
            d,
            font,
            detail,
            area.x + 8.0,
            area.y + 48.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
    }
}
