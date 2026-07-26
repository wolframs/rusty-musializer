//! The human identity of a track: whatever a user reads as its name.
//!
//! **Owner: Agent A.** Port of `../musializer/src/track_identity.c` and `.h`.
//!
//! The reason this is a module rather than an inline expression is a regression
//! the C repository paid for (`../musializer/tests/test_track_identity.c:10-12`):
//! the track rail, the export summary, the assist status line, and the
//! applied-suggestion notice each derived its label from the track's file path,
//! and saving *rewrites* that path to `<stem>.assets/audio/<sha256>.<ext>`. Every
//! one of them started showing a SHA-256. The project title is the identity that
//! survives open and save, so it wins whenever it exists.
//!
//! The Rust signature returns a borrow with the arguments' lifetime, which is
//! exactly the aliasing contract the C header documents in prose
//! (`track_identity.h:11-14`): nothing is copied, nothing is allocated, and the
//! result points into one of the arguments or at a `'static` empty string.

/// The label to show for a track (`track_identity.c:5-22`).
///
/// `None` and `Some("")` are equivalent for both arguments — C tests
/// `!= NULL && [0] != '\0'` and this mirrors it — so a caller holding a plain
/// `&str` can pass `Some(s)` without pre-checking for emptiness.
///
/// Precedence, in order:
///
/// 1. a non-empty `project_title`, used **verbatim** and never reinterpreted as
///    a path (so a title of `/not/a/path` stays `/not/a/path`);
/// 2. the file-name component of `audio_path`, splitting on `/` *and* `\` on
///    every platform, because a project written on one host is expected to open
///    on another with its stored asset path intact (`track_identity.c:11-13`);
/// 3. `""`.
///
/// A path ending in a separator yields `""`. That is deliberate: the empty tail
/// is honest, whereas falling back to the whole path would print a directory
/// chain into a control sized for a label (`track_identity.c:18-21`).
#[must_use]
pub fn display_name<'a>(project_title: Option<&'a str>, audio_path: Option<&'a str>) -> &'a str {
    if let Some(title) = project_title {
        if !title.is_empty() {
            return title;
        }
    }
    let path = match audio_path {
        Some(path) if !path.is_empty() => path,
        _ => return "",
    };
    // Splitting on ASCII separator *bytes* needs no char-boundary check on a
    // `str`: every byte of a multi-byte UTF-8 sequence has its high bit set, so
    // `/` and `\` can only ever occur as themselves.
    match path.rfind(['/', '\\']) {
        Some(index) => &path[index + 1..],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn degenerate_input_never_yields_a_directory_chain() {
        assert_eq!(display_name(None, None), "");
        assert_eq!(display_name(Some(""), Some("")), "");
        assert_eq!(display_name(None, Some("")), "");
        // A trailing separator has no file-name component. The empty tail is
        // honest; the enclosing directory chain would overflow a label control.
        assert_eq!(display_name(None, Some("/music/")), "");
        assert_eq!(display_name(None, Some("/")), "");
        assert_eq!(display_name(None, Some("C:\\")), "");
    }

    /// The regression the module exists for: after a save, the path *is* a hash.
    #[test]
    fn a_project_title_beats_the_hashed_asset_path() {
        let hashed = "demo.assets/audio/\
                      ec3646f6923d08996d02935214e43e458921a9dfe40fb5021cb02a5ad76abfeb.wav";
        assert_eq!(display_name(Some("demo"), Some(hashed)), "demo");
        assert_eq!(
            display_name(Some("A Title With Spaces"), Some(hashed)),
            "A Title With Spaces"
        );
        // A title is used verbatim, never reinterpreted as a path.
        assert_eq!(
            display_name(Some("/not/a/path"), Some("x.wav")),
            "/not/a/path"
        );
    }

    #[test]
    fn without_a_title_the_file_name_wins_on_either_separator() {
        // Audio opened directly carries no project metadata yet.
        assert_eq!(display_name(None, Some("/music/song.mp3")), "song.mp3");
        assert_eq!(display_name(Some(""), Some("/music/song.mp3")), "song.mp3");
        assert_eq!(display_name(None, Some("song.mp3")), "song.mp3");
        // A project authored on Windows is expected to open on a POSIX host with
        // its stored asset path intact, and the reverse.
        assert_eq!(display_name(None, Some("C:\\music\\song.mp3")), "song.mp3");
        assert_eq!(display_name(None, Some("a/b\\c/song.mp3")), "song.mp3");
    }

    /// Not in the C suite. C scans byte-wise and would only split a multi-byte
    /// sequence if one of its bytes equalled `/`, which UTF-8 forbids — so this
    /// is the same answer, pinned so a future rewrite cannot panic on a
    /// char boundary.
    #[test]
    fn non_ascii_names_are_returned_whole() {
        assert_eq!(display_name(None, Some("/音楽/曲.mp3")), "曲.mp3");
        assert_eq!(display_name(Some("naïve—dash"), None), "naïve—dash");
    }
}
