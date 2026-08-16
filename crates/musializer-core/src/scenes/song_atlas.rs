//! Song Atlas: deterministic state, update, and the render-side map sampling.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_song_atlas.c`'s
//! `init`/`update` half plus the pure sampling helpers from
//! `song_atlas_map.c/.h` (frozen at `9300af9`, read-only). The drawing half is
//! `musializer-app::scenes::song_atlas`.
//!
//! ## Two terrain sources, not one
//!
//! The normal path renders a bounded whole-song analysis map prepared when the
//! track loads ([`SongAtlasMap`]). The rolling ring in [`SongAtlasState`] is the
//! **live-input fallback** for sources that do not provide a complete decoded
//! track (`scene_song_atlas.c:8-10`). Both must exist: the map path is what makes
//! the scene show the song rather than the last twenty seconds of it, and the ring
//! is what stops the scene being empty when there is no map.
//!
//! ## Boundary note for Agent A
//!
//! `song_atlas_map.c`'s **builder** (`song_atlas_map_build`, whole-track FFT over
//! decoded PCM) is Agent A's, mapped to `core::audio::song_atlas_map`, and is
//! unported at the time of writing. Its **slice type, bounds, and render-side
//! sampling** are here because the scene's own ring stores the same slices and
//! could not compile without them. When the builder lands it should produce a
//! [`SongAtlasMap`] from this module rather than declaring a second slice type;
//! see the note in REWRITE_PLAN.md.

// Three lints are allowed deliberately, all for the same reason: the arithmetic
// here is checked line by line against the C and reads more truthfully in C's
// shape. `manual_clamp` also has a semantic edge — C's two `if`s let a NaN delta
// pass through, and `clamp` documents that only for the input, not the bounds.
#![allow(
    clippy::manual_range_contains,
    clippy::manual_div_ceil,
    clippy::manual_clamp
)]

use std::any::Any;

use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};

/// `SONG_ATLAS_BAND_COUNT` (`song_atlas_map.h:8`).
pub const BAND_COUNT: usize = 28;
/// `SONG_ATLAS_BASE_SLICES` (`song_atlas_map.h:9`).
pub const BASE_SLICES: usize = 192;
/// `SONG_ATLAS_MAX_DETAIL` (`song_atlas_map.h:10`).
pub const MAX_DETAIL: usize = 3;
/// `SONG_ATLAS_MAX_SLICES` (`song_atlas_map.h:11`). The hard ceiling on both the
/// map and the live ring, which is what keeps the state bounded.
pub const MAX_SLICES: usize = BASE_SLICES * MAX_DETAIL;

/// `ATLAS_BASE_CAPTURE_INTERVAL` (`scene_song_atlas.c:14`).
pub const BASE_CAPTURE_INTERVAL: f64 = 0.11;
/// `ATLAS_CAPTURE_INTERVAL` (`scene_song_atlas.c:15-16`): the base interval
/// divided by the maximum detail level, so the ring always holds enough slices
/// for the highest sampling detail the Tune panel can ask for.
pub const CAPTURE_INTERVAL: f64 = BASE_CAPTURE_INTERVAL / MAX_DETAIL as f64;
/// `ATLAS_SLICE_SPACING` (`scene_song_atlas.c:17`): world units between slices.
pub const SLICE_SPACING: f32 = 0.29;
/// A forward jump larger than this is a discontinuity, not playback
/// (`scene_song_atlas.c:141-142`).
pub const DISCONTINUITY_SECONDS: f64 = 3.0;

/// `scene_song_atlas.c:726`.
pub const STATE_VERSION: u32 = 4;

// `Slice` and `SongAtlasMap` are Agent A's, in `core::audio::song_atlas_map`, and
// re-exported here so the scene's callers keep one import.
//
// Both agents defined them: Agent C branched while A's module was still a
// placeholder, so trunk briefly had two field-identical `Slice` types and two
// `SongAtlasMap`s. A's is the survivor because it owns `build` — the whole-track
// preprocessing — and is differentially verified against the C over 72 slices x 28
// bands. The scene-specific readers that were methods on C's copy are free
// functions below, which is the right split: A owns the map, this module owns how
// Song Atlas reads it.
pub use crate::audio::song_atlas_map::{Slice, SongAtlasMap};

fn clamp01(value: f32) -> f32 {
    // `atlas_clamp01` (`scene_song_atlas.c:32-37`). Note that this one does *not*
    // reject NaN — unlike the ASCII Field and Orbital Lattice clamps — because
    // both comparisons fail and the value passes through. Reproduced rather than
    // corrected; a NaN band here would come from a broken analyzer, and the
    // oracle's behaviour is what the terrain was tuned against.
    if value < 0.0 {
        return 0.0;
    }
    if value > 1.0 {
        return 1.0;
    }
    value
}

/// `atlas_hash` (`scene_song_atlas.c:39-48`), splitmix64's finalizer.
fn hash(seed: u64, salt: u32) -> u32 {
    let mut value = seed ^ (u64::from(salt).wrapping_add(0x9e37_79b9_7f4a_7c15));
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value as u32
}

/// `atlas_hash_unit` (`scene_song_atlas.c:50-53`).
#[must_use]
pub fn hash_unit(seed: u64, salt: u32) -> f32 {
    (hash(seed, salt) & 0xffff) as f32 / 65535.0
}

/// Stable slice indices for a sampling-detail level.
///
/// The old range-relative interpolation redistributed the chosen rows whenever
/// `first` advanced. At detail 1 or 2 that made unchanged terrain ahead of the
/// playhead acquire different source slices every frame. Anchoring the pattern
/// to each slice's absolute index makes advancing the window remove old rows
/// without reshaping the rest of the song. Detail 3 still selects every row.
pub fn render_sample_indices(
    first: usize,
    available: usize,
    detail_level: usize,
) -> impl Iterator<Item = usize> {
    let valid = available >= 2
        && available <= MAX_SLICES
        && (1..=MAX_DETAIL).contains(&detail_level)
        && first <= usize::MAX - available;
    (0..available).filter_map(move |offset| {
        if !valid {
            return None;
        }
        let index = first + offset;
        (index % MAX_DETAIL < detail_level).then_some(index)
    })
}

#[must_use]
pub fn render_sample_count(first: usize, available: usize, detail_level: usize) -> usize {
    render_sample_indices(first, available, detail_level).count()
}

/// Converts a distance in slices into a distance in world spacing units
/// (`song_atlas_map_render_distance`, `song_atlas_map.c:70-74`).
#[must_use]
pub fn render_distance(source_distance: f32) -> f32 {
    if !source_distance.is_finite() {
        return 0.0;
    }
    source_distance / MAX_DETAIL as f32
}

/// The playhead as a fractional slice index (`atlas_map_playhead`,
/// `scene_song_atlas.c:406-413`). `0.0` for an invalid map.
///
/// A free function rather than a method because [`SongAtlasMap`] belongs to
/// `core::audio::song_atlas_map`, which builds the map and knows nothing about how
/// a scene reads it.
#[must_use]
pub fn map_playhead(map: &SongAtlasMap, time_seconds: f64) -> f32 {
    if !map.is_valid() || map.is_empty() {
        return 0.0;
    }
    let normalized = (time_seconds / map.duration_seconds()).clamp(0.0, 1.0);
    (normalized * (map.len() - 1) as f64) as f32
}

/// Linearly interpolated `(rms, flux)` at a fractional slice index
/// (`atlas_map_dynamics`, `scene_song_atlas.c:415-430`).
///
/// `None` for an invalid map, which is C leaving the caller's pre-seeded values
/// alone rather than zeroing them.
#[must_use]
pub fn map_dynamics(map: &SongAtlasMap, playhead: f32) -> Option<(f32, f32)> {
    if !map.is_valid() || map.is_empty() {
        return None;
    }
    let slices = map.slices();
    let lower = playhead.floor().max(0.0) as usize;
    let last = map.len() - 1;
    if lower >= last {
        return Some((slices[last].rms, slices[last].flux));
    }
    let amount = playhead - lower as f32;
    let a = &slices[lower];
    let b = &slices[lower + 1];
    Some((
        a.rms + (b.rms - a.rms) * amount,
        a.flux + (b.flux - a.flux) * amount,
    ))
}

/// The live fallback ring (`Song_Atlas_State`, `scene_song_atlas.c:19-30`).
///
/// A fixed [`MAX_SLICES`]-slot ring buffer plus damped camera envelopes. Bounded
/// by construction: `first`/`count` walk the ring and nothing grows.
#[derive(Clone, Debug, PartialEq)]
pub struct SongAtlasState {
    /// Boxed because 576 slices of 28 bands is about 70 KiB.
    slices: Box<[Slice]>,
    seed: u64,
    first: usize,
    count: usize,
    last_frame_time: f64,
    last_capture_time: f64,
    filtered_bands: [f32; BAND_COUNT],
    camera_energy: f32,
    camera_flux: f32,
    pending_onset: bool,
}

impl SongAtlasState {
    /// `song_atlas_init` (`scene_song_atlas.c:66-73`).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            slices: vec![Slice::ZERO; MAX_SLICES].into_boxed_slice(),
            seed,
            first: 0,
            count: 0,
            last_frame_time: -1.0,
            last_capture_time: -1.0,
            filtered_bands: [0.0; BAND_COUNT],
            camera_energy: 0.0,
            camera_flux: 0.0,
            pending_onset: false,
        }
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// How many slices the ring holds, never more than [`MAX_SLICES`].
    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub fn camera_energy(&self) -> f32 {
        self.camera_energy
    }

    #[must_use]
    pub fn camera_flux(&self) -> f32 {
        self.camera_flux
    }

    #[must_use]
    pub fn last_capture_time(&self) -> f64 {
        self.last_capture_time
    }

    /// The `chronological`-th oldest live slice (`atlas_slice`,
    /// `scene_song_atlas.c:171-175`). `None` past the end of the ring.
    #[must_use]
    pub fn slice(&self, chronological: usize) -> Option<&Slice> {
        if chronological >= self.count {
            return None;
        }
        Some(&self.slices[(self.first + chronological) % MAX_SLICES])
    }

    /// How far the terrain has scrolled since the last capture, in units of one
    /// capture interval (`atlas_scroll_phase`, `scene_song_atlas.c:177-182`).
    ///
    /// Geometry scrolls continuously between captures, so the terrain's apparent
    /// frame rate is the renderer's rather than the analysis history's.
    #[must_use]
    pub fn scroll_phase(&self, time_seconds: f64) -> f32 {
        if self.last_capture_time < 0.0 {
            return 0.0;
        }
        clamp01(((time_seconds - self.last_capture_time) / CAPTURE_INTERVAL) as f32)
    }

    /// `atlas_clear_history` (`scene_song_atlas.c:55-64`).
    ///
    /// Note what is *not* cleared: the slice storage itself. C only resets
    /// `first`/`count`, so stale slices stay in the buffer but become unreachable.
    fn clear_history(&mut self) {
        self.first = 0;
        self.count = 0;
        self.last_capture_time = -1.0;
        self.filtered_bands = [0.0; BAND_COUNT];
        self.camera_energy = 0.0;
        self.camera_flux = 0.0;
        self.pending_onset = false;
    }

    /// `atlas_resample_band` (`scene_song_atlas.c:75-91`): the peak of the source
    /// bands falling in one atlas band.
    ///
    /// Peak, not mean — a narrow transient must survive the reduction from ~104
    /// analyzer bands to 28 terrain bands or the ridge line disappears.
    fn resample_band(bands: &[f32], destination: usize) -> f32 {
        let count = bands.len();
        if count == 0 {
            return 0.0;
        }
        let mut begin = destination * count / BAND_COUNT;
        let mut end = (destination + 1) * count / BAND_COUNT;
        if end <= begin {
            end = begin + 1;
        }
        if begin >= count {
            begin = count - 1;
        }
        if end > count {
            end = count;
        }
        let mut peak = 0.0f32;
        for &value in &bands[begin..end] {
            let value = clamp01(value);
            if value > peak {
                peak = value;
            }
        }
        peak
    }

    /// `atlas_capture` (`scene_song_atlas.c:93-132`).
    fn capture(&mut self, frame: &SceneFrame<'_>) {
        let index = if self.count < MAX_SLICES {
            let index = (self.first + self.count) % MAX_SLICES;
            self.count += 1;
            index
        } else {
            let index = self.first;
            self.first = (self.first + 1) % MAX_SLICES;
            index
        };

        let bands = frame.audio.bands;
        let flux = self.camera_flux;
        let onset = self.pending_onset;
        let slice = &mut self.slices[index];
        slice.flux = flux;
        slice.onset = onset;
        for band in 0..BAND_COUNT {
            // A three-tap spatial blur across neighbouring terrain bands, which is
            // what keeps the surface readable as a landscape rather than a comb.
            let value = Self::resample_band(bands, band);
            let mut neighbor = value;
            let mut weight = 1.0f32;
            if band > 0 {
                neighbor += Self::resample_band(bands, band - 1) * 0.55;
                weight += 0.55;
            }
            if band + 1 < BAND_COUNT {
                neighbor += Self::resample_band(bands, band + 1) * 0.55;
                weight += 0.55;
            }
            let value = neighbor / weight;

            // Quick attacks preserve musical articulation; slower releases keep
            // adjacent terrain slices visually connected instead of sparkling.
            // The exponent makes the response independent of the capture rate.
            let base_response = if value > self.filtered_bands[band] {
                0.62f32
            } else {
                0.24f32
            };
            let response = 1.0 - (1.0 - base_response).powf(1.0 / MAX_DETAIL as f32);
            self.filtered_bands[band] += (value - self.filtered_bands[band]) * response;
            slice.bands[band] = clamp01(self.filtered_bands[band]).powf(0.78);
        }
        self.last_capture_time = frame.time_seconds;
        self.pending_onset = false;
    }
}

impl SceneState for SongAtlasState {
    fn id(&self) -> SceneId {
        SceneId::SongAtlas
    }

    /// `song_atlas_update` (`scene_song_atlas.c:134-169`).
    fn update(&mut self, frame: &SceneFrame<'_>) {
        let elapsed = frame.time_seconds - self.last_frame_time;

        // Backward time is a seek. A very large forward jump is also treated as a
        // discontinuity so unrelated regions are never joined by terrain.
        let discontinuity =
            self.last_frame_time >= 0.0 && (elapsed < -0.001 || elapsed > DISCONTINUITY_SECONDS);
        if discontinuity {
            self.clear_history();
            self.camera_energy = clamp01(frame.audio.rms * 1.8);
            self.camera_flux = clamp01(frame.audio.spectral_flux * 5.0);
        }

        let mut delta = frame.delta_seconds;
        if delta < 0.0 {
            delta = 0.0;
        }
        if delta > 0.1 {
            delta = 0.1;
        }
        // Note that the blend runs even on the discontinuity frame, right after
        // the snap above. One frame's worth of smoothing on top of a fresh value
        // is a no-op in practice, and reproducing it costs nothing.
        let camera_blend = 1.0 - (-3.2 * delta).exp();
        self.camera_energy += (clamp01(frame.audio.rms * 1.8) - self.camera_energy) * camera_blend;
        self.camera_flux +=
            (clamp01(frame.audio.spectral_flux * 5.0) - self.camera_flux) * camera_blend;
        if frame.audio.onset {
            self.pending_onset = true;
        }

        // Capture on a fixed cadence. Onsets are latched into the next slice, so a
        // beat between captures still becomes a survey line.
        if self.last_capture_time < 0.0
            || frame.time_seconds - self.last_capture_time >= CAPTURE_INTERVAL
        {
            self.capture(frame);
        }
        self.last_frame_time = frame.time_seconds;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The registry entry (`scene_song_atlas.c:723-731`).
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::SongAtlas,
        state_version: STATE_VERSION,
        make_state: |seed| Box::new(SongAtlasState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneAudioFrame, SceneSettings};

    fn frame_at<'a>(
        settings: &'a SceneSettings,
        time: f64,
        smooth: &'a [f32],
        smear: &'a [f32],
    ) -> SceneFrame<'a> {
        SceneFrame {
            time_seconds: time,
            delta_seconds: 1.0 / 60.0,
            audio: SceneAudioFrame::from_spectrum(smooth, smear),
            ..SceneFrame::idle(settings)
        }
    }

    fn ramp_map(count: usize) -> SongAtlasMap {
        let slices = (0..count)
            .map(|i| {
                let level = i as f32 / count as f32;
                Slice {
                    bands: [level; BAND_COUNT],
                    rms: level,
                    flux: level * 0.5,
                    onset: i % 8 == 0,
                }
            })
            .collect();
        SongAtlasMap::from_slices(slices, 120.0)
    }

    #[test]
    fn the_descriptor_matches_the_c_one() {
        let descriptor = descriptor();
        assert_eq!(descriptor.id, SceneId::SongAtlas);
        assert_eq!(descriptor.state_version, 4);
        assert_eq!(descriptor.name(), "Song Atlas");
    }

    #[test]
    fn the_c_bounds_are_reproduced_exactly() {
        assert_eq!(BAND_COUNT, 28);
        assert_eq!(MAX_SLICES, 576);
        assert!((CAPTURE_INTERVAL - 0.11 / 3.0).abs() < 1.0e-12);
    }

    #[test]
    fn the_ring_captures_on_a_fixed_cadence_and_never_exceeds_its_bound() {
        let settings = SceneSettings::default();
        let bands = [0.5f32; 64];
        let mut state = SongAtlasState::new(3);

        state.update(&frame_at(&settings, 0.0, &bands, &bands));
        assert_eq!(state.count(), 1, "the first frame always captures");

        // Frames well inside one interval must not capture.
        state.update(&frame_at(&settings, CAPTURE_INTERVAL * 0.5, &bands, &bands));
        assert_eq!(state.count(), 1);

        // Twelve minutes at 60 fps is far more than 576 captures.
        for index in 1..=43_200u32 {
            let time = f64::from(index) / 60.0;
            state.update(&frame_at(&settings, time, &bands, &bands));
        }
        assert_eq!(state.count(), MAX_SLICES, "the ring fills and stays full");
        assert!(state.slice(MAX_SLICES).is_none());
        assert!(state.slice(MAX_SLICES - 1).is_some());
    }

    #[test]
    fn a_seek_clears_the_ring_and_reseeds_the_camera() {
        let settings = SceneSettings::default();
        let loud = [1.0f32; 32];
        let mut state = SongAtlasState::new(3);
        for index in 0..=120u32 {
            state.update(&frame_at(
                &settings,
                f64::from(index) / 60.0,
                &loud,
                &[0.0f32; 32],
            ));
        }
        assert!(state.count() > 3);
        let energy_before = state.camera_energy();
        assert!(energy_before > 0.0);

        // Backwards: a seek.
        state.update(&frame_at(&settings, 0.5, &loud, &[0.0f32; 32]));
        assert_eq!(
            state.count(),
            1,
            "history discarded, then one fresh capture"
        );

        // Forward by more than the discontinuity window: also a seek.
        let mut state = SongAtlasState::new(3);
        for index in 0..=120u32 {
            state.update(&frame_at(
                &settings,
                f64::from(index) / 60.0,
                &loud,
                &[0.0f32; 32],
            ));
        }
        state.update(&frame_at(
            &settings,
            2.0 + DISCONTINUITY_SECONDS + 0.1,
            &loud,
            &[0.0f32; 32],
        ));
        assert_eq!(state.count(), 1);
    }

    #[test]
    fn an_onset_between_captures_is_latched_into_the_next_slice() {
        let settings = SceneSettings::default();
        // A spectrum whose bands sit well above their trails clears the onset
        // threshold; two equal arrays produce no flux and no onset.
        let loud = [1.0f32; 32];
        let quiet = [0.0f32; 32];
        let mut state = SongAtlasState::new(3);

        state.update(&frame_at(&settings, 0.0, &quiet, &quiet));
        assert!(
            !state.slice(0).unwrap().onset,
            "a silent slice has no onset"
        );

        // A beat that lands between captures: no slice is written for it, so the
        // flag has to be latched or the survey line is lost.
        state.update(&frame_at(&settings, 0.01, &loud, &quiet));
        assert_eq!(state.count(), 1, "no capture inside the interval");

        // The next capture is a silent frame, and still carries the onset.
        state.update(&frame_at(&settings, CAPTURE_INTERVAL, &quiet, &quiet));
        assert_eq!(state.count(), 2);
        assert!(
            state.slice(1).unwrap().onset,
            "the latched onset became a survey line"
        );
        // And it is consumed, not sticky.
        state.update(&frame_at(&settings, CAPTURE_INTERVAL * 2.0, &quiet, &quiet));
        assert!(!state.slice(2).unwrap().onset);
    }

    #[test]
    fn the_ring_update_is_deterministic() {
        let settings = SceneSettings::default();
        let mut bands = [0.0f32; 48];
        let mut first = SongAtlasState::new(0xfeed);
        let mut second = SongAtlasState::new(0xfeed);
        for index in 0..900u32 {
            for (i, band) in bands.iter_mut().enumerate() {
                *band = ((index as usize * 7 + i) % 23) as f32 / 22.0;
            }
            let time = f64::from(index) / 60.0;
            first.update(&frame_at(&settings, time, &bands, &bands));
            second.update(&frame_at(&settings, time, &bands, &bands));
        }
        assert_eq!(first, second);
    }

    #[test]
    fn scroll_phase_runs_zero_to_one_between_captures() {
        let settings = SceneSettings::default();
        let bands = [0.4f32; 32];
        let mut state = SongAtlasState::new(1);
        assert_eq!(state.scroll_phase(0.0), 0.0, "nothing captured yet");
        state.update(&frame_at(&settings, 10.0, &bands, &bands));
        assert_eq!(state.scroll_phase(10.0), 0.0);
        assert!((state.scroll_phase(10.0 + CAPTURE_INTERVAL * 0.5) - 0.5).abs() < 1.0e-5);
        assert_eq!(state.scroll_phase(10.0 + CAPTURE_INTERVAL * 4.0), 1.0);
    }

    #[test]
    fn render_sampling_is_anchored_to_the_song() {
        assert_eq!(
            render_sample_indices(0, 10, 1).collect::<Vec<_>>(),
            vec![0, 3, 6, 9]
        );
        assert_eq!(
            render_sample_indices(0, 10, 2).collect::<Vec<_>>(),
            vec![0, 1, 3, 4, 6, 7, 9]
        );
        assert_eq!(
            render_sample_indices(0, 10, 3).collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );

        // Advancing the visible window may discard rows behind the playhead,
        // but every row still ahead must retain exactly the same source slice.
        for detail in 1..=MAX_DETAIL {
            let before = render_sample_indices(30, 100, detail).collect::<Vec<_>>();
            let after = render_sample_indices(31, 99, detail).collect::<Vec<_>>();
            assert_eq!(
                before
                    .into_iter()
                    .filter(|&row| row >= 31)
                    .collect::<Vec<_>>(),
                after
            );
        }

        assert_eq!(render_sample_indices(0, 1, 3).count(), 0);
        assert_eq!(render_sample_indices(0, MAX_SLICES + 1, 3).count(), 0);
        assert_eq!(render_sample_indices(0, 100, 0).count(), 0);
        assert_eq!(render_sample_indices(usize::MAX, 100, 3).count(), 0);
    }

    #[test]
    fn render_distance_scales_by_the_detail_ceiling() {
        assert_eq!(render_distance(3.0), 1.0);
        assert_eq!(render_distance(0.0), 0.0);
        assert_eq!(render_distance(f32::NAN), 0.0);
        assert_eq!(render_distance(f32::INFINITY), 0.0);
    }

    #[test]
    fn a_map_validates_its_bounds_and_interpolates_its_dynamics() {
        let map = ramp_map(64);
        assert!(map.is_valid());
        assert_eq!(map.len(), 64);
        assert_eq!(map_playhead(&map, 0.0), 0.0);
        assert_eq!(map_playhead(&map, 120.0), 63.0);
        assert_eq!(map_playhead(&map, 1000.0), 63.0, "the playhead is clamped");
        assert_eq!(map_playhead(&map, -5.0), 0.0);
        let (rms, flux) = map_dynamics(&map, map_playhead(&map, 60.0)).expect("a valid map");
        assert!(rms > 0.4 && rms < 0.6);
        assert!(flux > 0.2 && flux < 0.3);

        // One slice cannot be interpolated, so it is not a valid map.
        assert!(!SongAtlasMap::from_slices(vec![Slice::ZERO], 10.0).is_valid());
        // Neither is a zero duration.
        assert!(!SongAtlasMap::from_slices(vec![Slice::ZERO; 4], 0.0).is_valid());
        // Nor a non-finite band.
        let mut broken = ramp_map(8);
        let mut slices = broken.slices().to_vec();
        slices[3].bands[0] = f32::NAN;
        broken = SongAtlasMap::from_slices(slices, broken.duration_seconds());
        assert!(!broken.is_valid());
        assert_eq!(map_dynamics(&broken, 0.0), None);
        assert_eq!(map_playhead(&broken, 1.0), 0.0);
        // And a map longer than the C's fixed array cannot be drawn.
        assert!(!SongAtlasMap::from_slices(vec![Slice::ZERO; MAX_SLICES + 1], 10.0).is_valid());
    }
}
