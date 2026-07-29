//! The Rust half of `tools/differential_preset_store.sh`.
//!
//! Prints the same four things `tests/differential/preset_store_oracle.c` does:
//! the derived scene tokens, the default-path precedence, the store document's
//! exact JSON bytes, and `merge`'s (imported, skipped) counts.
//!
//! The fixture generator is duplicated between the two sides on purpose
//! (`AGENTS.md`'s differential-testing rule) — a shared one can hide the very
//! difference the harness is looking for.

use musializer_core::project::preset_store::{
    self, PathEnvironment, Preset, PresetLibrary, PresetStoreError,
};
use musializer_core::scene::settings::{self, SceneSettings, SettingKind, PRESETS_PER_SCENE};
use musializer_core::scene::{SceneId, SCENE_COUNT};

/// The same deterministic value the C harness fills a snapshot with, written as
/// one expression in the same order: a differently associated `min + span*t` can
/// land one ULP away and the JSON would then differ.
fn harness_value(descriptor: &settings::SettingDescriptor, setting: usize, preset: usize) -> f32 {
    let t = ((setting * 3 + preset * 7) % 11) as f32 / 10.0;
    if descriptor.kind == SettingKind::Toggle {
        return if t >= 0.5 { 1.0 } else { 0.0 };
    }
    descriptor.minimum + (descriptor.maximum - descriptor.minimum) * t
}

fn fill(values: &mut SceneSettings, scene: SceneId, preset: usize) {
    for (index, descriptor) in settings::descriptors(scene).iter().enumerate() {
        values.set(scene, index, harness_value(descriptor, index, preset));
    }
}

fn dump_library(label: &str, library: &PresetLibrary) {
    println!(
        "{label} next_id {} valid {}",
        library.next_id(),
        u8::from(library.is_valid())
    );
    for (scene_index, scene) in SceneId::ALL.into_iter().enumerate() {
        for (index, preset) in library.presets(scene).iter().enumerate() {
            let Preset { id, name, snapshot } = preset;
            print!(
                "{label} preset {scene_index} {index} {id} {name} {}",
                snapshot.count
            );
            for value in &snapshot.values[..snapshot.count] {
                print!(" {}", format_g9(*value));
            }
            println!();
        }
    }
}

/// C's `%.9g`, which Rust has no direct format for.
///
/// Nine significant digits is not an arbitrary choice: it is exactly what an
/// `f32` needs to round-trip, so two values that print the same here *are* the
/// same value. The algorithm is C99's, 7.21.6.1p8: with precision `P` and decimal
/// exponent `X`, use `%f` at precision `P-1-X` when `P > X >= -4` and `%e` at
/// `P-1` otherwise, then strip trailing zeros and a trailing point.
fn format_g9(value: f32) -> String {
    const PRECISION: i32 = 9;
    if value == 0.0 {
        return "0".to_string();
    }
    // The decimal exponent, taken from Rust's own `{:e}` rather than from
    // `log10().floor()`, which is off by one for values that round up a digit.
    let exponential = format!("{value:e}");
    let exponent: i32 = exponential
        .rsplit_once('e')
        .and_then(|(_, tail)| tail.parse().ok())
        .unwrap_or(0);

    let text = if (-4..PRECISION).contains(&exponent) {
        format!("{:.*}", (PRECISION - 1 - exponent).max(0) as usize, value)
    } else {
        format!("{:.*e}", (PRECISION - 1).max(0) as usize, value)
    };
    if !text.contains('.') {
        return text;
    }
    let (mantissa, suffix) = match text.split_once('e') {
        Some((mantissa, tail)) => (mantissa, format!("e{tail}")),
        None => (text.as_str(), String::new()),
    };
    let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
    format!("{trimmed}{suffix}")
}

fn dump_path(
    label: &str,
    override_value: Option<&str>,
    data_home: Option<&str>,
    home: Option<&str>,
) {
    let resolved = preset_store::default_path(&PathEnvironment {
        preset_store_override: override_value,
        xdg_data_home: data_home,
        home,
    });
    println!("path {label} {}", resolved.as_deref().unwrap_or("none"));
}

/// The C's `Preset_Store_Result` numbering (`preset_store.h:14-23`), so both
/// sides print the same integer. `MISSING` is 1 and never reached here.
fn result_code(result: Result<(), PresetStoreError>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(PresetStoreError::Argument) => 2,
        Err(PresetStoreError::Path) => 3,
        Err(PresetStoreError::Read) => 4,
        Err(PresetStoreError::Format) => 5,
        Err(PresetStoreError::Directory) => 6,
        Err(PresetStoreError::Write) => 7,
    }
}

fn main() {
    // 1. Scene tokens, both directions.
    for scene_index in 0..=SCENE_COUNT {
        let Some(scene) = SceneId::from_index(scene_index) else {
            println!("token {scene_index} none");
            continue;
        };
        let Some(token) = preset_store::scene_token(scene) else {
            println!("token {scene_index} none");
            continue;
        };
        println!("token {scene_index} {token}");
        let back = preset_store::scene_from_token(token);
        println!(
            "from_token {token} {}",
            if back.is_some() { "yes" } else { "no" }
        );
        println!(
            "from_token_index {token} {}",
            back.map_or(scene_index, SceneId::index)
        );
    }
    for token in ["", "nope", "Loom", "loom.", " loom"] {
        // The C leaves its out-parameter untouched on refusal, and its caller
        // seeds it with 999. Same here, so the refusal is compared and not just
        // the boolean.
        let back = preset_store::scene_from_token(token);
        println!(
            "from_token_bad [{token}] {} {}",
            if back.is_some() { "yes" } else { "no" },
            back.map_or(999, SceneId::index)
        );
    }

    // 2. The path policy's precedence.
    dump_path(
        "override_wins",
        Some("/tmp/override.json"),
        Some("/xdg"),
        Some("/home/u"),
    );
    dump_path(
        "empty_override_ignored",
        Some(""),
        Some("/xdg"),
        Some("/home/u"),
    );
    dump_path("xdg", None, Some("/xdg"), Some("/home/u"));
    dump_path("empty_xdg_falls_back", None, Some(""), Some("/home/u"));
    dump_path("home", None, None, Some("/home/u"));
    dump_path("nothing", None, None, None);
    dump_path("empty_home", None, None, Some(""));

    // 3. The store document's exact bytes, then the load round trip.
    let mut values = SceneSettings::new();
    let mut library = PresetLibrary::new();
    for (scene_index, scene) in SceneId::ALL.into_iter().enumerate() {
        let presets = if scene_index == 0 { 3 } else { 2 };
        for k in 0..presets {
            let name = format!("Preset {}", library.next_id());
            fill(&mut values, scene, k);
            let snapshot = values.capture(scene).expect("a capturable scene");
            if library.push(scene, &name, &snapshot).is_none() {
                println!("save_failed {scene_index} {k}");
            }
        }
    }
    dump_library("built", &library);

    let encoded = preset_store::save_to_string(&library);
    println!(
        "store_save {}",
        result_code(encoded.as_ref().map(|_| ()).map_err(|error| *error))
    );
    match &encoded {
        Ok(text) => println!("store_bytes {}", text.replace('\n', "\\n")),
        Err(_) => println!("store_bytes none"),
    }
    let reloaded = encoded
        .as_ref()
        .map_err(|error| *error)
        .and_then(|text| preset_store::load_from_bytes(text.as_bytes()));
    println!(
        "store_load {}",
        result_code(reloaded.as_ref().map(|_| ()).map_err(|error| *error))
    );
    dump_library(
        "reloaded",
        reloaded.as_ref().unwrap_or(&PresetLibrary::new()),
    );

    // 4. Merge, whose identity is (scene, exact values) and never the name.
    let first = SceneId::ALL[0];
    let mut destination = PresetLibrary::new();
    let mut source = PresetLibrary::new();
    fill(&mut values, first, 0);
    let same = values.capture(first).expect("a capturable scene");
    destination
        .push(first, "kept", &same)
        .expect("room for one");
    source.push(first, "renamed", &same).expect("room for one");
    fill(&mut values, first, 4);
    let different = values.capture(first).expect("a capturable scene");
    source
        .push(first, "kept", &different)
        .expect("room for one");
    let merged = preset_store::merge(&mut destination, &source);
    println!("merge_ok {}", u8::from(merged.is_ok()));
    let (imported, skipped) = merged.unwrap_or((0, 0));
    println!("merge_counts {imported} {skipped}");
    dump_library("merged", &destination);

    let second = SceneId::ALL[1];
    let mut full = PresetLibrary::new();
    let mut extra = PresetLibrary::new();
    for k in 0..PRESETS_PER_SCENE {
        fill(&mut values, second, k);
        let snapshot = values.capture(second).expect("a capturable scene");
        full.push(second, &format!("full{k}"), &snapshot)
            .expect("room for eight");
    }
    fill(&mut values, second, 9);
    let overflow = values.capture(second).expect("a capturable scene");
    extra
        .push(second, "overflow", &overflow)
        .expect("room for one");
    let merged = preset_store::merge(&mut full, &extra);
    println!("merge_full_ok {}", u8::from(merged.is_ok()));
    let (imported, skipped) = merged.unwrap_or((0, 0));
    println!("merge_full_counts {imported} {skipped}");
}
