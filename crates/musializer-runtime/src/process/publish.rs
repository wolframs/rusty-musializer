//! Transactional file publication.
//!
//! **Owner: Agent E.** Ported from the atomic-replacement paths of the frozen C
//! oracle at `../musializer` (commit `9300af9`, read-only):
//! `musi_project_temporary_path` (`src/project_io.c:984-1004`),
//! `project_sync_parent_directory` (`:1017-1057`),
//! `musi_project_atomic_write` (`:1059-1210`),
//! `project_copy_asset_transaction` (`:1341-1496`), and the export temporary
//! path from `render_export_temporary_path` (`src/render_export.c:274-316`).
//!
//! The invariant this module exists for: **a failed or cancelled job never
//! destroys an existing destination.** Write to a temporary in the same
//! directory, `fsync`, then `rename`. Same directory matters twice — `rename(2)`
//! is only atomic within a filesystem, and a temporary next to the destination
//! inherits its mount's behaviour rather than `/tmp`'s.
//!
//! Nothing here hashes anything. Content identity is Agent B's
//! (`sha256.c` → `musializer-core`), so [`publish_content_addressed`] takes a
//! verifier callback instead of reaching for a digest implementation. The same
//! boundary keeps this module free of `serde` and of the project model.
//!
//! ## Boundary with Agent A and Agent B
//!
//! [`export_temporary_path`] and [`project_temporary_path`] are pure string
//! functions that the C keeps in `render_export.c` (Agent A) and
//! `project_io.c` (Agent B). They live here because both callers of the
//! filesystem half are here and neither owner had landed when this was written.
//! If either agent ports the original, delete the copy rather than keeping two.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The failure modes of [`atomic_write`], one per `Musi_Project_File_Result`
/// (`src/project_io.h:41-52`). The messages are
/// `musi_project_file_result_string` (`src/project_io.c:1584-1600`) so a Rust
/// diagnostic reads like the C one.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("null or empty argument")]
    Null,
    #[error("invalid or oversized destination path")]
    Path,
    #[error("could not create a transaction file: {0}")]
    Open(#[source] io::Error),
    #[error("could not write the complete project: {0}")]
    Write(#[source] io::Error),
    #[error("could not preserve project permissions: {0}")]
    Permissions(#[source] io::Error),
    #[error("could not flush the project to storage: {0}")]
    Sync(#[source] io::Error),
    #[error("could not close the project transaction: {0}")]
    Close(#[source] io::Error),
    #[error("could not atomically publish the project: {0}")]
    Publish(#[source] io::Error),
    /// The one failure that leaves the destination **correct**: the bytes are
    /// in place and only the parent directory's own durability is unconfirmed.
    /// The C is careful not to delete the temporary in this case
    /// (`src/project_io.c:1202-1205`) and neither is this.
    #[error("project was published but parent-directory durability was not confirmed: {0}")]
    Durability(#[source] io::Error),
}

/// How many distinct temporary names to try before giving up, matching the C's
/// `for (unsigned attempt = 0; attempt < 256u; ++attempt)`
/// (`src/project_io.c:1139`).
const NAME_ATTEMPTS: u32 = 256;

/// Per-process transaction counter, `project_next_transaction_nonce`
/// (`src/project_io.c:1006-1015`). Relaxed ordering is enough: the value only
/// has to be distinct, never ordered against anything else.
fn next_transaction_nonce() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed) + 1
}

/// Splits a destination into (directory prefix including its separator, rest).
///
/// `project_last_separator` accepts both `/` and `\` even on POSIX
/// (`src/project_io.c:994`, and the same in `src/render_export.c:293-299`),
/// because the path may have come from a `.musi` written on Windows. Reproduced:
/// a Linux file whose name genuinely contains a backslash gets a temporary in
/// the wrong place, which is a real if exotic divergence from what a
/// POSIX-only reading would do.
fn directory_prefix(path: &str) -> &str {
    let last = path
        .bytes()
        .enumerate()
        .filter(|(_, byte)| *byte == b'/' || *byte == b'\\')
        .map(|(index, _)| index)
        .next_back();
    match last {
        Some(index) => &path[..=index],
        None => "",
    }
}

fn ends_with_separator(path: &str) -> bool {
    matches!(path.as_bytes().last(), Some(b'/') | Some(b'\\'))
}

/// `musi_project_temporary_path`: `<dir>/.musializer-project-<pid>-<nonce>.tmp`
/// with the nonce in **16 hex digits** (`src/project_io.c:997-1003`).
///
/// Returns `None` for an empty destination or one that names a directory, which
/// is the C's `false`.
pub fn project_temporary_path(destination: &str, process_id: u64, nonce: u64) -> Option<PathBuf> {
    if destination.is_empty() || ends_with_separator(destination) {
        return None;
    }
    let directory = directory_prefix(destination);
    Some(PathBuf::from(format!(
        "{directory}.musializer-project-{process_id}-{nonce:016x}.tmp"
    )))
}

/// `render_export_temporary_path`: `<dir>/.musializer-<pid>-<nonce>.part.mp4`
/// with **both** numbers in decimal (`src/render_export.c:283-315`).
///
/// The asymmetry with [`project_temporary_path`] — decimal nonce here, hex
/// there — is in the oracle. It is preserved because these names appear in
/// error messages users are told to look for, and because a `.part.mp4`
/// suffix is what lets FFmpeg infer the muxer without an explicit `-f`.
///
/// The C also requires `process_id != 0 && nonce != 0`; `None` is its `false`.
pub fn export_temporary_path(output_path: &str, process_id: u64, nonce: u64) -> Option<PathBuf> {
    if output_path.is_empty() || process_id == 0 || nonce == 0 {
        return None;
    }
    let directory = directory_prefix(output_path);
    Some(PathBuf::from(format!(
        "{directory}.musializer-{process_id}-{nonce}.part.mp4"
    )))
}

/// `fsync`s the directory a published file lives in, so the *name* is durable
/// and not just the bytes.
///
/// `project_sync_parent_directory` (`src/project_io.c:1017-1057`). A path with
/// no separator syncs `"."`; a path like `/x` syncs `/`. Opening a directory
/// read-only and calling `File::sync_all` is `open(O_RDONLY|O_DIRECTORY)` plus
/// `fsync`, which is what the C does.
pub fn sync_parent_directory(destination: &Path) -> io::Result<()> {
    let text = destination.to_string_lossy();
    let prefix = directory_prefix(&text);
    let directory: PathBuf = match prefix {
        "" => PathBuf::from("."),
        // A destination directly under the root: the prefix is just the
        // separator, and trimming it would leave an empty path.
        "/" | "\\" => PathBuf::from("/"),
        other => PathBuf::from(other.trim_end_matches(['/', '\\'])),
    };
    File::open(directory)?.sync_all()
}

/// A staged write that publishes on [`commit`](Transaction::commit) and cleans
/// up on [`Drop`].
///
/// This is the reusable shape behind [`atomic_write`], exposed because Agent B's
/// saves and Agent F's exports both need "write a lot of bytes, then publish or
/// discard" rather than "hand me a `&[u8]`".
///
/// `Drop` is the safety net against abandonment — a panic or an early `?`
/// removes the temporary rather than leaving `.musializer-project-…tmp` litter
/// next to the user's project. It is deliberately *only* a net: `Drop` cannot
/// report, so callers that care about cleanup failure call
/// [`abort`](Transaction::abort), which returns the error.
#[derive(Debug)]
pub struct Transaction {
    destination: PathBuf,
    temporary: PathBuf,
    file: Option<File>,
}

impl Transaction {
    /// Creates the temporary next to `destination`, retrying distinct names on
    /// `EEXIST` up to 256 times (`src/project_io.c:1139-1150`).
    ///
    /// The mode of an existing regular destination is preserved
    /// (`:1132-1137`, `:1157-1163`), so publishing over a file the user
    /// `chmod`ded does not silently reset it to `0666 & ~umask`. A destination
    /// that does not exist yet gets `0666`, as in the C.
    pub fn begin(destination: &Path) -> Result<Self, PublishError> {
        let text = destination.to_str().ok_or(PublishError::Path)?;
        if text.is_empty() {
            return Err(PublishError::Null);
        }
        if ends_with_separator(text) {
            return Err(PublishError::Path);
        }

        let mut mode = 0o666;
        let mut preserve = false;
        if let Ok(metadata) = fs::metadata(destination) {
            if metadata.is_file() {
                mode = metadata.permissions().mode() & 0o7777;
                preserve = true;
            }
        }

        let process_id = u64::from(std::process::id());
        let mut last: Option<io::Error> = None;
        for _ in 0..NAME_ATTEMPTS {
            let nonce = next_transaction_nonce();
            let temporary =
                project_temporary_path(text, process_id, nonce).ok_or(PublishError::Path)?;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .custom_flags(libc_o_cloexec())
                .open(&temporary)
            {
                Ok(file) => {
                    // `create_new` honours `mode` only for a fresh file, and
                    // not at all if a umask trims it, so the C re-applies the
                    // preserved mode explicitly (src/project_io.c:1157-1163).
                    if preserve {
                        file.set_permissions(fs::Permissions::from_mode(mode))
                            .map_err(PublishError::Permissions)?;
                    }
                    return Ok(Transaction {
                        destination: destination.to_path_buf(),
                        temporary,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    last = Some(error);
                    continue;
                }
                Err(error) => return Err(PublishError::Open(error)),
            }
        }
        Err(PublishError::Open(last.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "no unused transaction name was available",
            )
        })))
    }

    /// The path currently holding the staged bytes. Useful in error messages:
    /// the C names it when an encode survives but publication does not
    /// (`src/ffmpeg_posix.c:267`).
    pub fn temporary_path(&self) -> &Path {
        &self.temporary
    }

    /// Where the bytes will land.
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Appends to the staged file.
    pub fn write_all(&mut self, data: &[u8]) -> Result<(), PublishError> {
        let file = self
            .file
            .as_mut()
            .ok_or(PublishError::Write(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "transaction has already been finished",
            )))?;
        // `write_all` retries on `Interrupted` and fails on a zero-length
        // write, which is the C's EINTR/`written == 0` handling
        // (src/project_io.c:1166-1183).
        file.write_all(data).map_err(PublishError::Write)
    }

    /// `fsync`, close, `rename` over the destination, then `fsync` the parent
    /// directory.
    ///
    /// The order is the whole point and is `src/project_io.c:1184-1201`. On
    /// failure the temporary is removed and the destination is untouched;
    /// [`PublishError::Durability`] is the one exception, because by then the
    /// destination is already correct.
    pub fn commit(mut self) -> Result<(), PublishError> {
        let file = self.file.take().ok_or(PublishError::Write(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "transaction has already been finished",
        )))?;
        if let Err(error) = file.sync_all() {
            self.discard_quietly();
            return Err(PublishError::Sync(error));
        }
        // Dropping a `File` closes it and discards a close error. `sync_all`
        // above has already forced the data out, so the C's separate
        // ERROR_CLOSE arm (src/project_io.c:1191) cannot be reproduced exactly
        // without raw fds; the failure it catches is a subset of what the fsync
        // catches on Linux.
        drop(file);

        if let Err(error) = fs::rename(&self.temporary, &self.destination) {
            self.discard_quietly();
            return Err(PublishError::Publish(error));
        }
        // Published. From here the destination is correct no matter what, so
        // the temporary must not be removed on the durability path — it no
        // longer exists under that name anyway.
        let destination = std::mem::take(&mut self.destination);
        self.temporary = PathBuf::new();
        sync_parent_directory(&destination).map_err(PublishError::Durability)
    }

    /// Discards the staged bytes and **reports** a cleanup failure.
    ///
    /// Prefer this over relying on `Drop` wherever the caller has somewhere to
    /// put the error, per the plan's rule that normal control flow reports
    /// cleanup failures rather than hiding them.
    pub fn abort(mut self) -> io::Result<()> {
        self.file = None;
        let temporary = std::mem::take(&mut self.temporary);
        match fs::remove_file(&temporary) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn discard_quietly(&mut self) {
        self.file = None;
        if !self.temporary.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temporary);
            self.temporary = PathBuf::new();
        }
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        self.discard_quietly();
    }
}

/// `O_CLOEXEC`. The C sets it wherever the platform has it
/// (`src/project_io.c:1144-1146`) so a staged fd does not leak into `ffmpeg` or
/// a Python helper. Hardcoded rather than pulled from `libc`, which is not a
/// dependency; the value is fixed ABI on Linux.
const fn libc_o_cloexec() -> i32 {
    0o2_000_000
}

/// Writes `data` to `destination` atomically.
///
/// `musi_project_atomic_write` (`src/project_io.c:1059-1210`), which is what
/// `.musi` saves and preset-store writes go through.
pub fn atomic_write(destination: &Path, data: &[u8]) -> Result<(), PublishError> {
    let mut transaction = Transaction::begin(destination)?;
    transaction.write_all(data)?;
    transaction.commit()
}

/// What publishing a content-addressed asset achieved.
///
/// The three success-ish outcomes of `project_copy_asset_transaction`
/// (`src/project_io.c:1341-1496`). A content-addressed name that already holds
/// the right bytes is a success, not a conflict — that is the whole point of
/// addressing by digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetPublication {
    /// The bytes were copied and linked into place by this call.
    Published,
    /// The destination already held bytes with the expected identity.
    AlreadyPresent,
}

/// The failure modes of [`publish_content_addressed`], one per
/// `Musi_Project_Bundle_Result` (`src/project_io.h:67-78`), with the messages
/// from `musi_project_bundle_result_string` (`src/project_io.c:1566-1582`).
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("asset bundle path is too long or malformed")]
    Path,
    #[error("source asset is missing or changed identity: {0}")]
    Source(#[source] io::Error),
    #[error("asset could not be copied completely: {0}")]
    Copy(#[source] io::Error),
    #[error("asset copy could not be made durable: {0}")]
    Sync(#[source] io::Error),
    #[error("copied asset did not match its expected SHA-256")]
    Identity,
    #[error("content-addressed destination contains different data")]
    Collision,
    #[error("asset could not be published: {0}")]
    Publish(#[source] io::Error),
}

/// Copies `source` into the content-addressed `destination`, verifying the
/// bytes before they are published and never overwriting a name that already
/// exists.
///
/// `verify` is called with a path and must answer "do these bytes have the
/// identity this asset claims?". Hashing lives in `musializer-core`
/// (`sha256.c`, Agent B), so it is a callback rather than a dependency here.
///
/// Two deliberate differences from [`atomic_write`], both from the C:
///
/// - Publication is [`fs::hard_link`], not [`fs::rename`]
///   (`src/project_io.c:1474`). `rename` would silently replace an existing
///   object; `link` fails with `EEXIST`, which is what turns a digest collision
///   into a detectable [`AssetError::Collision`] instead of data loss.
/// - The temporary is removed even on success, because `link` gave the bytes a
///   second name (`:1489-1492`).
pub fn publish_content_addressed(
    source: &Path,
    destination: &Path,
    verify: impl Fn(&Path) -> bool,
) -> Result<AssetPublication, AssetError> {
    // Fast path: the object is already published. Reused only after
    // verification (src/project_io.c:1344-1347).
    if destination.is_file() {
        return if verify(destination) {
            Ok(AssetPublication::AlreadyPresent)
        } else {
            Err(AssetError::Collision)
        };
    }

    let text = destination.to_str().ok_or(AssetError::Path)?;
    if text.is_empty() || ends_with_separator(text) {
        return Err(AssetError::Path);
    }

    let mut input = File::open(source).map_err(AssetError::Source)?;
    let staged = StagedCopy::create(text)?;
    {
        let mut output = staged.file.try_clone().map_err(AssetError::Copy)?;
        io::copy(&mut input, &mut output).map_err(AssetError::Copy)?;
        output.sync_all().map_err(AssetError::Sync)?;
    }

    if !verify(&staged.path) {
        return Err(AssetError::Identity);
    }

    match fs::hard_link(&staged.path, destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            // Someone published the same digest while this copy was running.
            // Identical content is fine; different content is a real collision.
            return if verify(destination) {
                Ok(AssetPublication::AlreadyPresent)
            } else {
                Err(AssetError::Collision)
            };
        }
        Err(error) => return Err(AssetError::Publish(error)),
    }
    sync_parent_directory(destination).map_err(AssetError::Sync)?;
    Ok(AssetPublication::Published)
}

/// A staged copy that unlinks itself on drop, including after a successful
/// `link` (the C's `bundle_posix_cleanup` runs on every path).
struct StagedCopy {
    path: PathBuf,
    file: File,
}

impl StagedCopy {
    fn create(destination: &str) -> Result<Self, AssetError> {
        let process_id = u64::from(std::process::id());
        for _ in 0..NAME_ATTEMPTS {
            let nonce = next_transaction_nonce();
            let path =
                project_temporary_path(destination, process_id, nonce).ok_or(AssetError::Path)?;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o666)
                .custom_flags(libc_o_cloexec())
                .open(&path)
            {
                Ok(file) => return Ok(StagedCopy { path, file }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(AssetError::Copy(error)),
            }
        }
        Err(AssetError::Path)
    }
}

impl Drop for StagedCopy {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test writes under `std::env::temp_dir()` in a directory named for
    /// this process, and removes it afterwards.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "musializer-publish-{}-{label}-{}",
                std::process::id(),
                next_transaction_nonce()
            ));
            fs::create_dir_all(&path).expect("scratch directory");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn temporary_paths_match_the_oracles_two_spellings() {
        assert_eq!(
            project_temporary_path("/tmp/show.musi", 4321, 7).unwrap(),
            PathBuf::from("/tmp/.musializer-project-4321-0000000000000007.tmp")
        );
        // A bare filename stages in the working directory, not in "/".
        assert_eq!(
            project_temporary_path("show.musi", 1, 1).unwrap(),
            PathBuf::from(".musializer-project-1-0000000000000001.tmp")
        );
        // The export path uses a decimal nonce and a .part.mp4 suffix, which is
        // what lets ffmpeg infer the muxer.
        assert_eq!(
            export_temporary_path("/tmp/out.mp4", 4321, 7).unwrap(),
            PathBuf::from("/tmp/.musializer-4321-7.part.mp4")
        );
        // Rejections: a directory-looking destination, an empty one, and the
        // zero pid/nonce the C refuses.
        assert!(project_temporary_path("/tmp/", 1, 1).is_none());
        assert!(project_temporary_path("", 1, 1).is_none());
        assert!(export_temporary_path("/tmp/out.mp4", 0, 1).is_none());
        assert!(export_temporary_path("/tmp/out.mp4", 1, 0).is_none());
    }

    #[test]
    fn atomic_write_publishes_and_replaces() {
        let scratch = Scratch::new("replace");
        let destination = scratch.join("show.musi");
        atomic_write(&destination, b"first").expect("first write");
        assert_eq!(fs::read(&destination).unwrap(), b"first");
        atomic_write(&destination, b"second").expect("second write");
        assert_eq!(fs::read(&destination).unwrap(), b"second");
        // No litter left behind.
        let leftovers: Vec<_> = fs::read_dir(&scratch.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().starts_with(".musializer"))
            .collect();
        assert!(leftovers.is_empty(), "staged files remained: {leftovers:?}");
    }

    #[test]
    fn a_dropped_transaction_leaves_the_existing_destination_intact() {
        // The invariant, stated as a test: a cancelled save does not destroy
        // what was already saved.
        let scratch = Scratch::new("cancel");
        let destination = scratch.join("show.musi");
        atomic_write(&destination, b"the good project").expect("seed");

        {
            let mut transaction = Transaction::begin(&destination).expect("begin");
            transaction.write_all(b"half a project").expect("write");
            // Dropped without commit, as a cancelled job would.
        }

        assert_eq!(fs::read(&destination).unwrap(), b"the good project");
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn abort_reports_cleanup_rather_than_hiding_it() {
        let scratch = Scratch::new("abort");
        let destination = scratch.join("show.musi");
        let mut transaction = Transaction::begin(&destination).expect("begin");
        transaction.write_all(b"discarded").expect("write");
        let staged = transaction.temporary_path().to_path_buf();
        transaction.abort().expect("abort must succeed");
        assert!(!staged.exists());
        assert!(
            !destination.exists(),
            "an aborted first save creates nothing"
        );
    }

    #[test]
    fn a_failed_write_never_touches_the_destination() {
        let scratch = Scratch::new("failed");
        let destination = scratch.join("show.musi");
        atomic_write(&destination, b"survivor").expect("seed");

        // A destination inside a read-only directory cannot be staged at all,
        // which is the closest reproducible analogue of a mid-write failure.
        let locked = Scratch::new("locked");
        let inner = locked.join("locked.musi");
        fs::write(&inner, b"existing").expect("seed");
        let mut permissions = fs::metadata(&locked.0).unwrap().permissions();
        permissions.set_mode(0o500);
        fs::set_permissions(&locked.0, permissions).unwrap();

        let result = atomic_write(&inner, b"replacement");

        let mut permissions = fs::metadata(&locked.0).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&locked.0, permissions).unwrap();

        assert!(matches!(result, Err(PublishError::Open(_))), "{result:?}");
        assert_eq!(fs::read(&inner).unwrap(), b"existing");
        assert_eq!(fs::read(&destination).unwrap(), b"survivor");
    }

    #[test]
    fn publication_preserves_the_destinations_mode() {
        let scratch = Scratch::new("mode");
        let destination = scratch.join("show.musi");
        atomic_write(&destination, b"one").expect("seed");
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o640)).unwrap();

        atomic_write(&destination, b"two").expect("republish");
        let mode = fs::metadata(&destination).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o640,
            "a chmodded project must not be reset by a save"
        );
    }

    #[test]
    fn sync_parent_directory_handles_bare_names_and_the_root() {
        // "." for a bare filename, "/" for a root-level path. Both must open.
        sync_parent_directory(Path::new("Cargo.toml")).expect("current directory");
        sync_parent_directory(Path::new("/etc")).expect("root");
    }

    #[test]
    fn a_content_addressed_asset_is_copied_verified_and_linked() {
        let scratch = Scratch::new("asset");
        let source = scratch.join("face.ttf");
        fs::write(&source, b"font bytes").expect("source");
        let destination = scratch.join("published.ttf");

        let outcome = publish_content_addressed(&source, &destination, |path| {
            fs::read(path)
                .map(|bytes| bytes == b"font bytes")
                .unwrap_or(false)
        })
        .expect("publish");
        assert_eq!(outcome, AssetPublication::Published);
        assert_eq!(fs::read(&destination).unwrap(), b"font bytes");

        // Publishing the same object again is a success, not a conflict.
        let again = publish_content_addressed(&source, &destination, |path| {
            fs::read(path)
                .map(|bytes| bytes == b"font bytes")
                .unwrap_or(false)
        })
        .expect("republish");
        assert_eq!(again, AssetPublication::AlreadyPresent);
    }

    #[test]
    fn a_content_addressed_name_holding_other_bytes_is_a_collision_not_a_replacement() {
        let scratch = Scratch::new("collision");
        let source = scratch.join("face.ttf");
        fs::write(&source, b"new bytes").expect("source");
        let destination = scratch.join("published.ttf");
        fs::write(&destination, b"someone else's bytes").expect("squatter");

        let error = publish_content_addressed(&source, &destination, |path| {
            fs::read(path)
                .map(|bytes| bytes == b"new bytes")
                .unwrap_or(false)
        })
        .expect_err("a digest collision must not overwrite");
        assert!(matches!(error, AssetError::Collision), "{error:?}");
        // The point of using link(2) rather than rename(2).
        assert_eq!(fs::read(&destination).unwrap(), b"someone else's bytes");
    }

    #[test]
    fn an_asset_that_fails_verification_is_never_published() {
        let scratch = Scratch::new("identity");
        let source = scratch.join("face.ttf");
        fs::write(&source, b"corrupted in transit").expect("source");
        let destination = scratch.join("published.ttf");

        let error = publish_content_addressed(&source, &destination, |_| false)
            .expect_err("verification must gate publication");
        assert!(matches!(error, AssetError::Identity), "{error:?}");
        assert!(!destination.exists());
        // And the staged copy is gone.
        let staged: Vec<_> = fs::read_dir(&scratch.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().starts_with(".musializer"))
            .collect();
        assert!(staged.is_empty(), "staged copy remained: {staged:?}");
    }

    #[test]
    fn a_missing_source_asset_is_reported_as_a_source_error() {
        let scratch = Scratch::new("missing");
        let error = publish_content_addressed(
            &scratch.join("absent.ttf"),
            &scratch.join("published.ttf"),
            |_| true,
        )
        .expect_err("a missing source cannot be published");
        assert!(matches!(error, AssetError::Source(_)), "{error:?}");
    }
}
