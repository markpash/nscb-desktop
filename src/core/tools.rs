//! The tools directory: backend binary, encryption keys, settings and verify scratch files.
//!
//! Everything lives in `<os temp dir>/nscb-desktop-tools`, the same location the
//! previous Tauri build used, so existing installs keep their keys and settings.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SETTING_TEMP_DIR: &str = "tempDir";
pub const SETTING_PROXY: &str = "proxy";
pub const SETTING_NUTDB_URL: &str = "nutdbUrl";
const SETTING_BACKEND_VERSION: &str = "backendVersion";

pub const DEFAULT_NUTDB_URL: &str =
    "https://raw.githubusercontent.com/blawar/titledb/master/US.en.json";

/// File name of the nscb_rust backend for the current platform.
pub fn nscb_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "nscb_rust.exe"
    } else if cfg!(target_os = "macos") {
        "nscb_rust-macos-arm64"
    } else {
        "nscb_rust-linux-amd64"
    }
}

/// Working directory for backend processes: the project root in debug builds,
/// the executable's directory in release builds.
pub fn app_root_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

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

    pub fn backend_path(&self) -> PathBuf {
        self.dir.join(nscb_binary_name())
    }

    pub fn has_keys(&self) -> bool {
        self.keys_path().exists() || self.dir.join("keys.txt").exists()
    }

    pub fn has_backend(&self) -> bool {
        self.backend_path().exists()
    }

    pub fn import_keys(&self, src: &Path) -> Result<(), String> {
        fs::copy(src, self.keys_path()).map_err(|e| format!("Failed to copy prod.keys: {e}"))?;
        Ok(())
    }

    /// Copy a user-selected backend binary into the tools dir.
    pub fn import_backend(&self, src: &Path) -> Result<(), String> {
        if !src.exists() {
            return Err("Selected file does not exist".into());
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !name.starts_with("nscb_rust") {
            return Err("Please select an nscb_rust backend binary".into());
        }
        let dst = self.backend_path();
        fs::copy(src, &dst).map_err(|e| format!("Failed to copy backend binary: {e}"))?;
        make_executable(&dst);
        Ok(())
    }

    /// Write downloaded backend bytes into the tools dir.
    pub fn install_backend(&self, bytes: &[u8]) -> Result<(), String> {
        let dst = self.backend_path();
        fs::write(&dst, bytes).map_err(|e| format!("Failed to save backend binary: {e}"))?;
        make_executable(&dst);
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

    /// Installed backend version, migrating the legacy `version.txt` if present.
    pub fn backend_version(&self) -> String {
        let v = self.setting(SETTING_BACKEND_VERSION);
        if !v.is_empty() {
            return v;
        }
        let legacy = self.dir.join("version.txt");
        if let Ok(text) = fs::read_to_string(&legacy) {
            let v = text.trim().to_string();
            if !v.is_empty() {
                let _ = self.save_setting(SETTING_BACKEND_VERSION, &v);
            }
            let _ = fs::remove_file(&legacy);
            return v;
        }
        String::new()
    }

    pub fn save_backend_version(&self, version: &str) -> Result<(), String> {
        self.save_setting(SETTING_BACKEND_VERSION, version)
    }
}

fn make_executable(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(_path, fs::Permissions::from_mode(0o755));
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
    fn backend_version_migrates_legacy_file() {
        let tools = temp_tools("version");
        fs::write(tools.dir().join("version.txt"), "v1.2.3\n").unwrap();
        assert_eq!(tools.backend_version(), "v1.2.3");
        assert!(!tools.dir().join("version.txt").exists());
        assert_eq!(tools.setting(SETTING_BACKEND_VERSION), "v1.2.3");
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
    fn import_backend_rejects_unrelated_binaries() {
        let tools = temp_tools("import");
        let other = tools.dir().join("something.exe");
        fs::write(&other, "x").unwrap();
        assert!(tools.import_backend(&other).is_err());
        let ok = tools.dir().join("nscb_rust-download");
        fs::write(&ok, "bin").unwrap();
        tools.import_backend(&ok).unwrap();
        assert!(tools.has_backend());
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
