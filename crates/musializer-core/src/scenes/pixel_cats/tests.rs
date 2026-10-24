//! Pixel Cats' evidence: the group's contract, every staging drawing and
//! reacting, and the shared passes.
//!
//! The reaction tests are the negative-control half of the scene check: each
//! staging is fed the same three seconds twice — once over a synthetic groove
//! with kicks, hats, a vocal and onsets, once over near-silence — and the two
//! frames must differ at `response` 1 and be **byte-identical** at `response` 0.
//! The second assertion is what proves the first is the music and not the seed,
//! the clock or the line.

use super::*;
use crate::scene::settings::{self, SceneSettings};
use crate::scene::{LyricCue, SceneAudioFrame};

const BANDS: usize = 40;

/// A synthetic track: kicks every third of a second, hats between them, a
/// vocal level in the mids and an onset on every kick. `loud == false` is the
/// near-silent control.
struct Track {
    loud: bool,
    bands: [f32; BANDS],
    trails: [f32; BANDS],
}

impl Track {
    fn new(loud: bool) -> Self {
        Self {
            loud,
            bands: [0.02; BANDS],
            trails: [0.02; BANDS],
        }
    }

    fn step(&mut self, time: f64) {
        let beat = (time * 3.0).fract() as f32;
        let hat = ((time * 3.0 + 0.5).fract()) as f32;
        for (i, band) in self.bands.iter_mut().enumerate() {
            let target = if !self.loud {
                0.02
            } else if i < BANDS / 4 {
                0.25 + 0.7 * (-beat * 9.0).exp()
            } else if i >= BANDS * 3 / 4 {
                0.2 + 0.6 * (-hat * 14.0).exp()
            } else {
                0.55 + 0.15 * (time as f32 * 2.0).sin()
            };
            *band = target;
        }
        for (trail, band) in self.trails.iter_mut().zip(self.bands.iter()) {
            *trail += (band - *trail) * 0.18;
        }
    }

    fn audio(&self, time: f64) -> SceneAudioFrame<'_> {
        let mut audio = SceneAudioFrame::from_spectrum(&self.bands, &self.trails);
        audio.beat_phase = (time * 3.0).fract() as f32;
        if self.loud {
            audio.onset = (time * 3.0).fract() < 0.05;
        } else {
            audio.onset = false;
        }
        audio
    }
}

fn cue(text: &str, start: f64, end: f64) -> LyricCue {
    LyricCue {
        id: 7,
        start_seconds: start,
        end_seconds: end,
        text: text.to_string(),
        voice: None,
    }
}

/// Runs `staging` from 0 to `until` seconds at 60 fps and renders the frame
/// at `until`.
fn run(
    staging: Staging,
    loud: bool,
    settings: &SceneSettings,
    lyric: Option<&LyricCue>,
    until: f64,
) -> (PixelCatsState, Vec<u8>, PixelReport) {
    let mut state = PixelCatsState::new(staging, 42);
    let mut track = Track::new(loud);
    let steps = (until * 60.0).round() as u64;
    let mut last = (Vec::new(), PixelReport::default());
    for step in 0..=steps {
        let time = step as f64 / 60.0;
        track.step(time);
        let mut frame = SceneFrame::idle(settings);
        frame.time_seconds = time;
        frame.duration_seconds = 120.0;
        frame.delta_seconds = if step == 0 { 0.0 } else { 1.0 / 60.0 };
        frame.frame_index = step;
        frame.audio = track.audio(time);
        frame.lyric = lyric.filter(|cue| time >= cue.start_seconds && time < cue.end_seconds);
        state.update(&frame);
        if step == steps {
            let mut raster = Raster::new(16, 16);
            let mut rgba = Vec::new();
            let report = render(&state, &frame, 320, 180, &mut raster, &mut rgba);
            last = (rgba, report);
        }
    }
    (state, last.0, last.1)
}

fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).abs())
        .sum::<f64>()
        / a.len() as f64
}

/// Not a flat field: at least three colours and no one colour over 95 %.
/// (A four-shade palette caps the count, so the count alone is no test.)
fn not_flat(rgba: &[u8]) -> bool {
    let mut counts = std::collections::HashMap::new();
    for pixel in rgba.chunks_exact(4) {
        *counts
            .entry([pixel[0], pixel[1], pixel[2]])
            .or_insert(0usize) += 1;
    }
    let top = counts.values().copied().max().unwrap_or(0);
    counts.len() >= 3 && top * 100 < rgba.len() / 4 * 95
}

fn distinct_colours(rgba: &[u8]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for pixel in rgba.chunks_exact(4) {
        seen.insert([pixel[0], pixel[1], pixel[2]]);
    }
    seen.len()
}

fn with(scene: SceneId, index: usize, value: f32) -> SceneSettings {
    let mut settings = SceneSettings::new();
    assert!(settings.set(scene, index, value));
    settings
}

#[test]
fn six_stagings_six_scenes_one_settings_shape() {
    let mut ids: Vec<SceneId> = Staging::ALL.iter().map(|s| s.scene()).collect();
    ids.dedup();
    assert_eq!(ids.len(), 6);
    let first = settings::descriptors(SceneId::PixelBoot);
    for staging in Staging::ALL {
        let scene = staging.scene();
        assert_eq!(Staging::of(scene), Some(staging));
        assert!(scene.stable_name().starts_with("pixel-"), "{scene:?}");
        assert!(!scene.exists_in_oracle());
        let table = settings::descriptors(scene);
        assert_eq!(
            table.len(),
            first.len(),
            "{scene:?} must keep the group's shape"
        );
        for (index, (mine, theirs)) in table.iter().zip(first).enumerate() {
            let short = |key: &str| key.rsplit_once('.').unwrap().1.to_string();
            assert_eq!(short(mine.key), short(theirs.key));
            assert!(mine
                .key
                .starts_with(&format!("settings.{}.", scene.stable_name())));
            assert_eq!(mine.minimum, theirs.minimum);
            assert_eq!(mine.maximum, theirs.maximum);
            assert_eq!(mine.precision, theirs.precision);
            assert_eq!(mine.kind, theirs.kind);
            if index != control::PALETTE {
                assert_eq!(mine.default_value, theirs.default_value, "{}", mine.key);
            }
        }
        let state = PixelCatsState::new(staging, 1);
        assert_eq!(state.id(), scene);
    }
    assert_eq!(Staging::of(SceneId::Stage), None);
    // The persisted palette order.
    for (index, palette) in Palette::ALL.into_iter().enumerate() {
        assert_eq!(palette as usize, index);
        assert_eq!(Palette::from_setting(index as f32), palette);
    }
    assert_eq!(Palette::from_setting(f32::NAN), Palette::Colour);
}

#[test]
fn the_canvas_is_about_180_rows_at_an_integer_factor_in_every_shape() {
    assert_eq!(canvas_size(1920.0, 1080.0, 180.0), (320, 180, 6));
    assert_eq!(canvas_size(1280.0, 720.0, 180.0), (320, 180, 4));
    assert_eq!(canvas_size(3840.0, 2160.0, 180.0), (320, 180, 12));
    assert_eq!(canvas_size(1080.0, 1920.0, 180.0), (180, 320, 6));
    assert_eq!(canvas_size(1080.0, 1080.0, 180.0), (180, 180, 6));
    assert_eq!(canvas_size(1080.0, 1350.0, 180.0), (180, 225, 6));
    assert_eq!(canvas_size(1920.0, 1080.0, 120.0), (213, 120, 9));
    for (w, h) in [
        (800.0, 450.0),
        (470.0, 264.0),
        (97.0, 61.0),
        (5000.0, 400.0),
    ] {
        let (cw, ch, k) = canvas_size(w, h, 180.0);
        assert!(k >= 1);
        // The canvas never overhangs the output, and the bezel is under one
        // factor's worth of pixels on each axis unless the canvas hit its cap.
        assert!(cw * k <= w as i32 && ch * k <= h as i32, "{w}x{h}");
        if cw < MAX_CANVAS {
            assert!(w as i32 - cw * k < k.max(1) + 1, "{w}x{h}");
        }
        assert!(ch * k > h as i32 - k - 1, "{w}x{h}");
    }
    assert_eq!(canvas_size(f32::NAN, 10.0, 180.0), (16, 16, 1));
}

#[test]
fn a_typed_line_lands_inside_the_first_half_of_its_cue() {
    for (text, duration) in [
        ("SUDO MEOW", 0.6),
        ("I found a terminal behind the drums", 4.0),
        ("I am root and I sing falsetto now", 1.2),
    ] {
        let at = |t: f64| Line::new(1, text, 10.0, 10.0 + duration, 10.0 + t, 1.0);
        assert_eq!(at(0.0).typed, 0);
        assert!(at(duration * 0.3).typed > 0, "{text}");
        assert!(at(duration * 0.56).complete(), "{text} at {duration}");
        assert!(at(duration * 0.99).complete());
        // The sung word walks forward and ends on the last word.
        assert_eq!(at(duration * 0.995).sung, at(0.0).words.len() - 1);
        assert_eq!(at(0.0).sung, 0);
    }
    let tagged = Line::new(1, "[Belted] I typed belted", 0.0, 2.0, 1.9, 1.0);
    assert_eq!(tagged.text, "I TYPED BELTED");
    assert_eq!(tagged.sung_word(), "BELTED");
    assert_eq!(leading_tag("[Belted] x").as_deref(), Some("BELTED"));
    assert_eq!(leading_tag("no tag"), None);
    assert_eq!(without_tag("  [x]  hello "), "hello");
}

#[test]
fn every_staging_draws_its_line_and_a_picture_without_one() {
    let settings = SceneSettings::new();
    let line = cue("I found a terminal behind the drums", 0.5, 4.5);
    for staging in Staging::ALL {
        let (_, with_line, report) = run(staging, true, &settings, Some(&line), 3.0);
        assert_eq!(report.line, "complete 35", "{staging:?}");
        assert!(
            report.glyphs >= 20,
            "{staging:?} drew {} glyphs",
            report.glyphs
        );
        assert!(not_flat(&with_line), "{staging:?} is flat");
        assert!(report.cats >= 1, "{staging:?} drew no cat");
        let (_, without, idle) = run(staging, true, &settings, None, 3.0);
        assert_eq!(idle.line, "none");
        assert!(not_flat(&without), "{staging:?} is flat with no line");
        assert!(
            mean_difference(&with_line, &without) > 0.5,
            "{staging:?} ignores the line"
        );
        assert_eq!(report.staging, staging.name());
    }
}

#[test]
fn every_staging_reacts_to_the_music_and_response_zero_shuts_it_out() {
    let line = cue("Kyrie meow ison", 0.5, 4.5);
    for staging in Staging::ALL {
        let scene = staging.scene();
        let live = SceneSettings::new();
        let (state, loud, loud_report) = run(staging, true, &live, Some(&line), 3.0);
        let (_, quiet, _) = run(staging, false, &live, Some(&line), 3.0);
        assert!(
            state.kicks() > 4,
            "{staging:?}: the groove's kicks were not heard"
        );
        assert!(state.ticks() > 4, "{staging:?}: the hats were not heard");
        assert!(loud_report.kicks > 0);
        let reacts = mean_difference(&loud, &quiet);
        assert!(
            reacts > 1.0,
            "{staging:?}: music moved the frame by only {reacts:.3}"
        );
        let deaf = with(scene, control::RESPONSE, 0.0);
        let (_, a, _) = run(staging, true, &deaf, Some(&line), 3.0);
        let (_, b, _) = run(staging, false, &deaf, Some(&line), 3.0);
        assert!(
            a == b,
            "{staging:?}: audio leaked through response 0 ({:.3})",
            mean_difference(&a, &b)
        );
    }
}

#[test]
fn the_same_track_draws_the_same_bytes() {
    let settings = SceneSettings::new();
    let line = cue("meow oh meow oh meow", 0.2, 2.2);
    for staging in Staging::ALL {
        let (_, a, _) = run(staging, true, &settings, Some(&line), 1.5);
        let (_, b, _) = run(staging, true, &settings, Some(&line), 1.5);
        assert!(a == b, "{staging:?} is not deterministic");
    }
}

#[test]
fn the_shells_tags_come_from_the_music_or_the_line() {
    let settings = SceneSettings::new();
    let tag = |loud: bool, text: &str| {
        let line = cue(text, 2.0, 4.0);
        let (state, _, report) = run(Staging::Shell, loud, &settings, Some(&line), 2.5);
        (state.current_tag(7), report.tag)
    };
    // The synthetic groove is a kick every third of a second at full level:
    // that is gabber, not merely belted.
    let named = |loud: bool, text: &str| tag(loud, text).0;
    assert_eq!(named(true, "I typed belted").as_deref(), Some("GABBER"));
    assert_eq!(
        named(false, "[Belted] I typed belted").as_deref(),
        Some("BELTED")
    );
    assert_eq!(
        named(false, "I typed whisper").as_deref(),
        Some("WHISPERED")
    );
    assert_eq!(
        named(false, "[Falsetto] I sing").as_deref(),
        Some("FALSETTO")
    );
    assert_eq!(
        named(false, "[key change up] did you").as_deref(),
        Some("KEY CHANGE UP")
    );
    assert_eq!(named(false, "sudo meow").as_deref(), Some("ROOT"));
    assert_eq!(tag(true, "sudo meow").1.as_deref(), Some("ROOT"));
    // History: two lines in, the first is remembered.
    let mut state = PixelCatsState::new(Staging::Shell, 3);
    let first = LyricCue {
        id: 1,
        ..cue("first line", 0.0, 1.0)
    };
    let second = LyricCue {
        id: 2,
        ..cue("second line", 1.0, 2.0)
    };
    for step in 0..120 {
        let time = step as f64 / 60.0;
        let mut frame = SceneFrame::idle(&settings);
        frame.time_seconds = time;
        frame.delta_seconds = 1.0 / 60.0;
        frame.lyric = Some(if time < 1.0 { &first } else { &second });
        state.update(&frame);
    }
    let history: Vec<_> = state.history().map(|sung| sung.text.clone()).collect();
    assert_eq!(history, ["FIRST LINE"]);
}

#[test]
fn a_four_shade_palette_leaves_only_its_four_shades() {
    for palette in [Palette::Handheld, Palette::Arcade, Palette::Dusk] {
        let settings = with(SceneId::PixelRave, control::PALETTE, palette as u32 as f32);
        let settings = {
            let mut s = settings;
            s.set(SceneId::PixelRave, control::GLITCH, 0.0);
            s
        };
        let (_, rgba, report) = run(Staging::Rave, true, &settings, None, 1.0);
        assert_eq!(report.palette, palette.name());
        let ramp = palette.ramp().unwrap();
        for pixel in rgba.chunks_exact(4) {
            let c = Rgb::new(pixel[0], pixel[1], pixel[2]);
            assert!(ramp.contains(&c), "{palette:?} leaked {c:?}");
        }
    }
    // The hue turns a ramp without leaving four colours.
    let mut settings = with(SceneId::PixelBoot, control::HUE, 120.0);
    settings.set(SceneId::PixelBoot, control::GLITCH, 0.0);
    let (_, rgba, _) = run(Staging::Boot, true, &settings, None, 1.0);
    assert!(distinct_colours(&rgba) <= 4);
    let (_, plain, _) = run(
        Staging::Boot,
        true,
        &with(SceneId::PixelBoot, control::GLITCH, 0.0),
        None,
        1.0,
    );
    assert!(
        mean_difference(&rgba, &plain) > 5.0,
        "the hue must turn the greens"
    );
}

#[test]
fn the_set_powers_off_at_the_end_of_the_track_only_with_the_crt_on() {
    let settings = SceneSettings::new();
    let mut state = PixelCatsState::new(Staging::Boot, 1);
    let mut frame = SceneFrame::idle(&settings);
    frame.duration_seconds = 10.0;
    frame.delta_seconds = 1.0 / 60.0;
    frame.time_seconds = 9.98;
    state.update(&frame);
    let mut raster = Raster::new(16, 16);
    let mut rgba = Vec::new();
    let report = render(&state, &frame, 320, 180, &mut raster, &mut rgba);
    assert_eq!(report.power, "off");
    assert!(rgba.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]));
    frame.time_seconds = 9.5;
    let report = render(&state, &frame, 320, 180, &mut raster, &mut rgba);
    assert_eq!(report.power, "switching-off");
    let lit = rgba.chunks_exact(4).filter(|p| p[..3] != [0, 0, 0]).count();
    assert!(lit > 0 && lit < 320 * 180 / 2, "{lit}");
    let off = with(SceneId::PixelBoot, control::CRT, 0.0);
    let frame = SceneFrame {
        settings: &off,
        ..frame
    };
    let report = render(&state, &frame, 320, 180, &mut raster, &mut rgba);
    assert_eq!(report.power, "on");
}

#[test]
fn onsets_pop_words_and_the_toggle_stops_them() {
    let settings = SceneSettings::new();
    let line = cue("meow meow meow meow", 0.2, 4.0);
    // Sample several moments so at least one lands inside a pop's life.
    let popped: usize = [1.02, 1.36, 1.69, 2.02]
        .into_iter()
        .map(|t| run(Staging::Rave, true, &settings, Some(&line), t).2.pops)
        .sum();
    assert!(popped > 0, "no pops on onsets");
    let off = with(SceneId::PixelRave, control::POPS, 0.0);
    let none: usize = [1.02, 1.36, 1.69, 2.02]
        .into_iter()
        .map(|t| run(Staging::Rave, true, &off, Some(&line), t).2.pops)
        .sum();
    assert_eq!(none, 0);
}

#[test]
fn every_staging_composes_in_portrait() {
    let settings = SceneSettings::new();
    let line = cue("now every bracket bows to me", 0.2, 3.0);
    for staging in Staging::ALL {
        let mut state = PixelCatsState::new(staging, 9);
        let mut track = Track::new(true);
        let mut raster = Raster::new(16, 16);
        let mut rgba = Vec::new();
        let mut report = PixelReport::default();
        for step in 0..120u64 {
            let time = step as f64 / 60.0;
            track.step(time);
            let mut frame = SceneFrame::idle(&settings);
            frame.duration_seconds = 60.0;
            frame.time_seconds = time;
            frame.delta_seconds = 1.0 / 60.0;
            frame.audio = track.audio(time);
            frame.lyric = Some(&line).filter(|cue| time >= cue.start_seconds);
            state.update(&frame);
            if step == 119 {
                report = render(&state, &frame, 180, 320, &mut raster, &mut rgba);
            }
        }
        assert!(
            report.glyphs > 10,
            "{staging:?} portrait drew {} glyphs",
            report.glyphs
        );
        assert!(not_flat(&rgba), "{staging:?} portrait is flat");
    }
}
