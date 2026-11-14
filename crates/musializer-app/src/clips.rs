//! A track's video clips (post-legacy, 2026-11-14, VG): importing a file into
//! the track's library, and the treatment's clip files.
//!
//! A clip in the library is its asset — digest, size, frame count, the name it
//! came in under — and where its bytes are on this machine. Importing reads
//! the file once through [`musializer_runtime::clip::ClipSource`]: the digest
//! and the probe come from the same handle. Saving bundles it like the audio
//! (`project.rs`); drawing opens it again, by its own handle, in a decoder
//! worker that checks the digest before it decodes a frame.

use std::path::{Path, PathBuf};

use musializer_core::project::clip::{self, ClipAsset, ClipPlacement, ClipRef};
use musializer_core::scene::SceneId;
use musializer_core::treatment::Treatment;
use musializer_core::ui::notice::Severity;
use musializer_runtime::clip::{probe, ClipError, ClipSource};

/// One clip a track holds.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackClip {
    /// `path` is where the last save bundled it, empty before the first.
    pub asset: ClipAsset,
    /// Where the bytes are on this machine: the imported file, or the bundled
    /// copy once saved.
    pub source: PathBuf,
    /// Why the project's record cannot be read, decided when it was opened: a
    /// bundled file that is missing, or a path that leaves the project's
    /// folder. Such a clip draws its card and is never opened.
    pub unusable: Option<ClipError>,
}

impl TrackClip {
    #[must_use]
    pub fn clip_ref(&self) -> Option<ClipRef> {
        self.asset.clip_ref()
    }
}

/// The clip a library holds for `clip`.
#[must_use]
pub fn find(clips: &[TrackClip], clip: ClipRef) -> Option<&TrackClip> {
    clips.iter().find(|entry| entry.clip_ref() == Some(clip))
}

/// The name a clip goes by: its file name, cut to the asset's bound at a
/// character boundary.
fn display_name(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || "clip".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let name = name.replace('\0', "");
    let mut end = name.len().min(clip::CLIP_NAME_MAX_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        "clip".to_owned()
    } else {
        name[..end].to_owned()
    }
}

/// Reads a video file into a library entry: opened once, hashed and probed
/// through that handle, and bounded.
///
/// # Errors
///
/// Whatever [`ClipSource::open`] or [`probe`] refuse, by name.
pub fn import(path: &Path) -> Result<TrackClip, ClipError> {
    let source = ClipSource::open(path)?;
    let sha256 = source.sha256_hex()?;
    let probe = probe(&source)?;
    let asset = ClipAsset {
        path: String::new(),
        sha256,
        name: display_name(path),
        width: probe.width,
        height: probe.height,
        frames: probe.timing.frame_count() as u32,
        duration_seconds: probe.timing.duration(),
    };
    Ok(TrackClip {
        asset,
        source: path.to_path_buf(),
        unusable: None,
    })
}

/// Adds a clip to a library, once: a file already there by content keeps its
/// entry — unless that entry cannot be read (missing, or recorded outside the
/// project), when the file imported now becomes its bytes. Returns its
/// digest, or `None` when the library is full.
pub fn add(clips: &mut Vec<TrackClip>, entry: TrackClip) -> Option<ClipRef> {
    let clip = entry.clip_ref()?;
    if let Some(existing) = clips
        .iter_mut()
        .find(|existing| existing.clip_ref() == Some(clip))
    {
        if existing.unusable.is_some() {
            existing.source = entry.source;
            existing.unusable = None;
        }
        return Some(clip);
    }
    if clips.len() >= clip::MAX_CLIPS {
        return None;
    }
    clips.push(entry);
    Some(clip)
}

/// The clips a track's placements name, in library order: what a save writes.
#[must_use]
pub fn referenced(
    clips: &[TrackClip],
    base: Option<ClipPlacement>,
    cues: impl Iterator<Item = Option<ClipPlacement>>,
) -> Vec<&TrackClip> {
    let mut used: Vec<ClipRef> = base.into_iter().map(|placement| placement.clip).collect();
    used.extend(cues.flatten().map(|placement| placement.clip));
    clips
        .iter()
        .filter(|entry| entry.clip_ref().is_some_and(|clip| used.contains(&clip)))
        .collect()
}

/// Reads every clip file a treatment names, relative to the treatment, and
/// attaches it. Returns the entries for the track's library.
///
/// # Errors
///
/// The first file that cannot be read or probed, named with the place in the
/// treatment that names it, or a refusal of the treatment's own (a pinned
/// digest, a `from` past the clip's end).
pub fn attach_treatment_clips(
    treatment: &mut Treatment,
    treatment_path: &Path,
) -> Result<Vec<(String, TrackClip)>, String> {
    let mut entries = Vec::new();
    for file in treatment.clip_files() {
        let declared = Path::new(&file.path);
        let resolved = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            treatment_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(declared)
        };
        let entry = import(&resolved)
            .map_err(|error| format!("{}: {}: {error}", file.place, resolved.display()))?;
        treatment
            .attach_clip(&file.path, entry.asset.clone())
            .map_err(|error| error.to_string())?;
        entries.push((file.path, entry));
    }
    Ok(entries)
}

/// Import a clip picked or dropped in the window (VG): read it, put it in the
/// track's library, and have it play — in the section the lane has selected,
/// else the one under the playhead, when the plan drives; as the track's own
/// clip otherwise. The scene becomes Video Clip there first, the way a tile
/// press retargets that section.
pub fn import_command(app: &mut crate::App, path: &Path, time_seconds: f64, now: f64) {
    if app.workspace.current().is_none() {
        app.shell.notify(
            Severity::Warning,
            "Open a track first",
            "A clip plays in a section of a track's video.",
        );
        return;
    }
    let entry = match import(path) {
        Ok(entry) => entry,
        Err(error) => {
            app.shell.notify(
                Severity::Error,
                "Clip could not be imported",
                &format!("{}: {error}.", path.display()),
            );
            return;
        }
    };
    if app.scene.id() != SceneId::Clip {
        app.select_scene_interactive(SceneId::Clip, time_seconds, now);
    }
    let selected = app.shell.scene_lane.selected_id;
    let Some(track) = app.workspace.current_mut() else {
        return;
    };
    let (name, size, seconds) = (
        entry.asset.name.clone(),
        (entry.asset.width, entry.asset.height),
        entry.asset.duration_seconds,
    );
    let Some(clip) = add(&mut track.clips, entry) else {
        app.shell.notify(
            Severity::Error,
            "Clip could not be imported",
            &format!("The project already holds {} clips.", clip::MAX_CLIPS),
        );
        return;
    };
    let placement = ClipPlacement::new(clip);
    let plan = &track.scene_switches;
    let target = (plan.enabled && !plan.is_empty())
        .then(|| {
            let cues = plan.cues();
            cues.iter()
                .position(|cue| cue.id == selected)
                .or_else(|| {
                    cues.iter().position(|cue| {
                        cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds
                    })
                })
                .map(|index| (index, cues[index]))
        })
        .flatten()
        .filter(|(_, cue)| cue.scene_index == SceneId::Clip.index() as u32);
    let place = match target {
        Some((index, cue)) => {
            track.scene_switches.set_clip(cue.id, Some(placement));
            format!("section {} of {}", index + 1, track.scene_switches.len())
        }
        None => {
            track.base_clip = Some(placement);
            "the Video Clip scene".to_owned()
        }
    };
    track.mark_dirty(now);
    app.shell.notify(
        Severity::Info,
        "Clip imported",
        &format!(
            "{name}, {}x{}, {seconds:.1} s, plays in {place}. Its sound is ignored; the track is the music.",
            size.0, size.1
        ),
    );
}
