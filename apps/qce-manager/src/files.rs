use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

use crate::error::{Failure, Result};

pub struct Files {
    root: PathBuf,
}

impl Files {
    pub fn new(path: Option<&Path>) -> Result<Self> {
        let root = match path {
            Some(path) => path.to_path_buf(),
            None => default_dir()?,
        };
        let root = std::path::absolute(root).map_err(|_| Failure::io())?;
        // Resolve the explicitly selected root, including OS aliases such as macOS /var.
        // Once anchored, all component-owned descendants are checked without following links.
        let mut existing = root.as_path();
        let mut missing = Vec::new();
        while !existing.try_exists().map_err(|_| Failure::io())? {
            missing.push(existing.file_name().ok_or_else(Failure::io)?.to_owned());
            existing = existing.parent().ok_or_else(Failure::io)?;
        }
        let mut root = fs::canonicalize(existing).map_err(|_| Failure::io())?;
        for part in missing.into_iter().rev() {
            root.push(part);
        }
        ensure_unlinked(&root)?;
        Ok(Self { root })
    }

    fn tmp(&self) -> PathBuf {
        self.root.join("tmp/qce-manager")
    }
    fn exports(&self) -> PathBuf {
        self.root.join("sources/qce/exports")
    }

    fn coordination(&self) -> Result<File> {
        let path = self.root.join("sources/qce/state/manager.lock");
        safe_mkdir(path.parent().ok_or_else(Failure::io)?)?;
        ensure_unlinked(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|_| Failure::io())?;
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => Failure::new(
                "E_RUN_IN_PROGRESS",
                7,
                "另一管理进程正在修改目录；请稍后重试",
            ),
            _ => Failure::io(),
        })?;
        Ok(file)
    }

    pub fn start(&self) -> Result<Job<'_>> {
        self.start_id(format!("qce_{}", uuid::Uuid::new_v4().simple()))
    }

    fn start_id(&self, id: String) -> Result<Job<'_>> {
        let _coordination = self.coordination()?;
        safe_mkdir(&self.tmp())?;
        safe_mkdir(&self.exports())?;
        let lock_path = self.tmp().join(format!("{id}.lock"));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|_| Failure::io())?;
        lock.lock().map_err(|_| Failure::io())?;
        let path = self.tmp().join(&id);
        if fs::create_dir(&path).is_err() {
            drop(lock);
            let _ = fs::remove_file(&lock_path);
            return Err(Failure::io());
        }
        let job = Job {
            files: self,
            id,
            path,
            lock_path,
            lock: Some(lock),
            published: false,
        };
        Ok(job)
    }

    pub fn clean(&self, dry_run: bool) -> Result<Value> {
        let tmp = self.tmp();
        let cache = self.root.join("cache/qce/downloads");
        ensure_unlinked(&tmp)?;
        ensure_unlinked(&cache)?;
        // dry-run does not even create the coordination lock or data directories.
        let _coordination = if dry_run {
            None
        } else {
            Some(self.coordination()?)
        };
        let mut removed = 0;
        let mut active = 0;
        let mut skipped = 0;
        if tmp.exists() {
            for entry in fs::read_dir(&tmp).map_err(|_| Failure::io())? {
                let entry = entry.map_err(|_| Failure::io())?;
                let path = entry.path();
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(Failure::io()),
                };
                if is_link(&metadata) {
                    skipped += 1;
                    continue;
                }
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    skipped += 1;
                    continue;
                };
                let id = name.strip_suffix(".lock").unwrap_or(name);
                if !valid_id(id) {
                    skipped += 1;
                    continue;
                }
                // Process a job and its companion lock together; orphan locks are reclaimed too.
                if name.ends_with(".lock") && tmp.join(id).exists() {
                    continue;
                }
                let lock_path = tmp.join(format!("{id}.lock"));
                ensure_unlinked(&lock_path)?;
                let lock = match OpenOptions::new().read(true).write(true).open(&lock_path) {
                    Ok(lock) => match lock.try_lock() {
                        Ok(()) => Some(lock),
                        Err(std::fs::TryLockError::WouldBlock) => {
                            active += 1;
                            continue;
                        }
                        Err(_) => return Err(Failure::io()),
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => return Err(Failure::io()),
                };
                let job_path = tmp.join(id);
                if job_path.exists() {
                    safe_tree(&job_path)?;
                }
                if !dry_run {
                    if job_path.exists() {
                        fs::remove_dir_all(&job_path).map_err(|_| Failure::io())?;
                    }
                    drop(lock);
                    if lock_path.exists() {
                        fs::remove_file(&lock_path).map_err(|_| Failure::io())?;
                    }
                }
                removed += 1;
            }
        }
        if cache.exists() {
            for entry in fs::read_dir(&cache).map_err(|_| Failure::io())? {
                let path = entry.map_err(|_| Failure::io())?.path();
                if safe_tree(&path).is_err() {
                    skipped += 1;
                    continue;
                }
                if !dry_run {
                    if path.is_dir() {
                        fs::remove_dir_all(&path)
                    } else {
                        fs::remove_file(&path)
                    }
                    .map_err(|_| Failure::io())?;
                }
                removed += 1;
            }
        }
        Ok(
            json!({"dry_run":dry_run,"candidates":removed,"removed":if dry_run {0} else {removed},"active":active,"skipped":skipped}),
        )
    }
}

pub struct Job<'a> {
    files: &'a Files,
    pub id: String,
    pub path: PathBuf,
    lock_path: PathBuf,
    lock: Option<File>,
    published: bool,
}

impl Job<'_> {
    pub fn publish(&mut self) -> Result<PathBuf> {
        let _coordination = self.files.coordination()?;
        let target = self.files.exports().join(&self.id);
        ensure_unlinked(&target)?;
        safe_tree(&self.path)?;
        if target.exists() {
            return Err(Failure::io());
        }
        // All manager publishers/cleaners serialize under the same coordination lock.
        // The sibling job lock remains held across the directory rename on Windows.
        fs::rename(&self.path, &target).map_err(|_| Failure::io())?;
        self.published = true;
        self.lock.take();
        // Publication is committed. A stale lock is harmless and clean can recover it.
        let _ = fs::remove_file(&self.lock_path);
        Ok(target)
    }
}

impl Drop for Job<'_> {
    fn drop(&mut self) {
        if !self.published {
            if self.path.exists() && safe_tree(&self.path).is_ok() {
                let _ = fs::remove_dir_all(&self.path);
            }
            self.lock.take();
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix("qce_")
        .is_some_and(|suffix| suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn ensure_unlinked(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link(&metadata) => return Err(Failure::io()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Failure::io()),
        }
    }
    Ok(())
}

fn safe_mkdir(path: &Path) -> Result<()> {
    ensure_unlinked(path)?;
    fs::create_dir_all(path).map_err(|_| Failure::io())?;
    ensure_unlinked(path)
}

fn safe_tree(path: &Path) -> Result<()> {
    ensure_unlinked(path)?;
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|_| Failure::io())? {
            safe_tree(&entry.map_err(|_| Failure::io())?.path())?;
        }
    }
    Ok(())
}

fn default_dir() -> Result<PathBuf> {
    let missing = || Failure::new("E_CONFIG", 4, "无法确定默认数据目录；请传 --data-dir");
    #[cfg(windows)]
    {
        Ok(PathBuf::from(
            std::env::var_os("APPDATA")
                .filter(|v| !v.is_empty())
                .ok_or_else(missing)?,
        )
        .join("chat-tldr"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(std::env::home_dir()
            .ok_or_else(missing)?
            .join("Library/Application Support/chat-tldr"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let base = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(value) => {
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(missing());
                }
                path
            }
            None => std::env::home_dir()
                .ok_or_else(missing)?
                .join(".local/share"),
        };
        Ok(base.join("chat-tldr"))
    }
}

#[cfg(test)]
mod tests;
