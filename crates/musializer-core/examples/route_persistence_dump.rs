//! Dumps the Rust route persistence over the same corpus and fixture as
//! `tests/differential/route_persistence_oracle.c`.
//!
//! These are the six functions Agent G moved out of `project::model` into
//! [`musializer_core::scene::routes`]. They had to move together: `.musi` v1
//! spells a slider value as a full-range RMS mapping with equal output endpoints
//! and an audio route as the same struct with unequal ones, so the constant rule
//! and the route rule are two halves of one decision. This dump therefore checks
//! them as a pair rather than one at a time.
//!
//! The settings fixture below is written out again rather than shared with the C
//! harness, on purpose: a shared generator can hide the difference the harness
//! exists to find.
//!
//! Run through `tools/differential_route_persistence.sh`.
//!
//! ## Why the fixture is `ORACLE_ALL` and not `ALL`
//!
//! What this harness pins is the **route persistence grammar** — how a mapping
//! is spelled, parsed and paired back up — against a frozen C binary with ten
//! scenes in it. The Rust tree has eleven since 2026-08-08.
//!
//! Sweeping the eleventh through here would not test the grammar harder; it
//! would shift every row's index, make the two sides compare different fixtures,
//! and fail forever for a reason that has nothing to do with parsing. So the
//! fixture stays the ten scenes both sides have, and the eleventh's routes are
//! covered by `a_post_legacy_scene_routes_and_round_trips` in
//! `scene::routes` — which can assert the round trip directly instead of
//! diffing it against an implementation that has no such scene.

use musializer_core::scene::routes::{
    self, AnalysisSource, Interpolation, ParameterMapping, RouteTable,
};
use musializer_core::scene::settings::{self, SettingKind};
use musializer_core::scene::{SceneId, SceneSettings};

/// Route specs applied to the table in this order.
const ROUTE_SPECS: [&str; 5] = [
    "spectrum.amplitude:band:2:0:1:0.4:2.2:smoothstep",
    "loom.weight:rms:0:0:1:0.4:2.5:noclamp",
    "atlas.wireframe:peak:0:0.1:0.9:0:1:step",
    "pulse.glow:spectral_flux:0:0:1:1.5:0.25:ease_in",
    "orbital.hue:beat_phase:0:0:1:-40:40:ease_out:noclamp",
];

/// Everything the grammar must accept, and everything it owes a rejection.
#[rustfmt::skip]
const PARSE_CORPUS: [&str; 35] = [
    "loom.weight:rms:0:0:1:0.4:2.2",
    "settings.loom.weight:rms:0:0:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:2.2:ease_in",
    "loom.weight:rms:0:0:1:0.4:2.2:noclamp",
    "loom.weight:rms:0:0:1:0.4:2.2:noclamp:ease_in",
    "loom.weight:rms:0:0:1:0.4:2.2:ease_in:noclamp",
    "loom.weight:rms:0:0:1:0.4:2.2:noclamp:clamp",
    "loom.weight:rms:0:0:1:0.4:2.2:step",
    "loom.weight:rms:0:0:1:0.4:2.2:linear",
    "loom.weight:rms:0:0:1:0.4:2.2:smoothstep",
    "loom.weight:rms:0:0:1:0.4:2.2:ease_out",
    "loom.weight:band:23:0:1:0.4:2.2",
    "loom.weight:peak:0:0.25:0.75:2.5:0.4",
    "spectrum.amplitude:spectral_flux:0:0:1:0.5:2",
    "spectrum.amplitude:beat_phase:0:0:1:0.5:2",
    "atlas.wireframe:rms:0:0:1:0:1",
    "",
    ":::::::",
    "loom.weight:rms:0:0:1:0.4",
    "loom.weight:rms:0:0:1:0.4:2.2:a:b:c",
    "loom.weight:bogus:0:0:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:2.2:bogus",
    "loom.weight:rms:0:1:0:0.4:2.2",
    "loom.weight:rms:0:0:0:0.4:2.2",
    "loom.weight:rms:3:0:1:0.4:2.2",
    "loom.weight:band:256:0:1:0.4:2.2",
    "loom.weight:band:70000:0:1:0.4:2.2",
    "nope.nothing:rms:0:0:1:0.4:2.2",
    "loom.weight:rms:0:nan:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:inf",
    "loom.weight:rms:0:0:1:1.0:1.0",
    "loom.weight:rms::0:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:2.2:",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:rms:0:0:1:0:1",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:rms:0:0:1:0:1",
];

/// Splits an exported mapping list into the C-era scenes and everything after.
///
/// `export_mappings` walks the whole registry, which is correct — a `.musi` file
/// must carry every scene's values — but it means the list is 93 long here and
/// 81 in the oracle. Partitioning by the mapping's own parameter key (rather
/// than by a count) keeps working if a post-legacy scene is ever inserted
/// somewhere other than the end.
fn split_by_era(mappings: &[ParameterMapping]) -> (Vec<ParameterMapping>, Vec<ParameterMapping>) {
    let mut oracle = Vec::new();
    let mut post = Vec::new();
    for mapping in mappings {
        let in_oracle = settings::descriptor_by_key(&mapping.parameter)
            .is_some_and(|(scene, _, _)| scene.exists_in_oracle());
        if in_oracle {
            oracle.push(mapping.clone());
        } else {
            post.push(mapping.clone());
        }
    }
    (oracle, post)
}

/// Separates the C-era section from everything added after the legacy decision.
const POST_LEGACY_MARKER: &str = "--- post-legacy (no oracle) ---";

fn main() {
    // -- parse ---------------------------------------------------------------
    for (at, spec) in PARSE_CORPUS.into_iter().enumerate() {
        match routes::parse_route_spec(spec) {
            None => println!("parse {at} 0"),
            Some((scene, route)) => {
                print!("parse {at} 1 {} ", scene.index());
                print_mapping_fields(&route);
                println!();
            }
        }
    }

    // -- export, with no routes: every slot is a constant ---------------------
    let settings_values = fixture_settings();
    let mappings = routes::export_mappings(&settings_values, None)
        .expect("the fixture settings are valid, so the constant export cannot refuse");
    let (constant_oracle, constant_post) = split_by_era(&mappings);
    println!("constant-count {}", constant_oracle.len());
    for (at, mapping) in constant_oracle.iter().enumerate() {
        print_mapping("constant", at, mapping);
        println!(
            "constant-is-constant {at} {}",
            u8::from(routes::mapping_is_constant(mapping))
        );
    }
    println!(
        "constant-supported {}",
        u8::from(routes::mappings_supported(&constant_oracle))
    );

    // -- export, with routes replacing slots in place -------------------------
    let mut table = RouteTable::new();
    for (at, spec) in ROUTE_SPECS.into_iter().enumerate() {
        let (scene, route) = routes::parse_route_spec(spec)
            .unwrap_or_else(|| panic!("fixture route rejected: {spec}"));
        println!(
            "table-add {at} {}",
            u8::from(table.add(scene, route).is_ok())
        );
    }

    let mappings = routes::export_mappings(&settings_values, Some(&table))
        .expect("every fixture route targets a descriptor the constant export lists");
    let (routed_oracle, routed_post) = split_by_era(&mappings);
    println!("routed-count {}", routed_oracle.len());
    for (at, mapping) in routed_oracle.iter().enumerate() {
        print_mapping("routed", at, mapping);
    }
    println!(
        "routed-supported {}",
        u8::from(routes::mappings_supported(&routed_oracle))
    );

    // -- import: the round trip, every descriptor out and back ----------------
    let (restored, restored_table) =
        routes::import_mappings(&mappings).expect("the exported list imports");
    for scene in SceneId::ORACLE_ALL {
        for index in 0..settings::count(scene) {
            println!(
                "import-value {} {index} {}",
                scene.index(),
                g9(f64::from(restored.get(scene, index)))
            );
        }
        for (at, route) in restored_table.scene(scene).items().iter().enumerate() {
            print!("import-route {} ", scene.index());
            print_mapping("at", at, route);
        }
    }

    // -- support probes -------------------------------------------------------
    let constant = ParameterMapping {
        parameter: "settings.loom.weight".to_string(),
        source: AnalysisSource::Rms,
        band_index: 0,
        input_min: 0.0,
        input_max: 1.0,
        output_min: 1.0,
        output_max: 1.0,
        interpolation: Interpolation::Linear,
        clamp: true,
    };
    let mut unknown = constant.clone();
    unknown.parameter = "settings.not.a.control".to_string();

    println!(
        "probe-constant 0 {}",
        u8::from(routes::mapping_is_constant(&constant))
    );
    println!(
        "probe-constant 2 {}",
        u8::from(routes::mapping_is_constant(&unknown))
    );
    println!(
        "probe-supported single {}",
        u8::from(routes::mappings_supported(std::slice::from_ref(&constant)))
    );
    println!(
        "probe-supported duplicate {}",
        u8::from(routes::mappings_supported(&[
            constant.clone(),
            constant.clone()
        ]))
    );
    println!(
        "probe-supported unknown {}",
        u8::from(routes::mappings_supported(std::slice::from_ref(&unknown)))
    );

    // Three ways to be *almost* the canonical constant. Each must be refused as a
    // constant and then refused again as a route, because its endpoints are
    // equal — the pairing the whole harness exists to check.
    let mut near = [constant.clone(), constant.clone(), constant];
    near[0].clamp = false;
    near[1].source = AnalysisSource::Band;
    near[1].band_index = 1;
    near[2].interpolation = Interpolation::Step;
    for (at, mapping) in near.iter().enumerate() {
        println!(
            "near-constant {at} {} {}",
            u8::from(routes::mapping_is_constant(mapping)),
            u8::from(routes::mappings_supported(std::slice::from_ref(mapping)))
        );
    }

    // -- everything the oracle has no row for ---------------------------------
    //
    // Below the marker, so the harness can diff the section above against the
    // frozen C byte-for-byte and this one against a pinned file. These mappings
    // go into a real `.musi`, so they are still a compatibility surface — they
    // just have nothing to be compared against except a decision somebody wrote
    // down.
    println!("{POST_LEGACY_MARKER}");
    println!("post-constant-count {}", constant_post.len());
    for (at, mapping) in constant_post.iter().enumerate() {
        print_mapping("post-constant", at, mapping);
        println!(
            "post-constant-is-constant {at} {}",
            u8::from(routes::mapping_is_constant(mapping))
        );
    }
    println!("post-routed-count {}", routed_post.len());
    for (at, mapping) in routed_post.iter().enumerate() {
        print_mapping("post-routed", at, mapping);
    }
    for scene in SceneId::ALL {
        if scene.exists_in_oracle() {
            continue;
        }
        for index in 0..settings::count(scene) {
            println!(
                "post-import-value {} {index} {}",
                scene.index(),
                g9(f64::from(restored.get(scene, index)))
            );
        }
    }
}

/// The settings fixture, written out independently of the C harness's copy.
///
/// The clamp is not decoration: `min + (max - min) * frac` in double can land an
/// ulp outside a float range, and `set` would then refuse the value on one side
/// of the harness and not the other.
fn fixture_settings() -> SceneSettings {
    let mut values = SceneSettings::new();
    for scene in SceneId::ORACLE_ALL {
        for (index, descriptor) in settings::descriptors(scene).iter().enumerate() {
            let frac = f64::from(((scene.index() * 7 + index * 13) % 11) as u32) / 10.0;
            let value = if descriptor.kind == SettingKind::Toggle {
                if frac >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            } else {
                let raw = (f64::from(descriptor.minimum)
                    + f64::from(descriptor.maximum - descriptor.minimum) * frac)
                    as f32;
                raw.clamp(descriptor.minimum, descriptor.maximum)
            };
            if !values.set(scene, index, value) {
                println!(
                    "fixture-refused {} {index} {}",
                    scene.index(),
                    g9(f64::from(value))
                );
            }
        }
    }
    values
}

fn print_mapping(tag: &str, at: usize, mapping: &ParameterMapping) {
    print!("{tag} {at} ");
    print_mapping_fields(mapping);
    println!();
}

fn print_mapping_fields(mapping: &ParameterMapping) {
    print!(
        "{} {} {} {} {} {} {} {} {}",
        mapping.parameter,
        mapping.source as u32,
        mapping.band_index,
        g9(mapping.input_min),
        g9(mapping.input_max),
        g9(mapping.output_min),
        g9(mapping.output_max),
        mapping.interpolation as u32,
        u8::from(mapping.clamp)
    );
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
