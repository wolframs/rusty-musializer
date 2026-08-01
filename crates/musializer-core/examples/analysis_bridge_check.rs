//! Parse one helper-produced bridge through the native application boundary.

use std::path::PathBuf;

use musializer_core::project::analysis_bridge;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: analysis_bridge_check FILE")?;
    let input = std::fs::read(&path)?;
    let bridge = analysis_bridge::parse(&input, None, None)?;
    println!(
        "bridge: lyrics={} sections={} semantics={} notes={}",
        bridge.lyrics.len(),
        bridge.sections.len(),
        bridge.semantic_cues.len(),
        bridge.semantic_notes.len()
    );
    Ok(())
}
