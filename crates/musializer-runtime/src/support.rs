//! Files that make the optional external workflows a runnable product.
//!
//! The application itself is Rust. Assisted analysis and Google Fonts import
//! remain independent first-party tools because they supervise FFmpeg,
//! whisper.cpp, Codex, and explicit network requests. Source runs discover the
//! helpers under `tools/`; a future portable archive must copy this exact list.
//! Keeping the allowlist here makes the runtime crate, packaging, and tests
//! share one authority instead of maintaining three almost-identical lists.

use std::path::{Path, PathBuf};

/// Repository-relative files required by the external support bundle.
///
/// Do not add caches, credentials, generated analysis, or Python bytecode.
pub const DISTRIBUTION_SUPPORT_FILES: &[&str] = &[
    "tools/ANALYSIS_ADAPTERS.md",
    "tools/MEASURED_ANALYSIS.md",
    "tools/analysis_io.py",
    "tools/analyze_audio.py",
    "tools/external_analysis.py",
    "tools/google_fonts.py",
    "tools/import_whisper.py",
    "tools/lyric_align.py",
    "tools/mimo_openrouter.py",
    "tools/musializer_doctor.py",
    "prompts/lyrics_cleanup_system.md",
    "schemas/analysis-cache-v1.schema.json",
    "schemas/analysis-provenance-v1.schema.json",
    "schemas/codex-lyric-review-output-v1.schema.json",
    "schemas/font-import-v1.schema.json",
    "schemas/lyric-review-v1.schema.json",
    "schemas/lyric-sync-v1.schema.json",
    "schemas/lyric-timing-v1.schema.json",
    "schemas/measured-analysis-v1.schema.json",
    "schemas/project-v1.schema.json",
    "schemas/scene-plan-v1.schema.json",
    "schemas/semantic-notes-v1.schema.json",
    "schemas/semantic-score-v1.schema.json",
];

/// Returns the manifest entries absent from a checkout or staged distribution.
#[must_use]
pub fn missing_support_files(root: &Path) -> Vec<PathBuf> {
    DISTRIBUTION_SUPPORT_FILES
        .iter()
        .map(PathBuf::from)
        .filter(|relative| !root.join(relative).is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_checked_out_support_bundle_matches_the_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(missing_support_files(&root), Vec::<PathBuf>::new());
    }

    #[test]
    fn manifest_entries_are_unique_safe_relative_paths() {
        let unique: BTreeSet<_> = DISTRIBUTION_SUPPORT_FILES.iter().copied().collect();
        assert_eq!(unique.len(), DISTRIBUTION_SUPPORT_FILES.len());
        for relative in DISTRIBUTION_SUPPORT_FILES {
            let path = Path::new(relative);
            assert!(path.is_relative(), "absolute support path: {relative}");
            assert!(
                !path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir)),
                "support path escapes its distribution root: {relative}"
            );
        }
    }

    #[test]
    fn missing_bundle_is_detected_by_name() {
        let absent = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitely-not-a-bundle");
        let missing = missing_support_files(&absent);
        assert_eq!(missing.len(), DISTRIBUTION_SUPPORT_FILES.len());
        assert!(missing.contains(&PathBuf::from("tools/external_analysis.py")));
        assert!(missing.contains(&PathBuf::from("schemas/measured-analysis-v1.schema.json")));
    }
}
