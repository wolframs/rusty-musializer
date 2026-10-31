//! Agent chats kept between launches, one file per chat, per project.
//!
//! **Data, not preferences:** `$MUSIALIZER_CHATS`, else
//! `$XDG_DATA_HOME/musializer/chats`, else `~/.local/share/musializer/chats`.
//! A project gets a folder named by the hash of its path, so listing a project
//! never reads another's chats. A `.musi` never holds one: a transcript can
//! carry whatever a tool printed, and a project file travels. For the same
//! reason the folders are 0700 and the files 0600.
//!
//! **A probe run keeps nothing** unless `$MUSIALIZER_CHATS` names a place, so a
//! gate's fake agent never lands in the operator's list, and a gate that is
//! about saved chats opts in by naming one.
//!
//! **Everything goes through one thread, in order.** A turn ending never stalls
//! a frame, and a chat closed and reopened at once is read after its last write,
//! not before it. That thread holds a lock on each chat it has open: a
//! `<id>.lock` beside the file, because a write replaces the file's inode. So two
//! windows cannot both own a chat, and the lock goes when the process does.
//!
//! A file this build cannot read is skipped and never touched.

use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use musializer_core::agent::{SavedChat, SAVED_CHAT_SCHEMA};
use sha2::{Digest, Sha256};

/// Far past a long session: a tool's output is cut to a preview in a transcript.
pub const MAX_CHAT_BYTES: usize = 16 * 1024 * 1024;
/// How many of a project's chats a list holds, newest first.
const MAX_LISTED: usize = 200;

/// One row of a project's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSummary {
    pub id: String,
    pub agent: String,
    pub title: String,
    pub updated_unix: i64,
    pub path: PathBuf,
}

impl ChatSummary {
    #[must_use]
    pub fn of(chat: &SavedChat, path: PathBuf) -> Self {
        Self {
            id: chat.id.clone(),
            agent: chat.agent.clone(),
            title: chat.title.clone(),
            updated_unix: chat.updated_unix,
            path,
        }
    }
}

/// Where chats live. `probe_run` needs the explicit place.
#[must_use]
pub fn root(probe_run: bool) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MUSIALIZER_CHATS") {
        return (!path.is_empty()).then(|| PathBuf::from(path));
    }
    if probe_run {
        return None;
    }
    if let Some(path) = std::env::var_os("XDG_DATA_HOME").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path).join("musializer/chats"));
    }
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join(".local/share/musializer/chats"))
}

/// The folder of `project`'s chats.
#[must_use]
pub fn project_folder(root: &Path, project: &str) -> PathBuf {
    let digest = Sha256::digest(project.as_bytes());
    let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    root.join(name)
}

/// A new chat's id, which is also its file's name. `seed` tells apart chats
/// opened in the same millisecond.
#[must_use]
pub fn new_id(seed: u64) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    format!("{millis:x}-{:x}-{seed:x}", std::process::id())
}

fn sane_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[must_use]
pub fn chat_path(folder: &Path, id: &str) -> PathBuf {
    folder.join(format!("{id}.json"))
}

#[must_use]
pub fn now_unix() -> i64 {
    super::preferences::recent::now_unix().unwrap_or(0)
}

/// Reads one chat, or says it will not.
pub fn load(path: &Path) -> Option<SavedChat> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_CHAT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_CHAT_BYTES {
        return None;
    }
    let chat: SavedChat = serde_json::from_slice(&bytes).ok()?;
    (chat.schema == SAVED_CHAT_SCHEMA && sane_id(&chat.id)).then_some(chat)
}

/// A project's chats, newest first. Blocks on the disk, so it runs off the
/// UI thread. The second value counts files it would not read.
#[must_use]
pub fn list(folder: &Path, project: &str) -> (Vec<ChatSummary>, usize) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return (Vec::new(), 0);
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    let mut refused = 0;
    let mut chats = Vec::new();
    for (_, path) in files.into_iter().take(MAX_LISTED) {
        match load(&path) {
            Some(chat) if chat_path(folder, &chat.id) == path && chat.project == project => {
                chats.push(ChatSummary::of(&chat, path));
            }
            _ => refused += 1,
        }
    }
    chats.sort_by_key(|chat| std::cmp::Reverse(chat.updated_unix));
    (chats, refused)
}

/// The folder a chat lives in, private even if it already existed wider. The
/// write's staging file takes the umask, so this is what keeps it unread.
fn private_folder(path: &Path) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|error| error.to_string())?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    private_folder(path)?;
    musializer_runtime::process::publish::atomic_write(path, bytes)
        .map_err(|error| error.to_string())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())
}

/// Takes `path`'s lock for this thread, or says who has it.
fn acquire(held: &mut HashMap<PathBuf, File>, path: &Path) -> Result<(), String> {
    if held.contains_key(path) {
        return Ok(());
    }
    private_folder(path)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path.with_extension("lock"))
        .map_err(|error| error.to_string())?;
    match lock.try_lock() {
        Ok(()) => {
            held.insert(path.to_path_buf(), lock);
            Ok(())
        }
        Err(TryLockError::WouldBlock) => Err("That chat is open in another window.".to_owned()),
        Err(TryLockError::Error(error)) => Err(error.to_string()),
    }
}

/// What a press on a row expects to find, checked against the file as it is
/// now rather than as it was listed.
fn load_expected(path: &Path, id: &str, project: &str) -> Option<SavedChat> {
    load(path).filter(|chat| {
        chat.id == id && chat.project == project && path.file_stem().is_some_and(|stem| stem == id)
    })
}

enum Job {
    Write(PathBuf, Vec<u8>),
    Load {
        path: PathBuf,
        id: String,
        project: String,
        reply: Sender<Result<SavedChat, String>>,
    },
    Forget {
        path: PathBuf,
        reply: Sender<Result<(), String>>,
    },
    Release(PathBuf),
}

fn work(inbox: &Receiver<Job>, report: &Sender<String>) {
    let mut held = HashMap::new();
    for job in inbox {
        match job {
            Job::Write(path, bytes) => {
                let done = acquire(&mut held, &path).and_then(|()| write(&path, &bytes));
                if let Err(error) = done {
                    let _ = report.send(format!("{}: {error}", path.display()));
                }
            }
            Job::Load {
                path,
                id,
                project,
                reply,
            } => {
                let chat = acquire(&mut held, &path).and_then(|()| {
                    load_expected(&path, &id, &project)
                        .ok_or_else(|| "That chat could not be read.".to_owned())
                });
                if chat.is_err() {
                    held.remove(&path);
                }
                let _ = reply.send(chat);
            }
            Job::Forget { path, reply } => {
                let gone = acquire(&mut held, &path)
                    .and_then(|()| std::fs::remove_file(&path).map_err(|error| error.to_string()));
                if gone.is_ok() {
                    let _ = std::fs::remove_file(path.with_extension("lock"));
                }
                held.remove(&path);
                let _ = reply.send(gone);
            }
            Job::Release(path) => {
                held.remove(&path);
            }
        }
    }
}

/// The one thread that touches chat files once a chat is in use.
pub struct Saver {
    jobs: Option<Sender<Job>>,
    failures: Receiver<String>,
    thread: Option<JoinHandle<()>>,
}

impl Saver {
    #[must_use]
    pub fn start() -> Self {
        let (jobs, inbox) = mpsc::channel();
        let (report, failures) = mpsc::channel();
        let thread = std::thread::spawn(move || work(&inbox, &report));
        Self {
            jobs: Some(jobs),
            failures,
            thread: Some(thread),
        }
    }

    fn send(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }

    pub fn save(&self, path: PathBuf, bytes: Vec<u8>) {
        self.send(Job::Write(path, bytes));
    }

    /// Reads a chat after every write queued so far, and takes its lock.
    #[must_use]
    pub fn load(
        &self,
        path: PathBuf,
        id: String,
        project: String,
    ) -> Receiver<Result<SavedChat, String>> {
        let (reply, answer) = mpsc::channel();
        self.send(Job::Load {
            path,
            id,
            project,
            reply,
        });
        answer
    }

    /// Removes a chat here, after every write queued so far. The agent's own
    /// history is its CLI's.
    #[must_use]
    pub fn forget(&self, path: PathBuf) -> Receiver<Result<(), String>> {
        let (reply, answer) = mpsc::channel();
        self.send(Job::Forget { path, reply });
        answer
    }

    /// Lets another window have a chat this one closed.
    pub fn release(&self, path: PathBuf) {
        self.send(Job::Release(path));
    }

    /// A write that failed since the last time of asking.
    pub fn failure(&self) -> Option<String> {
        self.failures.try_recv().ok()
    }

    /// Lets every queued write land. The window closing waits for this.
    pub fn finish(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mz-chats-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn chat(id: &str, updated_unix: i64) -> SavedChat {
        SavedChat {
            schema: SAVED_CHAT_SCHEMA.into(),
            id: id.into(),
            agent: "claude".into(),
            title: format!("title {id}"),
            updated_unix,
            access: "accept-edits".into(),
            ..SavedChat::default()
        }
    }

    fn put(dir: &Path, chat: &SavedChat) -> PathBuf {
        let path = chat_path(dir, &chat.id);
        let mut saver = Saver::start();
        saver.save(path.clone(), serde_json::to_vec(chat).unwrap());
        saver.finish();
        path
    }

    #[test]
    fn a_written_chat_is_private_and_lists_newest_first() {
        use std::os::unix::fs::MetadataExt;
        let dir = folder("list");
        put(&dir, &chat("old", 100));
        let newest = put(&dir, &chat("new", 200));
        assert_eq!(std::fs::metadata(&newest).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        let (chats, refused) = list(&dir, "");
        assert_eq!(
            chats.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["new", "old"]
        );
        assert_eq!(refused, 0);
        assert_eq!(load(&newest).unwrap(), chat("new", 200));
    }

    #[test]
    fn what_it_cannot_read_is_counted_and_left_alone() {
        let dir = folder("refused");
        put(&dir, &chat("good", 1));
        let broken = dir.join("broken.json");
        std::fs::write(&broken, "{").unwrap();
        let future = dir.join("future.json");
        let mut newer = chat("future", 2);
        newer.schema = "musializer.agent-chat/v2".into();
        std::fs::write(&future, serde_json::to_vec(&newer).unwrap()).unwrap();
        // A file whose id does not match its name is not this store's.
        let renamed = dir.join("renamed.json");
        std::fs::write(&renamed, serde_json::to_vec(&chat("other", 3)).unwrap()).unwrap();
        // Nor is one carrying another project's name.
        let mut stray = chat("stray", 4);
        stray.project = "/elsewhere.musi".into();
        std::fs::write(
            chat_path(&dir, "stray"),
            serde_json::to_vec(&stray).unwrap(),
        )
        .unwrap();
        let (chats, refused) = list(&dir, "");
        assert_eq!(chats.len(), 1);
        assert_eq!(refused, 4);
        assert_eq!(std::fs::read(&broken).unwrap(), b"{");
        assert!(future.exists() && renamed.exists());
    }

    #[test]
    fn a_chat_read_right_after_a_write_sees_that_write() {
        let dir = folder("order");
        let path = chat_path(&dir, "a");
        let saver = Saver::start();
        let mut latest = chat("a", 1);
        latest.draft = "x".repeat(4 * 1024 * 1024);
        saver.save(path.clone(), serde_json::to_vec(&chat("a", 0)).unwrap());
        saver.save(path.clone(), serde_json::to_vec(&latest).unwrap());
        saver.release(path.clone());
        let read = saver.load(path, "a".into(), String::new()).recv().unwrap();
        assert_eq!(read.unwrap(), latest);
    }

    #[test]
    fn two_windows_cannot_own_one_chat() {
        let dir = folder("two");
        let path = chat_path(&dir, "a");
        let (mut one, mut two) = (Saver::start(), Saver::start());
        one.save(path.clone(), serde_json::to_vec(&chat("a", 1)).unwrap());
        let load = |saver: &Saver| {
            saver
                .load(path.clone(), "a".into(), String::new())
                .recv()
                .unwrap()
        };
        assert!(load(&one).is_ok());
        assert!(load(&two).unwrap_err().contains("another window"));
        // Nor can the other one write over it or forget it.
        two.save(path.clone(), serde_json::to_vec(&chat("a", 2)).unwrap());
        assert!(two.forget(path.clone()).recv().unwrap().is_err());
        two.finish();
        assert_eq!(load(&one).unwrap().updated_unix, 1);
        assert!(two_failed(&two));
        one.release(path.clone());
        one.finish();
        let three = Saver::start();
        assert!(three.forget(path.clone()).recv().unwrap().is_ok());
        assert!(!path.exists() && !path.with_extension("lock").exists());
    }

    fn two_failed(saver: &Saver) -> bool {
        saver
            .failure()
            .is_some_and(|text| text.contains("another window"))
    }

    #[test]
    fn a_row_gets_the_chat_it_named_or_nothing() {
        let dir = folder("named");
        let path = chat_path(&dir, "a");
        let saver = Saver::start();
        let mut edited = chat("b", 1);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();
        assert!(saver
            .load(path.clone(), "a".into(), String::new())
            .recv()
            .unwrap()
            .is_err());
        edited.id = "a".into();
        edited.project = "/other.musi".into();
        std::fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();
        assert!(saver
            .load(path.clone(), "a".into(), String::new())
            .recv()
            .unwrap()
            .is_err());
        assert!(saver
            .load(path, "a".into(), "/other.musi".into())
            .recv()
            .unwrap()
            .is_ok());
    }

    #[test]
    fn an_existing_folder_is_made_private_before_a_transcript_lands_in_it() {
        use std::os::unix::fs::MetadataExt;
        let dir = folder("wide");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        put(&dir, &chat("a", 1));
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn an_id_cannot_climb_out_of_its_folder() {
        assert!(!sane_id("../../etc/passwd"));
        assert!(!sane_id(""));
        assert!(sane_id(&new_id(7)));
        let dir = folder("climb");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.json");
        let mut bad = chat("x", 1);
        bad.id = "../a".into();
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert_eq!(load(&path), None);
    }

    #[test]
    fn projects_do_not_share_a_folder_and_a_probe_run_needs_a_named_place() {
        let root = Path::new("/r");
        assert_ne!(
            project_folder(root, "/a.musi"),
            project_folder(root, "/b.musi")
        );
        assert_eq!(
            project_folder(root, "/a.musi"),
            project_folder(root, "/a.musi")
        );
        if std::env::var_os("MUSIALIZER_CHATS").is_none() {
            assert_eq!(root_for_test(), None);
        }
    }

    fn root_for_test() -> Option<PathBuf> {
        root(true)
    }
}
