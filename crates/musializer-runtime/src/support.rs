//! Files that make the optional external workflows a runnable product.
//!
//! The application itself is Rust. Assisted analysis and Google Fonts import
//! remain independent first-party tools because they supervise FFmpeg,
//! whisper.cpp, Codex, and explicit network requests. Source runs discover the
//! helpers under `tools/`; a future portable archive must copy this exact list.
//! Keeping the allowlist here makes the runtime crate, packaging, and tests
//! share one authority instead of maintaining three almost-identical lists.
//!
//! The Python half of the list is **not** trusted to stay right by hand. It was
//! wrong for months — six helpers were missing, so a distribution extracted from
//! it could not run Assist at all — and every one of the six was reachable by
//! reading the sources rather than by remembering: `musializer_doctor.py`
//! imports `runtime_inventory`, `external_analysis.py` spawns
//! `anchor_block_align.py`, which imports `lyric_anchor_block`, and the two
//! catalog helpers the interface spawns both import `atomic_cache`. So
//! `assist_helper_closure_matches_the_manifest` derives the set instead: it
//! seeds from the `"NAME.py"` literals the Rust actually spawns, follows local
//! imports and spawned-script names transitively, and compares the closure to
//! this list in **both** directions.

use std::path::{Path, PathBuf};

/// Repository-relative files required by the external support bundle.
///
/// The `tools/*.py` rows are derived rather than curated — see the module
/// comment and `assist_helper_closure_matches_the_manifest`.
///
/// Do not add caches, credentials, generated analysis, or Python bytecode.
pub const DISTRIBUTION_SUPPORT_FILES: &[&str] = &[
    "tools/ANALYSIS_ADAPTERS.md",
    "tools/MEASURED_ANALYSIS.md",
    "tools/analysis_io.py",
    "tools/analyze_audio.py",
    "tools/anchor_block_align.py",
    "tools/antigravity_audio.py",
    "tools/antigravity_lyrics.py",
    "tools/authored_audio_boundaries.py",
    "tools/authored_audio_occurrences.py",
    "tools/authored_audio_phrases.py",
    "tools/observed_audio_occurrences.py",
    "tools/local_lyric_spelling.py",
    "tools/local_lyric_recovery.py",
    "tools/ctc_window_align.py",
    "tools/atomic_cache.py",
    "tools/codex_model_discovery.py",
    "tools/external_analysis.py",
    "tools/force_align_lyrics.py",
    "tools/google_fonts.py",
    "tools/import_whisper.py",
    "tools/lyric_align.py",
    "tools/lyric_anchor_block.py",
    "tools/mimo_openrouter.py",
    "tools/musializer_doctor.py",
    "tools/provider_catalog.py",
    "tools/runtime_inventory.py",
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

    /// Every `tools/*.py` module name present in a checkout.
    fn helpers_on_disk(root: &Path) -> BTreeSet<String> {
        std::fs::read_dir(root.join("tools"))
            .expect("tools/ is readable")
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().is_some_and(|extension| extension == "py") {
                    path.file_stem()?.to_str().map(str::to_owned)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Every `*.rs` file under `directory`, recursively.
    fn rust_sources(directory: &Path, into: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, into);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                into.push(path);
            }
        }
    }

    /// The module names in every `NAME.py` token of `text`.
    ///
    /// Deliberately blunt: a helper named in a docstring counts, because
    /// over-inclusion costs one manifest line and under-inclusion costs a
    /// distribution that cannot run. The two forms that carry the dependency
    /// are an interpreter path (`ROOT / "tools" / "anchor_block_align.py"`) and
    /// a provenance string (`"adapter": "tools/analyze_audio.py"`).
    fn py_tokens(text: &str) -> BTreeSet<String> {
        let bytes = text.as_bytes();
        let mut found = BTreeSet::new();
        for (index, _) in text.match_indices(".py") {
            let mut start = index;
            while start > 0 {
                let previous = bytes[start - 1];
                if previous.is_ascii_alphanumeric() || previous == b'_' {
                    start -= 1;
                } else {
                    break;
                }
            }
            if start < index {
                found.insert(text[start..index].to_owned());
            }
        }
        found
    }

    /// The module names of `import X` / `from X import ...` statements.
    fn py_imports(text: &str) -> BTreeSet<String> {
        text.lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let rest = line
                    .strip_prefix("import ")
                    .or_else(|| line.strip_prefix("from "))?;
                let name = rest
                    .split(|character: char| {
                        !(character.is_ascii_alphanumeric() || character == '_')
                    })
                    .next()?;
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_owned())
                }
            })
            .collect()
    }

    /// The helpers the application itself reaches for: `"NAME.py"` literals
    /// outside a comment, kept only when `tools/NAME.py` exists.
    ///
    /// Comments are excluded because doc comments here cite tools that are not
    /// part of the product — `tools/timeline_lane_alignment.py` is a capture
    /// check — and a manifest carrying those would be making a different claim
    /// than "this is what Assist needs".
    fn rust_spawned_helpers(root: &Path, on_disk: &BTreeSet<String>) -> BTreeSet<String> {
        let mut sources = Vec::new();
        rust_sources(&root.join("crates"), &mut sources);
        let mut found = BTreeSet::new();
        for source in sources {
            let text = std::fs::read_to_string(&source).expect("a Rust source is readable");
            for line in text.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || trimmed.starts_with('*') {
                    continue;
                }
                for token in py_tokens(line) {
                    // A quoted literal is a spawn; a bare mention is prose.
                    if line.contains(&format!("\"{token}.py\"")) && on_disk.contains(&token) {
                        found.insert(token);
                    }
                }
            }
        }
        found
    }

    /// Every Python helper reachable from what the application spawns.
    ///
    /// Seeded **only** from the Rust, never from the manifest: seeding from the
    /// list under test would make every entry reachable by construction and the
    /// unreachable-entry half of the assertion vacuous. That mistake was made
    /// once while writing this and caught by its own control.
    fn assist_helper_closure(root: &Path) -> BTreeSet<String> {
        let on_disk = helpers_on_disk(root);
        let mut pending: Vec<String> = rust_spawned_helpers(root, &on_disk).into_iter().collect();
        let mut seen = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let text = std::fs::read_to_string(root.join("tools").join(format!("{name}.py")))
                .expect("a reachable helper is readable");
            for reference in py_tokens(&text).into_iter().chain(py_imports(&text)) {
                if on_disk.contains(&reference) && !seen.contains(&reference) {
                    pending.push(reference);
                }
            }
        }
        seen
    }

    /// The manifest's `tools/*.py` module names.
    fn listed_helpers() -> BTreeSet<String> {
        DISTRIBUTION_SUPPORT_FILES
            .iter()
            .filter_map(|entry| entry.strip_prefix("tools/")?.strip_suffix(".py"))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_checked_out_support_bundle_matches_the_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(missing_support_files(&root), Vec::<PathBuf>::new());
    }

    /// The check DX8 exists for: the manifest may not drift from the sources.
    ///
    /// Negative controls, 2026-08-29: dropping any one of the six helpers this
    /// item added fails the first assertion by name, and adding a real but
    /// unreached helper (`tools/code_map.py`) fails the second. The derived
    /// closure is exactly the fifteen the plan had hand-listed — found by
    /// following imports and spawn literals, not by memory.
    #[test]
    fn assist_helper_closure_matches_the_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let listed = listed_helpers();
        let reachable = assist_helper_closure(&root);

        let uncovered: Vec<_> = reachable.difference(&listed).cloned().collect();
        assert!(
            uncovered.is_empty(),
            "DISTRIBUTION_SUPPORT_FILES omits Python helpers Assist reaches, so an \
             extracted distribution would not run: {uncovered:?}"
        );

        // The other direction: an entry nothing reaches is dead weight, and
        // silently keeping one is how the list stops meaning anything.
        let unreached: Vec<_> = listed.difference(&reachable).cloned().collect();
        assert!(
            unreached.is_empty(),
            "DISTRIBUTION_SUPPORT_FILES lists Python helpers nothing reaches: {unreached:?}"
        );
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
