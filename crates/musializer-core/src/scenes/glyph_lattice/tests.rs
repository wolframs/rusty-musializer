//! Glyph Lattice's behaviour, without a window.

use super::field::{render, Field, Lattice};
use super::shapes::Form;
use super::*;
use crate::scene::{LyricCue, SceneSettings};

fn settings() -> SceneSettings {
    SceneSettings::new()
}

/// A groove: a kick every half second on the low bands, hats on the top,
/// a steady mid bed.
fn groove(
    state: &mut GlyphLatticeState,
    settings: &SceneSettings,
    seconds: f32,
    fps: f32,
    loud: bool,
) {
    let frames = (seconds * fps) as usize;
    let period = (fps / 2.0) as usize;
    let calm = [0.36f32; 40];
    for tick in 0..frames {
        let mut bands = calm;
        if loud && tick % period < 2 {
            bands[..10].fill(0.72);
        }
        if loud && tick % (period / 2).max(1) == 0 {
            bands[34..].fill(0.6);
        }
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = 1.0 / fps;
        frame.time_seconds = f64::from(tick as f32 / fps);
        frame.audio = crate::scene::SceneAudioFrame::from_spectrum(&bands, &calm);
        if !loud {
            frame.audio = crate::scene::SceneAudioFrame::from_spectrum(&[0.0; 40], &[0.0; 40]);
        }
        state.update(&frame);
    }
}

fn frame_at(settings: &SceneSettings) -> SceneFrame<'_> {
    SceneFrame::idle(settings)
}

#[test]
fn the_lattice_is_a_function_of_the_aspect_not_the_pixels() {
    let a = Lattice::new(1280.0, 720.0, 120.0);
    let b = Lattice::new(1920.0, 1080.0, 120.0);
    let c = Lattice::new(3840.0, 2160.0, 120.0);
    assert_eq!(a, b);
    assert_eq!(b, c);
    let portrait = Lattice::new(1080.0, 1920.0, 120.0);
    assert!(portrait.rows() > portrait.cols());
    assert!(a.cols() <= field::MAX_COLS && a.rows() <= field::MAX_ROWS);
    // A shape too long for the atlas coarsens rather than overflowing.
    let strip = Lattice::new(4000.0, 300.0, 200.0);
    assert!(strip.cols() <= field::MAX_COLS && strip.rows() <= field::MAX_ROWS);
    // Every cell is its own nearest cell.
    for (col, row) in [(0, 0), (5, 7), (a.cols() - 1, a.rows() - 1), (100, 33)] {
        assert_eq!(a.nearest(a.centre(col, row)), Some((col, row)));
    }
}

#[test]
fn seeded_replay_draws_the_same_cells() {
    let settings = settings();
    let mut one = GlyphLatticeState::new(7);
    let mut two = one.clone();
    groove(&mut one, &settings, 3.0, 60.0, true);
    groove(&mut two, &settings, 3.0, 60.0, true);
    assert_eq!(one, two);
    let frame = frame_at(&settings);
    let (mut fa, mut fb) = (Field::new(), Field::new());
    let ra = render(&one, &frame, 1920.0, 1080.0, &mut fa);
    let rb = render(&two, &frame, 1280.0, 720.0, &mut fb);
    assert_eq!(fa.rgba, fb.rgba, "same aspect, same cells");
    assert_eq!(ra, rb);
    assert!(ra.bright > 50 && ra.mid > 50 && ra.dim > 20, "{ra:?}");
}

#[test]
fn zero_response_is_deaf() {
    let mut settings = settings();
    settings.set(SceneId::GlyphLattice, control::RESPONSE, 0.0);
    let mut loud = GlyphLatticeState::new(3);
    let mut quiet = loud.clone();
    groove(&mut loud, &settings, 6.0, 60.0, true);
    groove(&mut quiet, &settings, 6.0, 60.0, false);
    assert_eq!(loud, quiet, "audio cannot leak through zero response");
    assert!(loud.fired() >= 2, "the ambient clock still fires pulses");
    let frame = frame_at(&settings);
    let (mut fa, mut fb) = (Field::new(), Field::new());
    render(&loud, &frame, 1920.0, 1080.0, &mut fa);
    render(&quiet, &frame, 1920.0, 1080.0, &mut fb);
    assert_eq!(fa.rgba, fb.rgba);
}

#[test]
fn kicks_fire_pulses_that_travel_the_edges() {
    let mut settings = settings();
    settings.set(SceneId::GlyphLattice, control::SOLID, 1.0);
    let mut state = GlyphLatticeState::new(11);
    groove(&mut state, &settings, 3.0, 60.0, true);
    assert!(state.kicks() >= 3, "kicks {}", state.kicks());
    // Step just past a kick and measure the heat twice as it travels.
    let mut field = Field::new();
    let frame = frame_at(&settings);
    let mut hot_counts = Vec::new();
    let mut probe = state.clone();
    let mut bands = [0.28f32; 40];
    bands[..10].fill(0.9);
    let calm = [0.28f32; 40];
    let mut f = SceneFrame::idle(&settings);
    f.delta_seconds = 1.0 / 60.0;
    f.audio = crate::scene::SceneAudioFrame::from_spectrum(&bands, &calm);
    probe.update(&f);
    let newest = probe.pulses().last().copied().expect("a pulse");
    let mut centroid = Vec::new();
    for _ in 0..3 {
        let report = render(&probe, &frame, 1920.0, 1080.0, &mut field);
        hot_counts.push(report.hot);
        centroid.push(field.heat().iter().sum::<f32>());
        f.audio = crate::scene::SceneAudioFrame::from_spectrum(&calm, &calm);
        for _ in 0..8 {
            probe.update(&f);
        }
    }
    assert!(probe.pulses().iter().any(|p| p.id == newest.id));
    assert!(hot_counts.iter().all(|h| *h > 0), "{hot_counts:?}");
    // Heat exists without a pulse only where strokes cross; a fresh kick adds.
    let mut silent = state.clone();
    let mut q = SceneFrame::idle(&settings);
    q.delta_seconds = 1.0 / 60.0;
    for _ in 0..200 {
        silent.update(&q);
    }
    let still = render(&silent, &frame, 1920.0, 1080.0, &mut field);
    let _ = still;
}

#[test]
fn a_new_solid_assembles_while_the_old_one_exhales() {
    let mut settings = settings();
    settings.set(SceneId::GlyphLattice, control::SOLID, 1.0);
    let mut state = GlyphLatticeState::new(2);
    groove(&mut state, &settings, 2.0, 60.0, true);
    assert_eq!(state.form(), Form::Cube);
    settings.set(SceneId::GlyphLattice, control::SOLID, 4.0);
    groove(&mut state, &settings, 0.3, 60.0, true);
    assert_eq!(state.form(), Form::Icosahedron);
    assert_eq!(state.previous(), Some(Form::Cube));
    let frame = frame_at(&settings);
    let mut field = Field::new();
    let early = render(&state, &frame, 1920.0, 1080.0, &mut field);
    assert!(early.assembled < 1.0);
    assert_eq!(early.previous, Some("cube"));
    let early_lit = early.dim + early.mid + early.bright + early.hot;
    groove(&mut state, &settings, 2.5, 60.0, true);
    let late = render(&state, &frame, 1920.0, 1080.0, &mut field);
    assert_eq!(late.assembled, 1.0);
    assert_eq!(late.previous, None);
    let late_lit = late.dim + late.mid + late.bright + late.hot;
    assert!(
        late_lit > early_lit,
        "assembled {late_lit} vs assembling {early_lit}"
    );
}

#[test]
fn the_cycle_moves_on_at_a_drop() {
    let settings = settings();
    let mut state = GlyphLatticeState::new(5);
    let first = state.form();
    assert_eq!(first, Form::Cube, "the cube opens");
    // Five seconds of quiet, then a loud groove.
    groove(&mut state, &settings, 5.0, 60.0, false);
    assert_eq!(state.form(), first);
    groove(&mut state, &settings, 4.0, 60.0, true);
    assert_ne!(state.form(), first, "a drop after quiet moves the cycle on");
}

#[test]
fn the_spectrum_lights_the_edges_by_height() {
    let mut settings = settings();
    settings.set(SceneId::GlyphLattice, control::SOLID, 1.0);
    settings.set(SceneId::GlyphLattice, control::SPIN, 0.0);
    let frame = frame_at(&settings);
    let run = |low: bool| {
        let mut state = GlyphLatticeState::new(9);
        let mut bands = [0.05f32; 40];
        if low {
            bands[..8].fill(0.6);
        } else {
            bands[32..].fill(0.6);
        }
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 60.0;
        f.audio = crate::scene::SceneAudioFrame::from_spectrum(&bands, &bands);
        for _ in 0..120 {
            state.update(&f);
        }
        let mut field = Field::new();
        render(&state, &frame, 1920.0, 1080.0, &mut field);
        // Brightness in the lower and upper halves of the frame.
        let lattice = field.lattice();
        let (mut top, mut bottom) = (0.0f32, 0.0f32);
        for row in 0..lattice.rows() {
            for col in 0..lattice.cols() {
                let lit = field.lit()[row * lattice.cols() + col];
                if lattice.centre(col, row)[1] < 0.0 {
                    top += lit;
                } else {
                    bottom += lit;
                }
            }
        }
        (top, bottom)
    };
    let (bass_top, bass_bottom) = run(true);
    let (treble_top, treble_bottom) = run(false);
    assert!(
        bass_bottom / bass_top > treble_bottom / treble_top * 1.05,
        "bass {bass_top}/{bass_bottom} treble {treble_top}/{treble_bottom}"
    );
}

#[test]
fn the_line_lights_word_by_word_and_dissolves() {
    let settings = settings();
    let cue = LyricCue {
        id: 4,
        start_seconds: 1.0,
        end_seconds: 4.0,
        text: "Light the lattice".into(),
        voice: None,
    };
    let mut state = GlyphLatticeState::new(1);
    let mut field = Field::new();
    let mut drawn = Vec::new();
    let mut f = SceneFrame::idle(&settings);
    f.delta_seconds = 1.0 / 30.0;
    for tick in 0..210 {
        f.time_seconds = f64::from(tick as f32 / 30.0);
        f.lyric = (f.time_seconds >= cue.start_seconds && f.time_seconds < cue.end_seconds)
            .then_some(&cue);
        state.update(&f);
        if tick % 6 == 0 {
            let report = render(&state, &f, 1920.0, 1080.0, &mut field);
            drawn.push((
                f.time_seconds,
                report.strokes_drawn,
                report.strokes_total,
                report.line_live,
            ));
        }
    }
    let total = drawn.iter().map(|d| d.2).max().unwrap();
    assert!(total > 20, "strokes {total}");
    let growing: Vec<usize> = drawn
        .iter()
        .filter(|d| d.0 >= 1.0 && d.0 < 2.4)
        .map(|d| d.1)
        .collect();
    assert!(growing.windows(2).all(|w| w[0] <= w[1]), "{growing:?}");
    assert!(growing.first().unwrap() < &total && growing.last().unwrap() == &total);
    // After the cue the line is still there, dissolving, then gone.
    assert!(drawn.iter().any(|d| d.0 > 4.0 && d.2 == total && !d.3));
    assert!(
        state.line().is_none(),
        "the line dissolved by seven seconds"
    );
}

#[test]
fn the_whole_line_stands_in_place_before_a_word_lights() {
    // Nothing appears anywhere but where it ends up: on the cue's first
    // frame every stroke of the line is already laid out, in the ghost tone,
    // and no word has lit yet.
    let cue = LyricCue {
        id: 9,
        start_seconds: 2.0,
        end_seconds: 6.0,
        text: "for this small interval".into(),
        voice: None,
    };
    let run = |words: bool| {
        let mut settings = settings();
        settings.set(
            SceneId::GlyphLattice,
            control::WORDS,
            if words { 1.0 } else { 0.0 },
        );
        settings.set(SceneId::GlyphLattice, control::GHOSTS, 0.0);
        let mut state = GlyphLatticeState::new(2);
        let mut f = SceneFrame::idle(&settings);
        f.delta_seconds = 1.0 / 30.0;
        for tick in 0..=60 {
            f.time_seconds = f64::from(tick as f32 / 30.0);
            f.lyric = (f.time_seconds >= cue.start_seconds).then_some(&cue);
            state.update(&f);
        }
        let mut field = Field::new();
        let report = render(&state, &f, 1920.0, 1080.0, &mut field);
        (report, f.time_seconds)
    };
    let (with_line, at) = run(true);
    let (without, _) = run(false);
    assert!((at - 2.0).abs() < 1.0e-9, "the cue's first frame: {at}");
    assert_eq!(with_line.strokes_drawn, 0, "no word lit yet");
    assert!(with_line.strokes_total > 20);
    assert!(
        with_line.ghost > without.ghost + 800,
        "the line is laid out in the ghost tone: {} vs {}",
        with_line.ghost,
        without.ghost
    );
}

#[test]
fn words_off_never_sets_the_line() {
    let mut settings = settings();
    settings.set(SceneId::GlyphLattice, control::WORDS, 0.0);
    let cue = LyricCue {
        id: 1,
        start_seconds: 0.0,
        end_seconds: 5.0,
        text: "nothing".into(),
        voice: None,
    };
    let mut state = GlyphLatticeState::new(1);
    let mut f = SceneFrame::idle(&settings);
    f.delta_seconds = 1.0 / 30.0;
    f.lyric = Some(&cue);
    for _ in 0..30 {
        state.update(&f);
    }
    assert!(state.line().is_none());
    assert_eq!(state.presence(), 0.0);
}

#[test]
fn onsets_ripple_out_and_are_counted() {
    let settings = settings();
    let mut state = GlyphLatticeState::new(1);
    let mut f = SceneFrame::idle(&settings);
    f.delta_seconds = 1.0 / 60.0;
    let spike = [0.9f32; 40];
    let rest = [0.1f32; 40];
    for tick in 0..120 {
        f.audio = if tick % 30 == 0 {
            crate::scene::SceneAudioFrame::from_spectrum(&spike, &rest)
        } else {
            crate::scene::SceneAudioFrame::from_spectrum(&rest, &rest)
        };
        state.update(&f);
    }
    assert_eq!(state.ripple_count(), 4);
    let mut field = Field::new();
    let report = render(&state, &f, 1920.0, 1080.0, &mut field);
    assert!(!report.ripple_fronts.is_empty());
    assert!(report.washed > 0, "a ripple washes over cells");
}

#[test]
fn pause_freezes_everything() {
    let settings = settings();
    let mut state = GlyphLatticeState::new(4);
    groove(&mut state, &settings, 1.5, 60.0, true);
    let before = state.clone();
    let mut f = SceneFrame::idle(&settings);
    f.audio.rms = 1.0;
    for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        f.delta_seconds = dt;
        state.update(&f);
        assert_eq!(state, before);
    }
}

#[test]
fn the_same_relative_phrase_reads_the_same_on_quiet_and_loud_masters() {
    use crate::audio::track_dynamics::TrackDynamics;
    let settings = settings();
    let mut quiet = GlyphLatticeState::new(1);
    let mut loud = quiet.clone();
    let mut absolute = quiet.clone();
    let mut f = SceneFrame::idle(&settings);
    f.delta_seconds = 0.25;
    for _ in 0..12 {
        f.audio.rms = 0.2;
        f.dynamics = TrackDynamics::from_bounds(0.1, 0.3);
        quiet.update(&f);
        f.audio.rms = 0.6;
        f.dynamics = TrackDynamics::from_bounds(0.3, 0.9);
        loud.update(&f);
        f.dynamics = None;
        absolute.update(&f);
    }
    assert!((quiet.music()[0] - loud.music()[0]).abs() < 1.0e-5);
    assert!(absolute.music()[0] > loud.music()[0] + 0.2);
}

#[test]
fn every_solid_draws_and_portrait_composes() {
    for (index, form) in Form::ALL.into_iter().enumerate() {
        let mut settings = settings();
        settings.set(SceneId::GlyphLattice, control::SOLID, index as f32 + 1.0);
        let mut state = GlyphLatticeState::new(3);
        groove(&mut state, &settings, 2.0, 30.0, true);
        assert_eq!(state.form(), form);
        let frame = frame_at(&settings);
        for (w, h) in [(1920.0, 1080.0), (1080.0, 1920.0), (1080.0, 1080.0)] {
            let mut field = Field::new();
            let report = render(&state, &frame, w, h, &mut field);
            let lit = report.dim + report.mid + report.bright + report.hot;
            assert!(lit > 400, "{form:?} at {w}x{h}: {report:?}");
            assert!(report.ghost > 100, "{form:?} ghosts at {w}x{h}");
        }
    }
}

#[test]
fn a_fresh_scene_shows_its_solid_and_the_top_of_a_track_builds_it() {
    let settings = settings();
    let frame = frame_at(&settings);
    let mut field = Field::new();
    // Picked while paused: never updated, and the solid is there.
    let fresh = GlyphLatticeState::new(8);
    let report = render(&fresh, &frame, 1920.0, 1080.0, &mut field);
    assert_eq!(report.assembled, 1.0);
    assert!(report.mid + report.bright > 200, "{report:?}");
    // Played from the top: it assembles.
    let mut opening = fresh.clone();
    let mut f = SceneFrame::idle(&settings);
    f.delta_seconds = 1.0 / 30.0;
    opening.update(&f);
    let report = render(&opening, &f, 1920.0, 1080.0, &mut field);
    assert!(report.assembled < 0.1, "{report:?}");
    // Entered mid-track: no assembly.
    let mut midway = fresh.clone();
    f.time_seconds = 92.0;
    midway.update(&f);
    let report = render(&midway, &f, 1920.0, 1080.0, &mut field);
    assert_eq!(report.assembled, 1.0);
}
