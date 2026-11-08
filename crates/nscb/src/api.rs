//! Typed library API for embedding nscb.
//!
//! [`Nscb`] exposes each operation as a method that takes typed options and returns
//! typed results (output paths, rename outcomes, container metadata, verification
//! findings). Progress and cancellation go through [`crate::util::report`].

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{NscbError, Result};
use crate::keys::KeyStore;
use crate::nutdb::{base_title_id, NutdbStore};

pub use crate::cli::{DlcRenameMode, NoVersionMode, RenameMode, RenameOptions, RenameOutcome, RenameSkip};
pub use crate::nutdb::RefreshStatus;
pub use crate::ops::info::{ContainerFormat, ContainerInfo, ContentInfo, ContentKind, TitleInfo, TitleKind};
pub use crate::ops::verify::{
    FindingStatus, VerifyEvent, VerifyFinding, VerifyLevel, VerifyReport, VerifyStage, VerifyStageKind,
};

/// Container format for outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Nsp,
    Xci,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Nsp => "nsp",
            Format::Xci => "xci",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOptions {
    pub format: Format,
    /// Skip delta fragment NCAs.
    pub exclude_deltas: bool,
    /// Cap RequiredSystemVersion.
    pub rsv_cap: Option<u32>,
    /// Lower NCA key generation.
    pub key_generation: Option<u8>,
    /// Log before/after firmware info.
    pub log_firmware: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NutdbRefresh {
    pub status: RefreshStatus,
    pub indexed_titles: usize,
    pub cache_dir: PathBuf,
}

/// A NUTDB entry for one title ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleLookup {
    pub title_id: String,
    pub base_title_id: String,
    pub display_name: Option<String>,
    pub name: Option<String>,
    pub publisher: Option<String>,
    pub languages: Vec<String>,
    pub version: Option<u64>,
}

/// Settings shared by every operation.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// `prod.keys`/`keys.txt`; when unset, nscb searches its default locations.
    pub keys_path: Option<PathBuf>,
    /// Directory for large temporary files (OS default when unset).
    pub temp_dir: Option<PathBuf>,
    pub nutdb_url: Option<String>,
    pub nutdb_cache_dir: Option<PathBuf>,
}

pub struct Nscb {
    config: Config,
    nutdb: NutdbStore,
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn is_nup_path(path: &Path) -> bool {
    crate::formats::nup::has_nup_extension(&path.to_string_lossy())
}

fn dir_entries(dir: &Path) -> HashSet<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default()
}

/// Paths that appeared in `dir` since `before` was taken, sorted.
fn new_entries(dir: &Path, before: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut added: Vec<PathBuf> = dir_entries(dir).difference(before).cloned().collect();
    added.sort();
    added
}

impl Nscb {
    pub fn new(config: Config) -> Self {
        let nutdb = NutdbStore::new(
            config.nutdb_cache_dir.as_deref().map(|p| p.to_str().unwrap_or_default()),
            config.nutdb_url.as_deref(),
        );
        Self { config, nutdb }
    }

    fn keys(&self) -> Result<KeyStore> {
        KeyStore::from_default_locations(self.config.keys_path.as_deref().and_then(Path::to_str))
    }

    fn temp_dir(&self) -> Result<Option<PathBuf>> {
        crate::util::temp::prepare_dir(self.config.temp_dir.as_deref().and_then(Path::to_str))
    }

    fn output_path(out_dir: &Path, default_name: &str) -> PathBuf {
        PathBuf::from(crate::cli::make_output_path(
            "",
            &Some(path_str(out_dir)),
            default_name,
        ))
    }

    /// NSP→NSZ or XCI→XCZ. Returns the output file.
    pub fn compress(&self, input: &Path, out_dir: &Path, level: i32) -> Result<PathBuf> {
        let ks = self.keys()?;
        let temp = self.temp_dir()?;
        let input = path_str(input);
        let output = Self::output_path(out_dir, &crate::cli::compress_ext(&input));
        crate::ops::compress::compress_with_temp_dir(&input, &path_str(&output), level, &ks, temp.as_deref())?;
        Ok(output)
    }

    /// NSZ→NSP, XCZ→XCI or NCZ→NCA. Returns the output file.
    pub fn decompress(&self, input: &Path, out_dir: &Path) -> Result<PathBuf> {
        let temp = self.temp_dir()?;
        let input = path_str(input);
        let output = Self::output_path(out_dir, &crate::cli::decompress_ext(&input));
        crate::ops::decompress::decompress_with_temp_dir(&input, &path_str(&output), temp.as_deref())?;
        Ok(output)
    }

    /// NSP↔XCI. Returns the output file.
    pub fn convert(&self, input: &Path, out_dir: &Path, format: Format) -> Result<PathBuf> {
        let ks = self.keys()?;
        let input = path_str(input);
        let output = Self::output_path(out_dir, &crate::cli::change_ext(&input, format.extension()));
        crate::ops::convert::convert(&input, &path_str(&output), format.extension(), &ks)?;
        Ok(output)
    }

    /// Merge base, update and DLC files into one container. Returns the output file.
    pub fn merge(&self, inputs: &[PathBuf], out_dir: &Path, options: &MergeOptions) -> Result<PathBuf> {
        let ks = self.keys()?;
        let temp = self.temp_dir()?;
        let inputs: Vec<String> = inputs
            .iter()
            .map(|p| path_str(p))
            .filter(|p| !crate::cli::is_ignored_merge_input_path(p))
            .collect();
        if inputs.is_empty() {
            return Err(NscbError::InvalidData(
                "No valid input files after filtering metadata sidecar entries".into(),
            ));
        }
        let refs: Vec<&str> = inputs.iter().map(String::as_str).collect();
        let format = options.format.extension();
        let output = crate::cli::merge_output_path(
            &refs,
            &Some(path_str(out_dir)),
            format,
            &ks,
            &self.nutdb,
            temp.as_deref(),
        );
        crate::ops::merge::merge_with_temp_dir(
            &refs,
            &output,
            &ks,
            options.exclude_deltas,
            format,
            options.format == Format::Nsp,
            options.rsv_cap,
            options.key_generation,
            options.log_firmware,
            temp.as_deref(),
        )?;
        Ok(PathBuf::from(output))
    }

    /// Split a multi-title container into per-title NCA folders. Returns the new entries
    /// in `out_dir`.
    pub fn split(&self, input: &Path, out_dir: &Path) -> Result<Vec<PathBuf>> {
        let ks = self.keys()?;
        let before = dir_entries(out_dir);
        crate::ops::split::split(&path_str(input), &path_str(out_dir), &ks)?;
        Ok(new_entries(out_dir, &before))
    }

    /// Split a multi-title container into one NSP/XCI per title. Returns the new files.
    pub fn split_to_files(&self, input: &Path, out_dir: &Path, format: Format) -> Result<Vec<PathBuf>> {
        let ks = self.keys()?;
        let before = dir_entries(out_dir);
        crate::ops::dspl::split_to_files(&path_str(input), &path_str(out_dir), format.extension(), &ks)?;
        Ok(new_entries(out_dir, &before))
    }

    /// Repack an NCA folder (as produced by [`Nscb::split`]) into `<folder name>.nsp`
    /// in `out_dir`. Returns the output file.
    pub fn create(&self, input_dir: &Path, out_dir: &Path) -> Result<PathBuf> {
        let ks = self.keys()?;
        let name = input_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| NscbError::InvalidData("Input folder has no name".into()))?;
        let output = out_dir.join(format!("{name}.nsp"));
        crate::ops::create::create_from_folder(&path_str(input_dir), &path_str(&output), &ks)?;
        Ok(output)
    }

    /// Rename a file, or every supported file under a folder, from metadata and NUTDB.
    /// A `.nup` file needs no keys.
    pub fn rename(&self, target: &Path, options: RenameOptions) -> Result<Vec<RenameOutcome>> {
        let ks = match self.keys() {
            Ok(ks) => ks,
            Err(_) if target.is_file() && is_nup_path(target) => KeyStore::from_string("")?,
            Err(e) => return Err(e),
        };
        let temp = self.temp_dir()?;
        let index = self.nutdb.ensure_index()?;
        crate::cli::rename_target(target, &ks, &index, options, temp.as_deref())
    }

    /// Read title metadata from a container. A `.nup` file needs no keys.
    pub fn inspect(&self, input: &Path) -> Result<ContainerInfo> {
        if is_nup_path(input) {
            return crate::ops::info::inspect_nup(input);
        }
        let ks = self.keys()?;
        crate::ops::info::inspect(&path_str(input), &ks)
    }

    /// Verify a container's integrity. For a `.nup` (unencrypted), every level checks
    /// its structure and `VerifyLevel::Full` also every file's SHA-256.
    pub fn verify(&self, input: &Path, level: VerifyLevel) -> Result<VerifyReport> {
        self.verify_with(input, level, &mut |_| {})
    }

    /// Like [`Nscb::verify`], reporting the items to check, each finding and each
    /// completed stage to `on_event` as verification proceeds.
    pub fn verify_with(
        &self,
        input: &Path,
        level: VerifyLevel,
        on_event: &mut dyn FnMut(VerifyEvent),
    ) -> Result<VerifyReport> {
        if is_nup_path(input) {
            return crate::ops::verify_nup::verify_nup_report_with(input, level, on_event);
        }
        let ks = self.keys()?;
        crate::ops::verify::verify_report_with(&path_str(input), &ks, level, on_event)
    }

    /// Refresh the local NUTDB cache.
    pub fn nutdb_refresh(&self) -> Result<NutdbRefresh> {
        let spinner = crate::util::progress::spinner("Refreshing NUTDB");
        let outcome = self.nutdb.refresh();
        spinner.finish_and_clear();
        let outcome = outcome?;
        Ok(NutdbRefresh {
            status: outcome.status,
            indexed_titles: outcome.indexed_titles,
            cache_dir: self.nutdb.cache_dir().to_path_buf(),
        })
    }

    /// Look up a title ID in the NUTDB cache (downloading it if needed).
    pub fn nutdb_lookup(&self, title_id: &str) -> Result<TitleLookup> {
        let index = self.nutdb.ensure_index()?;
        let normalized = title_id.trim().to_ascii_uppercase();
        let display_name = index.display_name_for(title_id);
        let entry = index.lookup(title_id);
        if entry.is_none() && display_name.is_none() {
            return Err(NscbError::InvalidData(format!(
                "No NUTDB entry found for title ID {title_id}"
            )));
        }
        Ok(TitleLookup {
            base_title_id: base_title_id(title_id),
            title_id: normalized,
            display_name,
            name: entry.and_then(|t| t.name.clone()),
            publisher: entry.and_then(|t| t.publisher.clone()),
            languages: entry.map(|t| t.languages.clone()).unwrap_or_default(),
            version: entry.and_then(|t| t.version),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nscb-api-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn output_paths_follow_cli_naming() {
        let out = Path::new("/out");
        assert_eq!(Nscb::output_path(out, &crate::cli::compress_ext("/g/Game [v0].nsp")), Path::new("/out/Game [v0].nsz"));
        assert_eq!(Nscb::output_path(out, &crate::cli::decompress_ext("/g/Game.xcz")), Path::new("/out/Game.xci"));
        assert_eq!(Nscb::output_path(out, &crate::cli::change_ext("/g/Game.nsp", "xci")), Path::new("/out/Game.xci"));
    }

    #[test]
    fn missing_keys_are_an_error() {
        let dir = temp("keys");
        let nscb = Nscb::new(Config { keys_path: Some(dir.join("missing.keys")), ..Default::default() });
        assert!(nscb.inspect(&dir.join("game.nsp")).is_err());
    }

    #[test]
    fn nup_needs_no_keys() {
        let dir = temp("nup");
        let path = dir.join("Sample.nup");
        fs::write(&path, crate::formats::nup::tests::sample()).unwrap();
        let nscb = Nscb::new(Config { keys_path: Some(dir.join("missing.keys")), ..Default::default() });
        let info = nscb.inspect(&path).unwrap();
        assert_eq!(info.format, ContainerFormat::Nup);
        let base = &info.titles[0];
        assert_eq!((base.name.as_str(), base.publisher.as_str(), base.kind), ("Sample Game", "Studio", TitleKind::Base));
        assert_eq!(base.build_id.as_deref(), Some("AB".repeat(32).as_str()));
        assert_eq!(base.contents.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["ExeFS (2 files)", "RomFS (2 files)"]);
        // The update inherits the game's name but keeps its own display version.
        assert_eq!((info.titles[1].name.as_str(), info.titles[1].display_version.as_str()), ("Sample Game", "1.0.1"));
        assert_eq!(info.titles[2].kind, TitleKind::Dlc);
        assert_eq!(info.titles[2].base_title_id, "0100000000010000");
        assert!(nscb.verify(&path, VerifyLevel::Full).unwrap().passed);
    }

    #[test]
    fn new_entries_lists_only_added_paths() {
        let dir = temp("entries");
        fs::write(dir.join("old.nsp"), "x").unwrap();
        let before = dir_entries(&dir);
        fs::write(dir.join("b.nsp"), "x").unwrap();
        fs::create_dir(dir.join("a")).unwrap();
        assert_eq!(new_entries(&dir, &before), vec![dir.join("a"), dir.join("b.nsp")]);
    }
}
