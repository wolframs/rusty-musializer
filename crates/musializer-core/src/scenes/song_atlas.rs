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

/// One spectral slice of the terrain (`Song_Atlas_Slice`, `song_atlas_map.h:14-19`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    pub bands: [f32; BAND_COUNT],
    pub rms: f32,
    pub flux: f32,
    pub onset: bool,
}

impl Default for Slice {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Slice {
    /// C's zeroed slice.
    pub const ZERO: Self = Self {
        bands: [0.0; BAND_COUNT],
        rms: 0.0,
        flux: 0.0,
        onset: false,
    };
}

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

/// How many rows to draw for `available` slices at `detail_level`
/// (`song_atlas_map_render_sample_count`, `song_atlas_map.c:48-58`).
///
/// Returns 0 for anything the renderer must skip.
#[must_use]
pub fn render_sample_count(available: usize, detail_level: usize) -> usize {
    if available < 2 || available > MAX_SLICES || detail_level < 1 || detail_level > MAX_DETAIL {
        return 0;
    }
    let mut sample_count = (available * detail_level + MAX_DETAIL - 1) / MAX_DETAIL;
    if sample_count < 2 {
        sample_count = 2;
    }
    if sample_count > available {
        sample_count = available;
    }
    sample_count
}

/// Which slice index sample `sample` maps to
/// (`song_atlas_map_render_sample_index`, `song_atlas_map.c:60-68`).
///
/// C returns `SIZE_MAX` for an invalid request, which every caller would then use
/// as an index; `None` makes that impossible.
#[must_use]
pub fn render_sample_index(
    first: usize,
    available: usize,
    sample_count: usize,
    sample: usize,
) -> Option<usize> {
    if available < 2
        || available > MAX_SLICES
        || sample_count < 2
        || sample_count > available
        || sample >= sample_count
        || first > usize::MAX - available
    {
        return None;
    }
    Some(first + (sample * (available - 1) + (sample_count - 1) / 2) / (sample_count - 1))
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

/// A bounded whole-track spectral map (`Song_Atlas_Map`, `song_atlas_map.h:21-25`).
///
/// C carries a fixed `Song_Atlas_Slice[MAX_SLICES]` array plus a `count`; the Vec
/// here holds exactly `count` slices and is still bounded by [`MAX_SLICES`] —
/// [`SongAtlasMap::is_valid`] rejects anything longer, and an invalid map is never
/// drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongAtlasMap {
    slices: Vec<Slice>,
    duration_seconds: f64,
}

impl SongAtlasMap {
    #[must_use]
    pub fn new(slices: Vec<Slice>, duration_seconds: f64) -> Self {
        Self {
            slices,
            duration_seconds,
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.slices.len()
    }

    #[must_use]
    pub fn slices(&self) -> &[Slice] {
        &self.slices
    }

    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    /// `song_atlas_map_valid` (`song_atlas_map.c:219-237`).
    ///
    /// Every band, rms and flux must be finite and within `0..=1`, the duration
    /// must be finite and positive, and there must be at least two slices to
    /// interpolate between.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if self.slices.len() < 2
            || self.slices.len() > MAX_SLICES
            || !self.duration_seconds.is_finite()
            || self.duration_seconds <= 0.0
        {
            return false;
        }
        self.slices.iter().all(|slice| {
            let unit = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
            unit(slice.rms) && unit(slice.flux) && slice.bands.iter().copied().all(unit)
        })
    }

    /// The playhead as a fractional slice index (`atlas_map_playhead`,
    /// `scene_song_atlas.c:406-413`). `0.0` for an invalid map.
    #[must_use]
    pub fn playhead(&self, time_seconds: f64) -> f32 {
        if !self.is_valid() {
            return 0.0;
        }
        let normalized = (time_seconds / self.duration_seconds).clamp(0.0, 1.0);
        (normalized * (self.count() - 1) as f64) as f32
    }

    /// Linearly interpolated `(rms, flux)` at a fractional slice index
    /// (`atlas_map_dynamics`, `scene_song_atlas.c:415-430`).
    ///
    /// `None` for an invalid map, which is C leaving the caller's pre-seeded
    /// values alone.
    #[must_use]
    pub fn dynamics(&self, playhead: f32) -> Option<(f32, f32)> {
        if !self.is_valid() {
            return None;
        }
        let lower = playhead.floor() as usize;
        let last = self.count() - 1;
        if lower >= last {
            return Some((self.slices[last].rms, self.slices[last].flux));
        }
        let amount = playhead - lower as f32;
        let a = &self.slices[lower];
        let b = &self.slices[lower + 1];
        Some((
            a.rms + (b.rms - a.rms) * amount,
            a.flux + (b.flux - a.flux) * amount,
        ))
    }
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
        SongAtlasMap::new(slices, 120.0)
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
    fn render_sampling_matches_the_c_arithmetic() {
        // Detail 3 of 3 samples every slice; detail 1 samples a third of them.
        assert_eq!(render_sample_count(100, 3), 100);
        assert_eq!(render_sample_count(100, 1), 34);
        assert_eq!(render_sample_count(100, 2), 67);
        // Guards: too few slices, too many, and an out-of-range detail level.
        assert_eq!(render_sample_count(1, 3), 0);
        assert_eq!(render_sample_count(MAX_SLICES + 1, 3), 0);
        assert_eq!(render_sample_count(100, 0), 0);
        assert_eq!(render_sample_count(100, MAX_DETAIL + 1), 0);

        // The index walk spans the whole range and is monotonic.
        let count = render_sample_count(100, 1);
        assert_eq!(render_sample_index(0, 100, count, 0), Some(0));
        assert_eq!(render_sample_index(0, 100, count, count - 1), Some(99));
        let mut previous = 0;
        for sample in 0..count {
            let index = render_sample_index(7, 100, count, sample).expect("in range");
            assert!(index >= previous, "the walk is monotonic");
            previous = index;
        }
        // Out of range requests must not become an index.
        assert_eq!(render_sample_index(0, 100, count, count), None);
        assert_eq!(render_sample_index(0, 1, 2, 0), None);
        assert_eq!(render_sample_index(usize::MAX, 100, count, 0), None);
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
        assert_eq!(map.count(), 64);
        assert_eq!(map.playhead(0.0), 0.0);
        assert_eq!(map.playhead(120.0), 63.0);
        assert_eq!(map.playhead(1000.0), 63.0, "the playhead is clamped");
        assert_eq!(map.playhead(-5.0), 0.0);
        let (rms, flux) = map.dynamics(map.playhead(60.0)).expect("a valid map");
        assert!(rms > 0.4 && rms < 0.6);
        assert!(flux > 0.2 && flux < 0.3);

        // One slice cannot be interpolated, so it is not a valid map.
        assert!(!SongAtlasMap::new(vec![Slice::ZERO], 10.0).is_valid());
        // Neither is a zero duration.
        assert!(!SongAtlasMap::new(vec![Slice::ZERO; 4], 0.0).is_valid());
        // Nor a non-finite band.
        let mut broken = ramp_map(8);
        let mut slices = broken.slices().to_vec();
        slices[3].bands[0] = f32::NAN;
        broken = SongAtlasMap::new(slices, broken.duration_seconds());
        assert!(!broken.is_valid());
        assert_eq!(broken.dynamics(0.0), None);
        assert_eq!(broken.playhead(1.0), 0.0);
        // And a map longer than the C's fixed array cannot be drawn.
        assert!(!SongAtlasMap::new(vec![Slice::ZERO; MAX_SLICES + 1], 10.0).is_valid());
    }
}
