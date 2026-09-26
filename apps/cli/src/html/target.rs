//! Protect business files from an accidentally reused report destination.
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::{
    Failure,
    paths::{Paths, absolute},
};

use super::output_error;

/// Return whether a replaceable report already exists. This is repeated just
/// before publishing; new destinations use persist_noclobber as well.
pub(super) fn validate(path: &Path, paths: &Paths) -> Result<bool, Failure> {
    let destination = canonical_allow_missing(&absolute(path)?)?;
    let protected = [
        paths.database.clone(),
        paths.data_dir.join("chat-tldr.db-wal"),
        paths.data_dir.join("chat-tldr.db-shm"),
        paths.data_dir.join("chat-tldr.db-journal"),
        paths.config_file.clone(),
        paths.data_dir.join("config.toml"),
        paths.data_dir.join("gui-state.json"),
    ];
    for protected in protected {
        if same_path(&destination, &canonical_allow_missing(&protected)?) {
            return Err(protected_output());
        }
    }
    for directory in [
        paths.data_dir.join("sources"),
        paths.data_dir.join("backups"),
    ] {
        let directory = canonical_allow_missing(&directory)?;
        if destination
            .ancestors()
            .any(|path| same_path(path, &directory))
        {
            return Err(protected_output());
        }
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(output_error(error)),
    };
    // Replacing a symlink silently destroys the link, even when its target is a
    // report. Require the caller to use the intended real report path instead.
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(protected_output());
    }
    if path.extension().is_some_and(|extension| {
        [
            "db", "db3", "sqlite", "sqlite3", "wal", "shm", "journal", "json", "jsonl", "toml",
            "csv",
        ]
        .iter()
        .any(|value| extension.eq_ignore_ascii_case(value))
    }) {
        return Err(protected_output());
    }
    // This catches SQLite and QCE/JSON exports renamed to .html. It deliberately
    // is not a universal file-type detector: ordinary existing reports remain
    // replaceable, including reports without an extension.
    let mut prefix = [0_u8; 4096];
    let length = fs::File::open(path)
        .and_then(|mut file| file.read(&mut prefix))
        .map_err(output_error)?;
    let prefix = prefix[..length]
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(&prefix[..length]);
    if prefix.starts_with(b"SQLite format 3\0")
        || prefix
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| matches!(byte, b'{' | b'['))
    {
        return Err(protected_output());
    }
    Ok(true)
}

fn protected_output() -> Failure {
    Failure::new(
        "E_OUTPUT_WRITE",
        8,
        "HTML output would replace protected business data, an original export, or a non-file destination; choose another report path",
    )
}

fn canonical_allow_missing(path: &Path) -> Result<PathBuf, Failure> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(protected_output)?;
            let name = path.file_name().ok_or_else(protected_output)?;
            Ok(canonical_allow_missing(parent)?.join(name))
        }
        Err(error) => Err(output_error(error)),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}
