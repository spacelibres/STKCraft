//! Stored conversations: one JSON Lines file per session.
//!
//! A transcript is append-only text, so it is not a relational row: a crash during a write must
//! cost the last line, not the conversation. It still belongs to this crate, because the rule is
//! that one crate opens files — a conversation is something the program produced, and a path
//! assembled elsewhere is how a session id becomes a path-traversal bug.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use stkcraft_framework_core::Error;

/// The largest transcript this crate will read, bytes.
///
/// A session file is input from outside the process (it can be edited or replaced); a read that
/// trusts its length is an allocation the file chooses. A conversation larger than this is not a
/// conversation anyone resumes by scrolling.
pub const MAX_TRANSCRIPT_BYTES: u64 = 64 * 1024 * 1024;

/// A session id, as a file name without a directory.
///
/// The id comes from the clock, but `chat --session <id>` lets a caller choose one; a value with a
/// separator or `..` would escape the sessions directory. Only the characters the clock produces
/// and a leading dot-free name are accepted.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

/// The file for one session, after checking that the id is a name and not a path.
///
/// # Errors
///
/// [`Error::Invalid`] when the id is empty, too long, or contains a path separator.
pub fn path(directory: &Path, id: &str) -> Result<PathBuf, Error> {
    if !valid_id(id) {
        return Err(Error::invalid(
            "session id",
            "must be a plain file name (letters, digits, '.', '-', '_')",
        ));
    }
    Ok(directory.join(format!("{id}.jsonl")))
}

/// Create the sessions directory, and report why if it cannot be.
///
/// # Errors
///
/// [`Error::Invalid`] when the directory cannot be created.
pub fn ensure_directory(directory: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(directory).map_err(|error| {
        Error::invalid(
            "sessions directory",
            format!("{}: {error}", directory.display()),
        )
    })
}

/// Read one transcript, bounded before the allocation.
///
/// # Errors
///
/// [`Error::Invalid`] when the file is too large or cannot be read.
pub fn read(path: &Path) -> Result<String, Error> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| Error::invalid("transcript", format!("{}: {error}", path.display())))?;
    if metadata.len() > MAX_TRANSCRIPT_BYTES {
        return Err(Error::invalid(
            "transcript",
            format!(
                "{} is {} bytes; the limit is {MAX_TRANSCRIPT_BYTES}",
                path.display(),
                metadata.len()
            ),
        ));
    }
    std::fs::read_to_string(path)
        .map_err(|error| Error::invalid("transcript", format!("{}: {error}", path.display())))
}

/// Append one line to a transcript, creating the file if the session is new.
///
/// # Errors
///
/// [`Error::Invalid`] when the file cannot be opened or written.
pub fn append_line(path: &Path, line: &str) -> Result<(), Error> {
    if let Some(directory) = path.parent() {
        ensure_directory(directory)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| Error::invalid("transcript", format!("{}: {error}", path.display())))?;
    writeln!(file, "{line}")
        .map_err(|error| Error::invalid("transcript", format!("{}: {error}", path.display())))
}

/// Replace a transcript's contents. Used by the compaction path that rewrites a session.
///
/// # Errors
///
/// [`Error::Invalid`] when the file cannot be written.
pub fn replace(path: &Path, text: &str) -> Result<(), Error> {
    if let Some(directory) = path.parent() {
        ensure_directory(directory)?;
    }
    std::fs::write(path, text)
        .map_err(|error| Error::invalid("transcript", format!("{}: {error}", path.display())))
}

/// The session ids in one directory, with a missing directory reported as empty.
///
/// A missing sessions directory is a first run, not an error; a directory that exists but cannot be
/// read is also treated as empty, because the alternative is a window that refuses to open.
#[must_use]
pub fn list(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".jsonl").map(str::to_owned)
        })
        .collect();
    ids.sort();
    ids.reverse();
    ids
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "stkcraft-transcripts-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_session_id_is_a_name_and_never_a_path() {
        assert!(valid_id("2026-11-07T12-00-00"));
        assert!(valid_id("session_1.test"));
        assert!(!valid_id(".."));
        assert!(!valid_id("."));
        assert!(!valid_id("a/b"));
        assert!(!valid_id("a\\b"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"a".repeat(129)));
        assert!(path(Path::new("/tmp"), "a/b").is_err());
    }

    #[test]
    fn a_transcript_round_trips_and_lists() {
        let directory = scratch("round-trip");
        let file = path(&directory, "2026-11-07T12-00-00").expect("a valid id");
        append_line(&file, "{\"note\":\"one\"}").expect("append");
        append_line(&file, "{\"note\":\"two\"}").expect("append");
        let text = read(&file).expect("read");
        assert_eq!(text, "{\"note\":\"one\"}\n{\"note\":\"two\"}\n");
        assert_eq!(list(&directory), vec!["2026-11-07T12-00-00".to_owned()]);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
