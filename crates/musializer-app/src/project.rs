//! Saving a [`Track`] to a `.musi` and opening one back into a [`Track`].
//!
//! Port of `plug.c`'s project half: `build_project` (`:4268-4411`),
//! `bundle_project_asset` (`:4413-4432`), `save_project_to_path` (`:4441-4613`),
//! `save_project_as` (`:4615-4639`), `save_project` (`:4641-4646`),
//! `open_project_path` (`:4665-5044`) and `poll_project_autosave` (`:5065-5075`).
//!
//! Everything underneath already exists: the codec is
//! [`musializer_core::project::io`], the model is `project::model`, the
//! transactional write is `runtime::process::publish`, and the filesystem half —
//! resolution, hashing, bundling — is [`musializer_runtime::project_files`].
//! What is here is the translation between a live track and the file, and the
//! order the two halves happen in.
//!
//! # Why open and save are asymmetric
//!
//! Saving publishes assets *before* it writes the project, and adopts the
//! published copies afterwards (`plug.c:4585-4600`), so a track that has been
//! saved once refers to its own bundle. Opening verifies every asset's digest
//! *before* it touches the workspace, and builds the whole track before anything
//! is replaced. Neither operation leaves a half-state: a failed save preserves
//! the previous file, and a failed open leaves the session exactly as it was.
//!
//! # Where `export_mappings` lives
//!
//! In [`musializer_core::scene::routes`], beside the evaluation it persists —
//! moved there from `project::model` by item G. Called from where it is rather
//! than duplicated here, because splitting the constant rule from the route rule
//! lets one parameter persist as both.

use std::path::{Path, PathBuf};

use musializer_core::project::clip::ClipAsset;
use musializer_core::project::io;
use musializer_core::project::model::{
    AsciiImageAsset, AssetMode, AudioAsset, Metadata, OutputQuality, OutputSettings, Project,
    SceneEntry, ScenePreset, SceneSwitchSuggestion, SceneSwitchSuggestions,
};
use musializer_core::project::preset_store::PresetLibrary;
use musializer_core::project::scene_switch::{SceneSwitchCue, SceneSwitchTimeline};
use musializer_core::scene::routes;
use musializer_core::scene::settings::SettingsSnapshot;
use musializer_core::scene::{SceneId, SCENE_COUNT};
use musializer_core::timing::render_export::{Quality, RenderExportConfig};
use musializer_runtime::clip::{ClipError, ClipSource};
use musializer_runtime::process::publish;
use musializer_runtime::project_files::{self, AssetCategory};

use crate::workspace::{AsciiImage, Track, Workspace};

/// The version string written into every project's metadata
/// (`plug.c:4294-4295`).
///
/// Deliberately the oracle's literal rather than this crate's version. It is a
/// **file field**: a `.musi` this build writes must be indistinguishable from one
/// the frozen C wrote, and the round-trip test at the parity gate compares them.
/// The three *display* spellings of the version are a separate open question and
/// do not belong here.
pub const APPLICATION_VERSION: &str = "musializer-2026.07";

/// Why a project could not be saved or opened.
///
/// Each carries the message the C's notice would have shown, because these reach
/// the tray verbatim and the wording is what tells a user which of several
/// similar failures they hit.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("Use the Musializer project extension so launchers and drag-and-drop can identify the file.")]
    NotMusiExtension,
    #[error("Choose another .musi path. The source track was not modified.")]
    AliasesSourceAudio,
    #[error("the audio file could not be hashed: {0}")]
    AudioIdentity(#[source] std::io::Error),
    #[error("A track path, authored lane, or output setting is outside the project contract: {0}")]
    Build(String),
    #[error("the project could not be encoded: {0}")]
    Encode(#[from] musializer_core::project::io::ProjectIoError),
    #[error("The previous project file was preserved: {0}.")]
    Write(#[source] publish::PublishError),
    #[error("an asset could not be bundled: {0}")]
    Bundle(#[from] musializer_runtime::project_files::BundleError),
    #[error("The file is empty or exceeds the 4 MiB project limit.")]
    Size,
    #[error("the project file could not be read: {0}")]
    Read(#[source] std::io::Error),
    #[error("{0}")]
    Unsupported(String),
    #[error("The referenced audio is missing or its SHA-256 identity changed.")]
    AudioMismatch,
    #[error("The bundled image is missing, outside the project, or changed identity.")]
    AsciiMismatch,
    #[error("The bundled face is missing, outside the project, or changed identity. Opening it would typeset the captions in a substitute face and then save that substitution over your choice.")]
    FontMismatch,
    #[error("The face is bundled but the licence recorded beside it is missing or changed. Saving would publish the font without the terms it was distributed under.")]
    LicenceMismatch,
    #[error("A stored control or route is unknown or outside its supported range.")]
    Mappings,
    #[error("Use an even resolution up to 7680x4320 and 1 to 240 fps.")]
    Output,
    #[error("the project scene is unavailable: {0}")]
    UnknownScene(String),
}

/// A non-fatal note about an otherwise successful open, so the caller can warn
/// without the open having failed (`plug.c:4841-4845`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenWarning {
    /// The audio resolved against the working directory rather than beside the
    /// project.
    LegacyAudioPath,
}

/// `build_project` (`plug.c:4268-4411`): the whole file, from a track.
///
/// The stored paths are parameters because only the bundler knows where an asset
/// ended up; the track knows the digest. Writing the track's own copy of the path
/// would record where it was *last* saved rather than where it is in this file
/// (`plug.c:4348-4351`).
///
/// The C reads sample rate and channels off the live `Music` handle; here they
/// come from the caller for the same reason the track model has no `Music`.
///
/// The track's clips (VG) are written with the paths their bytes are at now,
/// which is what a recovery snapshot wants; a save hands the bundled paths to
/// [`build_project_with_clips`] instead.
#[allow(clippy::too_many_arguments)]
pub fn build_project(
    track: &Track,
    stored_audio: &str,
    stored_ascii_image: Option<&str>,
    stored_font: Option<&str>,
    stored_licence: Option<&str>,
    sample_rate: u32,
    channels: u16,
) -> Result<Project, ProjectError> {
    let clips = referenced_clips(track)
        .into_iter()
        .map(|entry| ClipAsset {
            // A clip recorded outside its project has no path of its own here;
            // its name is written instead, which a recovery can never resolve,
            // so the refusal outlives the snapshot.
            path: if entry.unusable == Some(ClipError::OutsideProject) {
                entry.asset.name.clone()
            } else {
                entry.source.to_string_lossy().into_owned()
            },
            ..entry.asset.clone()
        })
        .collect();
    build_project_with_clips(
        track,
        stored_audio,
        stored_ascii_image,
        stored_font,
        stored_licence,
        sample_rate,
        channels,
        clips,
    )
}

/// The clips a save writes: those a placement names (VG).
fn referenced_clips(track: &Track) -> Vec<&crate::clips::TrackClip> {
    crate::clips::referenced(
        &track.clips,
        track.base_clip,
        track.scene_switches.cues().iter().map(|cue| cue.clip),
    )
}

/// [`build_project`] with the clip library as it will be stored.
#[allow(clippy::too_many_arguments)]
pub fn build_project_with_clips(
    track: &Track,
    stored_audio: &str,
    stored_ascii_image: Option<&str>,
    stored_font: Option<&str>,
    stored_licence: Option<&str>,
    sample_rate: u32,
    channels: u16,
    clips: Vec<ClipAsset>,
) -> Result<Project, ProjectError> {
    if stored_audio.is_empty() {
        return Err(ProjectError::Build("the audio asset has no path".into()));
    }
    let duration = track.duration_seconds;
    let mut metadata = match &track.project_metadata {
        Some(metadata) => metadata.clone(),
        None => Metadata {
            // `track-` plus the first 16 hex characters of the audio digest, so a
            // project id is derived from content rather than from a clock.
            project_id: format!(
                "track-{}",
                &track.audio_sha256[..track.audio_sha256.len().min(16)]
            ),
            title: file_stem(&track.file_path)
                .filter(|stem| !stem.is_empty())
                .unwrap_or("Untitled visualization")
                .to_string(),
            ..Metadata::default()
        },
    };
    metadata.application_version = APPLICATION_VERSION.to_string();
    // PXF-7: the playhead at this save is where a reopen resumes. Clamped onto
    // the track, and a non-number is the start, which is never written.
    let playhead = track.playhead_seconds;
    metadata.playhead_seconds = if playhead.is_finite() && playhead > 0.0 {
        playhead.min(duration)
    } else {
        0.0
    };
    let mut project = Project {
        metadata,
        ..Project::default()
    };

    project.audio = AudioAsset {
        mode: AssetMode::Imported,
        path: stored_audio.to_string(),
        sha256: track.audio_sha256.clone(),
        duration_seconds: duration,
        sample_rate,
        channels,
    };

    if let Some(ascii) = &track.ascii {
        let (Some(path), false) = (stored_ascii_image, ascii.sha256.is_empty()) else {
            return Err(ProjectError::Build(
                "the ASCII image has no bundled path or no digest".into(),
            ));
        };
        project.ascii_image = Some(AsciiImageAsset {
            path: path.to_string(),
            sha256: ascii.sha256.clone(),
            columns: ascii.columns as u32,
            rows: ascii.rows as u32,
        });
    }

    project.output = OutputSettings {
        width: track.render_config.width,
        height: track.render_config.height,
        fps_numerator: track.render_config.fps,
        fps_denominator: 1,
        start_seconds: 0.0,
        end_seconds: duration,
        quality: quality_to_output(track.render_config.quality),
        ..OutputSettings::default()
    };
    project.deterministic_seed = track.scene_seed;

    let mappings = routes::export_mappings(&track.scene_settings, Some(&track.scene_routes))
        .ok_or_else(|| ProjectError::Build("a setting or route is outside its bounds".into()))?;
    project.scenes = vec![SceneEntry {
        instance_id: if track.scene_instance_id != 0 {
            track.scene_instance_id
        } else {
            1
        },
        scene_type: track.base_scene.stable_name().to_string(),
        enabled: true,
        start_seconds: 0.0,
        end_seconds: duration,
        opacity: 1.0,
        mappings,
        clip: track.base_clip,
        ..SceneEntry::default()
    }];
    project.clips = clips;

    project.caption_style = track.caption_style.clone();
    if let Some(asset) = project.caption_style.font.as_mut() {
        let Some(font) = stored_font.filter(|path| !path.is_empty()) else {
            return Err(ProjectError::Build(
                "the caption face is present but was not bundled".into(),
            ));
        };
        asset.path = font.to_string();
        asset.licence_path = stored_licence.unwrap_or("").to_string();
        // A licence path and its digest are one fact. If the licence did not
        // travel, the digest and name must not claim that it did
        // (`plug.c:4358-4364`).
        if asset.licence_path.is_empty() {
            asset.licence_sha256.clear();
            asset.licence_name.clear();
        }
    }

    project.lyrics = track.lyrics.clone();
    project.semantic_events = track.semantic_events.clone();
    project.manual_events = track.manual_events.clone();
    project.scene_switches = SceneSwitchSuggestions {
        enabled: track.scene_switches.enabled,
        cues: track
            .scene_switches
            .cues()
            .iter()
            .map(|cue| SceneSwitchSuggestion {
                id: cue.id,
                scene_name: SceneId::from_index(cue.scene_index as usize)
                    .unwrap_or(SceneId::Spectrum)
                    .stable_name()
                    .to_string(),
                start_seconds: cue.start_seconds,
                end_seconds: cue.end_seconds,
                strength: cue.strength,
                // An uncaptured snapshot persists as no settings at all, rather
                // than as a zero-length captured one (`plug.c:4385-4386`).
                settings: if cue.settings.captured {
                    cue.settings.values[..cue.settings.count].to_vec()
                } else {
                    Vec::new()
                },
                transition: cue.transition,
                grade: cue.grade,
                glide: cue.glide,
                pixel: cue.pixel,
                caption_style: cue.caption_style,
                clip: cue.clip,
            })
            .collect(),
        outro: track.scene_switches.outro,
        finish: track.scene_switches.finish,
        titles: track.scene_switches.titles.clone(),
    };

    project.scene_presets = Vec::new();
    for scene in SceneId::ALL {
        for preset in track.scene_presets.presets(scene) {
            project.scene_presets.push(ScenePreset {
                id: preset.id,
                scene_name: scene.stable_name().to_string(),
                name: preset.name.clone(),
                settings: preset.snapshot.values[..preset.snapshot.count].to_vec(),
            });
        }
    }
    project.analysis_lanes = track.analysis_lanes.clone();

    project
        .validate()
        .map_err(|error| ProjectError::Build(error.to_string()))?;
    Ok(project)
}

/// `save_project_to_path` (`plug.c:4441-4613`).
///
/// On success the track adopts its own bundle: `file_path` becomes the published
/// audio, the caption face and licence become the published copies, and
/// `project_path` is recorded — so the next Save As re-bundles from inside this
/// project rather than from wherever the assets were first imported
/// (`plug.c:4585-4600`).
///
/// `reuse_published` is autosave's flag; see
/// [`project_files::bundle_asset`].
pub fn save_to_path(
    track: &mut Track,
    path: &Path,
    reuse_published: bool,
) -> Result<(), ProjectError> {
    // C4: the format comes off the track, not off a bound `Music` handle. That
    // change is what lets autosave write a background track at all — see
    // `Track::audio_sample_rate`.
    let (sample_rate, channels) = (track.audio_sample_rate, track.audio_channels);
    if sample_rate == 0 || channels == 0 {
        // Refused rather than written: a `.musi` recording 0 Hz would reopen as a
        // project whose audio asset disagrees with its own file, and the digest
        // check would not catch it because the bytes are fine.
        return Err(ProjectError::Build(
            "the track's audio format is not known yet, so it cannot be saved".into(),
        ));
    }
    let path_text = path
        .to_str()
        .ok_or_else(|| ProjectError::Build("the project path is not valid UTF-8".into()))?;
    if !path_text.ends_with(".musi") {
        return Err(ProjectError::NotMusiExtension);
    }
    // Before any work: writing a project over its own source audio would destroy
    // the thing being saved.
    if project_files::existing_files_alias(&track.file_path, path) {
        return Err(ProjectError::AliasesSourceAudio);
    }
    if track.audio_sha256.is_empty() {
        track.audio_sha256 = project_files::sha256_file_hex(&track.file_path)
            .map_err(ProjectError::AudioIdentity)?;
    }

    let audio = project_files::bundle_asset(
        path,
        AssetCategory::Audio,
        &track.file_path,
        &track.audio_sha256,
        reuse_published,
    )?;

    let ascii = match &track.ascii {
        Some(image) if !image.sha256.is_empty() => Some(project_files::bundle_asset(
            path,
            AssetCategory::Image,
            &image.path,
            &image.sha256,
            reuse_published,
        )?),
        _ => None,
    };

    let face = track.caption_style.font.clone();
    let font = match (&track.caption_font_path, &face) {
        (Some(source), Some(asset)) => Some(project_files::bundle_asset(
            path,
            AssetCategory::Font,
            source,
            &asset.sha256,
            reuse_published,
        )?),
        _ => None,
    };
    let licence = match (&track.caption_licence_path, &face) {
        (Some(source), Some(asset)) if font.is_some() && !asset.licence_sha256.is_empty() => {
            Some(project_files::bundle_asset(
                path,
                // Font, not a category of its own: the C bundles the licence
                // beside the face (`plug.c:4510`), and the content-addressed name
                // keeps them apart by digest.
                AssetCategory::Font,
                source,
                &asset.licence_sha256,
                reuse_published,
            )?)
        }
        _ => None,
    };

    // The clips a placement names (VG), bundled like the audio. A clip whose
    // bytes are gone is kept as it was recorded when this is the project it
    // was recorded in, so an autosave does not fail over a deleted take; it
    // cannot be bundled into a different one.
    let mut clips = Vec::new();
    let mut adopted = Vec::new();
    for entry in referenced_clips(track) {
        let same_project = track.project_path.as_deref() == Some(path);
        // A bundled copy is read the way its decoder reads it, beneath its
        // folder by its record (G): a symlink swapped in since the open makes
        // it unreadable, not a way to copy an outside file in.
        let held = entry
            .bundle
            .as_ref()
            .map(|folder| ClipSource::open_beneath(folder, &entry.asset.path));
        let unreadable = entry.unusable.is_some()
            || match &held {
                Some(opened) => opened.is_err(),
                None => !entry.source.is_file(),
            };
        if unreadable && same_project && !entry.asset.path.is_empty() {
            clips.push(entry.asset.clone());
            continue;
        }
        if let Some(error) = &entry.unusable {
            return Err(ProjectError::Build(format!(
                "the clip {} cannot be bundled: {error}",
                entry.asset.name
            )));
        }
        let held = held.transpose().map_err(|error| {
            ProjectError::Build(format!(
                "the clip {} cannot be bundled: {error}",
                entry.asset.name
            ))
        })?;
        let read = held
            .as_ref()
            .map_or_else(|| entry.source.clone(), ClipSource::held_path);
        let published = project_files::bundle_asset_named(
            path,
            AssetCategory::Clip,
            &read,
            &entry.source,
            &entry.asset.sha256,
            reuse_published,
        )?;
        clips.push(ClipAsset {
            path: published.stored.clone(),
            ..entry.asset.clone()
        });
        adopted.push((entry.asset.sha256.clone(), published));
    }

    let project = build_project_with_clips(
        track,
        &audio.stored,
        ascii.as_ref().map(|a| a.stored.as_str()),
        font.as_ref().map(|f| f.stored.as_str()),
        licence.as_ref().map(|l| l.stored.as_str()),
        sample_rate,
        channels,
        clips,
    )?;
    let json = io::serialize(&project)?;

    publish::atomic_write(path, json.as_bytes()).map_err(|error| {
        // The C sets this here rather than at the call site, so a failed autosave
        // stops retrying until the next edit clears it (`plug.c:4580`). The
        // reason travels with the latch so the panel can name it (UX0-B01).
        let failure = ProjectError::Write(error);
        track.mark_save_failed(failure.to_string());
        failure
    })?;

    track.file_path = audio.runtime;
    if let (Some(image), Some(published)) = (&mut track.ascii, &ascii) {
        image.path = published.runtime.clone();
    }
    if let Some(published) = &font {
        track.caption_font_path = Some(published.runtime.clone());
    }
    if let Some(published) = &licence {
        track.caption_licence_path = Some(published.runtime.clone());
    }
    for (sha256, published) in adopted {
        if let Some(entry) = track
            .clips
            .iter_mut()
            .find(|entry| entry.asset.sha256 == sha256)
        {
            entry.asset.path = published.stored;
            entry.source = published.runtime;
            entry.bundle = Some(project_folder(path));
        }
    }
    track.project_path = Some(path.to_path_buf());
    track.project_metadata = Some(project.metadata);
    track.mark_saved();
    Ok(())
}

/// The path Save As should suggest (`save_project_as`, `plug.c:4620-4634`).
///
/// A saved track's own project path is the right suggestion. By then `file_path`
/// points into the content-addressed bundle, so deriving from it would propose a
/// SHA-256-named project inside `<stem>.assets/`.
#[must_use]
pub fn suggested_save_path(track: &Track) -> PathBuf {
    let source = track
        .project_path
        .as_deref()
        .unwrap_or(track.file_path.as_path());
    source.with_extension("musi")
}

/// `poll_project_autosave` (`plug.c:5065-5075`).
///
/// Returns `Some` only when a save is due. The 1.5-second settle is what keeps a
/// slider drag from writing the file on every frame, and the `project_path`
/// requirement is what keeps autosave from inventing a destination.
///
/// `editor_dirty` is the two guards the C names — an unsaved lyric draft or an
/// open route edit — which belong to Agents I and G. Passing it in means their
/// panels hook in at one place rather than reaching into this.
#[must_use]
pub fn autosave_is_due(track: &Track, now_seconds: f64, editor_dirty: bool) -> bool {
    track.project_path.is_some()
        && track.project_dirty
        && !track.project_autosave_failed
        && !editor_dirty
        && now_seconds - track.project_dirty_since >= 1.5
}

/// Which workspace slots autosave should write this frame (C4).
///
/// Extracted from `main.rs`'s frame loop so the all-track claim is a unit test
/// rather than only a capture. The loop it replaced computed exactly this list
/// and then discarded every entry that was not the current track, so "every due
/// track" was one `continue` away from true and nothing failed when it was not —
/// a background track simply stopped being written, silently, which is the
/// failure mode this whole task exists to remove.
///
/// `editor_dirty` is asked per slot, not once: a draft belongs to the track it
/// was typed against, and a global answer would let one open lyric form freeze
/// autosave for every other track in the workspace.
#[must_use]
pub fn autosave_due_tracks(
    workspace: &Workspace,
    now_seconds: f64,
    mut editor_dirty: impl FnMut(usize) -> bool,
) -> Vec<usize> {
    (0..workspace.len())
        .filter(|&index| {
            workspace
                .get(index)
                .is_some_and(|track| autosave_is_due(track, now_seconds, editor_dirty(index)))
        })
        .collect()
}

/// What opening a project produced, before it replaces anything.
pub struct OpenedProject {
    pub track: Track,
    pub warning: Option<OpenWarning>,
}

/// Rehydrates the project payload embedded in an app-owned recovery snapshot.
///
/// Recovery paths are absolute runtime references, not a publish bundle. They
/// are still checked against the digests already stored with the live track;
/// the expensive audio hash happens here during an explicit restart action,
/// never while the frame thread writes a snapshot (CX-3).
pub fn open_recovery_json(
    json: &str,
    duration_for: impl FnOnce(&Path) -> Result<f64, String>,
) -> Result<OpenedProject, ProjectError> {
    if json.is_empty() || json.len() > io::MAX_INPUT {
        return Err(ProjectError::Size);
    }
    let project = io::deserialize(json.as_bytes())?;
    project
        .validate()
        .map_err(|error| ProjectError::Build(error.to_string()))?;
    project
        .editor_support()
        .map_err(|error| ProjectError::Unsupported(error.to_string()))?;

    let audio_path = PathBuf::from(&project.audio.path);
    if !audio_path.is_absolute()
        || !project_files::file_has_digest(&audio_path, &project.audio.sha256)
    {
        return Err(ProjectError::AudioMismatch);
    }
    let ascii_path = match &project.ascii_image {
        Some(asset) => {
            let path = PathBuf::from(&asset.path);
            if !path.is_absolute() || !project_files::file_has_digest(&path, &asset.sha256) {
                return Err(ProjectError::AsciiMismatch);
            }
            Some(path)
        }
        None => None,
    };
    let (font_path, licence_path) = match &project.caption_style.font {
        Some(asset) => {
            let font = PathBuf::from(&asset.path);
            if !font.is_absolute() || !project_files::file_has_digest(&font, &asset.sha256) {
                return Err(ProjectError::FontMismatch);
            }
            let licence = if asset.licence_path.is_empty() {
                None
            } else {
                let path = PathBuf::from(&asset.licence_path);
                if !path.is_absolute()
                    || !project_files::file_has_digest(&path, &asset.licence_sha256)
                {
                    return Err(ProjectError::LicenceMismatch);
                }
                Some(path)
            };
            (Some(font), licence)
        }
        None => (None, None),
    };
    // Clips are not checked here (VG): the decoder checks each one's digest
    // from its own handle before drawing it, and one that is gone draws a
    // placeholder naming it rather than refusing the whole recovery. A
    // recovery records runtime paths, all absolute; anything else is a clip
    // the session had refused, and stays refused.
    let clip_sources = project
        .clips
        .iter()
        .map(|asset| {
            let path = PathBuf::from(&asset.path);
            if path.is_absolute() {
                (path, None, None)
            } else {
                (PathBuf::new(), Some(ClipError::OutsideProject), None)
            }
        })
        .collect();
    let duration = duration_for(&audio_path).map_err(ProjectError::Build)?;
    hydrate_track(
        project,
        audio_path,
        ascii_path,
        font_path,
        licence_path,
        clip_sources,
        duration,
        None,
    )
}

/// `open_project_path` (`plug.c:4665-5044`), up to but not including the part
/// that mutates the workspace.
///
/// Every asset is resolved and its digest checked **before** a `Track` is built,
/// and the `Track` is complete before the caller replaces anything. That is what
/// makes a failed open a no-op rather than a workspace with a missing file in it.
///
/// The caller supplies the audio duration, because reading it needs the audio
/// device this crate half deliberately does not have here.
pub fn open_path(
    path: &Path,
    duration_for: impl FnOnce(&Path) -> Result<f64, String>,
) -> Result<OpenedProject, ProjectError> {
    let metadata = std::fs::metadata(path).map_err(ProjectError::Read)?;
    if metadata.len() == 0 || metadata.len() > io::MAX_INPUT as u64 {
        return Err(ProjectError::Size);
    }
    let input = std::fs::read(path).map_err(ProjectError::Read)?;
    let project = io::deserialize(&input)?;
    project
        .editor_support()
        .map_err(|error| ProjectError::Unsupported(error.to_string()))?;

    // Assets, in the C's order, each verified by digest before anything is built.
    let (audio_path, warning) = match project.audio.mode {
        AssetMode::Imported => (
            project_files::resolve_bundled_asset_path(path, &project.audio.path)
                .map_err(|_| ProjectError::AudioMismatch)?,
            None,
        ),
        AssetMode::Referenced => {
            let (resolved, how) = project_files::resolve_asset_path(path, &project.audio.path)
                .map_err(|_| ProjectError::AudioMismatch)?;
            let warning = (how == project_files::PathResolution::LegacyCwd)
                .then_some(OpenWarning::LegacyAudioPath);
            (resolved, warning)
        }
    };
    if !project_files::file_has_digest(&audio_path, &project.audio.sha256) {
        return Err(ProjectError::AudioMismatch);
    }

    let ascii_path = match &project.ascii_image {
        Some(asset) => {
            let resolved = project_files::resolve_bundled_asset_path(path, &asset.path)
                .map_err(|_| ProjectError::AsciiMismatch)?;
            if !project_files::file_has_digest(&resolved, &asset.sha256) {
                return Err(ProjectError::AsciiMismatch);
            }
            Some(resolved)
        }
        None => None,
    };

    let mut caption_font_path = None;
    let mut caption_licence_path = None;
    if let Some(asset) = &project.caption_style.font {
        let resolved = project_files::resolve_bundled_asset_path(path, &asset.path)
            .map_err(|_| ProjectError::FontMismatch)?;
        if !project_files::file_has_digest(&resolved, &asset.sha256) {
            return Err(ProjectError::FontMismatch);
        }
        caption_font_path = Some(resolved);
        if !asset.licence_path.is_empty() {
            let licence = project_files::resolve_bundled_asset_path(path, &asset.licence_path)
                .map_err(|_| ProjectError::LicenceMismatch)?;
            if !project_files::file_has_digest(&licence, &asset.licence_sha256) {
                return Err(ProjectError::LicenceMismatch);
            }
            caption_licence_path = Some(licence);
        }
    }

    // Each clip resolves inside the bundle (VG). Not refused when it is not
    // there or its digest changed: the decoder checks the bytes from its own
    // handle, and a clip that is gone draws a placeholder naming it, where a
    // refusal would lose the whole project over one take. A record that does
    // not resolve is never read at all: missing, or a path that leaves the
    // project (`../`, absolute, a symlink out), each its own placeholder.
    //
    // Resolving is a check on names, though, and the decoder opens the clip
    // later, maybe minutes later: a resolved clip keeps its project folder,
    // and is opened beneath it by its record, never by the resolved name.
    let folder = project_folder(path);
    let clip_sources = project
        .clips
        .iter()
        .map(
            |asset| match project_files::resolve_bundled_asset_path(path, &asset.path) {
                Ok(resolved) => (resolved, None, Some(folder.clone())),
                Err(_) => {
                    let (source, error) = unresolved_clip(path, &asset.path);
                    (source, error, None)
                }
            },
        )
        .collect();
    let duration = duration_for(&audio_path).map_err(ProjectError::Build)?;
    hydrate_track(
        project,
        audio_path,
        ascii_path,
        caption_font_path,
        caption_licence_path,
        clip_sources,
        duration,
        Some(path.to_path_buf()),
    )
    .map(|mut opened| {
        opened.warning = warning;
        opened
    })
}

/// The folder a project's bundled paths are relative to: the project file's
/// own, as `resolve_bundled_asset_path` reads them.
fn project_folder(project: &Path) -> PathBuf {
    match project.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Why a clip's record did not resolve inside its bundle, and the path it
/// names when that is still inside the project: a bundled file that is not
/// there is missing; anything else that fails containment — a path that is
/// not plainly relative, a symlink or a directory leading out, something that
/// is not a regular file — is refused and gets no path at all.
fn unresolved_clip(project: &Path, recorded: &str) -> (PathBuf, Option<ClipError>) {
    if !musializer_core::project::io::is_unambiguous_relative_path(recorded) {
        return (PathBuf::new(), Some(ClipError::OutsideProject));
    }
    let candidate = project.parent().unwrap_or(Path::new(".")).join(recorded);
    match candidate.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (candidate, Some(ClipError::Missing))
        }
        Ok(metadata) if !metadata.is_file() && !metadata.is_symlink() => {
            (PathBuf::new(), Some(ClipError::NotAFile))
        }
        // A regular file or a symlink that did not resolve back to this path:
        // reached through a link out of the bundle.
        Ok(_) => (PathBuf::new(), Some(ClipError::OutsideProject)),
        Err(error) => (PathBuf::new(), Some(ClipError::Read(error.to_string()))),
    }
}

#[allow(clippy::too_many_arguments)]
fn hydrate_track(
    project: Project,
    audio_path: PathBuf,
    ascii_path: Option<PathBuf>,
    caption_font_path: Option<PathBuf>,
    caption_licence_path: Option<PathBuf>,
    clip_sources: Vec<(PathBuf, Option<ClipError>, Option<PathBuf>)>,
    duration: f64,
    project_path: Option<PathBuf>,
) -> Result<OpenedProject, ProjectError> {
    let entry = project
        .scenes
        .first()
        .ok_or_else(|| ProjectError::Build("the project has no scene".into()))?;
    let base_scene = SceneId::from_stable_name(&entry.scene_type)
        .ok_or_else(|| ProjectError::UnknownScene(entry.scene_type.clone()))?;
    let entry_clip = entry.clip;
    let (scene_settings, scene_routes) =
        routes::import_mappings(&entry.mappings).ok_or(ProjectError::Mappings)?;
    let scene_switches = hydrate_scene_switches(&project.scene_switches, &project.audio)?;
    let scene_presets = hydrate_presets(&project.scene_presets)?;
    let mut render_config = RenderExportConfig {
        width: project.output.width,
        height: project.output.height,
        fps: project.output.fps_numerator,
        quality: output_to_quality(project.output.quality),
        supersample_factor: 1,
    };
    render_config.set_quality(output_to_quality(project.output.quality));
    render_config.validate().map_err(|_| ProjectError::Output)?;

    let mut track = Track::new(
        audio_path.clone(),
        duration,
        base_scene,
        project.deterministic_seed,
    )
    .map_err(|error| ProjectError::Build(error.to_string()))?;

    track.transport_seekable =
        musializer_core::timing::track_timeline::path_is_seekable(audio_path.to_str());
    track.audio_sha256 = project.audio.sha256.clone();
    track.scene_instance_id = entry.instance_id;
    track.scene_settings = scene_settings;
    track.playback_scene_settings = scene_settings;
    track.scene_routes = scene_routes;
    track.scene_switches = scene_switches;
    track.scene_presets = scene_presets;
    track.render_config = render_config;
    track
        .lyrics
        .replace(&project.lyrics)
        .map_err(|error| ProjectError::Build(error.to_string()))?;
    track
        .semantic_events
        .replace(&project.semantic_events)
        .map_err(|error| ProjectError::Build(error.to_string()))?;
    track
        .manual_events
        .replace(&project.manual_events)
        .map_err(|error| ProjectError::Build(error.to_string()))?;
    track.refresh_next_manual_event_id();
    track.caption_style = project.caption_style;
    track.caption_font_path = caption_font_path;
    track.caption_licence_path = caption_licence_path;
    if let (Some(asset), Some(resolved)) = (&project.ascii_image, ascii_path) {
        // The glyph grid stays `None`: decoding the image is item N's, and a
        // blank grid of the right size would claim the cells had been restored.
        track.ascii = Some(AsciiImage {
            grid: None,
            columns: asset.columns as usize,
            rows: asset.rows as usize,
            path: resolved,
            sha256: asset.sha256.clone(),
        });
    }
    track.base_clip = entry_clip;
    track.clips = project
        .clips
        .iter()
        .zip(clip_sources)
        .map(
            |(asset, (source, unusable, bundle))| crate::clips::TrackClip {
                asset: asset.clone(),
                source,
                unusable,
                bundle,
            },
        )
        .collect();
    track.analysis_lanes = project.analysis_lanes;
    // C4: seeded from the file the project recorded, so a track opened into a
    // background slot — one that never gets a bound stream — is still saveable.
    // The loader overwrites both once the audio is actually decoded; until then
    // the project's own numbers are the best available and were written by a
    // build that had decoded it.
    track.audio_sample_rate = project.audio.sample_rate;
    track.audio_channels = project.audio.channels;
    track.project_path = project_path;
    // PXF-7: until it is current and playing, the track is where it was saved.
    track.playhead_seconds = project.metadata.playhead_seconds;
    track.project_metadata = Some(project.metadata);
    track.mark_saved();

    Ok(OpenedProject {
        track,
        warning: None,
    })
}

fn hydrate_scene_switches(
    suggestions: &SceneSwitchSuggestions,
    audio: &AudioAsset,
) -> Result<SceneSwitchTimeline, ProjectError> {
    let mut timeline = SceneSwitchTimeline::new();
    if suggestions.cues.is_empty() {
        timeline.outro = suggestions.outro;
        timeline.finish = suggestions.finish;
        timeline.titles.clone_from(&suggestions.titles);
        timeline.enabled = suggestions.enabled;
        return Ok(timeline);
    }
    let mut cues = Vec::with_capacity(suggestions.cues.len());
    for suggestion in &suggestions.cues {
        let scene = SceneId::from_stable_name(&suggestion.scene_name)
            .ok_or_else(|| ProjectError::UnknownScene(suggestion.scene_name.clone()))?;
        let mut settings = SettingsSnapshot::default();
        if !suggestion.settings.is_empty() {
            settings.captured = true;
            settings.count = suggestion.settings.len();
            settings.values[..settings.count].copy_from_slice(&suggestion.settings);
            if !settings.is_valid_for(scene) {
                return Err(ProjectError::Build(
                    "a captured tuning snapshot does not match its scene".into(),
                ));
            }
        }
        cues.push(SceneSwitchCue {
            id: suggestion.id,
            start_seconds: suggestion.start_seconds,
            end_seconds: suggestion.end_seconds,
            scene_index: scene.index() as u32,
            strength: suggestion.strength,
            settings,
            transition: suggestion.transition,
            grade: suggestion.grade,
            glide: suggestion.glide,
            pixel: suggestion.pixel,
            caption_style: suggestion.caption_style,
            clip: suggestion.clip,
        });
    }
    timeline
        .replace(&cues, audio.duration_seconds, SCENE_COUNT as u32)
        .map_err(|error| ProjectError::Build(error.to_string()))?;
    timeline.enabled = suggestions.enabled;
    timeline.outro = suggestions.outro;
    timeline.finish = suggestions.finish;
    timeline.titles.clone_from(&suggestions.titles);
    Ok(timeline)
}

fn hydrate_presets(presets: &[ScenePreset]) -> Result<PresetLibrary, ProjectError> {
    let mut library = PresetLibrary::new();
    for preset in presets {
        let scene = SceneId::from_stable_name(&preset.scene_name)
            .ok_or_else(|| ProjectError::UnknownScene(preset.scene_name.clone()))?;
        let mut snapshot = SettingsSnapshot {
            captured: true,
            count: preset.settings.len(),
            ..SettingsSnapshot::default()
        };
        snapshot.values[..snapshot.count].copy_from_slice(&preset.settings);
        if !snapshot.is_valid_for(scene) {
            return Err(ProjectError::Build(
                "the stored tuning values do not match their scene".into(),
            ));
        }
        if !library.restore(scene, preset.id, &preset.name, &snapshot) {
            return Err(ProjectError::Build(
                "the preset scene, ID, or per-scene capacity is unsupported".into(),
            ));
        }
    }
    if !library.is_valid() {
        return Err(ProjectError::Build(
            "preset identifiers or names are invalid".into(),
        ));
    }
    Ok(library)
}

/// The two quality enums are the same three values in the same order, and the C
/// casts between them (`plug.c:4327`, `:4812`). Spelled out rather than cast, so
/// adding a value to one and not the other is a compile error.
fn quality_to_output(quality: Quality) -> OutputQuality {
    match quality {
        Quality::Balanced => OutputQuality::Balanced,
        Quality::High => OutputQuality::High,
        Quality::Master => OutputQuality::Master,
    }
}

fn output_to_quality(quality: OutputQuality) -> Quality {
    match quality {
        OutputQuality::Balanced => Quality::Balanced,
        OutputQuality::High => Quality::High,
        OutputQuality::Master => Quality::Master,
    }
}

fn file_stem(path: &Path) -> Option<&str> {
    path.file_stem().and_then(|stem| stem.to_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A track that is due for autosave in every respect except the one under
    /// test.
    fn saveable(name: &str, now: f64) -> Track {
        let mut track = Track::new(PathBuf::from(name), 120.0, SceneId::Spectrum, 7)
            .expect("120s is a valid duration");
        track.audio_sample_rate = 48_000;
        track.audio_channels = 2;
        track.project_path = Some(PathBuf::from(format!("{name}.musi")));
        track.mark_dirty(now);
        track
    }

    /// C4's headline claim, and the one the old loop broke: a workspace where
    /// **two** tracks are due yields both, not only the current one.
    ///
    /// The frame loop used to build exactly this list and then `continue` past
    /// every entry that was not `current_index()`, because the sample rate came
    /// off the bound stream. Nothing failed when it did that — the background
    /// track just stopped being written.
    #[test]
    fn every_due_track_is_returned_not_only_the_current_one() {
        let mut workspace = Workspace::new();
        workspace.push(saveable("/tmp/a.wav", 0.0));
        workspace.push(saveable("/tmp/b.wav", 0.0));
        assert_eq!(workspace.current_index(), Some(0));

        let due = autosave_due_tracks(&workspace, 2.0, |_| false);

        assert_eq!(due, vec![0, 1], "the background track must autosave too");
    }

    #[test]
    fn the_settle_window_holds_a_track_back_until_it_has_elapsed() {
        let mut workspace = Workspace::new();
        workspace.push(saveable("/tmp/a.wav", 10.0));

        assert!(autosave_due_tracks(&workspace, 11.4, |_| false).is_empty());
        assert_eq!(autosave_due_tracks(&workspace, 11.5, |_| false), vec![0]);
    }

    #[test]
    fn a_dirty_draft_holds_back_its_own_track_and_no_other() {
        // The over-correction this guards against is as bad as the gap: a global
        // "some editor is dirty" answer would freeze every track's autosave
        // because one of them has a half-typed cue.
        let mut workspace = Workspace::new();
        workspace.push(saveable("/tmp/a.wav", 0.0));
        workspace.push(saveable("/tmp/b.wav", 0.0));

        let due = autosave_due_tracks(&workspace, 2.0, |index| index == 0);

        assert_eq!(due, vec![1]);
    }

    #[test]
    fn a_track_with_no_project_path_is_never_due() {
        // Autosave must not invent a destination.
        let mut workspace = Workspace::new();
        let mut track = saveable("/tmp/a.wav", 0.0);
        track.project_path = None;
        workspace.push(track);

        assert!(autosave_due_tracks(&workspace, 99.0, |_| false).is_empty());
    }

    #[test]
    fn a_latched_failure_stops_the_retry_without_stopping_the_other_track() {
        // One failure must not become a write-every-frame loop, and must not
        // silence a sibling whose destination is a different file entirely.
        let mut workspace = Workspace::new();
        workspace.push(saveable("/tmp/a.wav", 0.0));
        workspace.push(saveable("/tmp/b.wav", 0.0));
        workspace
            .get_mut(0)
            .expect("two tracks")
            .mark_save_failed("no space left on device");

        assert_eq!(autosave_due_tracks(&workspace, 2.0, |_| false), vec![1]);

        // And the next edit on the failed track earns it a fresh attempt.
        workspace.get_mut(0).expect("two tracks").mark_dirty(2.0);
        assert_eq!(autosave_due_tracks(&workspace, 4.0, |_| false), vec![0, 1]);
    }

    #[test]
    fn a_track_whose_audio_format_is_unknown_is_refused_rather_than_written() {
        // The cached format is what replaced the bound stream. Zero means the
        // decode has not happened yet, and writing it would produce a `.musi`
        // claiming 0 Hz whose bytes are otherwise perfectly valid — so nothing
        // downstream would catch it.
        let directory = std::env::temp_dir().join("musializer-c4-format-test");
        let _ = std::fs::create_dir_all(&directory);
        let audio = directory.join("silent.wav");
        std::fs::write(&audio, b"not really a wav").expect("the fixture directory is writable");

        let mut track = Track::new(audio, 120.0, SceneId::Spectrum, 7).expect("valid duration");
        track.audio_sample_rate = 0;
        track.audio_channels = 0;

        let error = save_to_path(&mut track, &directory.join("x.musi"), false)
            .expect_err("a 0 Hz track must not be written");
        assert!(
            error.to_string().contains("audio format"),
            "the refusal must name the reason, got {error}"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// PXF-7: a save records the track's playhead, clamped onto the track, and
    /// a playhead that is not a position records the start (never written).
    #[test]
    fn a_save_records_the_playhead_clamped_onto_the_track() {
        let mut track = saveable("/tmp/a.wav", 0.0);
        track.audio_sha256 = "a".repeat(64);
        let written = |track: &Track| {
            build_project(track, "a.assets/audio/x.wav", None, None, None, 48_000, 2)
                .expect("the track builds")
                .metadata
                .playhead_seconds
        };
        assert_eq!(written(&track), 0.0);
        track.playhead_seconds = 73.5;
        assert_eq!(written(&track), 73.5);
        track.playhead_seconds = 500.0;
        assert_eq!(written(&track), 120.0);
        for nonsense in [f64::NAN, f64::INFINITY, -3.0] {
            track.playhead_seconds = nonsense;
            assert_eq!(written(&track), 0.0, "{nonsense}");
        }
    }

    /// A saved project with one clip as its base scene's, in a scratch
    /// directory that removes itself: (directory, project, recorded path).
    struct ClipProject(PathBuf);

    impl Drop for ClipProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn clip_project(name: &str) -> (ClipProject, PathBuf, String) {
        let root = std::env::temp_dir().join(format!(
            "musializer-clip-project-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let directory = root.join("show");
        std::fs::create_dir_all(&directory).unwrap();
        let audio = directory.join("song.wav");
        std::fs::write(&audio, b"pcm bytes for the digest").unwrap();
        let take = root.join("take.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i"])
            .args([
                "testsrc2=size=64x36:rate=10",
                "-frames:v",
                "5",
                "-c:v",
                "libx264",
            ])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&take)
            .stdin(std::process::Stdio::null())
            .status()
            .expect("ffmpeg runs");
        assert!(status.success());
        let mut track = Track::new(audio, 120.0, SceneId::Clip, 7).unwrap();
        track.audio_sample_rate = 48_000;
        track.audio_channels = 2;
        let entry = crate::clips::import(&take).expect("the take imports");
        let clip = crate::clips::add(&mut track.clips, entry).unwrap();
        track.base_clip = Some(musializer_core::project::clip::ClipPlacement::new(clip));
        let project = directory.join("song.musi");
        save_to_path(&mut track, &project, false).expect("saves with its clip");
        let recorded = track.clips[0].asset.path.clone();
        assert!(recorded.starts_with("song.assets/clips/"), "{recorded}");
        (ClipProject(root), project, recorded)
    }

    fn reopened_clip(project: &Path) -> crate::clips::TrackClip {
        open_path(project, |_| Ok(120.0))
            .expect("a clip that cannot be read does not refuse the open")
            .track
            .clips[0]
            .clone()
    }

    fn rewrite_clip_path(project: &Path, recorded: &str, replacement: &str) {
        let text = std::fs::read_to_string(project).unwrap();
        let needle = format!("\"{recorded}\"");
        assert!(text.contains(&needle), "the record names {recorded}");
        let rewritten = text.replace(&needle, &serde_json_string(replacement));
        std::fs::write(project, rewritten).unwrap();
    }

    fn serde_json_string(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }

    /// F1b defect 3: a clip record that fails containment is an invalid
    /// reference — the placeholder by name, its path never opened — however
    /// it leaves: `../`, an absolute path, or a symlink in the bundle. An
    /// outside copy with the recorded digest sits there each time, so the
    /// old fallback would have decoded it and bundled it again.
    #[test]
    fn a_clip_record_that_leaves_the_bundle_is_refused_not_read() {
        let (scratch, project, recorded) = clip_project("contain");
        let outside = scratch.0.join("take.mp4");
        let bundled = project.parent().unwrap().join(&recorded);

        let legit = reopened_clip(&project);
        assert_eq!(legit.unusable, None);
        assert!(existing_files_alias(&legit.source, &bundled));

        std::fs::remove_file(&bundled).unwrap();
        std::os::unix::fs::symlink(&outside, &bundled).unwrap();
        let linked = reopened_clip(&project);
        assert_eq!(
            linked.unusable,
            Some(ClipError::OutsideProject),
            "symlink out"
        );
        assert_eq!(linked.source, PathBuf::new());

        std::fs::remove_file(&bundled).unwrap();
        let missing = reopened_clip(&project);
        assert_eq!(missing.unusable, Some(ClipError::Missing));

        rewrite_clip_path(&project, &recorded, "../take.mp4");
        let dotdot = reopened_clip(&project);
        assert_eq!(dotdot.unusable, Some(ClipError::OutsideProject), "../");
        assert_eq!(dotdot.source, PathBuf::new());

        rewrite_clip_path(&project, "../take.mp4", outside.to_str().unwrap());
        let mut opened = open_path(&project, |_| Ok(120.0)).unwrap().track;
        assert_eq!(
            opened.clips[0].unusable,
            Some(ClipError::OutsideProject),
            "absolute"
        );
        // The same project keeps the record as it was; another refuses it,
        // since it cannot bundle bytes it may not read.
        save_to_path(&mut opened, &project, false).expect("re-save keeps the record");
        let elsewhere = scratch.0.join("other.musi");
        let error = save_to_path(&mut opened, &elsewhere, false)
            .expect_err("an unreadable clip is not bundled elsewhere");
        assert!(error.to_string().contains("leaves the project"), "{error}");
        assert!(!scratch.0.join("other.assets/clips").exists());
        // A recovery snapshot of it never names the outside file.
        let snapshot = build_project(&opened, "/a.wav", None, None, None, 48_000, 2).unwrap();
        assert!(!snapshot.clips[0].path.starts_with('/'));
        // Importing the file again gives the clip its bytes back.
        let again = crate::clips::import(&outside).unwrap();
        crate::clips::add(&mut opened.clips, again).unwrap();
        assert_eq!(opened.clips[0].unusable, None);
        assert_eq!(opened.clips[0].source, outside);
    }

    /// G defect 1: the open's containment check and the decoder's open are
    /// about the same file. Between the two, the bundled file is swapped for
    /// a symlink to an outside copy with the recorded digest, then the
    /// directory above it for a symlink to an outside directory holding one:
    /// the decoder refuses both, where opening the resolved name decoded the
    /// outside copy.
    #[test]
    fn a_clip_swapped_out_of_its_bundle_after_the_open_is_never_read() {
        use std::time::Duration;
        let (scratch, project, recorded) = clip_project("swap");
        let outside = scratch.0.join("take.mp4");
        let bundled = project.parent().unwrap().join(&recorded);
        let clip = reopened_clip(&project);
        assert_eq!(clip.unusable, None);
        assert_eq!(
            clip.location(),
            musializer_runtime::clip::ClipLocation::Bundled {
                folder: project.parent().unwrap().to_path_buf(),
                relative: recorded.clone(),
            }
        );
        clip.worker()
            .wait_for(0, Duration::from_secs(20))
            .expect("the bundled clip decodes before any swap");
        // Save As reads it through the same opening, and keeps its extension.
        let mut track = open_path(&project, |_| Ok(120.0)).unwrap().track;
        let copy = scratch.0.join("copy.musi");
        save_to_path(&mut track, &copy, false).expect("a bundled clip saves elsewhere");
        assert!(track.clips[0].asset.path.starts_with("copy.assets/clips/"));
        assert!(track.clips[0].asset.path.ends_with(".mp4"));
        assert!(scratch.0.join(&track.clips[0].asset.path).is_file());
        let mut track = open_path(&project, |_| Ok(120.0)).unwrap().track;

        std::fs::remove_file(&bundled).unwrap();
        std::os::unix::fs::symlink(&outside, &bundled).unwrap();
        assert_eq!(
            clip.worker()
                .wait_for(0, Duration::from_secs(20))
                .unwrap_err(),
            ClipError::OutsideProject,
            "file swapped for a symlink"
        );
        // Nor does a save copy it: the same project keeps the record, another
        // refuses, and nothing reaches its bundle.
        save_to_path(&mut track, &project, true).expect("autosave keeps the record");
        let other = scratch.0.join("other.musi");
        let error = save_to_path(&mut track, &other, false).expect_err("not bundled elsewhere");
        assert!(error.to_string().contains("leaves the project"), "{error}");
        assert!(!scratch.0.join("other.assets/clips").exists());

        let clips = bundled.parent().unwrap();
        let elsewhere = scratch.0.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::copy(&outside, elsewhere.join(bundled.file_name().unwrap())).unwrap();
        std::fs::remove_dir_all(clips).unwrap();
        std::os::unix::fs::symlink(&elsewhere, clips).unwrap();
        assert_eq!(
            clip.worker()
                .wait_for(0, Duration::from_secs(20))
                .unwrap_err(),
            ClipError::OutsideProject,
            "directory swapped for a symlink"
        );
    }

    use musializer_runtime::project_files::existing_files_alias;
}
