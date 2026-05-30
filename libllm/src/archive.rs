//! Tar+zstd snapshot of a directory tree, used by the Danger tab's "Destroy All Data"
//! flow as a recovery escape hatch before we delete the user's data dir.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use walkdir::WalkDir;

const ZSTD_LEVEL: i32 = 3;

/// Walks `data_dir`, packs every file into a tar stream wrapped in zstd, writes to
/// `output_path`. Skips any path whose first component within `data_dir` matches
/// `exclude_subdir`. Returns the number of bytes written to `output_path`.
pub fn snapshot_data_dir(data_dir: &Path, output_path: &Path, exclude_subdir: &str) -> Result<u64> {
    let file = crate::crypto::create_file_restricted(output_path)
        .with_context(|| format!("failed to create snapshot file: {}", output_path.display()))?;
    let zstd_encoder = zstd::Encoder::new(file, ZSTD_LEVEL)
        .context("failed to initialize zstd encoder")?;
    let mut tar = tar::Builder::new(zstd_encoder);
    tar.mode(tar::HeaderMode::Deterministic);

    for entry in WalkDir::new(data_dir).into_iter().filter_entry(|e| {
        let path = e.path();
        match path.strip_prefix(data_dir) {
            Ok(rel) => rel
                .components()
                .next()
                .map(|c| c.as_os_str() != exclude_subdir)
                .unwrap_or(true),
            Err(_) => true,
        }
    }) {
        let entry = entry.context("walkdir failed")?;
        let path = entry.path();
        let rel = path.strip_prefix(data_dir).unwrap_or(path);
        if rel.as_os_str().is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            tar.append_dir(rel, path)
                .with_context(|| format!("failed to append dir to tar: {}", path.display()))?;
        } else if entry.file_type().is_file() {
            let mut f = File::open(path)
                .with_context(|| format!("failed to open file for snapshot: {}", path.display()))?;
            tar.append_file(rel, &mut f)
                .with_context(|| format!("failed to append file to tar: {}", path.display()))?;
        }
    }

    let zstd_encoder = tar.into_inner().context("failed to finalize tar")?;
    let mut file = zstd_encoder.finish().context("failed to finalize zstd")?;
    file.flush().context("failed to flush snapshot file")?;
    let bytes = std::fs::metadata(output_path)
        .with_context(|| format!("failed to stat snapshot file: {}", output_path.display()))?
        .len();
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn read_archive_entries(archive_path: &Path) -> Vec<PathBuf> {
        let f = File::open(archive_path).unwrap();
        let zstd_decoder = zstd::Decoder::new(f).unwrap();
        let mut tar = tar::Archive::new(zstd_decoder);
        tar.entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path().unwrap().into_owned())
            .collect()
    }

    #[test]
    fn snapshot_includes_all_files_outside_excluded_subdir() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"b").unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/c.txt"), b"c").unwrap();
        std::fs::create_dir(dir.path().join("backups")).unwrap();
        std::fs::write(dir.path().join("backups/skip.bin"), b"x").unwrap();

        let archive = dir.path().join("snap.tar.zst");
        let bytes = snapshot_data_dir(dir.path(), &archive, "backups").unwrap();
        assert!(bytes > 0);

        let entries = read_archive_entries(&archive);
        let strs: Vec<String> = entries
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(strs.iter().any(|s| s.ends_with("a.txt")));
        assert!(strs.iter().any(|s| s.ends_with("b.txt")));
        assert!(
            strs.iter()
                .any(|s| s.ends_with("nested/c.txt") || s.ends_with("nested\\c.txt"))
        );
        assert!(
            !strs.iter().any(|s| s.contains("backups")),
            "excluded dir leaked: {strs:?}"
        );
    }

    #[test]
    fn snapshot_returns_bytes_written() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let archive = dir.path().join("snap.tar.zst");
        let bytes = snapshot_data_dir(dir.path(), &archive, "backups").unwrap();
        let stat_bytes = std::fs::metadata(&archive).unwrap().len();
        assert_eq!(bytes, stat_bytes);
    }

    #[test]
    fn snapshot_extracts_to_original_content() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("data.bin"), b"original-bytes").unwrap();
        let archive = dir.path().join("snap.tar.zst");
        snapshot_data_dir(dir.path(), &archive, "backups").unwrap();

        let extract_dir = TempDir::new().unwrap();
        let f = File::open(&archive).unwrap();
        let zstd_decoder = zstd::Decoder::new(f).unwrap();
        let mut tar = tar::Archive::new(zstd_decoder);
        tar.unpack(extract_dir.path()).unwrap();

        let extracted = std::fs::read(extract_dir.path().join("data.bin")).unwrap();
        assert_eq!(&extracted, b"original-bytes");
    }

    #[test]
    fn snapshot_create_new_rejects_existing_path() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let existing = dir.path().join("existing.tar.zst");
        std::fs::write(&existing, b"already here").unwrap();

        let err = snapshot_data_dir(dir.path(), &existing, "backups")
            .expect_err("must refuse to overwrite existing path");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("failed to create snapshot file"),
            "unexpected error: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_output_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("data.bin"), b"content").unwrap();
        let archive = dir.path().join("snap_mode.tar.zst");

        snapshot_data_dir(dir.path(), &archive, "backups").unwrap();

        let mode = std::fs::metadata(&archive).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "snapshot archive must be owner read/write only"
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_create_new_rejects_symlink() {
        use std::os::unix::fs;

        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, b"DO_NOT_TOUCH").unwrap();
        let link = dir.path().join("libllm-attack.tar.zst");
        fs::symlink(&victim, &link).unwrap();

        let err = snapshot_data_dir(dir.path(), &link, "backups")
            .expect_err("must refuse to open a symlink");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("failed to create snapshot file"),
            "unexpected error: {msg}"
        );
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"DO_NOT_TOUCH",
            "victim file must be untouched"
        );
    }
}
