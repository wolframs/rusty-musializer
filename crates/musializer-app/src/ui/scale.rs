//! Logical UI units and their mapping to the window framebuffer.
//!
//! The shell was originally authored directly in window pixels. Keeping those
//! numbers as design units preserves every 1x layout while this type makes the
//! physical boundary explicit: layout and hit testing are logical, drawing and
//! scissors are transformed once, and the scene keeps the framebuffer pixels it
//! was given.

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{Camera2D, RaylibDrawHandle, Vector2};

/// Supported scale rungs. Stepped values keep font atlases and pixel snapping
/// deterministic and avoid rebuilding GPU fonts for every fractional DPI value.
pub const UI_SCALE_STEPS: [f32; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];

/// The logical minimum whose content policies are already exhaustively tested.
pub const LOGICAL_MINIMUM: (f32, f32) = (960.0, 640.0);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiScale(f32);

impl Default for UiScale {
    fn default() -> Self {
        Self(1.0)
    }
}

impl UiScale {
    #[must_use]
    pub fn new(value: f32) -> Option<Self> {
        UI_SCALE_STEPS
            .iter()
            .copied()
            .find(|step| (value - step).abs() < 0.001)
            .map(Self)
    }

    #[must_use]
    pub fn from_percent(percent: u16) -> Option<Self> {
        Self::new(f32::from(percent) / 100.0)
    }

    #[must_use]
    pub fn value(self) -> f32 {
        self.0
    }

    #[must_use]
    pub fn percent(self) -> u16 {
        (self.0 * 100.0).round() as u16
    }

    #[must_use]
    pub fn logical_size(self, physical: (f32, f32)) -> (f32, f32) {
        (physical.0 / self.0, physical.1 / self.0)
    }

    #[must_use]
    pub fn logical_point(self, physical: Vector2) -> Vector2 {
        Vector2::new(physical.x / self.0, physical.y / self.0)
    }

    #[must_use]
    pub fn mouse(self, d: &RaylibDrawHandle<'_>) -> Vector2 {
        self.logical_point(d.get_mouse_position())
    }

    /// Transform a logical rectangle to framebuffer pixels, rounding edges
    /// rather than width/height independently so adjacent regions still tile.
    #[must_use]
    pub fn physical_rect(self, logical: UiRect) -> UiRect {
        let left = (logical.x * self.0).round();
        let top = (logical.y * self.0).round();
        let right = ((logical.x + logical.width) * self.0).round();
        let bottom = ((logical.y + logical.height) * self.0).round();
        UiRect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    #[must_use]
    pub fn camera(self) -> Camera2D {
        Camera2D {
            offset: Vector2::zero(),
            target: Vector2::zero(),
            rotation: 0.0,
            zoom: self.0,
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        UI_SCALE_STEPS
            .iter()
            .copied()
            .find(|step| *step > self.0 + 0.001)
            .and_then(Self::new)
            .unwrap_or(self)
    }

    #[must_use]
    pub fn previous(self) -> Self {
        UI_SCALE_STEPS
            .iter()
            .copied()
            .rev()
            .find(|step| *step < self.0 - 0.001)
            .and_then(Self::new)
            .unwrap_or(self)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum UiScalePreference {
    #[default]
    Auto,
    Fixed(UiScale),
}

impl UiScalePreference {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text.eq_ignore_ascii_case("auto") {
            return Some(Self::Auto);
        }
        let percent = text.trim_end_matches('%').parse::<u16>().ok()?;
        UiScale::from_percent(percent).map(Self::Fixed)
    }
}

/// Resolve Auto/manual preference, then constrain it so the tested logical
/// minimum remains representable in the current window.
#[must_use]
pub fn effective_scale(
    preference: UiScalePreference,
    physical: (f32, f32),
    desktop_scale: Vector2,
) -> UiScale {
    let auto = || {
        let dpi = desktop_scale.x.max(desktop_scale.y);
        let wanted = if dpi >= 1.875 || physical.1 >= 1800.0 {
            2.0
        } else if dpi >= 1.625 {
            1.75
        } else if dpi >= 1.375 || physical.1 >= 1260.0 {
            1.5
        } else if dpi >= 1.125 || physical.1 >= 900.0 {
            1.25
        } else {
            1.0
        };
        UiScale::new(wanted).expect("auto scale belongs to the step table")
    };
    let requested = match preference {
        UiScalePreference::Auto => auto(),
        UiScalePreference::Fixed(scale) => scale,
    };
    let fit = (physical.0 / LOGICAL_MINIMUM.0).min(physical.1 / LOGICAL_MINIMUM.1);
    UI_SCALE_STEPS
        .iter()
        .copied()
        .rev()
        .find(|step| *step <= requested.value() + 0.001 && *step <= fit + 0.001)
        .and_then(UiScale::new)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_steps_parse_and_walk_without_fractional_drift() {
        assert_eq!(
            UiScalePreference::parse("auto"),
            Some(UiScalePreference::Auto)
        );
        assert_eq!(
            UiScalePreference::parse("150%"),
            UiScale::new(1.5).map(UiScalePreference::Fixed)
        );
        assert_eq!(UiScale::new(1.25).unwrap().next().percent(), 150);
        assert_eq!(UiScale::new(1.25).unwrap().previous().percent(), 100);
        assert!(UiScalePreference::parse("133").is_none());
    }

    #[test]
    fn auto_uses_large_windows_but_never_breaks_the_logical_minimum() {
        let one = effective_scale(
            UiScalePreference::Auto,
            (1280.0, 720.0),
            Vector2::new(1.0, 1.0),
        );
        assert_eq!(one.percent(), 100);
        let large = effective_scale(
            UiScalePreference::Auto,
            (2560.0, 1440.0),
            Vector2::new(1.0, 1.0),
        );
        assert_eq!(large.percent(), 150);
        let constrained = effective_scale(
            UiScalePreference::Fixed(UiScale::new(2.0).unwrap()),
            (1280.0, 720.0),
            Vector2::new(2.0, 2.0),
        );
        assert_eq!(constrained.percent(), 100);
    }

    #[test]
    fn physical_rect_rounds_shared_edges_identically() {
        let scale = UiScale::new(1.25).unwrap();
        let left = scale.physical_rect(UiRect::new(0.0, 0.0, 101.0, 20.0));
        let right = scale.physical_rect(UiRect::new(101.0, 0.0, 99.0, 20.0));
        assert_eq!(left.x + left.width, right.x);
        assert_eq!(right.x + right.width, 250.0);
    }
}
