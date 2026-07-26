//! Staged analysis results awaiting Apply or Discard.
//!
//! **Owner: Agent B.** Port of `../musializer/src/analysis_candidate.c/.h`.
//!
//! This module is the "model output is **staged**" invariant made into a type.
//! Nothing in the editor changes because a job finished: a completed run becomes an
//! [`AnalysisCandidate`], which is validated, bounded, and inert until someone calls
//! [`AnalysisCandidate::apply`]. Discarding it is dropping it.
//!
//! Two authority rules are enforced rather than trusted:
//!
//! - a candidate only ever *contains* lanes the caller authorized, because
//!   [`AnalysisCandidate::prepare`] skips the rest;
//! - [`AnalysisCandidate::apply`] re-checks that nothing available exceeds what was
//!   authorized, so a candidate that was tampered with between preparation and
//!   application is refused rather than applied.

use crate::project::analysis_bridge::AnalysisBridge;
use crate::project::event_timeline::EventTimeline;
use crate::project::lyrics::LyricsDocument;
use crate::project::scene_switch::{SceneSwitchCue, SceneSwitchTimeline, CAPACITY};
use crate::scene::events::{EventRecord, EventType};
use crate::scene::settings::SettingsSnapshot;

/// Namespaces a bridge cue id into the semantic event lane
/// (`analysis_candidate.c:68`, the ASCII bytes of `MIMO`).
///
/// The lanes are independently authored, so an id from one must not collide with an
/// id from another. XOR rather than addition so the mapping is reversible.
pub const SEMANTIC_ID_SALT: u64 = 0x4D49_4D4F_0000_0000;

/// Which evidence lanes a run may touch (`Analysis_Candidate_Lane`,
/// `analysis_candidate.h:15-22`).
///
/// C carries these as a bitmask and has to check for unknown bits
/// (`lanes_are_valid`, `analysis_candidate.c:7-10`). Three named booleans make that
/// check unrepresentable; all that remains is "at least one".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lanes {
    pub lyrics: bool,
    pub sections: bool,
    pub semantics: bool,
}

impl Lanes {
    /// Every lane, which is what an unrestricted Assist run authorizes.
    pub const ALL: Lanes = Lanes {
        lyrics: true,
        sections: true,
        semantics: true,
    };

    #[must_use]
    pub const fn lyrics_only() -> Self {
        Self {
            lyrics: true,
            sections: false,
            semantics: false,
        }
    }

    /// At least one lane. An empty authority is a caller bug, not a no-op run.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.lyrics || self.sections || self.semantics
    }

    /// True when `self` contains a lane `authority` does not.
    #[must_use]
    pub fn exceeds(self, authority: Lanes) -> bool {
        (self.lyrics && !authority.lyrics)
            || (self.sections && !authority.sections)
            || (self.semantics && !authority.semantics)
    }
}

/// Why a candidate could not be prepared or applied
/// (`Analysis_Candidate_Result`, `analysis_candidate.h:38-47`).
///
/// C's `ERROR_SCHEMA` has no counterpart: a Rust [`AnalysisBridge`] can only exist
/// by having parsed as v1, so there is no version left to disagree about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisCandidateError {
    #[error("invalid lane authority")]
    Authority,
    #[error("invalid duration")]
    Duration,
    #[error("invalid lyric candidate")]
    Lyrics,
    #[error("invalid scene candidate")]
    Sections,
    #[error("invalid semantic candidate")]
    Semantics,
}

/// Validated, mode-bounded, **inert** analysis state (`Analysis_Candidate`,
/// `analysis_candidate.h:24-36`).
#[derive(Clone, Debug)]
pub struct AnalysisCandidate {
    authorized: Lanes,
    available: Lanes,
    lyrics: LyricsDocument,
    sections: SceneSwitchTimeline,
    semantic_events: EventTimeline,
    uncertain_lyric_count: usize,
    semantic_note_count: usize,
}

impl AnalysisCandidate {
    #[must_use]
    pub fn authorized(&self) -> Lanes {
        self.authorized
    }

    /// Which authorized lanes the run actually produced. The Assist panel shows this,
    /// because "authorized but empty" and "not authorized" mean different things to a
    /// user deciding whether to apply.
    #[must_use]
    pub fn available(&self) -> Lanes {
        self.available
    }

    #[must_use]
    pub fn lyrics(&self) -> &LyricsDocument {
        &self.lyrics
    }

    #[must_use]
    pub fn sections(&self) -> &SceneSwitchTimeline {
        &self.sections
    }

    #[must_use]
    pub fn semantic_events(&self) -> &EventTimeline {
        &self.semantic_events
    }

    /// How many staged lyric cues the helper flagged as estimated. Worth surfacing
    /// before Apply: an estimated window is timing the user may want to check.
    #[must_use]
    pub fn uncertain_lyric_count(&self) -> usize {
        self.uncertain_lyric_count
    }

    /// How many free-form interpretation notes came with the run. They are not
    /// applied to anything — they are for the user to read.
    #[must_use]
    pub fn semantic_note_count(&self) -> usize {
        self.semantic_note_count
    }

    /// `analysis_candidate_prepare` (`analysis_candidate.c:91-130`).
    ///
    /// Converts a parsed bridge into candidate state. Nothing in the destination
    /// editor is touched; that is [`Self::apply`]'s job, and only if the user asks.
    ///
    /// `duration_seconds` is the **decoder's** authoritative duration, which may
    /// differ from the bridge's by rounding — see [`Self::normalize_sections`].
    pub fn prepare(
        bridge: &AnalysisBridge,
        authorized: Lanes,
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<Self, AnalysisCandidateError> {
        if !authorized.is_valid() {
            return Err(AnalysisCandidateError::Authority);
        }
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 || scene_count == 0 {
            return Err(AnalysisCandidateError::Duration);
        }

        let mut candidate = Self {
            authorized,
            available: Lanes::default(),
            lyrics: LyricsDocument::new(duration_seconds)
                .map_err(|_| AnalysisCandidateError::Duration)?,
            sections: SceneSwitchTimeline::new(),
            semantic_events: EventTimeline::new(),
            uncertain_lyric_count: 0,
            semantic_note_count: 0,
        };

        if authorized.lyrics && bridge.lyrics_present() {
            bridge
                .lyrics
                .validate()
                .map_err(|_| AnalysisCandidateError::Lyrics)?;
            candidate.lyrics = bridge.lyrics.clone();
            candidate.available.lyrics = true;
            candidate.uncertain_lyric_count = bridge.uncertain_lyric_count();
        }

        if authorized.sections && bridge.sections_present() {
            if bridge.sections.len() > CAPACITY {
                return Err(AnalysisCandidateError::Sections);
            }
            let cues: Vec<SceneSwitchCue> = bridge
                .sections
                .iter()
                .map(|section| SceneSwitchCue {
                    id: section.id,
                    start_seconds: section.start_ms as f64 / 1000.0,
                    end_seconds: section.end_ms as f64 / 1000.0,
                    scene_index: section.recommended_scene.index() as u32,
                    strength: f32::from(section.transition_strength_milli) / 1000.0,
                    settings: SettingsSnapshot::default(),
                })
                .collect();
            candidate
                .sections
                .replace(&cues, duration_seconds, scene_count)
                .map_err(|_| AnalysisCandidateError::Sections)?;
            candidate.available.sections = true;
        }

        if authorized.semantics {
            candidate.semantic_note_count = bridge.semantic_notes.len();
            if bridge.semantic_cues_present() {
                for cue in &bridge.semantic_cues {
                    // Namespaced so a MiMo cue id cannot masquerade as a manually
                    // authored event's. `0` is reserved, hence the nudge to 1.
                    let mut id = cue.id ^ SEMANTIC_ID_SALT;
                    if id == 0 {
                        id = 1;
                    }
                    candidate
                        .semantic_events
                        .record(EventRecord {
                            timestamp_seconds: cue.start_ms as f64 / 1000.0,
                            id,
                            event_type: EventType::Semantic as u32,
                            value_count: 4,
                            values: [
                                f32::from(cue.energy_milli) / 1000.0,
                                f32::from(cue.tension_milli) / 1000.0,
                                f32::from(cue.valence_milli) / 1000.0,
                                f32::from(cue.confidence_milli) / 1000.0,
                            ],
                        })
                        .map_err(|_| AnalysisCandidateError::Semantics)?;
                }
                candidate.available.semantics = true;
            }
        }

        Ok(candidate)
    }

    /// `analysis_candidate_normalize_sections` (`analysis_candidate.c:132-163`).
    ///
    /// Revalidates the sections lane against the decoder's authoritative duration by
    /// stretching the final cue to reach it. The candidate is left **unchanged** when
    /// that would make the last cue invalid — for example when decoder padding ends
    /// before that cue's start — because a half-normalized plan is worse than an
    /// un-normalized one.
    pub fn normalize_sections(
        &mut self,
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<(), AnalysisCandidateError> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 || scene_count == 0 {
            return Err(AnalysisCandidateError::Duration);
        }
        if !self.available.sections || self.sections.is_empty() {
            return Ok(());
        }
        let mut cues = self.sections.cues().to_vec();
        let last = cues.len() - 1;
        cues[last].end_seconds = duration_seconds;

        let mut normalized = SceneSwitchTimeline::new();
        normalized
            .replace(&cues, duration_seconds, scene_count)
            .map_err(|_| AnalysisCandidateError::Sections)?;
        normalized.enabled = self.sections.enabled;
        self.sections = normalized;
        Ok(())
    }

    /// `analysis_candidate_apply` (`analysis_candidate.c:165-200`).
    ///
    /// Applies only lanes that are both authorized **and** present. Suggestions for
    /// sections preserve the user's existing auto-scene opt-in: applying a plan does
    /// not decide for them that auto scenes are on.
    ///
    /// Every source was validated during preparation, so these replacements cannot
    /// fail for content. The independently revisioned documents go first, and the
    /// section timeline is published last, so a failure cannot leave the section plan
    /// pointing at lyrics that were never applied.
    pub fn apply(
        &self,
        lyrics: &mut LyricsDocument,
        sections: &mut SceneSwitchTimeline,
        semantic_events: &mut EventTimeline,
    ) -> Result<(), AnalysisCandidateError> {
        if !self.authorized.is_valid() || self.available.exceeds(self.authorized) {
            return Err(AnalysisCandidateError::Authority);
        }
        if self.available.lyrics {
            lyrics
                .replace(&self.lyrics)
                .map_err(|_| AnalysisCandidateError::Lyrics)?;
        }
        if self.available.semantics {
            semantic_events
                .replace(&self.semantic_events)
                .map_err(|_| AnalysisCandidateError::Semantics)?;
        }
        if self.available.sections {
            let enabled = sections.enabled;
            *sections = self.sections.clone();
            sections.enabled = enabled;
            sections.reset();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::analysis_bridge::{self, AnalysisBridgeError};
    use crate::project::lyrics::base64_encode;
    use crate::project::sha256;
    use crate::scene::SceneId;
    use crate::scene::SCENE_COUNT;

    const SCENES: u32 = SCENE_COUNT as u32;
    const DURATION: f64 = 60.0;

    fn b64(text: &str) -> String {
        base64_encode(text.as_bytes())
    }

    /// A bridge document with all three lanes, built inline the way the C suite
    /// builds its fixtures.
    fn document() -> String {
        let digest = sha256::digest_hex(b"audio");
        let reasons = b64("[\"chorus\"]");
        format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n\
             LYRIC\t1\t1000\t2000\t900\tnone\t{}\n\
             LYRIC\t2\t2000\t3000\t-1\tuncertain\t{}\n\
             SECTION\t10\t0\t30000\tspectrum\t500\t{reasons}\n\
             SECTION\t11\t30000\t60000\tloom\t750\t{reasons}\n\
             SEMANTIC\t20\t0\t30000\t400\t300\t-200\t900\t{}\n\
             SEMANTIC\t21\t30000\t60000\t800\t700\t600\t950\t{}\n",
            b64("first line"),
            b64("second line"),
            b64("calm"),
            b64("bright")
        )
    }

    fn bridge() -> AnalysisBridge {
        analysis_bridge::parse(document().as_bytes(), None, None).expect("the fixture parses")
    }

    fn notes_document() -> String {
        let digest = sha256::digest_hex(b"audio");
        let reasons = b64("[\"x\"]");
        format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n\
             SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n\
             SEMANTIC_NOTE\t2\t{}\nSEMANTIC_NOTE\t3\t{}\n",
            b64("a thought"),
            b64("another")
        )
    }

    #[test]
    fn preparing_stages_every_authorized_lane_and_touches_nothing() {
        let bridge = bridge();
        let candidate = AnalysisCandidate::prepare(&bridge, Lanes::ALL, DURATION, SCENES).unwrap();
        assert_eq!(candidate.available(), Lanes::ALL);
        assert_eq!(candidate.lyrics().len(), 2);
        assert_eq!(candidate.sections().len(), 2);
        assert_eq!(candidate.semantic_events().len(), 2);
        assert_eq!(candidate.uncertain_lyric_count(), 1);
        assert_eq!(candidate.semantic_note_count(), 0);
        assert_eq!(
            candidate.sections().cues()[1].scene_index,
            SceneId::Loom.index() as u32
        );
        assert_eq!(candidate.sections().cues()[1].strength, 0.75);
    }

    #[test]
    fn an_unauthorized_lane_is_never_even_staged() {
        let bridge = bridge();
        let candidate =
            AnalysisCandidate::prepare(&bridge, Lanes::lyrics_only(), DURATION, SCENES).unwrap();
        assert_eq!(
            candidate.available(),
            Lanes {
                lyrics: true,
                sections: false,
                semantics: false
            }
        );
        assert!(
            candidate.sections().is_empty(),
            "a lane the run was not authorized for must not exist in the candidate"
        );
        assert!(candidate.semantic_events().is_empty());
    }

    #[test]
    fn an_empty_authority_is_a_caller_bug() {
        assert_eq!(
            AnalysisCandidate::prepare(&bridge(), Lanes::default(), DURATION, SCENES).unwrap_err(),
            AnalysisCandidateError::Authority
        );
    }

    #[test]
    fn an_unusable_duration_or_registry_is_refused() {
        for (duration, scenes) in [
            (0.0, SCENES),
            (-1.0, SCENES),
            (f64::NAN, SCENES),
            (f64::INFINITY, SCENES),
            (DURATION, 0),
        ] {
            assert_eq!(
                AnalysisCandidate::prepare(&bridge(), Lanes::ALL, duration, scenes).unwrap_err(),
                AnalysisCandidateError::Duration,
                "{duration} {scenes}"
            );
        }
    }

    #[test]
    fn semantic_ids_are_namespaced_away_from_manual_ones() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        for event in candidate.semantic_events().events() {
            assert_ne!(event.id, 20);
            assert_ne!(event.id, 21);
            assert_ne!(event.id, 0);
            assert_eq!(
                event.id ^ SEMANTIC_ID_SALT,
                if event.timestamp_seconds == 0.0 {
                    20
                } else {
                    21
                },
                "the mapping is reversible, so a staged event can be traced back"
            );
        }
    }

    #[test]
    fn semantic_values_are_scaled_from_thousandths() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let first = &candidate.semantic_events().events()[0];
        assert_eq!(first.values, [0.4, 0.3, -0.2, 0.9]);
        // And they land inside the project model's per-position bounds.
        assert!((0.0..=1.0).contains(&first.values[0]));
        assert!((-1.0..=1.0).contains(&first.values[2]));
    }

    #[test]
    fn free_form_notes_are_counted_but_never_applied_to_anything() {
        let bridge = analysis_bridge::parse(notes_document().as_bytes(), None, None).unwrap();
        let candidate = AnalysisCandidate::prepare(&bridge, Lanes::ALL, DURATION, SCENES).unwrap();
        assert_eq!(candidate.semantic_note_count(), 2);
        assert!(
            !candidate.available().semantics,
            "notes are for the user to read, not events to apply"
        );
        assert!(candidate.semantic_events().is_empty());
    }

    #[test]
    fn applying_replaces_only_available_lanes() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::lyrics_only(), DURATION, SCENES).unwrap();

        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut sections = SceneSwitchTimeline::new();
        sections
            .replace(
                &[SceneSwitchCue {
                    id: 99,
                    start_seconds: 0.0,
                    end_seconds: DURATION,
                    scene_index: 3,
                    strength: 0.5,
                    settings: SettingsSnapshot::default(),
                }],
                DURATION,
                SCENES,
            )
            .unwrap();
        let mut semantic_events = EventTimeline::new();

        candidate
            .apply(&mut lyrics, &mut sections, &mut semantic_events)
            .unwrap();
        assert_eq!(lyrics.len(), 2);
        assert_eq!(
            sections.cues()[0].id,
            99,
            "an unauthorized lane leaves the editor's own plan alone"
        );
        assert!(semantic_events.is_empty());
    }

    #[test]
    fn applying_sections_preserves_the_users_auto_scene_opt_in() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut semantic_events = EventTimeline::new();

        for opted_in in [false, true] {
            let mut sections = SceneSwitchTimeline::new();
            sections.enabled = opted_in;
            candidate
                .apply(&mut lyrics, &mut sections, &mut semantic_events)
                .unwrap();
            assert_eq!(sections.len(), 2);
            assert_eq!(
                sections.enabled, opted_in,
                "applying a plan must not decide the toggle for the user"
            );
            assert_eq!(sections.active_index(), None, "and the cursor is reset");
        }
    }

    #[test]
    fn applying_advances_the_destination_revisions() {
        let candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut sections = SceneSwitchTimeline::new();
        let mut semantic_events = EventTimeline::new();
        let lyric_revision = lyrics.revision();
        let event_revision = semantic_events.revision();
        candidate
            .apply(&mut lyrics, &mut sections, &mut semantic_events)
            .unwrap();
        assert_eq!(lyrics.revision(), lyric_revision + 1);
        assert_eq!(semantic_events.revision(), event_revision + 1);
    }

    #[test]
    fn a_candidate_carrying_more_than_it_was_authorized_for_is_refused() {
        // The available/authorized re-check at apply time exists for exactly this:
        // state that was tampered with between preparation and application.
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::lyrics_only(), DURATION, SCENES).unwrap();
        candidate.available.sections = true;
        let mut lyrics = LyricsDocument::new(DURATION).unwrap();
        let mut sections = SceneSwitchTimeline::new();
        let mut semantic_events = EventTimeline::new();
        assert_eq!(
            candidate
                .apply(&mut lyrics, &mut sections, &mut semantic_events)
                .unwrap_err(),
            AnalysisCandidateError::Authority
        );
        assert_eq!(lyrics.len(), 0, "and nothing was applied on the way out");
    }

    #[test]
    fn normalizing_stretches_the_last_section_to_the_decoded_duration() {
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        candidate.sections.enabled = true;
        // The decoder found 60.02 s where the helper reported 60.
        candidate.normalize_sections(60.02, SCENES).unwrap();
        assert_eq!(
            candidate.sections().cues().last().unwrap().end_seconds,
            60.02
        );
        assert_eq!(candidate.sections().cues()[0].end_seconds, 30.0);
        assert!(
            candidate.sections().enabled,
            "normalization is not a reason to change the toggle"
        );
    }

    #[test]
    fn normalizing_leaves_the_candidate_alone_when_it_cannot_succeed() {
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, SCENES).unwrap();
        let before = candidate.sections().clone();
        // A duration that ends before the last cue starts cannot be normalized into a
        // valid plan; a half-normalized one would be worse than none.
        assert_eq!(
            candidate.normalize_sections(10.0, SCENES).unwrap_err(),
            AnalysisCandidateError::Sections
        );
        assert_eq!(candidate.sections(), &before);
        for (duration, scenes) in [(0.0, SCENES), (f64::NAN, SCENES), (DURATION, 0)] {
            assert_eq!(
                candidate.normalize_sections(duration, scenes).unwrap_err(),
                AnalysisCandidateError::Duration
            );
        }
        assert_eq!(candidate.sections(), &before);
    }

    #[test]
    fn normalizing_a_candidate_without_sections_is_a_no_op() {
        let mut candidate =
            AnalysisCandidate::prepare(&bridge(), Lanes::lyrics_only(), DURATION, SCENES).unwrap();
        assert!(candidate.normalize_sections(60.02, SCENES).is_ok());
        assert!(candidate.sections().is_empty());
    }

    #[test]
    fn a_bridge_whose_sections_do_not_fit_the_decoded_duration_is_refused() {
        // The bridge covers 60 s; preparing against a 30 s decode cannot produce a
        // contiguous plan, and that is a staged-lane failure rather than something to
        // truncate silently.
        assert_eq!(
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, 30.0, SCENES).unwrap_err(),
            AnalysisCandidateError::Sections
        );
    }

    #[test]
    fn a_bridge_naming_a_scene_outside_the_registry_cannot_be_staged() {
        // `scene_count` smaller than the registry is how a build with fewer scenes
        // would see a newer plan.
        assert_eq!(
            AnalysisCandidate::prepare(&bridge(), Lanes::ALL, DURATION, 1).unwrap_err(),
            AnalysisCandidateError::Sections
        );
    }

    #[test]
    fn a_bridge_that_never_parsed_cannot_be_staged_at_all() {
        // Restating the boundary: candidates are built from parsed bridges, so every
        // malformed document is rejected before this module sees it.
        let malformed = document().replace("MUSIALIZER_BRIDGE\t1", "MUSIALIZER_BRIDGE\t2");
        assert_eq!(
            analysis_bridge::parse(malformed.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Header
        );
    }
}
