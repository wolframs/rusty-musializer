//! Dumps the Rust route evaluation over the same sweep as
//! `tests/differential/routes_oracle.c`, for every curve and both clamp settings.
//!
//! Run through `tools/differential_routes.sh`.

use musializer_core::scene::routes::{AnalysisSource, Interpolation, ParameterMapping};
use musializer_core::scene::settings;

const TARGETS: [&str; 2] = ["settings.loom.weight", "settings.atlas.wireframe"];

fn main() {
    for target in TARGETS {
        let (_scene, _index, descriptor) =
            settings::descriptor_by_key(target).expect("target parameter exists");

        for (curve_index, interpolation) in Interpolation::ALL.into_iter().enumerate() {
            for clamp in [false, true] {
                let route = ParameterMapping {
                    parameter: target.to_string(),
                    source: AnalysisSource::Rms,
                    band_index: 0,
                    input_min: 0.0,
                    input_max: 1.0,
                    output_min: 0.4,
                    output_max: 2.2,
                    interpolation,
                    clamp,
                };

                // Sweeps outside 0..1 so the clamped and unclamped paths diverge
                // where they are supposed to.
                for step in -4..=14 {
                    let source = f64::from(step) / 10.0;
                    let evaluated = route.evaluate(source);
                    let mapped = route.output_value(descriptor, source);
                    println!(
                        "{target} {curve_index} {} {step} {} {} {} {}",
                        u8::from(clamp),
                        u8::from(evaluated.is_some()),
                        g9(evaluated.unwrap_or(0.0)),
                        u8::from(mapped.is_some()),
                        g9(mapped.unwrap_or(0.0)),
                    );
                }
            }
        }
    }
}

/// Formats like C's `%.9g`.
fn g9(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let exponent = value.abs().log10().floor() as i32;
    if (-5..9).contains(&exponent) {
        let precision = (8 - exponent).max(0) as usize;
        let s = format!("{value:.precision$}");
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        return trimmed.to_string();
    }
    let s = format!("{value:.8e}");
    let (mantissa, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    format!("{mantissa}e{exp:+03}")
}
