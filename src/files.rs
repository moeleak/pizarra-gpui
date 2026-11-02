//! Small file-system helpers shared by save and export.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// Returns `path` with the given extension, replacing a different one.
pub fn ensure_extension(path: &Path, extension: &str) -> PathBuf {
    match path.extension() {
        Some(ext) if ext.eq_ignore_ascii_case(extension) => path.to_path_buf(),
        _ => path.with_extension(extension),
    }
}

/// Writes `contents` to a temporary file next to `path` and renames it into
/// place, so a failed write never leaves a truncated drawing behind.
pub fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().context("the path has no file name")?;
    let tmp = dir.join(format!(".{}.{}.tmp", name.to_string_lossy(), std::process::id()));

    let result = (|| -> Result<()> {
        let mut file = fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(ensure_extension(Path::new("a/dibujo"), "svg"), PathBuf::from("a/dibujo.svg"));
        assert_eq!(ensure_extension(Path::new("dibujo.png"), "svg"), PathBuf::from("dibujo.svg"));
        assert_eq!(ensure_extension(Path::new("dibujo.SVG"), "svg"), PathBuf::from("dibujo.SVG"));
    }

    #[test]
    fn atomic_write_replaces_contents() {
        let dir = std::env::temp_dir().join(format!("pizarra-gpui-files-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.svg");

        write_atomically(&path, b"one").unwrap();
        write_atomically(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary files left behind");

        fs::remove_dir_all(&dir).unwrap();
    }
}
