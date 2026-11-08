use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{NscbError, Result};

/// Prepare a user-selected directory for the potentially large temporary
/// files produced by compression, decompression, merge, and rename metadata.
pub fn prepare_dir(path: Option<&str>) -> Result<Option<PathBuf>> {
    let Some(path) = path else {
        return Ok(None);
    };

    if path.trim().is_empty() {
        return Err(NscbError::InvalidData(
            "--temp-dir must not be empty".to_string(),
        ));
    }

    let path = PathBuf::from(path);
    fs::create_dir_all(&path)?;
    if !path.is_dir() {
        return Err(NscbError::InvalidData(format!(
            "Temporary path is not a directory: {}",
            path.display()
        )));
    }

    // Fail before starting an operation if the selected directory is not
    // writable, rather than after processing a large amount of input.
    drop(named_file(Some(&path))?);
    Ok(Some(path))
}

pub fn named_file(dir: Option<&Path>) -> io::Result<tempfile::NamedTempFile> {
    match dir {
        Some(dir) => tempfile::Builder::new().prefix("nscb-").tempfile_in(dir),
        None => tempfile::NamedTempFile::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_nested_custom_directory_and_creates_files_there() {
        let root = tempfile::tempdir().expect("temp root");
        let custom = root.path().join("large").join("temporary-files");
        let prepared = prepare_dir(Some(custom.to_str().expect("UTF-8 path")))
            .expect("prepare custom temp dir")
            .expect("custom path");

        let file = named_file(Some(&prepared)).expect("custom temp file");
        assert_eq!(file.path().parent(), Some(custom.as_path()));
    }
}
