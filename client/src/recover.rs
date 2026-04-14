use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use backup::index::{BackupEntry, BackupIndex, BackupType, load_index, parse_backup_filename, save_index};
use backup::verify::verify_chain;
use backup::restore::restore_to_point;

use crate::cli::RecoverCommand;

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn backup_type_label(entry_type: BackupType) -> &'static str {
    match entry_type {
        BackupType::Base => "base",
        BackupType::Diff => "diff",
    }
}

pub fn run(data_dir: &Path, passkey: Option<&str>, command: &RecoverCommand) -> Result<()> {
    match command {
        RecoverCommand::List => cmd_list(data_dir),
        RecoverCommand::Verify { full } => cmd_verify(data_dir, passkey, *full),
        RecoverCommand::Restore { id, yes } => cmd_restore(data_dir, passkey, id, *yes),
        RecoverCommand::RebuildIndex => cmd_rebuild_index(data_dir, passkey),
    }
}

fn cmd_list(data_dir: &Path) -> Result<()> {
    let index_path = data_dir.join("backups").join("index.json");
    let index = load_index(&index_path)?;

    if index.entries.is_empty() {
        println!("No backup points found.");
        return Ok(());
    }

    println!(
        "{:<20} {:<6} {:<12} {:<12} {:<10} {}",
        "ID", "Type", "Plain Size", "Stored Size", "Encrypted", "Created"
    );
    println!("{}", "-".repeat(80));

    for entry in &index.entries {
        println!(
            "{:<20} {:<6} {:<12} {:<12} {:<10} {}",
            entry.id,
            backup_type_label(entry.entry_type),
            format_size(entry.plaintext_size),
            format_size(entry.stored_size),
            if entry.encrypted { "yes" } else { "no" },
            entry.created_at.format("%Y-%m-%d %H:%M:%S UTC"),
        );
    }

    Ok(())
}

fn cmd_verify(data_dir: &Path, passkey: Option<&str>, full: bool) -> Result<()> {
    let result = verify_chain(data_dir, passkey, full)?;

    println!("Checked {} backup(s).", result.checked_count);

    if result.errors.is_empty() {
        println!("All checks passed.");
        Ok(())
    } else {
        for error in &result.errors {
            eprintln!("error: {error}");
        }
        std::process::exit(1);
    }
}

fn cmd_restore(data_dir: &Path, passkey: Option<&str>, id: &str, yes: bool) -> Result<()> {
    let index_path = data_dir.join("backups").join("index.json");
    let index = load_index(&index_path)?;

    let entry = index
        .find_entry(id)
        .with_context(|| format!("backup id not found: {id}"))?;

    println!("Restore target:");
    println!("  ID:          {}", entry.id);
    println!("  Type:        {}", backup_type_label(entry.entry_type));
    println!("  Plain size:  {}", format_size(entry.plaintext_size));
    println!("  Created:     {}", entry.created_at.format("%Y-%m-%d %H:%M:%S UTC"));

    if !yes {
        print!("Continue? [y/N] ");
        io::stdout().flush().context("failed to flush stdout")?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read confirmation")?;

        if input.trim().to_lowercase() != "y" {
            println!("Aborted.");
            return Ok(());
        }
    }

    restore_to_point(data_dir, id, passkey)?;
    println!("Restore to '{id}' completed successfully.");
    Ok(())
}

fn cmd_rebuild_index(data_dir: &Path, passkey: Option<&str>) -> Result<()> {
    let backups_dir = data_dir.join("backups");

    if !backups_dir.exists() {
        bail!("backups directory does not exist: {}", backups_dir.display());
    }

    let backup_key: Option<[u8; 32]> = match passkey {
        Some(pk) => {
            let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt"))?;
            Some(backup::crypto::derive_backup_key(pk, &salt)?)
        }
        None => None,
    };

    struct FileInfo {
        filename: String,
        id: String,
        entry_type: BackupType,
        mtime: std::time::SystemTime,
    }

    let mut files: Vec<FileInfo> = std::fs::read_dir(&backups_dir)
        .with_context(|| format!("failed to read backups dir: {}", backups_dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let filename = entry.file_name().to_string_lossy().to_string();
            let (id, entry_type) = parse_backup_filename(&filename)?;
            let mtime = entry.metadata().ok()?.modified().ok()?;
            Some(FileInfo { filename, id, entry_type, mtime })
        })
        .collect();

    files.sort_by_key(|f| f.mtime);

    let mut entries: Vec<BackupEntry> = Vec::new();

    for file in &files {
        let file_path = backups_dir.join(&file.filename);
        let stored_bytes = std::fs::read(&file_path)
            .with_context(|| format!("failed to read backup file: {}", file_path.display()))?;

        let stored_size = stored_bytes.len() as u64;
        let file_hash = backup::hash::hash_bytes(&stored_bytes);
        let encrypted = backup_key.is_some();

        let created_at = chrono::DateTime::from(file.mtime);

        let (plaintext_hash, plaintext_size) = match file.entry_type {
            BackupType::Base => {
                let decrypted = match &backup_key {
                    Some(key) => backup::crypto::decrypt_payload(&stored_bytes, key)
                        .with_context(|| format!("failed to decrypt base file: {}", file.filename))?,
                    None => stored_bytes.clone(),
                };
                let decompressed = backup::diff::decompress(&decrypted)
                    .with_context(|| format!("failed to decompress base file: {}", file.filename))?;
                let hash = backup::hash::hash_bytes(&decompressed);
                let size = decompressed.len() as u64;
                (hash, size)
            }
            BackupType::Diff => (String::new(), 0u64),
        };

        let base_id: Option<String> = match file.entry_type {
            BackupType::Base => None,
            BackupType::Diff => entries
                .iter()
                .rev()
                .find(|e| e.entry_type == BackupType::Base)
                .map(|e| e.id.clone()),
        };

        entries.push(BackupEntry {
            id: file.id.clone(),
            entry_type: file.entry_type,
            filename: file.filename.clone(),
            base_id,
            plaintext_hash,
            file_hash,
            plaintext_size,
            stored_size,
            encrypted,
            created_at,
        });
    }

    let rebuilt = BackupIndex { version: 1, entries };
    let index_path = backups_dir.join("index.json");
    save_index(&index_path, &rebuilt)?;

    println!("Rebuilt index with {} entry/entries.", rebuilt.entries.len());
    Ok(())
}
