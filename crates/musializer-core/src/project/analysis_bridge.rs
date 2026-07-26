//! The bounded reader for helper analysis output.
//!
//! **Owner: Agent B.** Port of `../musializer/src/analysis_bridge.c/.h`.
//!
//! The Python helpers are independent evidence-producing tools, and this is the
//! whole boundary between them and the application: a bounded, line-oriented,
//! tab-separated document, base64 for anything that could contain a tab, and no
//! trust extended to any of it. Parsing is atomic — the destination is written only
//! after every record has been accepted.
//!
//! ```text
//! MUSIALIZER_BRIDGE<TAB>1
//! AUDIO<TAB><sha256><TAB><duration_ms>
//! LYRIC<TAB>id<TAB>start_ms<TAB>end_ms<TAB>confidence_milli<TAB>none|uncertain<TAB>b64
//! SECTION<TAB>id<TAB>start_ms<TAB>end_ms<TAB>scene<TAB>strength_milli<TAB>b64_reasons
//! SEMANTIC<TAB>id<TAB>start<TAB>end<TAB>energy<TAB>tension<TAB>valence<TAB>conf<TAB>b64
//! SEMANTIC_NOTE<TAB>id<TAB>b64_text
//! ```
//!
//! Three rules do real work and are easy to lose in a rewrite:
//!
//! - **Record kinds are ordered.** Lyrics, then sections, then semantics. A record
//!   that appears after a later kind has started is an
//!   [`AnalysisBridgeError::Order`], which keeps the parser single-pass and bounded.
//! - **Timed lanes must cover the audio.** Sections are mandatory and must start at
//!   0 and end exactly at `duration_ms`, each cue beginning where the last ended.
//!   Semantic cues, when present, must do the same.
//! - **`SEMANTIC` and `SEMANTIC_NOTE` are mutually exclusive.** One run produces
//!   scored cues or free-form notes, never both, and mixing them would leave two
//!   incompatible readings of the same track in one artifact.

use crate::project::lyrics::{
    base64_decode, Base64Error, LyricCue, LyricsDocument, LyricsError, TEXT_MAX_BYTES,
};
use crate::project::sha256;
use crate::scene::SceneId;

/// `ANALYSIS_BRIDGE_INPUT_MAX_BYTES` (`analysis_bridge.h:11`): 4 MiB.
pub const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
/// `analysis_bridge.h:13`.
pub const SECTION_CAPACITY: usize = 256;
/// `analysis_bridge.h:14`.
pub const SEMANTIC_CAPACITY: usize = 256;
/// `analysis_bridge.h:15`.
pub const NOTE_CAPACITY: usize = 16;
/// `analysis_bridge.h:16`, minus the NUL.
pub const REASONS_MAX_BYTES: usize = 4095;
/// `analysis_bridge.h:17`, minus the NUL.
pub const SUMMARY_MAX_BYTES: usize = 2047;
/// `analysis_bridge.h:18`, minus the NUL.
pub const NOTE_TEXT_MAX_BYTES: usize = 16383;

/// Largest millisecond value an `f64` can represent exactly
/// (`ANALYSIS_BRIDGE_MAX_EXACT_MS`, `analysis_bridge.c:9`).
///
/// Bounded here rather than at conversion because every timestamp in the document
/// becomes seconds as an `f64`, and past 2^53 that conversion silently loses
/// milliseconds.
pub const MAX_EXACT_MS: u64 = 9_007_199_254_740_991;

/// The header's first line, exactly (`analysis_bridge.c:225`).
pub const HEADER_TAG: &str = "MUSIALIZER_BRIDGE";
/// `ANALYSIS_BRIDGE_SCHEMA_VERSION` (`analysis_bridge.h:10`).
pub const SCHEMA_VERSION: &str = "1";

/// Why a bridge document was refused (`Analysis_Bridge_Result`,
/// `analysis_bridge.h:88-106`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisBridgeError {
    #[error("invalid bridge input size")]
    InputSize,
    #[error("invalid bridge header")]
    Header,
    #[error("invalid audio record")]
    Audio,
    #[error("bridge audio does not match")]
    AudioMismatch,
    #[error("invalid bridge record")]
    Record,
    #[error("records are out of order")]
    Order,
    #[error("duplicate stable id")]
    DuplicateId,
    #[error("value or timing out of range")]
    Range,
    #[error("unknown scene")]
    Scene,
    #[error("invalid base64")]
    Base64,
    #[error("invalid UTF-8")]
    Utf8,
    #[error("decoded field too large")]
    DecodedSize,
    #[error("bridge capacity exceeded")]
    Capacity,
    #[error("timed lane does not cover audio")]
    Coverage,
}

/// Per-cue lyric evidence quality (`Analysis_Lyric_Metadata`,
/// `analysis_bridge.h:34-38`).
///
/// Kept beside the lyric document rather than inside it: the document holds
/// *authored truth*, and how confident an aligner was is evidence about it, not part
/// of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LyricMetadata {
    pub id: u64,
    /// Thousandths, or `-1` for "not reported".
    pub confidence_milli: i16,
    /// The helper flagged this window as estimated rather than measured.
    pub uncertain: bool,
}

/// One suggested section (`Analysis_Section`, `analysis_bridge.h:40-47`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub id: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub recommended_scene: SceneId,
    /// Thousandths, `0..=1000`.
    pub transition_strength_milli: u16,
    /// A JSON array, carried verbatim. The bridge checks only that it *is* an array;
    /// interpreting the reasons is the panel's business, and re-encoding them here
    /// would lose whatever the helper actually said.
    pub reasons_json: String,
}

/// One scored semantic cue (`Analysis_Semantic_Cue`, `analysis_bridge.h:49-58`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticCue {
    pub id: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Thousandths, `0..=1000`.
    pub energy_milli: u16,
    /// Thousandths, `0..=1000`.
    pub tension_milli: u16,
    /// Thousandths, `-1000..=1000` — the only signed value.
    pub valence_milli: i16,
    /// Thousandths, `0..=1000`.
    pub confidence_milli: u16,
    pub summary: String,
}

/// One free-form interpretation note (`Analysis_Semantic_Note`,
/// `analysis_bridge.h:60-63`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticNote {
    pub id: u64,
    pub text: String,
}

/// A parsed bridge document (`Analysis_Bridge`, `analysis_bridge.h:65-86`).
///
/// C carries four `*_present` booleans; here each is `!<lane>.is_empty()`, which is
/// the same fact without a second place to get it wrong.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisBridge {
    pub audio_sha256: String,
    pub duration_ms: u64,
    pub lyrics: LyricsDocument,
    pub lyric_metadata: Vec<LyricMetadata>,
    pub sections: Vec<Section>,
    pub semantic_cues: Vec<SemanticCue>,
    pub semantic_notes: Vec<SemanticNote>,
}

impl AnalysisBridge {
    #[must_use]
    pub fn lyrics_present(&self) -> bool {
        !self.lyric_metadata.is_empty()
    }

    #[must_use]
    pub fn sections_present(&self) -> bool {
        !self.sections.is_empty()
    }

    #[must_use]
    pub fn semantic_cues_present(&self) -> bool {
        !self.semantic_cues.is_empty()
    }

    #[must_use]
    pub fn semantic_notes_present(&self) -> bool {
        !self.semantic_notes.is_empty()
    }

    /// How many lyric cues the helper flagged as estimated rather than measured.
    #[must_use]
    pub fn uncertain_lyric_count(&self) -> usize {
        self.lyric_metadata
            .iter()
            .filter(|metadata| metadata.uncertain)
            .count()
    }
}

/// `parse_u64` (`analysis_bridge.c:54-68`).
///
/// Stricter than the lyrics bridge's field parser: a leading zero is rejected, so
/// `007` and `7` cannot both mean the same id and a document has one spelling per
/// number.
fn parse_u64(field: &str) -> Option<u64> {
    if field.is_empty() || (field.len() > 1 && field.starts_with('0')) {
        return None;
    }
    let mut value = 0u64;
    for byte in field.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
    }
    Some(value)
}

/// `parse_i32` (`analysis_bridge.c:70-85`). `-0` is rejected, again so one value has
/// one spelling.
fn parse_i32(field: &str) -> Option<i32> {
    let negative = field.starts_with('-');
    let magnitude = parse_u64(if negative { &field[1..] } else { field })?;
    if negative && magnitude == 0 {
        return None;
    }
    if negative {
        (magnitude <= i32::MAX as u64 + 1).then(|| (magnitude as i64).wrapping_neg() as i32)
    } else {
        i32::try_from(magnitude).ok()
    }
}

/// The scene names the bridge uses (`scene_names`, `analysis_bridge.c:16-19`).
///
/// Verified identical to [`SceneId::stable_name`], in the same order, which is why
/// C's `Analysis_Scene` enum has no separate Rust counterpart — the comment at
/// `analysis_candidate.c:43-44` says the order "mirrors the built-in scene
/// registry", and `bridge_scene_names_match_the_registry` checks that rather than
/// trusting it.
fn parse_scene(field: &str) -> Option<SceneId> {
    SceneId::from_stable_name(field)
}

/// `timed_record` (`analysis_bridge.c:180-188`): the `id/start/end` triple every
/// timed record begins with.
fn timed_record(fields: &[&str], duration_ms: u64) -> Result<(u64, u64, u64), AnalysisBridgeError> {
    let id = parse_u64(fields[1]).ok_or(AnalysisBridgeError::Range)?;
    let start = parse_u64(fields[2]).ok_or(AnalysisBridgeError::Range)?;
    let end = parse_u64(fields[3]).ok_or(AnalysisBridgeError::Range)?;
    if id == 0 || start >= end || end > duration_ms {
        return Err(AnalysisBridgeError::Range);
    }
    Ok((id, start, end))
}

fn decode_field(field: &str, max_bytes: usize) -> Result<String, AnalysisBridgeError> {
    let decoded = base64_decode(field.as_bytes(), max_bytes).map_err(|error| match error {
        Base64Error::Format => AnalysisBridgeError::Base64,
        Base64Error::TooLarge => AnalysisBridgeError::DecodedSize,
        // An embedded NUL is reported as a UTF-8 fault, matching
        // `analysis_bridge.c:132`.
        Base64Error::EmbeddedNul => AnalysisBridgeError::Utf8,
    })?;
    String::from_utf8(decoded).map_err(|_| AnalysisBridgeError::Utf8)
}

/// Which kinds of record have been seen, so ordering can be enforced in one pass
/// (`record_rank`, `analysis_bridge.c:210`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    None = 0,
    Lyric = 1,
    Section = 2,
    Semantic = 3,
}

/// `analysis_bridge_parse` (`analysis_bridge.c:190-379`).
///
/// `expected_audio_sha256` and `expected_duration_ms` may be `None` when the caller
/// only needs syntactic validation. Otherwise they are **exact guards against
/// attaching cached analysis to the wrong audio**, which is the failure this whole
/// format exists to make impossible.
pub fn parse(
    input: &[u8],
    expected_audio_sha256: Option<&str>,
    expected_duration_ms: Option<u64>,
) -> Result<AnalysisBridge, AnalysisBridgeError> {
    if input.is_empty()
        || input.len() > MAX_INPUT_BYTES
        || input.last() != Some(&b'\n')
        || input.contains(&0)
        || input.contains(&b'\r')
    {
        return Err(AnalysisBridgeError::InputSize);
    }
    let text = core::str::from_utf8(input).map_err(|_| AnalysisBridgeError::Utf8)?;

    let mut audio_sha256 = String::new();
    let mut duration_ms = 0u64;
    let mut lyrics = LyricsDocument::new(1.0).expect("1.0 is a valid placeholder duration");
    let mut lyric_metadata: Vec<LyricMetadata> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut semantic_cues: Vec<SemanticCue> = Vec::new();
    let mut semantic_notes: Vec<SemanticNote> = Vec::new();

    let mut ids: Vec<u64> = Vec::new();
    let mut rank = Rank::None;
    let mut previous_lyric_start: Option<u64> = None;
    let mut line_number = 0usize;

    // `input` ends in a newline, so the final split yields one empty piece to drop.
    for line in text.split('\n').take(text.split('\n').count() - 1) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() > 10 {
            return Err(AnalysisBridgeError::Record);
        }

        if line_number == 0 {
            if fields.len() != 2 || fields[0] != HEADER_TAG || fields[1] != SCHEMA_VERSION {
                return Err(AnalysisBridgeError::Header);
            }
            line_number += 1;
            continue;
        }
        if line_number == 1 {
            if fields.len() != 3 || fields[0] != "AUDIO" || !sha256::is_hex_digest(fields[1]) {
                return Err(AnalysisBridgeError::Audio);
            }
            duration_ms = parse_u64(fields[2]).ok_or(AnalysisBridgeError::Audio)?;
            if duration_ms == 0 || duration_ms > MAX_EXACT_MS {
                return Err(AnalysisBridgeError::Audio);
            }
            audio_sha256 = fields[1].to_owned();
            if expected_audio_sha256.is_some_and(|expected| expected != audio_sha256)
                || expected_duration_ms.is_some_and(|expected| expected != duration_ms)
            {
                return Err(AnalysisBridgeError::AudioMismatch);
            }
            lyrics = LyricsDocument::new(duration_ms as f64 / 1000.0)
                .map_err(|_| AnalysisBridgeError::Audio)?;
            line_number += 1;
            continue;
        }

        let claim_id = |id: u64, ids: &mut Vec<u64>| -> Result<(), AnalysisBridgeError> {
            if id == 0 || ids.contains(&id) {
                return Err(AnalysisBridgeError::DuplicateId);
            }
            ids.push(id);
            Ok(())
        };

        match fields[0] {
            "LYRIC" => {
                if rank > Rank::Lyric || fields.len() != 7 {
                    return Err(AnalysisBridgeError::Order);
                }
                rank = Rank::Lyric;
                let (id, start, end) = timed_record(&fields, duration_ms)?;
                let confidence = parse_i32(fields[4]).ok_or(AnalysisBridgeError::Range)?;
                if !(-1..=1000).contains(&confidence) || !matches!(fields[5], "none" | "uncertain")
                {
                    return Err(AnalysisBridgeError::Range);
                }
                // Lyric cues may overlap, so only the *start* order is checked.
                if previous_lyric_start.is_some_and(|previous| start < previous) {
                    return Err(AnalysisBridgeError::Order);
                }
                claim_id(id, &mut ids)?;
                let text = decode_field(fields[6], TEXT_MAX_BYTES)?;
                lyrics
                    .insert(LyricCue {
                        id,
                        start_seconds: start as f64 / 1000.0,
                        end_seconds: end as f64 / 1000.0,
                        text,
                    })
                    .map_err(|error| match error {
                        LyricsError::Capacity => AnalysisBridgeError::Capacity,
                        _ => AnalysisBridgeError::Record,
                    })?;
                lyric_metadata.push(LyricMetadata {
                    id,
                    confidence_milli: confidence as i16,
                    uncertain: fields[5] == "uncertain",
                });
                previous_lyric_start = Some(start);
            }
            "SECTION" => {
                if rank > Rank::Section || fields.len() != 7 {
                    return Err(AnalysisBridgeError::Order);
                }
                rank = Rank::Section;
                if sections.len() >= SECTION_CAPACITY {
                    return Err(AnalysisBridgeError::Capacity);
                }
                let (id, start, end) = timed_record(&fields, duration_ms)?;
                let scene = parse_scene(fields[4]).ok_or(AnalysisBridgeError::Scene)?;
                let strength = parse_u64(fields[5]).ok_or(AnalysisBridgeError::Range)?;
                if strength > 1000 {
                    return Err(AnalysisBridgeError::Range);
                }
                // Contiguous coverage from zero: each section begins exactly where
                // the last ended, so the plan has an answer at every instant.
                let expected_start = sections.last().map_or(0, |section| section.end_ms);
                if start != expected_start {
                    return Err(AnalysisBridgeError::Coverage);
                }
                claim_id(id, &mut ids)?;
                let reasons_json = decode_field(fields[6], REASONS_MAX_BYTES)?;
                if reasons_json.len() < 2
                    || !reasons_json.starts_with('[')
                    || !reasons_json.ends_with(']')
                {
                    return Err(AnalysisBridgeError::Record);
                }
                sections.push(Section {
                    id,
                    start_ms: start,
                    end_ms: end,
                    recommended_scene: scene,
                    transition_strength_milli: strength as u16,
                    reasons_json,
                });
            }
            "SEMANTIC" => {
                if rank > Rank::Semantic || fields.len() != 9 || !semantic_notes.is_empty() {
                    return Err(AnalysisBridgeError::Order);
                }
                rank = Rank::Semantic;
                if semantic_cues.len() >= SEMANTIC_CAPACITY {
                    return Err(AnalysisBridgeError::Capacity);
                }
                let (id, start, end) = timed_record(&fields, duration_ms)?;
                let energy = parse_i32(fields[4]).ok_or(AnalysisBridgeError::Range)?;
                let tension = parse_i32(fields[5]).ok_or(AnalysisBridgeError::Range)?;
                let valence = parse_i32(fields[6]).ok_or(AnalysisBridgeError::Range)?;
                let confidence = parse_i32(fields[7]).ok_or(AnalysisBridgeError::Range)?;
                if !(0..=1000).contains(&energy)
                    || !(0..=1000).contains(&tension)
                    || !(-1000..=1000).contains(&valence)
                    || !(0..=1000).contains(&confidence)
                {
                    return Err(AnalysisBridgeError::Range);
                }
                let expected_start = semantic_cues.last().map_or(0, |cue| cue.end_ms);
                if start != expected_start {
                    return Err(AnalysisBridgeError::Coverage);
                }
                claim_id(id, &mut ids)?;
                let summary = decode_field(fields[8], SUMMARY_MAX_BYTES)?;
                semantic_cues.push(SemanticCue {
                    id,
                    start_ms: start,
                    end_ms: end,
                    energy_milli: energy as u16,
                    tension_milli: tension as u16,
                    valence_milli: valence as i16,
                    confidence_milli: confidence as u16,
                    summary,
                });
            }
            "SEMANTIC_NOTE" => {
                if rank > Rank::Semantic || fields.len() != 3 || !semantic_cues.is_empty() {
                    return Err(AnalysisBridgeError::Order);
                }
                rank = Rank::Semantic;
                if semantic_notes.len() >= NOTE_CAPACITY {
                    return Err(AnalysisBridgeError::Capacity);
                }
                let id = parse_u64(fields[1]).ok_or(AnalysisBridgeError::Range)?;
                if id == 0 {
                    return Err(AnalysisBridgeError::Range);
                }
                claim_id(id, &mut ids)?;
                let text = decode_field(fields[2], NOTE_TEXT_MAX_BYTES)?;
                if text.is_empty() {
                    return Err(AnalysisBridgeError::Utf8);
                }
                semantic_notes.push(SemanticNote { id, text });
            }
            _ => return Err(AnalysisBridgeError::Record),
        }
        line_number += 1;
    }

    if line_number < 2 {
        return Err(AnalysisBridgeError::Header);
    }
    // Sections are mandatory and must reach the end of the audio: an incomplete
    // plan cannot answer "which scene now?" for the tail of the track.
    if sections
        .last()
        .map_or(true, |section| section.end_ms != duration_ms)
    {
        return Err(AnalysisBridgeError::Coverage);
    }
    if let Some(last) = semantic_cues.last() {
        if last.end_ms != duration_ms {
            return Err(AnalysisBridgeError::Coverage);
        }
    }

    Ok(AnalysisBridge {
        audio_sha256,
        duration_ms,
        lyrics,
        lyric_metadata,
        sections,
        semantic_cues,
        semantic_notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::lyrics::base64_encode;

    fn audio_digest() -> String {
        sha256::digest_hex(b"audio")
    }

    fn b64(text: &str) -> String {
        base64_encode(text.as_bytes())
    }

    /// The smallest document the parser accepts: header, audio, and one section
    /// covering the whole track.
    fn minimal() -> String {
        format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{}\t60000\nSECTION\t1\t0\t60000\tspectrum\t500\t{}\n",
            audio_digest(),
            b64("[]")
        )
        .replace(&b64("[]"), &b64("[\"quiet\"]"))
    }

    fn parsed(document: &str) -> AnalysisBridge {
        parse(document.as_bytes(), None, None).expect("the fixture parses")
    }

    #[test]
    fn bridge_scene_names_match_the_registry() {
        // The C keeps its own `Analysis_Scene` enum and comments that its order
        // "mirrors the built-in scene registry". This checks the claim so the Rust
        // port can reuse `SceneId` instead of carrying a second enum that could
        // drift from it.
        let bridge_names = [
            "spectrum",
            "pulse",
            "orbital",
            "ascii",
            "atlas",
            "terrarium",
            "constellation",
            "cadence",
            "loom",
            "pentagram",
        ];
        assert_eq!(bridge_names.len(), SceneId::ALL.len());
        for (index, name) in bridge_names.iter().enumerate() {
            assert_eq!(SceneId::ALL[index].stable_name(), *name);
            assert_eq!(parse_scene(name), Some(SceneId::ALL[index]));
        }
        assert_eq!(parse_scene("nonexistent"), None);
        assert_eq!(parse_scene("Spectrum"), None, "case matters");
    }

    #[test]
    fn a_minimal_document_parses() {
        let bridge = parsed(&minimal());
        assert_eq!(bridge.audio_sha256, audio_digest());
        assert_eq!(bridge.duration_ms, 60_000);
        assert!(bridge.sections_present());
        assert!(!bridge.lyrics_present());
        assert!(!bridge.semantic_cues_present());
        assert!(!bridge.semantic_notes_present());
        assert_eq!(bridge.sections[0].recommended_scene, SceneId::Spectrum);
        assert_eq!(bridge.sections[0].transition_strength_milli, 500);
        assert_eq!(bridge.sections[0].reasons_json, "[\"quiet\"]");
        assert_eq!(bridge.lyrics.duration_seconds(), 60.0);
    }

    #[test]
    fn input_is_bounded_before_a_byte_is_interpreted() {
        assert_eq!(
            parse(b"", None, None).unwrap_err(),
            AnalysisBridgeError::InputSize
        );
        // A missing trailing newline means a truncated document, and accepting it
        // would silently drop the last record.
        let no_newline = minimal().trim_end().to_owned();
        assert_eq!(
            parse(no_newline.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::InputSize
        );
        // CRLF is refused rather than tolerated: a stray '\r' would end up inside a
        // base64 field or a scene name.
        let crlf = minimal().replace('\n', "\r\n");
        assert_eq!(
            parse(crlf.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::InputSize
        );
        let mut with_nul = minimal().into_bytes();
        with_nul.insert(0, 0);
        assert_eq!(
            parse(&with_nul, None, None).unwrap_err(),
            AnalysisBridgeError::InputSize
        );
        let oversized = vec![b'\n'; MAX_INPUT_BYTES + 1];
        assert_eq!(
            parse(&oversized, None, None).unwrap_err(),
            AnalysisBridgeError::InputSize
        );
    }

    #[test]
    fn the_header_must_be_exactly_right() {
        for header in [
            "MUSIALIZER_BRIDGE\t2",
            "MUSIALIZER_BRIDGE",
            "MUSIALIZER_BRIDGE\t1\textra",
            "musializer_bridge\t1",
            "",
        ] {
            let document = minimal().replacen("MUSIALIZER_BRIDGE\t1", header, 1);
            assert_eq!(
                parse(document.as_bytes(), None, None).unwrap_err(),
                AnalysisBridgeError::Header,
                "{header:?}"
            );
        }
        // A header with no audio line at all.
        assert_eq!(
            parse(b"MUSIALIZER_BRIDGE\t1\n", None, None).unwrap_err(),
            AnalysisBridgeError::Header
        );
    }

    #[test]
    fn the_audio_record_is_checked_field_by_field() {
        let digest = audio_digest();
        for audio in [
            format!("AUDIO\t{digest}\t0"),
            format!("AUDIO\t{}\t60000", digest.to_uppercase()),
            format!("AUDIO\t{}\t60000", &digest[..63]),
            format!("AUDIO\t{digest}\t9007199254740992"),
            format!("AUDIO\t{digest}\t060000"),
            format!("AUDIO\t{digest}"),
            format!("TRACK\t{digest}\t60000"),
        ] {
            let document = minimal().replacen(&format!("AUDIO\t{digest}\t60000"), &audio, 1);
            assert_eq!(
                parse(document.as_bytes(), None, None).unwrap_err(),
                AnalysisBridgeError::Audio,
                "{audio}"
            );
        }
    }

    #[test]
    fn the_audio_guards_stop_analysis_reaching_the_wrong_track() {
        let document = minimal();
        assert!(parse(document.as_bytes(), Some(&audio_digest()), Some(60_000)).is_ok());
        assert_eq!(
            parse(
                document.as_bytes(),
                Some(&sha256::digest_hex(b"other")),
                None
            )
            .unwrap_err(),
            AnalysisBridgeError::AudioMismatch
        );
        assert_eq!(
            parse(document.as_bytes(), None, Some(59_000)).unwrap_err(),
            AnalysisBridgeError::AudioMismatch
        );
    }

    #[test]
    fn sections_must_cover_the_audio_contiguously() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");

        // A gap between sections.
        let gapped = format!(
            "{head}SECTION\t1\t0\t20000\tspectrum\t0\t{reasons}\n\
             SECTION\t2\t30000\t60000\tloom\t0\t{reasons}\n"
        );
        assert_eq!(
            parse(gapped.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Coverage
        );

        // Not starting at zero.
        let late = format!("{head}SECTION\t1\t1000\t60000\tspectrum\t0\t{reasons}\n");
        assert_eq!(
            parse(late.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Coverage
        );

        // Short of the end.
        let short = format!("{head}SECTION\t1\t0\t50000\tspectrum\t0\t{reasons}\n");
        assert_eq!(
            parse(short.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Coverage
        );

        // No sections at all: they are mandatory.
        assert_eq!(
            parse(head.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Coverage
        );

        // Contiguous is accepted.
        let good = format!(
            "{head}SECTION\t1\t0\t20000\tspectrum\t0\t{reasons}\n\
             SECTION\t2\t20000\t60000\tloom\t1000\t{reasons}\n"
        );
        let bridge = parsed(&good);
        assert_eq!(bridge.sections.len(), 2);
        assert_eq!(bridge.sections[1].recommended_scene, SceneId::Loom);
    }

    #[test]
    fn section_reasons_must_be_a_json_array() {
        let document = minimal();
        for reasons in ["", "{}", "\"x\"", "[", "]", "null"] {
            let broken = document.replace(&b64("[\"quiet\"]"), &b64(reasons));
            assert!(broken != document || reasons.is_empty());
            let error = parse(broken.as_bytes(), None, None).unwrap_err();
            assert_eq!(error, AnalysisBridgeError::Record, "{reasons:?}");
        }
    }

    #[test]
    fn an_unknown_scene_is_named_as_such() {
        let document = minimal().replace("\tspectrum\t", "\ttapestry\t");
        assert_eq!(
            parse(document.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Scene
        );
    }

    #[test]
    fn record_kinds_must_appear_in_order() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n");
        let lyric = format!("LYRIC\t2\t0\t1000\t900\tnone\t{}\n", b64("hello"));

        // Lyrics before sections: fine.
        assert!(parse(format!("{head}{lyric}{section}").as_bytes(), None, None).is_ok());
        // Sections before lyrics: out of order.
        assert_eq!(
            parse(format!("{head}{section}{lyric}").as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Order
        );
        // Semantics before sections, likewise.
        let semantic = format!("SEMANTIC\t3\t0\t60000\t500\t500\t0\t900\t{}\n", b64("calm"));
        assert!(parse(format!("{head}{section}{semantic}").as_bytes(), None, None).is_ok());
        assert_eq!(
            parse(format!("{head}{semantic}{section}").as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Order
        );
    }

    #[test]
    fn scored_cues_and_free_form_notes_are_mutually_exclusive() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n");
        let semantic = format!("SEMANTIC\t2\t0\t60000\t500\t500\t0\t900\t{}\n", b64("calm"));
        let note = format!("SEMANTIC_NOTE\t3\t{}\n", b64("a thought"));

        assert!(parse(format!("{head}{section}{semantic}").as_bytes(), None, None).is_ok());
        assert!(parse(format!("{head}{section}{note}").as_bytes(), None, None).is_ok());
        // One run produces one reading of the track, not two.
        assert_eq!(
            parse(
                format!("{head}{section}{semantic}{note}").as_bytes(),
                None,
                None
            )
            .unwrap_err(),
            AnalysisBridgeError::Order
        );
        assert_eq!(
            parse(
                format!("{head}{section}{note}{semantic}").as_bytes(),
                None,
                None
            )
            .unwrap_err(),
            AnalysisBridgeError::Order
        );
    }

    #[test]
    fn semantic_cues_must_also_cover_the_audio() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n");
        let short = format!("SEMANTIC\t2\t0\t30000\t500\t500\t0\t900\t{}\n", b64("calm"));
        assert_eq!(
            parse(format!("{head}{section}{short}").as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Coverage
        );
        let late = format!(
            "SEMANTIC\t2\t1000\t60000\t500\t500\t0\t900\t{}\n",
            b64("calm")
        );
        assert_eq!(
            parse(format!("{head}{section}{late}").as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Coverage
        );
    }

    #[test]
    fn semantic_values_are_bounded_per_position() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n");
        let summary = b64("calm");
        for values in [
            "1001\t500\t0\t900",
            "-1\t500\t0\t900",
            "500\t1001\t0\t900",
            "500\t500\t1001\t900",
            "500\t500\t-1001\t900",
            "500\t500\t0\t1001",
            "500\t500\t0\t-1",
        ] {
            let semantic = format!("SEMANTIC\t2\t0\t60000\t{values}\t{summary}\n");
            assert_eq!(
                parse(format!("{head}{section}{semantic}").as_bytes(), None, None).unwrap_err(),
                AnalysisBridgeError::Range,
                "{values}"
            );
        }
        // Valence is the only signed value, and its endpoints are legal.
        for valence in ["-1000", "1000"] {
            let semantic = format!("SEMANTIC\t2\t0\t60000\t0\t0\t{valence}\t0\t{summary}\n");
            assert!(
                parse(format!("{head}{section}{semantic}").as_bytes(), None, None).is_ok(),
                "{valence}"
            );
        }
    }

    #[test]
    fn lyric_records_carry_evidence_quality_separately() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t9\t0\t60000\tspectrum\t0\t{reasons}\n");
        let document = format!(
            "{head}LYRIC\t1\t0\t1000\t900\tnone\t{}\n\
             LYRIC\t2\t1000\t2000\t-1\tuncertain\t{}\n{section}",
            b64("first"),
            b64("second")
        );
        let bridge = parsed(&document);
        assert_eq!(bridge.lyrics.len(), 2);
        assert_eq!(bridge.lyrics.cues()[0].text, "first");
        assert_eq!(bridge.lyric_metadata[0].confidence_milli, 900);
        assert!(!bridge.lyric_metadata[0].uncertain);
        assert_eq!(
            bridge.lyric_metadata[1].confidence_milli, -1,
            "-1 means the helper did not report one"
        );
        assert!(bridge.lyric_metadata[1].uncertain);
        assert_eq!(bridge.uncertain_lyric_count(), 1);
    }

    #[test]
    fn lyric_flags_and_confidence_are_bounded() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t9\t0\t60000\tspectrum\t0\t{reasons}\n");
        for (confidence, flag) in [
            ("1001", "none"),
            ("-2", "none"),
            ("900", "maybe"),
            ("900", ""),
            ("900", "None"),
        ] {
            let document = format!(
                "{head}LYRIC\t1\t0\t1000\t{confidence}\t{flag}\t{}\n{section}",
                b64("x")
            );
            assert_eq!(
                parse(document.as_bytes(), None, None).unwrap_err(),
                AnalysisBridgeError::Range,
                "{confidence} {flag}"
            );
        }
    }

    #[test]
    fn lyric_cues_may_overlap_but_must_not_go_backwards() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t9\t0\t60000\tspectrum\t0\t{reasons}\n");
        // Overlapping is fine in this lane.
        let overlapping = format!(
            "{head}LYRIC\t1\t0\t5000\t0\tnone\t{}\nLYRIC\t2\t2000\t7000\t0\tnone\t{}\n{section}",
            b64("a"),
            b64("b")
        );
        assert!(parse(overlapping.as_bytes(), None, None).is_ok());
        // Going backwards is not.
        let backwards = format!(
            "{head}LYRIC\t1\t5000\t6000\t0\tnone\t{}\nLYRIC\t2\t1000\t2000\t0\tnone\t{}\n{section}",
            b64("a"),
            b64("b")
        );
        assert_eq!(
            parse(backwards.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Order
        );
    }

    #[test]
    fn ids_are_unique_across_every_lane() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        // The same id used by a lyric and a section: still a duplicate, because a
        // staged suggestion is addressed by id alone.
        let document = format!(
            "{head}LYRIC\t1\t0\t1000\t0\tnone\t{}\nSECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n",
            b64("a")
        );
        assert_eq!(
            parse(document.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::DuplicateId
        );
    }

    #[test]
    fn timings_must_be_ordered_and_inside_the_audio() {
        let digest = audio_digest();
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let reasons = b64("[\"x\"]");
        for record in [
            format!("SECTION\t0\t0\t60000\tspectrum\t0\t{reasons}"),
            format!("SECTION\t1\t0\t0\tspectrum\t0\t{reasons}"),
            format!("SECTION\t1\t60000\t0\tspectrum\t0\t{reasons}"),
            format!("SECTION\t1\t0\t60001\tspectrum\t0\t{reasons}"),
            format!("SECTION\t1\t0\t-1\tspectrum\t0\t{reasons}"),
            format!("SECTION\t01\t0\t60000\tspectrum\t0\t{reasons}"),
        ] {
            let document = format!("{head}{record}\n");
            assert_eq!(
                parse(document.as_bytes(), None, None).unwrap_err(),
                AnalysisBridgeError::Range,
                "{record}"
            );
        }
    }

    #[test]
    fn base64_faults_are_reported_apart_from_size_and_encoding_faults() {
        let digest = audio_digest();
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section =
            |reasons: &str| format!("{head}SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n");
        assert_eq!(
            parse(section("!!!!").as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Base64
        );
        assert_eq!(
            parse(section("aGk").as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Base64
        );
        // Non-canonical padding bits: one byte string, one spelling.
        assert_eq!(
            parse(section("aGl=").as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Base64
        );
        // An embedded NUL is reported as a UTF-8 fault, matching the C.
        assert_eq!(
            parse(section(&b64("[\u{0}]")).as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Utf8
        );
        // Too large for the destination field.
        let huge = format!("[\"{}\"]", "x".repeat(REASONS_MAX_BYTES));
        assert_eq!(
            parse(section(&b64(&huge)).as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::DecodedSize
        );
    }

    #[test]
    fn a_note_must_actually_say_something() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let head = format!("MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n");
        let section = format!("SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n");
        let empty = format!("{head}{section}SEMANTIC_NOTE\t2\t\n");
        assert_eq!(
            parse(empty.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Utf8
        );
        let good = format!("{head}{section}SEMANTIC_NOTE\t2\t{}\n", b64("a thought"));
        let bridge = parsed(&good);
        assert_eq!(bridge.semantic_notes[0].text, "a thought");
        assert!(bridge.semantic_notes_present());
    }

    #[test]
    fn note_capacity_is_bounded() {
        let digest = audio_digest();
        let reasons = b64("[\"x\"]");
        let mut document = format!(
            "MUSIALIZER_BRIDGE\t1\nAUDIO\t{digest}\t60000\n\
             SECTION\t1\t0\t60000\tspectrum\t0\t{reasons}\n"
        );
        for index in 0..=NOTE_CAPACITY {
            document.push_str(&format!("SEMANTIC_NOTE\t{}\t{}\n", index + 2, b64("note")));
        }
        assert_eq!(
            parse(document.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Capacity
        );
    }

    #[test]
    fn an_unknown_record_kind_is_refused() {
        let document = format!("{}SURPRISE\t1\n", minimal());
        assert_eq!(
            parse(document.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Record
        );
        // As is a blank line, which is not a record at all.
        let blank = minimal().replace("SECTION", "\nSECTION");
        assert_eq!(
            parse(blank.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Record
        );
    }

    #[test]
    fn a_record_with_too_many_fields_is_refused() {
        let document = format!("{}{}\n", minimal(), "x\t".repeat(12));
        assert_eq!(
            parse(document.as_bytes(), None, None).unwrap_err(),
            AnalysisBridgeError::Record
        );
    }

    #[test]
    fn numbers_have_exactly_one_spelling() {
        assert_eq!(parse_u64("0"), Some(0));
        assert_eq!(parse_u64("07"), None, "no leading zeros");
        assert_eq!(parse_u64(""), None);
        assert_eq!(parse_u64("-1"), None);
        assert_eq!(parse_u64("1 "), None);
        assert_eq!(
            parse_u64("18446744073709551616"),
            None,
            "overflow is refused"
        );
        assert_eq!(parse_i32("-0"), None, "no negative zero");
        assert_eq!(parse_i32("-1"), Some(-1));
        assert_eq!(parse_i32("2147483647"), Some(i32::MAX));
        assert_eq!(parse_i32("2147483648"), None);
        assert_eq!(parse_i32("-2147483648"), Some(i32::MIN));
        assert_eq!(parse_i32("-2147483649"), None);
    }
}
