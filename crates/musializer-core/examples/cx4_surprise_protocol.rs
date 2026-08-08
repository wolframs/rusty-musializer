//! Emit the CX-4 Surprise keepability protocol (HX ∘ CX-4).
//!
//! CX-4 revised three of Surprise's five constants and replaced the blanket
//! endpoint inset with per-descriptor metadata. Whether the revised draws are
//! *keepable* needs ears, and the comparison has to be blind — which is what
//! the HX protocol runner exists for. This example prints one track's
//! `musializer.protocol/v1` to **stdout** and the unblinding key to **stderr**;
//! the driver script writes them to two files and tells the operator not to
//! open the key until the answers are in.
//!
//! # The frozen sampler
//!
//! "Current" draws must come from the sampler as it was **before** CX-4 — but
//! that code is gone from the tree, deliberately. It is frozen here instead,
//! and the freeze is *checked*, not trusted: run with `--self-check`, this
//! example reproduces the headless gate's old pinned draw (seed 4242 through
//! the probe's seed transform) byte-for-byte, and the library's new draw,
//! which must differ. A frozen copy that drifted would fail that check, and a
//! CX-4 change that quietly didn't change the sampler would too.
//!
//! Usage:
//!   cargo run --example cx4_surprise_protocol -- --self-check
//!   cargo run --example cx4_surprise_protocol -- \
//!       --audio PATH --sha256 HEX --duration SECONDS --track 0|1 --title TITLE \
//!       > x.protocol.json 2> x.key.json

use musializer_core::feedback::{AnswerKind, Apply, Protocol, ProtocolItem, Window};
use musializer_core::scene::settings::{
    self, SceneSettings, SettingDescriptor, SettingKind, SettingsSnapshot, MAX_CONTROLS,
};
use musializer_core::scene::SceneId;
use musializer_core::ui::tune_explore::{self, conform, RandomSource, SplitMix64, Strength};

// -- the pre-CX-4 sampler, frozen ----------------------------------------------
//
// A verbatim transcription of `tune_explore.rs`'s Surprise path as of commit
// 8a49415^ — constants 0.75/0.25, a 5 % inset on every end of every slider,
// and "circular" inferred from the bounds. Nothing here is called by the
// application; it exists so the comparison protocol can carry draws from a
// sampler the tree no longer contains.

const OLD_SURPRISE_INSET: f32 = 0.05;
const OLD_SURPRISE_MOVE_CHANCE: f64 = 0.75;
const OLD_SURPRISE_TOGGLE_CHANCE: f64 = 0.25;

fn old_triangular(rng: &mut impl RandomSource, low: f64, high: f64, mode: f64) -> f64 {
    if high <= low {
        return low;
    }
    let mode = mode.clamp(low, high);
    let span = high - low;
    let u = rng.next_unit();
    let split = (mode - low) / span;
    if u < split {
        low + (u * span * (mode - low)).sqrt()
    } else {
        high - ((1.0 - u) * span * (high - mode)).sqrt()
    }
}

fn old_is_angle(descriptor: &SettingDescriptor) -> bool {
    descriptor.minimum < 0.0 && descriptor.maximum > 0.0
}

fn old_explore_value(
    rng: &mut impl RandomSource,
    descriptor: &SettingDescriptor,
    current: f32,
) -> f32 {
    if descriptor.kind == SettingKind::Toggle {
        return if rng.chance(OLD_SURPRISE_TOGGLE_CHANCE) {
            conform(descriptor, 1.0 - current)
        } else {
            conform(descriptor, current)
        };
    }
    if !rng.chance(OLD_SURPRISE_MOVE_CHANCE) {
        return conform(descriptor, current);
    }
    let span = descriptor.maximum - descriptor.minimum;
    let low = f64::from(descriptor.minimum + span * OLD_SURPRISE_INSET);
    let high = f64::from(descriptor.maximum - span * OLD_SURPRISE_INSET);
    let value = if old_is_angle(descriptor) {
        rng.next_range(low, high)
    } else {
        old_triangular(rng, low, high, f64::from(descriptor.default_value))
    };
    // The old conform and the new are the same function; only the sampling
    // around it changed.
    conform(descriptor, value as f32)
}

fn old_explore(
    rng: &mut impl RandomSource,
    scene: SceneId,
    base: &SceneSettings,
) -> SettingsSnapshot {
    let descriptors = settings::descriptors(scene);
    let mut values = [0.0f32; MAX_CONTROLS];
    let mut changed = false;
    for (index, descriptor) in descriptors.iter().enumerate() {
        let current = base.get(scene, index);
        let next = old_explore_value(rng, descriptor, current);
        values[index] = next;
        changed |= next.to_bits() != conform(descriptor, current).to_bits();
    }
    if !changed {
        let pick = (rng.next_u64() as usize) % descriptors.len().max(1);
        if let Some(descriptor) = descriptors.get(pick) {
            for _ in 0..16 {
                let candidate = old_explore_value(rng, descriptor, values[pick]);
                if candidate.to_bits() != values[pick].to_bits() {
                    values[pick] = candidate;
                    break;
                }
            }
        }
    }
    SettingsSnapshot {
        captured: true,
        count: descriptors.len(),
        values,
    }
}

fn new_explore(
    rng: &mut impl RandomSource,
    scene: SceneId,
    base: &SceneSettings,
) -> SettingsSnapshot {
    tune_explore::explore(rng, scene, base, Strength::Surprise)
}

// -- the freeze check ----------------------------------------------------------

/// The probe path's seed transform (`tune.rs::next_explore_seed`, one press):
/// what `tune-seed=4242` actually feeds `SplitMix64::new` for a given scene.
fn probe_seed(seed: u64, scene: SceneId) -> u64 {
    seed ^ 1u64.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((scene.index() as u64) << 48)
}

fn values_line(snapshot: &SettingsSnapshot) -> String {
    let values: Vec<String> = snapshot.values[..snapshot.count]
        .iter()
        .map(|value| value.to_string())
        .collect();
    values.join(" ")
}

fn self_check() {
    let base = SceneSettings::default();
    let mut rng = SplitMix64::new(probe_seed(4242, SceneId::Spectrum));
    let frozen = old_explore(&mut rng, SceneId::Spectrum, &base);
    let frozen_line = values_line(&frozen);
    // The headless gate's pin as it stood from PX6 until CX-4 re-pinned it.
    assert_eq!(
        frozen_line, "0.68 1 1 4.28 105 1.77 0.74 0.9",
        "the frozen pre-CX-4 sampler no longer reproduces the old gate pin"
    );

    let mut rng = SplitMix64::new(probe_seed(4242, SceneId::Spectrum));
    let revised = new_explore(&mut rng, SceneId::Spectrum, &base);
    let revised_line = values_line(&revised);
    assert_eq!(
        revised_line, "0.68 1 1 3 55 1 0.5 0.3",
        "the library's draw does not match CX-4's re-pinned gate value"
    );
    assert_ne!(frozen_line, revised_line);
    println!("self-check ok: frozen sampler reproduces the old pin, revised differs");
}

// -- protocol construction -----------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sampler {
    Current,
    Revised,
}

impl Sampler {
    fn token(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Revised => "revised",
        }
    }

    fn draw(self, seed: u64, scene: SceneId, base: &SceneSettings) -> SettingsSnapshot {
        let mut rng = SplitMix64::new(seed);
        match self {
            Self::Current => old_explore(&mut rng, scene, base),
            Self::Revised => new_explore(&mut rng, scene, base),
        }
    }
}

struct KeyRow {
    id: String,
    scene: SceneId,
    detail: String,
}

/// Fisher-Yates over a small vec, seeded — the blind depends on the C/R
/// assignment not being positional.
fn shuffle<T>(items: &mut [T], rng: &mut impl RandomSource) {
    for index in (1..items.len()).rev() {
        let swap = (rng.next_u64() as usize) % (index + 1);
        items.swap(index, swap);
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--self-check") {
        self_check();
        return;
    }
    let value_of = |flag: &str| -> String {
        arguments
            .iter()
            .position(|argument| argument == flag)
            .and_then(|index| arguments.get(index + 1))
            .unwrap_or_else(|| panic!("missing {flag}"))
            .clone()
    };
    let audio = value_of("--audio");
    let sha256 = value_of("--sha256");
    let duration: f64 = value_of("--duration").parse().expect("--duration SECONDS");
    let track: u64 = value_of("--track").parse().expect("--track 0|1");
    let title = value_of("--title");

    // The freeze check runs before every emission, so a drifted frozen
    // sampler can never silently produce a mislabelled protocol.
    let base = SceneSettings::default();
    {
        let mut rng = SplitMix64::new(probe_seed(4242, SceneId::Spectrum));
        assert_eq!(
            values_line(&old_explore(&mut rng, SceneId::Spectrum, &base)),
            "0.68 1 1 4.28 105 1.77 0.74 0.9",
            "frozen sampler drifted; do not trust this protocol"
        );
    }

    let scenes = [SceneId::SongAtlas, SceneId::Cadence];
    // Across both tracks each scene gets exactly 5 current and 5 revised
    // single draws; the per-track split alternates so neither file is "the
    // current one".
    let singles_split: [[usize; 2]; 2] = match track {
        0 => [[3, 2], [2, 3]], // [scene][current, revised]
        _ => [[2, 3], [3, 2]],
    };

    let mut shuffle_rng = SplitMix64::new(0xCB4_5EED ^ (track.wrapping_mul(0x9E37_79B9)));
    let mut items: Vec<(f64, ProtocolItem, KeyRow)> = Vec::new();

    for (scene_index, scene) in scenes.into_iter().enumerate() {
        let [current_count, revised_count] = singles_split[scene_index];
        let mut samplers: Vec<Sampler> = std::iter::repeat_n(Sampler::Current, current_count)
            .chain(std::iter::repeat_n(Sampler::Revised, revised_count))
            .collect();
        shuffle(&mut samplers, &mut shuffle_rng);

        // Five anchors spread over the middle of the track, offset per scene
        // so the two scenes interleave rather than stack.
        for (draw_index, sampler) in samplers.iter().enumerate() {
            let fraction = 0.18 + 0.13 * draw_index as f64 + 0.05 * scene_index as f64;
            let at = (duration * fraction).min(duration - 15.0).max(5.0);
            let seed = 100 * (track + 1) + 100 * scene_index as u64 + draw_index as u64 + 1;
            let snapshot = sampler.draw(seed, scene, &base);
            items.push((
                at,
                ProtocolItem {
                    id: String::new(), // named after the time sort below
                    at_seconds: at,
                    window: Window {
                        pre: 1.5,
                        post: 10.0,
                    },
                    question: format!(
                        "{}: would you keep this look for this track?",
                        scene.display_name()
                    ),
                    kind: AnswerKind::Choice,
                    options: vec![
                        "keep".to_string(),
                        "interesting but needs fixing".to_string(),
                        "reject".to_string(),
                    ],
                    apply: Some(Apply {
                        scene,
                        seed: Some(seed),
                        a: snapshot,
                        b: None,
                    }),
                },
                KeyRow {
                    id: String::new(),
                    scene,
                    detail: format!("single sampler={} seed={seed}", sampler.token()),
                },
            ));
        }

        // One blind A/B per scene per track: a current draw against a revised
        // draw, the a/b assignment itself shuffled — the runner already
        // shuffles which plays first and records the order it played.
        let ab_seed = 500 * (track + 1) + scene_index as u64 + 1;
        let current_draw = Sampler::Current.draw(ab_seed, scene, &base);
        let revised_draw = Sampler::Revised.draw(ab_seed.wrapping_add(17), scene, &base);
        let revised_is_a = shuffle_rng.chance(0.5);
        let (a, b, a_token, b_token) = if revised_is_a {
            (revised_draw, current_draw, "revised", "current")
        } else {
            (current_draw, revised_draw, "current", "revised")
        };
        let at = (duration * (0.86 + 0.04 * scene_index as f64))
            .min(duration - 14.0)
            .max(5.0);
        items.push((
            at,
            ProtocolItem {
                id: String::new(),
                at_seconds: at,
                window: Window {
                    pre: 1.5,
                    post: 9.0,
                },
                question: format!(
                    "{}: two looks — B swaps between them. Which would you keep?",
                    scene.display_name()
                ),
                kind: AnswerKind::Choice,
                options: vec![
                    "the first look".to_string(),
                    "the second look".to_string(),
                    "either".to_string(),
                    "neither".to_string(),
                ],
                apply: Some(Apply {
                    scene,
                    seed: Some(ab_seed),
                    a,
                    b: Some(b),
                }),
            },
            KeyRow {
                id: String::new(),
                scene,
                detail: format!("ab a={a_token} b={b_token} seed={ab_seed}"),
            },
        ));
    }

    // Ids in listening order, opaque about provenance: q01, q02, ...
    items.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut protocol_items = Vec::new();
    let mut key_rows = Vec::new();
    for (index, (_, mut item, mut key)) in items.into_iter().enumerate() {
        let id = format!("q{:02}", index + 1);
        item.id.clone_from(&id);
        key.id = id;
        protocol_items.push(item);
        key_rows.push(key);
    }

    let protocol = Protocol {
        title,
        audio_path: audio,
        audio_sha256: sha256.to_ascii_lowercase(),
        items: protocol_items,
    };
    // Emitting through the same codec the app parses is its own round-trip
    // check: an invalid item would refuse here, not in the operator's session.
    let json = protocol.to_json_pretty();
    Protocol::parse(json.as_bytes()).expect("the emitted protocol must parse");
    print!("{json}");

    // The unblinding key, to stderr: scene, sampler and seed per item, plus
    // which label carries which sampler in the A/B items. The operator is
    // told not to open this until the answers are in.
    eprintln!("{{");
    eprintln!("  \"note\": \"UNBLINDING KEY - do not open before answering\",");
    eprintln!("  \"track\": {track},");
    eprintln!("  \"items\": [");
    let last = key_rows.len().saturating_sub(1);
    for (index, row) in key_rows.iter().enumerate() {
        let comma = if index == last { "" } else { "," };
        eprintln!(
            "    {{ \"id\": \"{}\", \"scene\": \"{}\", \"key\": \"{}\" }}{comma}",
            row.id,
            row.scene.stable_name(),
            row.detail
        );
    }
    eprintln!("  ]");
    eprintln!("}}");
}
