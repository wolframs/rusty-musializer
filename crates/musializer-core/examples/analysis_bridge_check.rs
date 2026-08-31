//! Parse one helper-produced bridge through the native application boundary.
//!
//! The identity arguments are optional so the shell gate can also feed this a
//! deliberately broken artifact and assert the refusal. When they are given
//! they are the point of the check: `analysis_bridge::parse` guards the AUDIO
//! record against them exactly, and the audit's finding B9 was that this
//! example passed `(None, None)` — so a helper writing the wrong digest or the
//! wrong duration produced a bridge that the end-to-end gate imported happily,
//! and neither test suite could see it.

use std::path::PathBuf;

use musializer_core::project::analysis_bridge;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: analysis_bridge_check FILE [SHA256 [DURATION_MS]]")?;
    let expected_sha256 = match args.next() {
        Some(value) => Some(value.into_string().map_err(|_| "SHA256 is not UTF-8")?),
        None => None,
    };
    let expected_duration_ms = match args.next() {
        Some(value) => Some(
            value
                .into_string()
                .map_err(|_| "DURATION_MS is not UTF-8")?
                .parse::<u64>()?,
        ),
        None => None,
    };

    let input = std::fs::read(&path)?;
    let bridge = analysis_bridge::parse(&input, expected_sha256.as_deref(), expected_duration_ms)?;
    println!(
        "bridge: lyrics={} sections={} semantics={} notes={} audio={} duration_ms={}",
        bridge.lyrics.len(),
        bridge.sections.len(),
        bridge.semantic_cues.len(),
        bridge.semantic_notes.len(),
        // Echoed rather than assumed: with no identity argument these are the
        // only statement of what was imported, and with one they prove the
        // guard compared the record rather than an argument it also printed.
        bridge.audio_sha256,
        bridge.duration_ms,
    );
    Ok(())
}
