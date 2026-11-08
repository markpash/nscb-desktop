//! The tools directory: encryption keys, settings and verify scratch files.
//!
//! Everything lives in `<os temp dir>/nscb-desktop-tools`, the same location the
//! previous Tauri build used, so existing installs keep their keys and settings.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SETTING_TEMP_DIR: &str = "tempDir";
pub const SETTING_PROXY: &str = "proxy";
pub const SETTING_NUTDB_URL: &str = "nutdbUrl";
pub const DEFAULT_NUTDB_URL: &str =
    "https://raw.githubusercontent.com/blawar/titledb/master/US.en.json";

#[derive(Clone, Debug)]
pub struct Tools {
    dir: PathBuf,
}

impl Tools {
    /// The tools directory in the OS temp dir (or `$NSCB_DESKTOP_TOOLS_DIR`), created if missing.
    pub fn locate() -> Result<Self, String> {
        match std::env::var_os("NSCB_DESKTOP_TOOLS_DIR") {
            Some(dir) if !dir.is_empty() => Self::at(PathBuf::from(dir)),
            _ => Self::at(std::env::temp_dir().join("nscb-desktop-tools")),
        }
    }

    pub fn at(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create tools dir: {e}"))?;
        Ok(Self { dir })
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

    /// Write a one-line file list for `--verify all --text_file` (non-interactive verify)
    /// and remove any cached result so the backend runs fresh.
    pub fn create_verify_filelist(&self, target: &str) -> Result<PathBuf, String> {
        let stem = Path::new(target)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("verify");
        let safe: String = stem
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let list_path = self.dir.join(format!("{safe}-vflist.txt"));
        let cached = self
            .dir
            .join("INFO")
            .join("MASSVERIFY")
            .join(format!("{safe}-vflist-verify.txt"));
        let _ = fs::remove_file(cached);
        fs::write(&list_path, format!("{target}\n"))
            .map_err(|e| format!("Failed to create verify filelist: {e}"))?;
        Ok(list_path)
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
    fn keys_detected_as_prod_keys_or_keys_txt() {
        let tools = temp_tools("keys");
        assert!(!tools.has_keys());
        fs::write(tools.dir().join("keys.txt"), "x").unwrap();
        assert!(tools.has_keys());
        let _ = fs::remove_dir_all(tools.dir());
    }

    #[test]
    fn verify_filelist_sanitizes_name_and_contains_target() {
        let tools = temp_tools("verify");
        let path = tools.create_verify_filelist("/games/My Game [v0].nsp").unwrap();
        assert_eq!(path.file_name().unwrap(), "My_Game__v0_-vflist.txt");
        assert_eq!(fs::read_to_string(&path).unwrap(), "/games/My Game [v0].nsp\n");
        let _ = fs::remove_dir_all(tools.dir());
    }
}
