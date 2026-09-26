//! Publish completed artifacts from a sibling temporary location. Never overwrite.
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use tempfile::{NamedTempFile, TempDir};

struct OutputLock {
    path: PathBuf,
    file: Option<File>,
}
impl Drop for OutputLock {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}

fn missing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!("output already exists: {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect output: {error}")),
    }
}

fn prepare(out: &Path) -> Result<(PathBuf, OutputLock), String> {
    missing(out)?;
    let name = out
        .file_name()
        .ok_or("output must name a file or a new directory")?;
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| format!("cannot create output parent: {error}"))?;
    let mut lock_name = name.to_os_string();
    lock_name.push(".chat-tldr-eval.lock");
    let path = parent.join(lock_name);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            format!("cannot reserve output (another writer or stale lock): {error}")
        })?;
    let lock = OutputLock {
        path,
        file: Some(file),
    };
    missing(out)?;
    Ok((parent.to_owned(), lock))
}

pub fn file(out: &Path, write: impl FnOnce(&mut File) -> Result<(), String>) -> Result<(), String> {
    let (parent, _lock) = prepare(out)?;
    let mut temporary =
        NamedTempFile::new_in(parent).map_err(|error| format!("cannot stage output: {error}"))?;
    write(temporary.as_file_mut())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("cannot flush output: {error}"))?;
    temporary
        .persist_noclobber(out)
        .map_err(|error| format!("cannot publish output: {}", error.error))?;
    Ok(())
}

pub fn gold(
    out: &Path,
    messages: &[impl Serialize],
    items: &[impl Serialize],
) -> Result<(), String> {
    let (parent, _lock) = prepare(out)?;
    let temporary =
        TempDir::new_in(parent).map_err(|error| format!("cannot stage gold directory: {error}"))?;
    jsonl(&temporary.path().join("messages.jsonl"), messages)?;
    jsonl(&temporary.path().join("items.jsonl"), items)?;
    missing(out)?;
    rename_new_directory(temporary.path(), out)
        .map_err(|error| format!("cannot publish gold directory: {error}"))?;
    Ok(())
}

// The reservation lock coordinates this tool's writers. An unrelated writer
// can still create the target after missing(out), so publication itself must
// refuse replacement. Plain rename can replace an existing empty folder on
// POSIX and on current Windows implementations of Rust's standard library.
fn rename_new_directory(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        // On Windows this path-only operation calls MoveFileExW without
        // MOVEFILE_REPLACE_EXISTING, which handles directories as well. Keep
        // cleanup with the caller's TempDir, rather than TempPath's file unlink.
        let mut path = tempfile::TempPath::try_from_path(from)?;
        path.disable_cleanup(true);
        path.persist_noclobber(to).map_err(|error| error.error)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            from,
            rustix::fs::CWD,
            to,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(Into::into)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = (from, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic directory publication without replacement is unsupported on this platform",
        ))
    }
}

fn jsonl(path: &Path, values: &[impl Serialize]) -> Result<(), String> {
    use std::io::Write;
    let file = File::create(path).map_err(|error| format!("cannot stage gold file: {error}"))?;
    let mut file = io::BufWriter::new(file);
    for value in values {
        serde_json::to_writer(&mut file, value)
            .map_err(|error| format!("cannot serialize gold: {error}"))?;
        file.write_all(b"\n")
            .map_err(|error| format!("cannot write gold: {error}"))?;
    }
    file.flush()
        .map_err(|error| format!("cannot flush gold: {error}"))?;
    file.get_ref()
        .sync_all()
        .map_err(|error| format!("cannot flush gold: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_write_failure_leaves_no_published_file_or_lock() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("sheet.csv");
        let result = file(&out, |file| {
            file.write_all(b"partial output").unwrap();
            Err("synthetic write failure".into())
        });
        assert!(result.is_err());
        assert!(!out.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    struct InvalidItem;
    impl Serialize for InvalidItem {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("synthetic serialization failure"))
        }
    }

    #[test]
    fn second_gold_file_failure_does_not_publish_the_first_file() {
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("gold");
        let result = gold(
            &out,
            &[serde_json::json!({"message_id":"synthetic"})],
            &[InvalidItem],
        );
        assert!(result.is_err());
        assert!(!out.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn directory_created_after_reservation_is_never_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("gold");
        let (parent, _lock) = prepare(&out).unwrap();
        let temporary = TempDir::new_in(parent).unwrap();
        fs::write(temporary.path().join("messages.jsonl"), "synthetic").unwrap();
        // Simulate a different application creating an empty folder between
        // validation and the actual filesystem publication operation.
        missing(&out).unwrap();
        fs::create_dir(&out).unwrap();
        assert!(rename_new_directory(temporary.path(), &out).is_err());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
        assert!(temporary.path().join("messages.jsonl").exists());
    }
}
