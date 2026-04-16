//! Database backup recovery subcommands: list, verify, restore, rebuild-index.

use std::borrow::Cow;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use backup::index::{
    BackupEntry, BackupIndex, BackupType, load_index, parse_backup_filename, save_index,
};
use backup::restore::restore_to_point;
use backup::verify::verify_chain;

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

pub fn run(data_dir: &Path, passkey: Option<&str>, command: &RecoverCommand) -> Result<()> {
    let command_name = match command {
        RecoverCommand::List => "list",
        RecoverCommand::Verify { .. } => "verify",
        RecoverCommand::Restore { .. } => "restore",
        RecoverCommand::RebuildIndex => "rebuild_index",
    };
    libllm::debug_log::log_kv(
        "recover.run",
        &[
            libllm::debug_log::field("command", command_name),
            libllm::debug_log::field("data_dir", data_dir.display()),
            libllm::debug_log::field("has_passkey", passkey.is_some()),
        ],
    );

    match command {
        RecoverCommand::List => libllm::debug_log::timed_result(
            "recover.phase",
            &[libllm::debug_log::field("phase", "list")],
            || cmd_list(data_dir),
        ),
        RecoverCommand::Verify { full } => libllm::debug_log::timed_result(
            "recover.phase",
            &[
                libllm::debug_log::field("phase", "verify"),
                libllm::debug_log::field("full", *full),
            ],
            || cmd_verify(data_dir, passkey, *full),
        ),
        RecoverCommand::Restore { id, yes } => libllm::debug_log::timed_result(
            "recover.phase",
            &[
                libllm::debug_log::field("phase", "restore"),
                libllm::debug_log::field("id", id),
                libllm::debug_log::field("yes", *yes),
            ],
            || cmd_restore(data_dir, passkey, id, *yes),
        ),
        RecoverCommand::RebuildIndex => libllm::debug_log::timed_result(
            "recover.phase",
            &[libllm::debug_log::field("phase", "rebuild_index")],
            || cmd_rebuild_index(data_dir, passkey),
        ),
    }
}

fn cmd_list(data_dir: &Path) -> Result<()> {
    let index_path = data_dir.join("backups").join("index.json");
    let index = load_index(&index_path)?;
    libllm::debug_log::log_kv(
        "recover.list",
        &[
            libllm::debug_log::field("index_path", index_path.display()),
            libllm::debug_log::field("entry_count", index.entries.len()),
        ],
    );

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
            entry.entry_type,
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
    libllm::debug_log::log_kv(
        "recover.verify",
        &[
            libllm::debug_log::field("full", full),
            libllm::debug_log::field("checked_count", result.checked_count),
            libllm::debug_log::field("error_count", result.errors.len()),
        ],
    );

    println!("Checked {} backup(s).", result.checked_count);

    if result.errors.is_empty() {
        println!("All checks passed.");
        Ok(())
    } else {
        for error in &result.errors {
            eprintln!("error: {error}");
        }
        bail!("verification failed with {} error(s)", result.errors.len());
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
    println!("  Type:        {}", entry.entry_type);
    println!("  Plain size:  {}", format_size(entry.plaintext_size));
    println!(
        "  Created:     {}",
        entry.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    );

    if !yes {
        print!("Continue? [y/N] ");
        io::stdout().flush().context("failed to flush stdout")?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read confirmation")?;

        if input.trim().to_lowercase() != "y" {
            libllm::debug_log::log_kv(
                "recover.restore",
                &[
                    libllm::debug_log::field("id", id),
                    libllm::debug_log::field("result", "aborted"),
                    libllm::debug_log::field("reason", "user_declined"),
                ],
            );
            println!("Aborted.");
            return Ok(());
        }
    }

    restore_to_point(data_dir, id, passkey)?;
    libllm::debug_log::log_kv(
        "recover.restore",
        &[
            libllm::debug_log::field("id", id),
            libllm::debug_log::field("result", "ok"),
        ],
    );
    println!("Restore to '{id}' completed successfully.");
    Ok(())
}

fn cmd_rebuild_index(data_dir: &Path, passkey: Option<&str>) -> Result<()> {
    let backups_dir = data_dir.join("backups");

    if !backups_dir.exists() {
        bail!(
            "backups directory does not exist: {}",
            backups_dir.display()
        );
    }

    let backup_key = backup::crypto::resolve_backup_key(data_dir, passkey)?;

    struct FileInfo {
        filename: String,
        id: String,
        entry_type: BackupType,
        stored_size: u64,
        mtime: std::time::SystemTime,
    }

    let mut files: Vec<FileInfo> = std::fs::read_dir(&backups_dir)
        .with_context(|| format!("failed to read backups dir: {}", backups_dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let filename = entry.file_name().to_string_lossy().to_string();
            let (id, entry_type) = parse_backup_filename(&filename)?;
            let metadata = entry.metadata().ok()?;
            let mtime = metadata.modified().ok()?;
            let stored_size = metadata.len();
            Some(FileInfo {
                filename,
                id,
                entry_type,
                stored_size,
                mtime,
            })
        })
        .collect();
    libllm::debug_log::log_kv(
        "recover.rebuild_index",
        &[
            libllm::debug_log::field("backups_dir", backups_dir.display()),
            libllm::debug_log::field("file_count", files.len()),
            libllm::debug_log::field("encrypted", backup_key.is_some()),
        ],
    );

    files.sort_by_key(|f| f.mtime);

    let mut entries: Vec<BackupEntry> = Vec::new();
    let mut last_base_id: Option<String> = None;

    for file in &files {
        let file_path = backups_dir.join(&file.filename);
        let stored_bytes = std::fs::read(&file_path)
            .with_context(|| format!("failed to read backup file: {}", file_path.display()))?;

        let file_hash = backup::hash::hash_bytes(&stored_bytes);
        let encrypted = backup_key.is_some();

        let created_at = chrono::DateTime::from(file.mtime);

        let (plaintext_hash, plaintext_size) = match file.entry_type {
            BackupType::Base => {
                let decrypted: Cow<[u8]> = match &backup_key {
                    Some(key) => Cow::Owned(
                        backup::crypto::decrypt_payload(&stored_bytes, key).with_context(|| {
                            format!("failed to decrypt base file: {}", file.filename)
                        })?,
                    ),
                    None => Cow::Borrowed(&stored_bytes),
                };
                let decompressed = backup::diff::decompress(&decrypted).with_context(|| {
                    format!("failed to decompress base file: {}", file.filename)
                })?;
                let hash = backup::hash::hash_bytes(&decompressed);
                let size = decompressed.len() as u64;
                (hash, size)
            }
            BackupType::Diff => (String::new(), 0u64),
        };

        let base_id: Option<String> = match file.entry_type {
            BackupType::Base => {
                last_base_id = Some(file.id.clone());
                None
            }
            BackupType::Diff => last_base_id.clone(),
        };

        entries.push(BackupEntry {
            id: file.id.clone(),
            entry_type: file.entry_type,
            filename: file.filename.clone(),
            base_id,
            plaintext_hash,
            file_hash,
            plaintext_size,
            stored_size: file.stored_size,
            encrypted,
            created_at,
        });
    }

    let rebuilt = BackupIndex {
        version: 1,
        entries,
    };
    let index_path = backups_dir.join("index.json");
    save_index(&index_path, &rebuilt)?;
    libllm::debug_log::log_kv(
        "recover.rebuild_index",
        &[
            libllm::debug_log::field("index_path", index_path.display()),
            libllm::debug_log::field("entry_count", rebuilt.entries.len()),
            libllm::debug_log::field("result", "ok"),
        ],
    );

    println!(
        "Rebuilt index with {} entry/entries.",
        rebuilt.entries.len()
    );
    Ok(())
}
