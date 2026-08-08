//! Dumps the Rust scene-settings descriptor table in the same format as
//! `tests/differential/settings_oracle.c`, so the hand-transcribed table can be
//! verified against the frozen C mechanically rather than by eye.
//!
//! Run through `tools/differential_settings.sh`.
//!
//! ## Two sections, since 2026-08-08
//!
//! The tree now has a scene the frozen C does not (Phosphor Dream, id 10). The
//! oracle cannot dump a table for it, so a single-file diff would fail forever
//! and the honest contract — *the C-era descriptors are still byte-exact* —
//! would be lost in the noise.
//!
//! So the dump prints the C-era ten, then [`POST_LEGACY_MARKER`], then
//! everything added since. The harness diffs the first section against the
//! oracle exactly and the second against a checked-in expectation. Changing a
//! C-era bound still fails against the C; changing a post-legacy one fails
//! against a file somebody has to update on purpose. Neither can drift quietly.

use musializer_core::scene::settings::{self, SettingKind};
use musializer_core::scene::SceneId;

/// Separates the C-era table from everything added after the legacy decision.
pub const POST_LEGACY_MARKER: &str = "--- post-legacy (no oracle) ---";

fn main() {
    let mut marked = false;
    for scene in SceneId::ALL {
        if !scene.exists_in_oracle() && !marked {
            println!("{POST_LEGACY_MARKER}");
            marked = true;
        }
        let descriptors = settings::descriptors(scene);
        println!("scene {} count {}", scene.index(), descriptors.len());
        for (index, d) in descriptors.iter().enumerate() {
            // `{:.9}` would print trailing zeros where C's `%.9g` prints none, so
            // the float fields are formatted to match `%.9g`: shortest form that
            // round-trips at nine significant digits.
            println!(
                "{} {} {}|{}|{}|{}|{}|{}|{}",
                scene.index(),
                index,
                d.key,
                d.label,
                g9(d.minimum),
                g9(d.maximum),
                g9(d.default_value),
                d.precision,
                match d.kind {
                    SettingKind::Slider => 0,
                    SettingKind::Toggle => 1,
                },
            );
        }
    }
}

/// Formats like C's `%.9g`: nine significant digits, no trailing zeros, no
/// unnecessary decimal point.
fn g9(value: f32) -> String {
    let mut s = format!("{:.*e}", 8, f64::from(value));
    // Convert Rust's `4.00000000e-1` into a plain decimal the way %g does for
    // exponents in [-5, 9), which covers every value in this table.
    if let Some((mantissa, exponent)) = s.split_once('e') {
        let exponent: i32 = exponent.parse().unwrap();
        if (-5..9).contains(&exponent) {
            let precision = (8 - exponent).max(0) as usize;
            s = format!("{:.*}", precision, f64::from(value));
            let trimmed = s.trim_end_matches('0').trim_end_matches('.');
            return trimmed.to_string();
        }
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{mantissa}e{exponent:+03}");
    }
    s
}
