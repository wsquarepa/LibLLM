use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

fn timestamp_compact() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{secs}")
}

fn collect_backup_files(data_dir: &Path) -> Vec<PathBuf> {
    let subdirs = ["sessions", "characters", "worldinfo", "system", "personas"];
    let mut files: Vec<PathBuf> = Vec::new();

    for subdir in &subdirs {
        let dir = data_dir.join(subdir);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    files.push(path);
                }
            }
        }
    }

    let index_meta = data_dir.join("index.meta");
    if index_meta.is_file() {
        files.push(index_meta);
    }

    let salt = data_dir.join(".salt");
    if salt.is_file() {
        files.push(salt);
    }

    let key_check = data_dir.join(".key_check");
    if key_check.is_file() {
        files.push(key_check);
    }

    files
}

pub fn create_backup(data_dir: &Path) -> Result<PathBuf> {
    let archive_name = format!("backup-{}.7z", timestamp_compact());
    let archive_path = data_dir.join(&archive_name);

    let files = collect_backup_files(data_dir);
    if files.is_empty() {
        anyhow::bail!("no files to back up");
    }

    let dest = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&archive_path)
        .with_context(|| format!("failed to create archive: {}", archive_path.display()))?;

    let mut encoder = sevenz_rust::SevenZWriter::new(dest)
        .context("failed to initialize 7z writer")?;

    for file_path in &files {
        let relative = file_path
            .strip_prefix(data_dir)
            .unwrap_or(file_path);
        let entry_name = relative.to_string_lossy().to_string();
        let content = std::fs::read(file_path)
            .with_context(|| format!("failed to read file for backup: {}", file_path.display()))?;

        let entry = sevenz_rust::SevenZArchiveEntry::from_path(
            &std::path::PathBuf::from(&entry_name),
            entry_name.clone(),
        );
        encoder
            .push_archive_entry(entry, Some(content.as_slice()))
            .with_context(|| format!("failed to add {} to archive", entry_name))?;
    }

    encoder.finish().context("failed to finalize 7z archive")?;

    Ok(archive_path)
}
