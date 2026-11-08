//! The app data directory: encryption keys and settings.
//!
//! It lives in the OS app data folder (`~/Library/Application Support/NSCB Desktop`,
//! `%APPDATA%\NSCB Desktop`, `$XDG_DATA_HOME/nscb-desktop`). Earlier releases kept these
//! files in `<os temp dir>/nscb-desktop-tools`, which macOS and temp cleaners empty;
//! they are moved over on first start.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SETTING_TEMP_DIR: &str = "tempDir";
pub const SETTING_PROXY: &str = "proxy";
pub const SETTING_NUTDB_URL: &str = "nutdbUrl";
pub const DEFAULT_NUTDB_URL: &str =
    "https://raw.githubusercontent.com/blawar/titledb/master/US.en.json";

/// Folder name inside the OS app data directory.
const APP_DIR_NAME: &str = if cfg!(any(target_os = "macos", target_os = "windows")) {
    "NSCB Desktop"
} else {
    "nscb-desktop"
};

/// Files carried over from the legacy location.
const MIGRATED_FILES: &[&str] = &["prod.keys", "keys.txt", "settings.json"];

/// Where releases up to 2.7.x kept keys and settings.
pub fn legacy_dir() -> PathBuf {
    std::env::temp_dir().join("nscb-desktop-tools")
}

#[derive(Clone, Debug)]
pub struct Tools {
    dir: PathBuf,
}

impl Tools {
    /// The app data directory (or `$NSCB_DESKTOP_TOOLS_DIR`), created if missing, with
    /// keys and settings migrated from the legacy temp location.
    pub fn locate() -> Result<Self, String> {
        if let Some(dir) = std::env::var_os("NSCB_DESKTOP_TOOLS_DIR").filter(|d| !d.is_empty()) {
            return Self::at(PathBuf::from(dir));
        }
        let data = dirs::data_dir()
            .ok_or_else(|| "Could not find the user's app data folder".to_string())?
            .join(APP_DIR_NAME);
        let tools = Self::at(data)?;
        tools.migrate_from(&legacy_dir());
        Ok(tools)
    }

    pub fn at(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create tools dir: {e}"))?;
        Ok(Self { dir })
    }

    /// Move keys and settings from `legacy` unless this directory already has them.
    /// Best effort: anything that fails stays where it is.
    pub fn migrate_from(&self, legacy: &Path) {
        if legacy == self.dir {
            return;
        }
        for name in MIGRATED_FILES {
            let (src, dst) = (legacy.join(name), self.dir.join(name));
            if !src.is_file() || dst.exists() {
                continue;
            }
            if fs::rename(&src, &dst).is_err() && fs::copy(&src, &dst).is_ok() {
                let _ = fs::remove_file(&src);
            }
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn keys_path(&self) -> PathBuf {
        self.dir.join("prod.keys")
    }

    pub fn has_keys(&self) -> bool {
        self.keys_path().exists() || self.dir.join("keys.txt").exists()
    }

    pub fn import_keys(&self, src: &Path) -> Result<(), String> {
        fs::copy(src, self.keys_path()).map_err(|e| format!("Failed to copy prod.keys: {e}"))?;
        Ok(())
    }

    // ---- settings ----

    fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    fn read_settings(&self) -> BTreeMap<String, String> {
        fs::read_to_string(self.settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write_settings(&self, settings: &BTreeMap<String, String>) -> Result<(), String> {
        let data = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Failed to serialize settings: {e}"))?;
        fs::write(self.settings_path(), data).map_err(|e| format!("Failed to write settings: {e}"))
    }

    pub fn setting(&self, key: &str) -> String {
        self.read_settings().get(key).cloned().unwrap_or_default()
    }

    /// Save a setting; an empty value removes the key.
    pub fn save_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let mut settings = self.read_settings();
        if value.is_empty() {
            settings.remove(key);
        } else {
            settings.insert(key.to_string(), value.to_string());
        }
        self.write_settings(&settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tools(name: &str) -> Tools {
        let dir = std::env::temp_dir().join(format!("nscb-desktop-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Tools::at(dir).unwrap()
    }

    #[test]
    fn settings_roundtrip_and_empty_value_removes_key() {
        let tools = temp_tools("settings");
        assert_eq!(tools.setting(SETTING_PROXY), "");
        tools.save_setting(SETTING_PROXY, "https://gh-proxy.org/").unwrap();
        assert_eq!(tools.setting(SETTING_PROXY), "https://gh-proxy.org/");
        tools.save_setting(SETTING_PROXY, "").unwrap();
        assert_eq!(tools.setting(SETTING_PROXY), "");
        let _ = fs::remove_dir_all(tools.dir());
    }

    #[test]
    fn migrates_keys_and_settings_without_overwriting() {
        let legacy = temp_tools("legacy");
        let tools = temp_tools("migrated");
        fs::write(legacy.keys_path(), "old keys").unwrap();
        fs::write(legacy.dir().join("settings.json"), r#"{"proxy":"p"}"#).unwrap();
        fs::write(tools.dir().join("keys.txt"), "new").unwrap();
        fs::write(legacy.dir().join("keys.txt"), "stale").unwrap();

        tools.migrate_from(legacy.dir());

        assert_eq!(fs::read_to_string(tools.keys_path()).unwrap(), "old keys");
        assert_eq!(tools.setting(SETTING_PROXY), "p");
        assert!(!legacy.keys_path().exists(), "moved, not copied");
        assert_eq!(fs::read_to_string(tools.dir().join("keys.txt")).unwrap(), "new", "existing files win");
        let _ = fs::remove_dir_all(legacy.dir());
        let _ = fs::remove_dir_all(tools.dir());
    }

    #[test]
    fn keys_detected_as_prod_keys_or_keys_txt() {
        let tools = temp_tools("keys");
        assert!(!tools.has_keys());
        fs::write(tools.dir().join("keys.txt"), "x").unwrap();
        assert!(tools.has_keys());
        let _ = fs::remove_dir_all(tools.dir());
    }
}
