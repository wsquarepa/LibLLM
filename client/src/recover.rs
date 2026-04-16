//! Database backup recovery subcommands: list, verify, restore, rebuild-index.

use std::borrow::Cow;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use libllm::debug_log;

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

pub fn run(
    data_dir: &Path,
    passkey: Option<&str>,
    command: Option<&RecoverCommand>,
) -> Result<()> {
    run_with_interactivity(data_dir, passkey, command, crate::interactive::is_interactive())
}

pub fn run_with_interactivity(
    data_dir: &Path,
    passkey: Option<&str>,
    command: Option<&RecoverCommand>,
    interactive: bool,
) -> Result<()> {
    let subcommand = match command {
        Some(RecoverCommand::List) => "list",
        Some(RecoverCommand::Verify { .. }) => "verify",
        Some(RecoverCommand::Restore { .. }) => "restore",
        Some(RecoverCommand::RebuildIndex) => "rebuild_index",
        None if interactive => "interactive",
        None => "help",
    };
    debug_log::log_kv(
        "recover.run",
        &[
            debug_log::field("phase", "start"),
            debug_log::field("subcommand", subcommand),
            debug_log::field("data_dir", data_dir.display()),
            debug_log::field("has_passkey", passkey.is_some()),
            debug_log::field("interactive", interactive),
        ],
    );
    match command {
        Some(RecoverCommand::List) => cmd_list(data_dir),
        Some(RecoverCommand::Verify { full }) => cmd_verify(data_dir, passkey, *full),
        Some(RecoverCommand::Restore { id, yes }) => cmd_restore(data_dir, passkey, id, *yes),
        Some(RecoverCommand::RebuildIndex) => cmd_rebuild_index(data_dir, passkey),
        None if interactive => run_interactive_menu(data_dir, passkey),
        None => print_recover_help(),
    }
}

fn print_recover_help() -> Result<()> {
    use clap::CommandFactory;
    let mut root = crate::cli::Args::command();
    let recover_cmd = root
        .find_subcommand_mut("recover")
        .context("clap schema missing `recover` subcommand")?;
    recover_cmd.print_long_help().context("failed to print help")?;
    println!();
    Ok(())
}

fn run_interactive_menu(_data_dir: &Path, _passkey: Option<&str>) -> Result<()> {
    anyhow::bail!("interactive recover not yet implemented")
}

fn cmd_list(data_dir: &Path) -> Result<()> {
    let index_path = data_dir.join("backups").join("index.json");
    let index = load_index(&index_path)?;
    debug_log::log_kv(
        "recover.list",
        &[
            debug_log::field("result", "ok"),
            debug_log::field("entry_count", index.entries.len()),
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
    let result = debug_log::timed_result(
        "recover.verify",
        &[debug_log::field("full", full)],
        || verify_chain(data_dir, passkey, full).map_err(anyhow::Error::from),
    )?;
    debug_log::log_kv(
        "recover.verify",
        &[
            debug_log::field("phase", "summary"),
            debug_log::field("checked_count", result.checked_count),
            debug_log::field("error_count", result.errors.len()),
            debug_log::field(
                "result",
                if result.errors.is_empty() { "ok" } else { "error" },
            ),
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
            debug_log::log_kv(
                "recover.restore",
                &[
                    debug_log::field("phase", "aborted"),
                    debug_log::field("id", id),
                    debug_log::field("result", "skipped"),
                ],
            );
            return Ok(());
        }
    }

    debug_log::timed_result(
        "recover.restore",
        &[
            debug_log::field("id", id),
            debug_log::field("entry_type", entry.entry_type.to_string()),
            debug_log::field("plaintext_size", entry.plaintext_size),
            debug_log::field("stored_size", entry.stored_size),
            debug_log::field("encrypted", entry.encrypted),
        ],
        || restore_to_point(data_dir, id, passkey).map_err(anyhow::Error::from),
    )?;
    println!("Restore to '{id}' completed successfully.");
    Ok(())
}

fn cmd_rebuild_index(data_dir: &Path, passkey: Option<&str>) -> Result<()> {
    debug_log::timed_result("recover.rebuild_index", &[], || {
        let backups_dir = data_dir.join("backups");

        if !backups_dir.exists() {
            bail!("backups directory does not exist: {}", backups_dir.display());
        }

        let backup_key = debug_log::timed_result(
            "recover.resolve_backup_key",
            &[debug_log::field("has_passkey", passkey.is_some())],
            || backup::crypto::resolve_backup_key(data_dir, passkey).map_err(anyhow::Error::from),
        )?;

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
                Some(FileInfo { filename, id, entry_type, stored_size, mtime })
            })
            .collect();

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
                            backup::crypto::decrypt_payload(&stored_bytes, key).with_context(
                                || format!("failed to decrypt base file: {}", file.filename),
                            )?,
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

        let base_count = entries
            .iter()
            .filter(|e| matches!(e.entry_type, BackupType::Base))
            .count();
        let diff_count = entries
            .iter()
            .filter(|e| matches!(e.entry_type, BackupType::Diff))
            .count();
        let encrypted_any = entries.iter().any(|e| e.encrypted);
        debug_log::log_kv(
            "recover.rebuild_index",
            &[
                debug_log::field("phase", "summary"),
                debug_log::field("file_count", files.len()),
                debug_log::field("base_count", base_count),
                debug_log::field("diff_count", diff_count),
                debug_log::field("encrypted", encrypted_any),
            ],
        );

        let rebuilt = BackupIndex { version: 1, entries };
        let index_path = backups_dir.join("index.json");
        save_index(&index_path, &rebuilt)?;

        println!("Rebuilt index with {} entry/entries.", rebuilt.entries.len());
        Ok(())
    })
}
