//! Logical UI units and their mapping to the window framebuffer.
//!
//! The shell was originally authored directly in window pixels. Keeping those
//! numbers as design units preserves every 1x layout while this type makes the
//! physical boundary explicit: layout and hit testing are logical, drawing and
//! scissors are transformed once, and the scene keeps the framebuffer pixels it
//! was given.

use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{KeyboardKey, RaylibDrawHandle, Vector2};
use raylib_sys::Camera2D;

/// Supported scale rungs. Stepped values keep font atlases and pixel snapping
/// deterministic and avoid rebuilding GPU fonts for every fractional DPI value.
pub const UI_SCALE_STEPS: [f32; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];

/// The logical minimum whose content policies are already exhaustively tested.
pub const LOGICAL_MINIMUM: (f32, f32) = (960.0, 640.0);

#[derive(Clone, Copy, Debug)]
pub struct UiScale(f32, Option<UiRect>);

/// Equal when the scale is: the occluder is a property of one frame's input,
/// not of the scale, and nothing that keys on a scale (font atlases, the
/// preference file) may see two values for one rung.
impl PartialEq for UiScale {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Default for UiScale {
    fn default() -> Self {
        Self(1.0, None)
    }
}

/// Where [`UiScale::mouse`] reports a pointer that is over an occluder: far
/// outside any window, and finite, so arithmetic on it cannot turn into NaN.
const OCCLUDED_POINTER: f32 = -1.0e6;

impl UiScale {
    #[must_use]
    pub fn new(value: f32) -> Option<Self> {
        UI_SCALE_STEPS
            .iter()
            .copied()
            .find(|step| (value - step).abs() < 0.001)
            .map(|step| Self(step, None))
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

    /// The pointer in logical units, or a point far outside the window when
    /// it is over this scale's occluder.
    #[must_use]
    pub fn mouse(self, d: &RaylibDrawHandle<'_>) -> Vector2 {
        self.occlude(self.logical_point(d.get_mouse_position()))
    }

    /// The same scale with `occluder` hiding the pointer (WX-5c, the rail's
    /// flyouts).
    ///
    /// A flyout floats over the preview, the transport and the band, and is
    /// modal for presses. Most of what is under it reads the pointer through
    /// this type — every widget, and the lanes and panels that test a raw
    /// press against a rect — so a frame's input handed to them through an
    /// occluded scale cannot reach anything under the flyout: the pointer is
    /// simply not there. One seam, rather than a guard at each of the forty
    /// call sites, two of which are in panels other agents own.
    #[must_use]
    pub fn occluded(self, occluder: Option<UiRect>) -> Self {
        Self(self.0, occluder.filter(|rect| !rect.is_empty()))
    }

    fn occlude(self, point: Vector2) -> Vector2 {
        match self.1 {
            Some(rect) if rect.contains_point(point.x, point.y) => {
                Vector2::new(OCCLUDED_POINTER, OCCLUDED_POINTER)
            }
            _ => point,
        }
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
            offset: Vector2::zero().into(),
            target: Vector2::zero().into(),
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

/// The rung a desktop scale asks for by itself: what Auto means before a
/// window exists, and so the scale a window has to be sized to hold.
#[must_use]
pub fn desktop_rung(desktop_scale: Vector2) -> UiScale {
    let dpi = desktop_scale.x.max(desktop_scale.y);
    let wanted = if dpi >= 1.875 {
        2.0
    } else if dpi >= 1.625 {
        1.75
    } else if dpi >= 1.375 {
        1.5
    } else if dpi >= 1.125 {
        1.25
    } else {
        1.0
    };
    UiScale::new(wanted).expect("auto scale belongs to the step table")
}

/// Auto's second opinion: a tall window gets a larger shell even on a desktop
/// that reports no scale.
fn height_rung(physical_height: f32) -> UiScale {
    let wanted = if physical_height >= 1800.0 {
        2.0
    } else if physical_height >= 1260.0 {
        1.5
    } else if physical_height >= 900.0 {
        1.25
    } else {
        1.0
    };
    UiScale::new(wanted).expect("auto scale belongs to the step table")
}

/// The scale a preference asks for with no window to measure: Auto is the
/// desktop's rung, a fixed choice is itself.
///
/// [`effective_scale`] may still give more (a tall window under Auto) or less
/// (a window too small to hold it). This is the value window sizes are derived
/// from, so that neither happens by accident on the first frame.
#[must_use]
pub fn wanted_scale(preference: UiScalePreference, desktop_scale: Vector2) -> UiScale {
    match preference {
        UiScalePreference::Auto => desktop_rung(desktop_scale),
        UiScalePreference::Fixed(scale) => scale,
    }
}

/// How much of a monitor the window floor may take, either way. A floor near
/// the whole monitor is a window that cannot be made smaller than the screen;
/// below this share the shell steps down a rung instead, as it always has.
const FLOOR_MONITOR_SHARE: f32 = 0.8;

/// How much of a monitor the first window may take, either way: room for the
/// title bar and a panel, which raylib cannot ask the desktop about.
const STARTUP_MONITOR_SHARE: f32 = 0.9;

/// The smallest window that still holds `wanted`: the logical minimum at the
/// largest rung up to `wanted` that leaves the monitor room.
///
/// Window sizes were raw pixels while the shell was in scaled units, so on a
/// 125 % desktop a window could be dragged (or simply opened) below
/// 1200x800 and the shell dropped to 100 % there. `monitor` is `None` when
/// the desktop did not name one; nothing is capped then.
#[must_use]
pub fn window_floor(wanted: UiScale, monitor: Option<(i32, i32)>) -> (i32, i32) {
    let size = |scale: f32| {
        (
            (LOGICAL_MINIMUM.0 * scale).ceil() as i32,
            (LOGICAL_MINIMUM.1 * scale).ceil() as i32,
        )
    };
    let fits = |(width, height): (i32, i32)| {
        monitor.is_none_or(|(monitor_width, monitor_height)| {
            width as f32 <= monitor_width as f32 * FLOOR_MONITOR_SHARE
                && height as f32 <= monitor_height as f32 * FLOOR_MONITOR_SHARE
        })
    };
    UI_SCALE_STEPS
        .iter()
        .copied()
        .rev()
        .filter(|step| *step <= wanted.value() + 0.001)
        .map(size)
        .find(|candidate| fits(*candidate))
        .unwrap_or_else(|| size(1.0))
}

/// The window a wanted scale is given when one has to be made or grown for
/// it: the logical minimum at exactly that rung when nine tenths of the
/// monitor has room, the floor otherwise.
///
/// Larger than [`window_floor`] where the two caps differ. On a 1080p monitor
/// the floor for 150 % stops at 1200x800 (a 1440x960 minimum would be nearly
/// the screen), but Ctrl+plus asking for 150 % in that window should get it:
/// the window grows to 1440x960 and may be dragged smaller again.
#[must_use]
pub fn window_to_hold(wanted: UiScale, monitor: Option<(i32, i32)>) -> (i32, i32) {
    let exact = (
        (LOGICAL_MINIMUM.0 * wanted.value()).ceil() as i32,
        (LOGICAL_MINIMUM.1 * wanted.value()).ceil() as i32,
    );
    let fits = monitor.is_none_or(|(monitor_width, monitor_height)| {
        exact.0 as f32 <= monitor_width as f32 * STARTUP_MONITOR_SHARE
            && exact.1 as f32 <= monitor_height as f32 * STARTUP_MONITOR_SHARE
    });
    if fits {
        exact
    } else {
        window_floor(wanted, monitor)
    }
}

/// Which keys are plus and which are minus on this keyboard.
///
/// raylib's key codes are positions on a US keyboard, and the scale shortcuts
/// were written against them: `KEY_EQUAL` and `KEY_MINUS`. On a German
/// keyboard the key labelled `+` is `KEY_RIGHT_BRACKET` and `-` is
/// `KEY_SLASH`, so Ctrl+plus and Ctrl+minus did nothing there (the US
/// positions are `´` and `ß`). `names` is each punctuation key with the
/// character the layout gives it, unshifted (`RaylibHandle::get_key_name`).
///
/// A key named `+` or `=` is plus (US plus is Shift+`=`), a key named `-` is
/// minus. A role no key is named for keeps its US position, so a layout
/// GLFW cannot name still has the shortcuts it had.
#[must_use]
pub fn scale_keys(names: &[(KeyboardKey, Option<String>)]) -> ScaleKeys {
    let named = |wanted: &[&str]| -> Vec<KeyboardKey> {
        names
            .iter()
            .filter(|(_, name)| name.as_deref().is_some_and(|name| wanted.contains(&name)))
            .map(|(key, _)| *key)
            .collect()
    };
    let mut up = named(&["+", "="]);
    if up.is_empty() {
        up.push(KeyboardKey::KEY_EQUAL);
    }
    let mut down = named(&["-"]);
    if down.is_empty() {
        down.push(KeyboardKey::KEY_MINUS);
    }
    ScaleKeys { up, down }
}

/// The punctuation positions a layout can put `+`, `=` or `-` on.
pub const SCALE_KEY_CANDIDATES: [KeyboardKey; 11] = [
    KeyboardKey::KEY_APOSTROPHE,
    KeyboardKey::KEY_COMMA,
    KeyboardKey::KEY_MINUS,
    KeyboardKey::KEY_PERIOD,
    KeyboardKey::KEY_SLASH,
    KeyboardKey::KEY_SEMICOLON,
    KeyboardKey::KEY_EQUAL,
    KeyboardKey::KEY_LEFT_BRACKET,
    KeyboardKey::KEY_BACKSLASH,
    KeyboardKey::KEY_RIGHT_BRACKET,
    KeyboardKey::KEY_GRAVE,
];

/// The main-block keys the scale shortcuts answer to; the numpad's plus and
/// minus always do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScaleKeys {
    pub up: Vec<KeyboardKey>,
    pub down: Vec<KeyboardKey>,
}

/// The window to open when the command line named no size: the default in
/// the desktop's units, kept on the monitor and never under the floor.
///
/// At a wanted scale of 100 % this is `default` untouched, whatever the
/// monitor: every existing capture opens that window, and a small virtual
/// screen must not start shrinking it.
#[must_use]
pub fn startup_window(
    default: (i32, i32),
    wanted: UiScale,
    monitor: Option<(i32, i32)>,
) -> (i32, i32) {
    if wanted.value() <= 1.0 {
        return default;
    }
    let scaled = |length: i32, monitor_length: Option<i32>| {
        let scaled = (length as f32 * wanted.value()).round();
        let capped = monitor_length.map_or(scaled, |monitor_length| {
            scaled.min((monitor_length as f32 * STARTUP_MONITOR_SHARE).floor())
        });
        capped as i32
    };
    let hold = window_to_hold(wanted, monitor);
    (
        scaled(default.0, monitor.map(|size| size.0)).max(hold.0),
        scaled(default.1, monitor.map(|size| size.1)).max(hold.1),
    )
}

/// Resolve Auto/manual preference, then constrain it so the tested logical
/// minimum remains representable in the current window.
#[must_use]
pub fn effective_scale(
    preference: UiScalePreference,
    physical: (f32, f32),
    desktop_scale: Vector2,
) -> UiScale {
    let requested = match preference {
        UiScalePreference::Auto => {
            let desktop = desktop_rung(desktop_scale);
            let height = height_rung(physical.1);
            if height.value() > desktop.value() {
                height
            } else {
                desktop
            }
        }
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
    fn an_occluded_scale_hides_the_pointer_over_its_rect_and_nowhere_else() {
        let scale = UiScale::new(1.5).unwrap();
        let occluded = scale.occluded(Some(UiRect::new(52.0, 46.0, 336.0, 674.0)));
        // Equal as a scale: nothing keyed on the rung may see two values.
        assert_eq!(occluded, scale);
        assert_eq!(
            occluded.occlude(Vector2::new(100.0, 100.0)),
            Vector2::new(OCCLUDED_POINTER, OCCLUDED_POINTER)
        );
        assert_eq!(
            occluded.occlude(Vector2::new(400.0, 100.0)),
            Vector2::new(400.0, 100.0)
        );
        // An empty occluder is no occluder.
        let none = scale.occluded(Some(UiRect::new(0.0, 0.0, 0.0, 0.0)));
        assert_eq!(none.occlude(Vector2::new(0.0, 0.0)), Vector2::new(0.0, 0.0));
    }

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

    /// The ladder `effective_scale` carried inline until 2026-11-19, kept here
    /// verbatim as the oracle for its split into two rungs.
    fn auto_ladder_before_the_split(physical_height: f32, dpi: f32) -> f32 {
        if dpi >= 1.875 || physical_height >= 1800.0 {
            2.0
        } else if dpi >= 1.625 {
            1.75
        } else if dpi >= 1.375 || physical_height >= 1260.0 {
            1.5
        } else if dpi >= 1.125 || physical_height >= 900.0 {
            1.25
        } else {
            1.0
        }
    }

    #[test]
    fn auto_is_the_same_ladder_it_was_before_it_became_two_rungs() {
        // A window wide and tall enough that the fit clamp never speaks.
        for height in [640.0, 899.0, 900.0, 1259.0, 1260.0, 1799.0, 1800.0, 2160.0] {
            for dpi in [
                1.0, 1.124, 1.125, 1.25, 1.374, 1.375, 1.5, 1.624, 1.625, 1.75, 1.874, 1.875, 2.0,
                3.0,
            ] {
                let resolved = effective_scale(
                    UiScalePreference::Auto,
                    (8000.0, height),
                    Vector2::new(dpi, 1.0),
                );
                let fit = height / LOGICAL_MINIMUM.1;
                let wanted = auto_ladder_before_the_split(height, dpi);
                let expected = UI_SCALE_STEPS
                    .iter()
                    .copied()
                    .rev()
                    .find(|step| *step <= wanted + 0.001 && *step <= fit + 0.001)
                    .unwrap_or(1.0);
                assert_eq!(resolved.value(), expected, "height {height} dpi {dpi}");
            }
        }
    }

    #[test]
    fn the_default_window_on_a_scaled_desktop_holds_the_desktop_scale() {
        let desktop = Vector2::new(1.25, 1.25);
        // The defect: 1280x720 raw pixels cannot hold 960x640 at 125 %.
        assert_eq!(
            effective_scale(UiScalePreference::Auto, (1280.0, 720.0), desktop).percent(),
            100
        );
        let wanted = wanted_scale(UiScalePreference::Auto, desktop);
        assert_eq!(wanted.percent(), 125);
        let window = startup_window((1280, 720), wanted, Some((2560, 1440)));
        assert_eq!(window, (1600, 900));
        assert_eq!(
            effective_scale(
                UiScalePreference::Auto,
                (window.0 as f32, window.1 as f32),
                desktop
            )
            .percent(),
            125
        );
    }

    #[test]
    fn the_startup_window_at_100_percent_is_the_default_on_any_monitor() {
        let one = UiScale::default();
        for monitor in [
            None,
            Some((1024, 768)),
            Some((1280, 720)),
            Some((3840, 2160)),
        ] {
            assert_eq!(startup_window((1280, 720), one, monitor), (1280, 720));
        }
    }

    #[test]
    fn the_startup_window_stays_on_the_monitor_and_over_the_floor() {
        let scale = |percent| UiScale::from_percent(percent).unwrap();
        // 1080p at 150 %: the default times 1.5 is the whole monitor.
        let window = startup_window((1280, 720), scale(150), Some((1920, 1080)));
        assert_eq!(window, (1728, 972));
        assert_eq!(
            effective_scale(
                UiScalePreference::Auto,
                (window.0 as f32, window.1 as f32),
                Vector2::new(1.5, 1.5)
            )
            .percent(),
            150
        );
        // A saved 200 % on a 1440p desktop opens a window that holds it.
        let window = startup_window((1280, 720), scale(200), Some((2560, 1440)));
        assert_eq!(window, (2304, 1296));
        // A monitor too small for the wanted scale: the cap wins over the
        // scale, the floor (stepped down to fit) wins over the cap.
        let window = startup_window((1280, 720), scale(125), Some((1366, 768)));
        assert_eq!(window, (1229, 691));
        assert_eq!(window_floor(scale(125), Some((1366, 768))), (960, 640));
        // No monitor named: nothing to cap against.
        assert_eq!(startup_window((1280, 720), scale(175), None), (2240, 1260));
    }

    #[test]
    fn the_window_floor_is_the_logical_minimum_at_the_wanted_rung() {
        let scale = |percent| UiScale::from_percent(percent).unwrap();
        assert_eq!(window_floor(scale(100), None), (960, 640));
        assert_eq!(window_floor(scale(125), Some((2560, 1440))), (1200, 800));
        assert_eq!(window_floor(scale(200), None), (1920, 1280));
        // 1280 is more than four fifths of 1440: one rung down.
        assert_eq!(window_floor(scale(200), Some((2560, 1440))), (1680, 1120));
        // 1080p at 150 %: 960 of 1080 is too much, 800 is not.
        assert_eq!(window_floor(scale(150), Some((1920, 1080))), (1200, 800));
        // Nothing fits: the application's own minimum, never less.
        assert_eq!(window_floor(scale(200), Some((800, 600))), (960, 640));
        // A window at the floor resolves to the floor's rung, not the one below.
        for percent in [100, 125, 150, 175, 200] {
            let floor = window_floor(scale(percent), None);
            assert_eq!(
                effective_scale(
                    UiScalePreference::Fixed(scale(percent)),
                    (floor.0 as f32, floor.1 as f32),
                    Vector2::new(1.0, 1.0)
                )
                .percent(),
                percent
            );
        }
    }

    #[test]
    fn a_fixed_preference_is_wanted_as_given_and_auto_as_the_desktop_rung() {
        let desktop = Vector2::new(1.25, 1.25);
        assert_eq!(
            wanted_scale(UiScalePreference::Auto, desktop).percent(),
            125
        );
        assert_eq!(
            wanted_scale(
                UiScalePreference::Fixed(UiScale::new(1.0).unwrap()),
                desktop
            )
            .percent(),
            100
        );
        assert_eq!(
            wanted_scale(UiScalePreference::Auto, Vector2::new(1.0, 1.0)).percent(),
            100
        );
    }

    #[test]
    fn a_window_grown_for_a_scale_holds_it_where_the_monitor_has_room() {
        let scale = |percent| UiScale::from_percent(percent).unwrap();
        // 1080p, 150 %: the floor stops a rung short, the grow does not.
        assert_eq!(window_floor(scale(150), Some((1920, 1080))), (1200, 800));
        assert_eq!(window_to_hold(scale(150), Some((1920, 1080))), (1440, 960));
        // 175 % needs 1120 of 1080: no room, so the floor's answer.
        assert_eq!(window_to_hold(scale(175), Some((1920, 1080))), (1200, 800));
        // 1440p, 200 %: 1280 fits nine tenths (1296) though not four fifths.
        assert_eq!(window_to_hold(scale(200), Some((2560, 1440))), (1920, 1280));
        assert_eq!(window_to_hold(scale(100), Some((800, 600))), (960, 640));
        assert_eq!(window_to_hold(scale(125), None), (1200, 800));
        // Never under the floor, and a window of that size resolves to the
        // rung wherever it is the exact one.
        for percent in [100, 125, 150, 175, 200] {
            for monitor in [
                None,
                Some((1366, 768)),
                Some((1920, 1080)),
                Some((3840, 2160)),
            ] {
                let floor = window_floor(scale(percent), monitor);
                let hold = window_to_hold(scale(percent), monitor);
                assert!(hold.0 >= floor.0 && hold.1 >= floor.1);
            }
        }
    }

    #[test]
    fn plus_and_minus_are_the_keys_the_layout_names_so() {
        use KeyboardKey as Key;
        let named = |pairs: &[(Key, &str)]| -> Vec<(Key, Option<String>)> {
            SCALE_KEY_CANDIDATES
                .iter()
                .map(|key| {
                    (
                        *key,
                        pairs
                            .iter()
                            .find(|(named, _)| named == key)
                            .map(|(_, name)| (*name).to_string()),
                    )
                })
                .collect()
        };
        // US: `=` and `-` where raylib's names say.
        let us = scale_keys(&named(&[
            (Key::KEY_MINUS, "-"),
            (Key::KEY_EQUAL, "="),
            (Key::KEY_LEFT_BRACKET, "["),
            (Key::KEY_RIGHT_BRACKET, "]"),
            (Key::KEY_SLASH, "/"),
        ]));
        assert_eq!(us.up, vec![Key::KEY_EQUAL]);
        assert_eq!(us.down, vec![Key::KEY_MINUS]);
        // German: `+` right of `ü`, `-` right of `.`; the US positions are
        // `ß` and a dead accent, which GLFW does not name.
        let german = scale_keys(&named(&[
            (Key::KEY_MINUS, "ß"),
            (Key::KEY_RIGHT_BRACKET, "+"),
            (Key::KEY_SLASH, "-"),
            (Key::KEY_BACKSLASH, "#"),
        ]));
        assert_eq!(german.up, vec![Key::KEY_RIGHT_BRACKET]);
        assert_eq!(german.down, vec![Key::KEY_SLASH]);
        // Nothing named: the positions the shortcuts always had.
        let unnamed = scale_keys(&named(&[]));
        assert_eq!(unnamed.up, vec![Key::KEY_EQUAL]);
        assert_eq!(unnamed.down, vec![Key::KEY_MINUS]);
    }
}
