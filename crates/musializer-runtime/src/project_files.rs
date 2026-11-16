//! The filesystem half of `.musi`: resolving asset references, hashing files,
//! and bundling assets beside a project.
//!
//! This is exactly what [`musializer_core::project::io`]'s module comment says
//! is deliberately absent from that crate — `canonicalize_existing_file`,
//! `existing_files_alias`, `resolve_asset_path`, `bundle_asset` and the
//! whole-file SHA-256. They open files, so they live here.
//!
//! Ported from `../musializer/src/project_io.c:494-935` and `:1498-1582`. The
//! pure rules — what a stored relative path may look like, how a directory is
//! derived from a project path, what a bundle is named — stay in the core crate
//! and are called from here, so there is one definition of each.
//!
//! # The one rule worth stating up front
//!
//! **A bundled asset must resolve back to the very same relative path it was
//! stored as.** `resolve_bundled_asset_path` does not merely check that a file
//! exists beside the project: it canonicalizes both the asset and the project
//! directory, recomputes the descendant path, and rejects the asset unless that
//! recomputation is byte-identical to what the file claimed
//! (`project_io.c:874-884`). A symlink pointing out of the bundle, a `..`
//! segment that happens to land somewhere real, or a path that reaches the right
//! file by a different route are all refused. Without it, "bundled" would mean
//! "found", and a project could quietly depend on a file outside itself.

use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use musializer_core::project::{assets, clip};
// Re-exported so a caller bundling an asset needs one import, not two from two
// crates.
pub use musializer_core::project::assets::AssetCategory;
use musializer_core::project::io::{
    directory_of, is_unambiguous_relative_path, path_is_absolute, relative_descendant_path,
};
use musializer_core::project::sha256::Sha256;

use crate::process::publish::{self, AssetError, AssetPublication};

/// How an asset reference resolved (`Musi_Project_Path_Result`,
/// `project_io.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathResolution {
    /// The stored path was absolute and names an existing file.
    Absolute,
    /// The stored path resolved beside the project, which is the portable case.
    ProjectRelative,
    /// The stored path did not resolve beside the project but does resolve
    /// against the **working directory**.
    ///
    /// Honoured, and warned about, because old projects were written before
    /// paths were project-relative and refusing them would strand them
    /// (`plug.c:4841-4845`). New writes never produce this.
    LegacyCwd,
}

/// Why an asset reference did not resolve (`project_io.h`'s error arm).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("null or empty path argument")]
    Null,
    #[error("asset file is missing")]
    NotFound,
    #[error("resolved asset path is too long")]
    TooLong,
}

/// Whether a path names an existing *regular* file
/// (`project_regular_file_exists`).
///
/// A directory is not an asset, and neither is a device node. `is_file` follows
/// symlinks, which is the C's `stat` behaviour rather than `lstat`.
#[must_use]
pub fn regular_file_exists(path: &Path) -> bool {
    path.is_file()
}

/// `musi_project_canonicalize_existing_file` (`project_io.c:619-631`).
///
/// `realpath`, plus the check the C adds on top: a canonical path that is not a
/// regular file is a failure, not a result.
pub fn canonicalize_existing_file(path: &Path) -> Result<PathBuf, PathError> {
    if path.as_os_str().is_empty() {
        return Err(PathError::Null);
    }
    let canonical = path.canonicalize().map_err(|_| PathError::NotFound)?;
    if !regular_file_exists(&canonical) {
        return Err(PathError::NotFound);
    }
    Ok(canonical)
}

/// Whether two paths name the same existing file
/// (`musi_project_existing_files_alias`, `project_io.c:909-...`).
///
/// Device and inode, not string comparison: this is what stops a Save As from
/// writing a project over its own source audio reached by a symlink or a second
/// hard link. A path that does not exist aliases nothing.
#[must_use]
pub fn existing_files_alias(first: &Path, second: &Path) -> bool {
    let (Ok(a), Ok(b)) = (first.metadata(), second.metadata()) else {
        return false;
    };
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// `musi_project_resolve_asset_path` (`project_io.c:494-577`).
///
/// An absolute stored path is used as-is. A relative one is joined to the
/// project's directory first, and only if that misses is the working directory
/// tried — in that order, so a file beside the project always wins over a
/// same-named file in the launch directory.
///
/// Backslashes after the first character are normalized to `/` before joining,
/// because a stored relative path uses backslash as a portable separator
/// (`project_io.c:531-537`).
pub fn resolve_asset_path(
    project_path: &Path,
    asset_path: &str,
) -> Result<(PathBuf, PathResolution), PathError> {
    if project_path.as_os_str().is_empty() || asset_path.is_empty() {
        return Err(PathError::Null);
    }
    if path_is_absolute(asset_path) {
        let candidate = PathBuf::from(asset_path);
        return if regular_file_exists(&candidate) {
            Ok((candidate, PathResolution::Absolute))
        } else {
            Err(PathError::NotFound)
        };
    }

    let project_text = project_path.to_str().ok_or(PathError::Null)?;
    let directory = directory_of(project_text).ok_or(PathError::Null)?;

    // The first character is left alone: the C starts its loop at `+ 1`, so a
    // leading backslash is not a separator to normalize. It cannot appear in an
    // unambiguous relative path anyway, and preserving the difference keeps this
    // diffable against the line it came from.
    let mut normalized = String::with_capacity(asset_path.len());
    for (index, character) in asset_path.chars().enumerate() {
        normalized.push(if index > 0 && character == '\\' {
            '/'
        } else {
            character
        });
    }

    let candidate = Path::new(directory).join(&normalized);
    if regular_file_exists(&candidate) {
        return Ok((candidate, PathResolution::ProjectRelative));
    }
    let fallback = PathBuf::from(&normalized);
    if regular_file_exists(&fallback) {
        return Ok((fallback, PathResolution::LegacyCwd));
    }
    Err(PathError::NotFound)
}

/// `musi_project_resolve_bundled_asset_path` (`project_io.c:853-899`).
///
/// Everything [`resolve_asset_path`] does, restricted to the project-relative
/// case, plus the round-trip described in this module's header: the resolved
/// file is canonicalized, the project directory is canonicalized, and the
/// descendant path recomputed from the two must equal the stored path exactly.
///
/// The C returns the *canonical* path on success, not the joined candidate, so
/// the caller records where the file really is.
pub fn resolve_bundled_asset_path(
    project_path: &Path,
    asset_path: &str,
) -> Result<PathBuf, PathError> {
    if project_path.as_os_str().is_empty() {
        return Err(PathError::Null);
    }
    if !is_unambiguous_relative_path(asset_path) {
        return Err(PathError::NotFound);
    }
    let (candidate, resolution) = resolve_asset_path(project_path, asset_path)?;
    if resolution != PathResolution::ProjectRelative {
        return Err(PathError::NotFound);
    }
    let canonical_asset = canonicalize_existing_file(&candidate)?;

    let project_text = project_path.to_str().ok_or(PathError::Null)?;
    let directory = directory_of(project_text).ok_or(PathError::NotFound)?;
    let canonical_directory = Path::new(directory)
        .canonicalize()
        .map_err(|_| PathError::NotFound)?;
    if !canonical_directory.is_dir() {
        return Err(PathError::NotFound);
    }

    let directory_text = canonical_directory.to_str().ok_or(PathError::NotFound)?;
    let asset_text = canonical_asset.to_str().ok_or(PathError::NotFound)?;
    match relative_descendant_path(directory_text, asset_text) {
        Some(relative) if relative == asset_path => Ok(canonical_asset),
        // Either the asset is not under the project directory at all, or it is
        // but by a different route than the one recorded. Both are refusals.
        _ => Err(PathError::NotFound),
    }
}

/// Streaming whole-file SHA-256, hex-encoded (`sha256_file_hex`, `sha256.c`).
///
/// Chunked rather than read-to-end because this runs over audio files: a
/// `Vec<u8>` of a 60 MB WAV to compute 32 bytes is avoidable. Opened once
/// without blocking and refused unless the handle is a regular file, so a FIFO
/// put under an asset's name answers at once instead of parking the caller.
pub fn sha256_file_hex(path: &Path) -> io::Result<String> {
    sha256_file_hex_within(path, u64::MAX)
}

/// What a file looked like on disk when its digest was taken: enough to tell,
/// with one `stat`, whether a digest recorded earlier may still describe it.
///
/// Length, modification and change times, and the inode. A replace by rename
/// moves the inode, an in-place rewrite moves the change time (which no tool
/// can set back), so a file whose stamp is unchanged was not written since.
/// A stamp that differs says only "look again": a `touch` or a `chmod` moves
/// it without changing a byte, and the caller re-hashes before believing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub modified_ns: i128,
    pub changed_ns: i128,
    pub inode: u64,
    pub device: u64,
}

impl FileStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        let ns = |seconds: i64, nanos: i64| i128::from(seconds) * 1_000_000_000 + i128::from(nanos);
        Self {
            len: metadata.len(),
            modified_ns: ns(metadata.mtime(), metadata.mtime_nsec()),
            changed_ns: ns(metadata.ctime(), metadata.ctime_nsec()),
            inode: metadata.ino(),
            device: metadata.dev(),
        }
    }
}

/// The stamp of the file at `path` now (one `stat`, following links as an
/// open does).
pub fn file_stamp(path: &Path) -> io::Result<FileStamp> {
    std::fs::metadata(path).map(|metadata| FileStamp::of(&metadata))
}

/// [`sha256_file_hex`] and the stamp of the file it read.
///
/// The stamp is taken from the open handle before and after the read; when the
/// two differ the file changed while it was hashed, the digest describes no
/// single state of it, and the stamp is `None` so the next check hashes again.
pub fn sha256_file_hex_stamped(path: &Path) -> io::Result<(String, Option<FileStamp>)> {
    let before = file_stamp(path)?;
    let digest = sha256_file_hex(path)?;
    let after = file_stamp(path)?;
    Ok((digest, (before == after).then_some(after)))
}

/// [`sha256_file_hex`] for a file of at most `limit` bytes, read through that
/// cap; a larger one, or one that grows past it while it is read, is an error.
pub fn sha256_file_hex_within(path: &Path, limit: u64) -> io::Result<String> {
    let file = publish::open_source(path, limit).map_err(|error| match error {
        AssetError::Source(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
    })?;
    let mut reader = file.take(limit.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                AssetError::TooLarge(limit).to_string(),
            ));
        }
        hasher.update(&buffer[..read]);
    }
    Ok(musializer_core::project::sha256::hex(&hasher.finalize()))
}

/// Whether the file at `path` hashes to `expected`.
///
/// The check every asset opened from a project goes through.
#[must_use]
pub fn file_has_digest(path: &Path, expected: &str) -> bool {
    sha256_file_hex(path).is_ok_and(|actual| actual == expected)
}

/// The largest file each kind of asset may be, the same bound its importer
/// holds it to, so a save cannot copy what an import would have refused: a
/// clip's [`clip::MAX_CLIP_BYTES`], a picture's
/// [`crate::decode::PICTURE_LARGEST_FILE`], a face's (and its licence's)
/// [`assets::IMPORTED_FONT_BYTE_LIMIT`]. Audio had no importer bound; 4 GiB is
/// the WAV format's own ceiling (a 32-bit RIFF size) and days of compressed
/// audio, recorded in `docs/DIVERGENCES.md`.
#[must_use]
pub fn byte_limit(category: AssetCategory) -> u64 {
    match category {
        AssetCategory::Audio => 4 * 1024 * 1024 * 1024,
        AssetCategory::Image => crate::decode::PICTURE_LARGEST_FILE,
        AssetCategory::Font => assets::IMPORTED_FONT_BYTE_LIMIT,
        AssetCategory::Clip => clip::MAX_CLIP_BYTES,
    }
}

/// A SHA-256 over the bytes a copy passes, compared with the asset's digest.
struct CopySha256<'a> {
    hasher: Sha256,
    expected: &'a str,
}

impl publish::CopyDigest for CopySha256<'_> {
    fn update(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    fn matches(self) -> bool {
        musializer_core::project::sha256::hex(&self.hasher.finalize()) == self.expected
    }
}

/// Where a bundled asset ended up: the path to record in the file, and the path
/// it lives at on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundledAsset {
    /// The project-relative path to write into the `.musi`.
    pub stored: String,
    /// The absolute path on this machine.
    pub runtime: PathBuf,
    pub publication: AssetPublication,
}

/// Why bundling failed.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("asset bundle path is invalid: {0}")]
    Path(#[from] musializer_core::project::assets::BundleError),
    #[error("bundle directory could not be created: {0}")]
    Directory(#[source] io::Error),
    #[error(transparent)]
    Asset(#[from] AssetError),
}

/// Publishes one asset into the project's sibling bundle
/// (`bundle_project_asset`, `plug.c:4413-4432`, over
/// `musi_project_bundle_asset`, `project_io.c:1498-1533`).
///
/// `reuse_published` is the autosave path: re-saving a project whose asset is
/// already bundled must not rewrite the file, so an existing object of the right
/// identity is adopted rather than re-copied. A genuine copy only follows when
/// that fails, which is the C's two-step and the reason autosave does not do
/// whole-file I/O every 1.5 seconds.
pub fn bundle_asset(
    project_path: &Path,
    category: AssetCategory,
    source: &Path,
    sha256: &str,
    reuse_published: bool,
) -> Result<BundledAsset, BundleError> {
    bundle_asset_named(
        project_path,
        category,
        source,
        source,
        sha256,
        reuse_published,
    )
}

/// [`bundle_asset`] reading `source` and naming the copy's extension after
/// `named`: a file already held open is read through its handle's
/// `/proc/self/fd/N`, which has no extension of its own.
pub fn bundle_asset_named(
    project_path: &Path,
    category: AssetCategory,
    source: &Path,
    named: &Path,
    sha256: &str,
    reuse_published: bool,
) -> Result<BundledAsset, BundleError> {
    let project_text = project_path
        .to_str()
        .ok_or(musializer_core::project::assets::BundleError::Path)?;
    let source_text = named
        .to_str()
        .ok_or(musializer_core::project::assets::BundleError::Path)?;
    let paths = assets::bundle_paths(project_text, category, source_text, sha256)?;

    let limit = byte_limit(category);
    let has_digest =
        |path: &Path| sha256_file_hex_within(path, limit).is_ok_and(|actual| actual == sha256);
    let runtime = PathBuf::from(&paths.runtime);
    if reuse_published && runtime.is_file() && has_digest(&runtime) {
        return Ok(BundledAsset {
            stored: paths.stored,
            runtime,
            publication: AssetPublication::AlreadyPresent,
        });
    }

    if let Some(parent) = runtime.parent() {
        std::fs::create_dir_all(parent).map_err(BundleError::Directory)?;
    }
    // The source is opened once without blocking, must be a regular file
    // within the category's limit, and is hashed as it is copied: a FIFO, a
    // device or a file that grows without end never reaches the bundle, and
    // bytes that are not the recorded digest are never published.
    let publication = publish::publish_content_addressed(
        source,
        &runtime,
        limit,
        has_digest,
        CopySha256 {
            hasher: Sha256::new(),
            expected: sha256,
        },
    )?;
    Ok(BundledAsset {
        stored: paths.stored,
        runtime,
        publication,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::OpenOptionsExt;

    /// A private directory under `build/`, which is gitignored. Named per test
    /// so a parallel run cannot collide.
    fn scratch(name: &str) -> PathBuf {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-project-files")
            .join(name);
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("scratch directory");
        directory.canonicalize().expect("scratch is real")
    }

    #[test]
    fn a_known_vector_hashes_the_way_the_oracle_does() {
        let directory = scratch("sha256");
        let path = directory.join("abc.txt");
        fs::write(&path, b"abc").expect("write");
        assert_eq!(
            sha256_file_hex(&path).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(file_has_digest(
            &path,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
        assert!(!file_has_digest(&path, &"0".repeat(64)));
    }

    #[test]
    fn an_empty_file_hashes_and_a_missing_one_does_not() {
        let directory = scratch("sha256-edges");
        let path = directory.join("empty");
        fs::write(&path, b"").expect("write");
        assert_eq!(
            sha256_file_hex(&path).expect("hash"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(sha256_file_hex(&directory.join("absent")).is_err());
    }

    #[test]
    fn a_relative_asset_beside_the_project_beats_one_in_the_working_directory() {
        let directory = scratch("resolve");
        let project = directory.join("song.musi");
        fs::write(&project, b"{}").expect("write");
        let asset = directory.join("audio.wav");
        fs::write(&asset, b"pcm").expect("write");

        let (resolved, how) = resolve_asset_path(&project, "audio.wav").expect("resolves");
        assert_eq!(how, PathResolution::ProjectRelative);
        assert!(existing_files_alias(&resolved, &asset));

        // Nothing beside the project and nothing in the working directory.
        assert_eq!(
            resolve_asset_path(&project, "absent.wav"),
            Err(PathError::NotFound)
        );
    }

    #[test]
    fn an_absolute_asset_is_taken_as_written() {
        let directory = scratch("resolve-absolute");
        let project = directory.join("song.musi");
        fs::write(&project, b"{}").expect("write");
        let elsewhere = directory.join("outside.wav");
        fs::write(&elsewhere, b"pcm").expect("write");

        let text = elsewhere.to_str().expect("utf-8");
        let (resolved, how) = resolve_asset_path(&project, text).expect("resolves");
        assert_eq!(how, PathResolution::Absolute);
        assert_eq!(resolved, elsewhere);
    }

    #[test]
    fn a_bundled_asset_must_resolve_back_to_the_path_it_claimed() {
        let directory = scratch("bundled");
        let project = directory.join("song.musi");
        fs::write(&project, b"{}").expect("write");
        let bundle = directory.join("song.assets");
        fs::create_dir_all(&bundle).expect("bundle");
        let asset = bundle.join("audio.wav");
        fs::write(&asset, b"pcm").expect("write");

        let resolved =
            resolve_bundled_asset_path(&project, "song.assets/audio.wav").expect("resolves");
        assert!(existing_files_alias(&resolved, &asset));

        // Reaching the same bytes by a route the stored path does not describe is
        // refused, which is the check that makes "bundled" mean something.
        let link = directory.join("shortcut.wav");
        std::os::unix::fs::symlink(&asset, &link).expect("symlink");
        assert_eq!(
            resolve_bundled_asset_path(&project, "shortcut.wav"),
            Err(PathError::NotFound),
            "a symlink resolves to a canonical path that is not the stored one"
        );

        // And an escape attempt never reaches the filesystem at all.
        assert_eq!(
            resolve_bundled_asset_path(&project, "../song.musi"),
            Err(PathError::NotFound)
        );
        assert_eq!(
            resolve_bundled_asset_path(&project, "/etc/hostname"),
            Err(PathError::NotFound)
        );
    }

    #[test]
    fn two_names_for_one_file_alias_and_two_files_do_not() {
        let directory = scratch("alias");
        let original = directory.join("a.wav");
        fs::write(&original, b"pcm").expect("write");
        let link = directory.join("b.wav");
        fs::hard_link(&original, &link).expect("hard link");
        let other = directory.join("c.wav");
        fs::write(&other, b"pcm").expect("write");

        assert!(existing_files_alias(&original, &link));
        assert!(!existing_files_alias(&original, &other));
        assert!(
            !existing_files_alias(&original, &directory.join("absent")),
            "a path that does not exist aliases nothing"
        );
    }

    #[test]
    fn bundling_publishes_once_and_then_reuses() {
        let directory = scratch("bundle");
        let project = directory.join("song.musi");
        fs::write(&project, b"{}").expect("write");
        let source = directory.join("source.wav");
        fs::write(&source, b"pcm bytes").expect("write");
        let digest = sha256_file_hex(&source).expect("hash");

        let first =
            bundle_asset(&project, AssetCategory::Audio, &source, &digest, false).expect("bundles");
        assert_eq!(first.publication, AssetPublication::Published);
        assert!(first.runtime.is_file());

        // The stored path must be exactly what the resolver will accept back.
        let resolved = resolve_bundled_asset_path(&project, &first.stored).expect("resolves back");
        assert!(existing_files_alias(&resolved, &first.runtime));

        let second =
            bundle_asset(&project, AssetCategory::Audio, &source, &digest, true).expect("reuses");
        assert_eq!(second.publication, AssetPublication::AlreadyPresent);
        assert_eq!(second.stored, first.stored);
    }

    /// F1b defect 1: the save's bundling opens its source the way the clip
    /// decoder does. A FIFO under a clip's name answers at once instead of
    /// parking autosave in `open(2)`, `/dev/zero` is refused instead of copied
    /// until the disk fills, and bytes that are not the recorded digest are
    /// never published under it.
    #[test]
    fn bundling_refuses_what_is_not_a_bounded_regular_file_with_its_digest() {
        let directory = scratch("bundle-hostile");
        let project = directory.join("song.musi");
        fs::write(&project, b"{}").expect("write");
        let digest = "ab".repeat(32);

        let fifo = directory.join("take.mp4");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let (done, answer) = std::sync::mpsc::channel();
        let (from, into, sha) = (fifo.clone(), project.clone(), digest.clone());
        std::thread::spawn(move || {
            let outcome = bundle_asset(&into, AssetCategory::Clip, &from, &sha, false);
            let _ = done.send(outcome.map(|_| ()).map_err(|error| error.to_string()));
        });
        let outcome = answer
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("bundling a FIFO must not block");
        assert!(
            outcome
                .as_ref()
                .is_err_and(|error| error.contains("not a regular file")),
            "{outcome:?}"
        );
        // Release the thread if a regression ever opened the FIFO blocking.
        let _ = fs::OpenOptions::new()
            .write(true)
            .custom_flags(0o4000)
            .open(&fifo);

        let error = bundle_asset(
            &project,
            AssetCategory::Clip,
            Path::new("/dev/zero"),
            &digest,
            false,
        )
        .expect_err("a device is not an asset");
        assert!(error.to_string().contains("not a regular file"), "{error}");
        assert!(
            sha256_file_hex(Path::new("/dev/zero")).is_err(),
            "hashing refuses a device too"
        );

        let wrong = directory.join("wrong.mp4");
        fs::write(&wrong, b"not the recorded bytes").expect("write");
        let error = bundle_asset(&project, AssetCategory::Clip, &wrong, &digest, false)
            .expect_err("a digest mismatch never publishes");
        assert!(error.to_string().contains("SHA-256"), "{error}");
        let clips = directory.join("song.assets").join("clips");
        let published: Vec<_> = fs::read_dir(&clips)
            .map(|entries| entries.map(|entry| entry.unwrap().file_name()).collect())
            .unwrap_or_default();
        assert!(published.is_empty(), "published: {published:?}");

        // Each kind is bounded by its importer's limit.
        assert_eq!(byte_limit(AssetCategory::Clip), clip::MAX_CLIP_BYTES);
        assert_eq!(byte_limit(AssetCategory::Font), 32 * 1024 * 1024);
        assert_eq!(byte_limit(AssetCategory::Image), 64 * 1024 * 1024);
        assert_eq!(
            sha256_file_hex_within(&wrong, 4).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
