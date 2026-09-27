//! GUI-only preferences. No database access and no credentials.
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub cli: String,
    pub data_dir: String,
    pub config: String,
    pub dark: bool,
    pub cloud_notice_accepted: bool,
    pub qce: crate::qce::Settings,
}

impl Preferences {
    pub fn normalize_paths(&mut self) -> Result<(), String> {
        for value in [&mut self.cli, &mut self.data_dir, &mut self.config] {
            if !value.is_empty() {
                *value = std::path::absolute(&*value)
                    .map_err(|error| format!("路径无法解析：{error}"))?
                    .to_string_lossy()
                    .into_owned();
            }
        }
        Ok(())
    }
}

pub fn default_data_dir() -> Result<PathBuf, String> {
    if !cfg!(any(windows, target_os = "macos"))
        && let Some(value) = std::env::var_os("XDG_DATA_HOME").filter(|s| !s.is_empty())
    {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err("XDG_DATA_HOME 必须是绝对路径".into());
        }
        return Ok(path.join("chat-tldr"));
    }
    let variable = if cfg!(windows) { "APPDATA" } else { "HOME" };
    let home = std::env::var_os(variable)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("缺少 {variable}；请使用 --data-dir 指定数据目录"))?;
    if cfg!(windows) {
        return Ok(PathBuf::from(home).join("chat-tldr"));
    }
    if cfg!(target_os = "macos") {
        return Ok(PathBuf::from(home).join("Library/Application Support/chat-tldr"));
    }
    Ok(PathBuf::from(home).join(".local/share/chat-tldr"))
}

pub fn load(path: &Path) -> Result<Preferences, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("GUI 设置无法读取：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(format!("GUI 设置无法读取：{e}")),
    }
}

/// Default launches follow the selected profile; explicit --data-dir wins.
pub fn load_profile(path: &Path, follow_selected: bool) -> Result<Preferences, String> {
    let mut current = load(path)?;
    if !follow_selected {
        return Ok(current);
    }
    let mut path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..16 {
        if current.data_dir.is_empty() {
            return Ok(current);
        }
        let next = std::path::absolute(PathBuf::from(&current.data_dir).join("gui-state.json"))
            .map_err(|e| e.to_string())?;
        if next == path {
            return Ok(current);
        }
        if !visited.insert(path) {
            return Err("GUI 设置中的数据目录引用形成循环，请使用 --data-dir 指定目录。".into());
        }
        current = match std::fs::read(&next) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("GUI 设置无法读取：{e}"))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(current),
            Err(error) => return Err(format!("GUI 设置无法读取：{error}")),
        };
        path = next;
    }
    Err("GUI 设置的数据目录引用过多，请使用 --data-dir 指定目录。".into())
}

/// Called only on the background worker, after explicit settings changes.
pub fn save(path: &Path, prefs: &Preferences) -> Result<(), String> {
    let save = || -> Result<(), Box<dyn std::error::Error>> {
        let parent = path.parent().ok_or("设置文件缺少父目录")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut file, prefs)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    };
    save().map_err(|e| format!("GUI 设置未保存：{e}"))
}

/// Commit the selected profile before changing the launch pointer. Otherwise a
/// failed/interrupted switch from B back to A can leave A -> B -> A on disk.
pub fn save_profile(launch: Option<&Path>, prefs: &Preferences) -> Result<(), String> {
    let active = PathBuf::from(&prefs.data_dir).join("gui-state.json");
    save(&active, prefs)?;
    if let Some(launch) = launch.filter(|launch| *launch != active) {
        save(launch, prefs)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_replace_atomically_and_corrupt_input_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui-state.json");
        let mut value = Preferences {
            data_dir: "路径 含空格".into(),
            ..Default::default()
        };
        save(&path, &value).unwrap();
        value.dark = true;
        save(&path, &value).unwrap();
        let loaded = load(&path).unwrap();
        assert!(loaded.dark);
        assert_eq!(loaded.data_dir, value.data_dir);
        std::fs::write(&path, "broken").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
    }

    #[test]
    fn selected_profile_is_loaded_once_and_explicit_profile_does_not_follow() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a/gui-state.json");
        let b = dir.path().join("b/gui-state.json");
        let selected = b.parent().unwrap().to_string_lossy().into_owned();
        save(
            &a,
            &Preferences {
                cli: "old".into(),
                data_dir: selected.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        save(
            &b,
            &Preferences {
                cli: "new".into(),
                data_dir: selected,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load_profile(&a, true).unwrap().cli, "new");
        assert_eq!(load_profile(&a, false).unwrap().cli, "old");
        save(
            &b,
            &Preferences {
                data_dir: a.parent().unwrap().to_string_lossy().into_owned(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(load_profile(&a, true).is_err());
    }

    #[test]
    fn failed_target_save_keeps_launch_profile_intact() {
        let dir = tempfile::tempdir().unwrap();
        let launch = dir.path().join("launch/gui-state.json");
        let original = Preferences {
            data_dir: launch.parent().unwrap().to_string_lossy().into_owned(),
            ..Default::default()
        };
        save(&launch, &original).unwrap();
        let blocked = dir.path().join("file-not-directory");
        std::fs::write(&blocked, "keep me").unwrap();
        let changed = Preferences {
            data_dir: blocked.to_string_lossy().into_owned(),
            ..Default::default()
        };
        assert!(save_profile(Some(&launch), &changed).is_err());
        assert_eq!(load(&launch).unwrap().data_dir, original.data_dir);
        assert_eq!(std::fs::read_to_string(blocked).unwrap(), "keep me");
    }

    #[test]
    fn switching_back_commits_self_pointing_target_before_launch_redirect() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a/gui-state.json");
        let b = dir.path().join("b/gui-state.json");
        let selected_b = Preferences {
            data_dir: b.parent().unwrap().to_string_lossy().into_owned(),
            ..Default::default()
        };
        save_profile(Some(&a), &selected_b).unwrap();
        let selected_a = Preferences {
            data_dir: a.parent().unwrap().to_string_lossy().into_owned(),
            ..Default::default()
        };
        save_profile(Some(&b), &selected_a).unwrap();
        assert_eq!(
            load_profile(&a, true).unwrap().data_dir,
            selected_a.data_dir
        );
        assert_eq!(
            load_profile(&b, true).unwrap().data_dir,
            selected_a.data_dir
        );
    }
}
