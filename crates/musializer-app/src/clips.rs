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
use musializer_core::project::scene_switch::SceneSwitchTimeline;
use musializer_core::scene::SceneId;
use musializer_core::treatment::Treatment;
use musializer_core::ui::notice::Severity;
use musializer_runtime::clip::{probe, ClipError, ClipLocation, ClipSource, ClipWorker, Expected};

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
    /// The project folder `source` was bundled into, when it is a bundled
    /// copy: its decoder then opens `asset.path` beneath that folder, one name
    /// at a time, so a symlink swapped in after the open's containment check
    /// leads nowhere ([`ClipSource::open_beneath`]). `None` for a file the
    /// person chose, which is opened as it is.
    pub bundle: Option<PathBuf>,
}

impl TrackClip {
    /// What a decoder opens for this clip.
    #[must_use]
    pub fn location(&self) -> ClipLocation {
        match &self.bundle {
            Some(folder) => ClipLocation::Bundled {
                folder: folder.clone(),
                relative: self.asset.path.clone(),
            },
            None => ClipLocation::File(self.source.clone()),
        }
    }

    /// A decoder for this clip, checking the bytes against its record.
    #[must_use]
    pub fn worker(&self) -> ClipWorker {
        ClipWorker::spawn(
            self.location(),
            Expected {
                sha256: self.asset.sha256.clone(),
                width: self.asset.width,
                height: self.asset.height,
                frames: self.asset.frames,
            },
        )
    }

    #[must_use]
    pub fn clip_ref(&self) -> Option<ClipRef> {
        self.asset.clip_ref()
    }
}

/// Why "Generate…" is drawn disabled, wherever it is (the Scenes flyout's
/// footer, Look's section card): the one sentence both say.
pub const GENERATE_NOT_BUILT: &str =
    "Generating clips is not built yet: a model that starts and ends on this \
     section's frames, with its price shown before anything is spent";

/// What "Import clip…" does, wherever it is.
pub const IMPORT_HINT: &str =
    "Play a video file in this section: a generated clip, footage, or an export. \
     It is bundled with the project; its sound is ignored";

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
        bundle: None,
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
            existing.bundle = entry.bundle;
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

/// Which section an imported clip goes to (VG, F1b): what the user pointed
/// at, decided before anything changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImportTarget {
    /// No plan drives the video: the clip becomes the track's own, and the
    /// track's scene Video Clip.
    Track,
    /// One section of the plan plays it.
    Section {
        index: usize,
        total: usize,
        cue_id: u64,
        start_seconds: f64,
        end_seconds: f64,
        /// Picked in the lane, or under the playhead (nothing picked).
        selected: bool,
        /// It plays another scene and becomes Video Clip first.
        retarget: bool,
    },
}

/// The section an import lands in, by the rule a scene tile press uses
/// (`App::select_scene_interactive`): the section selected in the lane when
/// there is one, else the one under the playhead. Whatever scene it plays,
/// and whatever scene is under the playhead: a selected Spectrum section with
/// the playhead in a clip section gets the clip, rather than the track.
#[must_use]
pub fn import_target(plan: &SceneSwitchTimeline, selected: u64, time_seconds: f64) -> ImportTarget {
    if !plan.enabled || plan.is_empty() {
        return ImportTarget::Track;
    }
    let cues = plan.cues();
    let picked = cues.iter().position(|cue| cue.id == selected);
    let index = picked
        .or_else(|| {
            cues.iter()
                .position(|cue| cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds)
        })
        .unwrap_or(cues.len() - 1);
    let cue = cues[index];
    ImportTarget::Section {
        index,
        total: cues.len(),
        cue_id: cue.id,
        start_seconds: cue.start_seconds,
        end_seconds: cue.end_seconds,
        selected: picked.is_some(),
        retarget: cue.scene_index != SceneId::Clip.index() as u32,
    }
}

/// Where the import's notice says the clip plays.
#[must_use]
pub fn import_place(target: ImportTarget) -> String {
    match target {
        ImportTarget::Track => {
            "as the track's own clip (Video Clip is the track's scene; no plan drives the video)"
                .to_owned()
        }
        ImportTarget::Section {
            index,
            total,
            start_seconds,
            end_seconds,
            selected,
            retarget,
            ..
        } => format!(
            "in section {} of {total} ({} to {}), the one {}{}",
            index + 1,
            crate::ui::widgets::format_timestamp(start_seconds),
            crate::ui::widgets::format_timestamp(end_seconds),
            if selected {
                "selected in the lane"
            } else {
                "under the playhead"
            },
            if retarget { ", now Video Clip" } else { "" }
        ),
    }
}

/// Import a clip picked or dropped in the window (VG): read it, put it in the
/// track's library, and have it play where [`import_target`] says — the
/// section selected in the lane, else the one under the playhead, made Video
/// Clip first when it plays another scene; with no plan driving, as the
/// track's own clip, the track's scene becoming Video Clip. The notice names
/// which.
pub fn import_command(app: &mut crate::App, path: &Path, time_seconds: f64, now: f64) {
    let Some(track) = app.workspace.current() else {
        app.shell.notify(
            Severity::Warning,
            "Open a track first",
            "A clip plays in a section of a track's video.",
        );
        return;
    };
    let target = import_target(
        &track.scene_switches,
        app.shell.scene_lane.selected_id,
        time_seconds,
    );
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
    match target {
        ImportTarget::Section {
            cue_id, retarget, ..
        } => {
            if retarget {
                if let Err(error) = track.retarget_scene_cue(cue_id, SceneId::Clip) {
                    app.shell.notify(
                        Severity::Warning,
                        "Clip could not be placed",
                        &format!("{name} is in the project, but the section could not become Video Clip: {error}."),
                    );
                    return;
                }
            }
            track.scene_switches.set_clip(cue_id, Some(placement));
            track.mark_dirty(now);
            // A retarget rewinds the plan's cursor: this rebinds the live
            // scene when the section is the one playing.
            app.apply_auto_scene_switch(time_seconds);
        }
        ImportTarget::Track => {
            track.base_clip = Some(placement);
            track.mark_dirty(now);
            app.select_scene(SceneId::Clip, now);
        }
    }
    app.shell.notify(
        Severity::Info,
        "Clip imported",
        &format!(
            "{name}, {}x{}, {seconds:.1} s, plays {}. Its sound is ignored; the track is the music.",
            size.0,
            size.1,
            import_place(target)
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::project::scene_switch::SceneSwitchCue;
    use musializer_core::scene::SCENE_COUNT;

    fn cue(id: u64, start: f64, end: f64, scene: SceneId) -> SceneSwitchCue {
        SceneSwitchCue {
            id,
            start_seconds: start,
            end_seconds: end,
            scene_index: scene.index() as u32,
            strength: 0.5,
            settings: Default::default(),
            transition: musializer_core::project::transition::Transition::CUT,
            grade: None,
            glide: 0.0,
            pixel: None,
            caption_style: None,
            clip: None,
        }
    }

    /// Spectrum 0-10, Video Clip 10-20, Aurora 20-30.
    fn plan(enabled: bool) -> SceneSwitchTimeline {
        let mut plan = SceneSwitchTimeline::new();
        plan.replace(
            &[
                cue(1, 0.0, 10.0, SceneId::Spectrum),
                cue(2, 10.0, 20.0, SceneId::Clip),
                cue(3, 20.0, 30.0, SceneId::AuroraVeil),
            ],
            30.0,
            SCENE_COUNT as u32,
        )
        .unwrap();
        plan.enabled = enabled;
        plan
    }

    fn landed(target: ImportTarget) -> (u64, bool, bool) {
        match target {
            ImportTarget::Section {
                cue_id,
                selected,
                retarget,
                ..
            } => (cue_id, selected, retarget),
            ImportTarget::Track => (0, false, false),
        }
    }

    /// F1b defect 8: every combination of playhead and selection lands where
    /// the user pointed. The case the review found: the playhead in the clip
    /// section and the Spectrum section selected put the clip on the track.
    #[test]
    fn an_import_lands_in_the_section_pointed_at() {
        let on = plan(true);
        // Nothing selected: the section under the playhead, whatever it plays.
        assert_eq!(landed(import_target(&on, 0, 15.0)), (2, false, false));
        assert_eq!(landed(import_target(&on, 0, 5.0)), (1, false, true));
        // Selected: that section, wherever the playhead is.
        assert_eq!(landed(import_target(&on, 1, 15.0)), (1, true, true));
        assert_eq!(landed(import_target(&on, 2, 25.0)), (2, true, false));
        assert_eq!(landed(import_target(&on, 3, 5.0)), (3, true, true));
        // A selection that is not in the plan is no selection; past the end
        // is the last section, as a tile press does.
        assert_eq!(landed(import_target(&on, 99, 15.0)), (2, false, false));
        assert_eq!(landed(import_target(&on, 0, 30.0)), (3, false, true));
        // No plan driving: the track, selection or not.
        assert_eq!(import_target(&plan(false), 1, 15.0), ImportTarget::Track);
        assert_eq!(
            import_target(&SceneSwitchTimeline::new(), 0, 0.0),
            ImportTarget::Track
        );
    }

    #[test]
    fn the_notice_says_where_it_landed() {
        let place = import_place(import_target(&plan(true), 1, 15.0));
        assert!(place.contains("section 1 of 3"), "{place}");
        assert!(place.contains("selected in the lane"), "{place}");
        assert!(place.contains("now Video Clip"), "{place}");
        let place = import_place(import_target(&plan(true), 0, 15.0));
        assert!(place.contains("section 2 of 3") && place.contains("under the playhead"));
        assert!(!place.contains("now Video Clip"), "{place}");
        assert!(import_place(ImportTarget::Track).contains("track's own clip"));
    }
}
