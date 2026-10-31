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
//! Writes go through one thread, in order, so a turn ending never stalls a
//! frame. A file this build cannot read is skipped and never touched.

use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
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
pub fn list(folder: &Path) -> (Vec<ChatSummary>, usize) {
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
            Some(chat) if chat_path(folder, &chat.id) == path => {
                chats.push(ChatSummary::of(&chat, path));
            }
            _ => refused += 1,
        }
    }
    chats.sort_by_key(|chat| std::cmp::Reverse(chat.updated_unix));
    (chats, refused)
}

/// Removes a chat here. The agent's own history is its CLI's.
pub fn forget(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|error| error.to_string())?;
    }
    musializer_runtime::process::publish::atomic_write(path, bytes)
        .map_err(|error| error.to_string())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())
}

/// The one thread that writes chats.
pub struct Saver {
    jobs: Option<Sender<(PathBuf, Vec<u8>)>>,
    failures: Receiver<String>,
    thread: Option<JoinHandle<()>>,
}

impl Saver {
    #[must_use]
    pub fn start() -> Self {
        let (jobs, inbox) = mpsc::channel::<(PathBuf, Vec<u8>)>();
        let (report, failures) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            for (path, bytes) in inbox {
                if let Err(error) = write(&path, &bytes) {
                    let _ = report.send(format!("{}: {error}", path.display()));
                }
            }
        });
        Self {
            jobs: Some(jobs),
            failures,
            thread: Some(thread),
        }
    }

    pub fn save(&self, path: PathBuf, bytes: Vec<u8>) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send((path, bytes));
        }
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
        let (chats, refused) = list(&dir);
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
        let (chats, refused) = list(&dir);
        assert_eq!(chats.len(), 1);
        assert_eq!(refused, 3);
        assert_eq!(std::fs::read(&broken).unwrap(), b"{");
        assert!(future.exists() && renamed.exists());
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
