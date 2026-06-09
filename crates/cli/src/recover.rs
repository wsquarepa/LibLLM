//! Database backup recovery subcommands: list, verify, restore, rebuild-index.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::CommandFactory;

use libllm_backup::index::{BackupType, load_index, open_index};
use libllm_backup::restore::restore_to_point;
use libllm_backup::snapshot::rebuild_index;
use libllm_backup::verify::verify_chain;

use crate::cli::RecoverCommand;
use crate::time::format_relative;

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

pub fn run(data_dir: &Path, passkey: Option<&str>, command: Option<&RecoverCommand>) -> Result<()> {
    run_with_interactivity(
        data_dir,
        passkey,
        command,
        crate::interactive::is_interactive(),
    )
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
    tracing::info!(phase = "start", subcommand = subcommand, data_dir = %data_dir.display(), has_passkey = passkey.is_some(), interactive = interactive, "recover.run");
    match command {
        Some(RecoverCommand::List) => cmd_list(data_dir, passkey),
        Some(RecoverCommand::Verify {
            full,
            archived_passkey,
        }) => cmd_verify(data_dir, passkey, archived_passkey.as_deref(), *full),
        Some(RecoverCommand::Restore {
            id,
            yes,
            archived_passkey,
        }) => cmd_restore(data_dir, passkey, id, *yes, archived_passkey.as_deref()),
        Some(RecoverCommand::RebuildIndex) => cmd_rebuild_index(data_dir, passkey, interactive),
        None if interactive => run_interactive_menu(data_dir, passkey),
        None => print_recover_help(),
    }
}

fn print_recover_help() -> Result<()> {
    let mut root = crate::cli::Args::command();
    let recover_cmd = root
        .find_subcommand_mut("recover")
        .context("clap schema missing `recover` subcommand")?;
    recover_cmd
        .print_long_help()
        .context("failed to print help")?;
    Ok(())
}

fn run_interactive_menu(data_dir: &Path, passkey: Option<&str>) -> Result<()> {
    tracing::debug!(phase = "start", "recover.interactive");
    const ITEMS: &[&str] = &[
        "Restore from backup",
        "Verify backups",
        "Verify backups (full content check)",
        "Rebuild backup index",
        "Quit",
    ];

    loop {
        let choice = crate::interactive::select("What would you like to do?", ITEMS)?;
        let Some(index) = choice else {
            tracing::debug!(phase = "exit", reason = "cancelled", "recover.interactive");
            return Ok(());
        };

        tracing::debug!(
            phase = "action_selected",
            action = ITEMS[index],
            "recover.interactive"
        );

        match index {
            0 => {
                if let Err(err) = interactive_restore(data_dir, passkey) {
                    eprintln!("error: {err}");
                }
            }
            1 => {
                if let Err(err) = cmd_verify(data_dir, passkey, None, false) {
                    eprintln!("error: {err}");
                }
            }
            2 => {
                if let Err(err) = cmd_verify(data_dir, passkey, None, true) {
                    eprintln!("error: {err}");
                }
            }
            3 => {
                if let Err(err) = cmd_rebuild_index(data_dir, passkey, true) {
                    eprintln!("error: {err}");
                }
            }
            4 => {
                tracing::debug!(phase = "exit", reason = "quit", "recover.interactive");
                return Ok(());
            }
            _ => unreachable!("select returned an out-of-range index"),
        }

        println!();
    }
}

fn interactive_restore(data_dir: &Path, passkey: Option<&str>) -> Result<()> {
    let index_path = data_dir.join("backups").join("index.json");
    let backup_key = libllm_backup::crypto::resolve_backup_key(data_dir, passkey)?;
    let index = open_index(&index_path, backup_key.as_ref())?;

    if index.entries.is_empty() {
        println!("No backup points found.");
        return Ok(());
    }

    let now = chrono::Utc::now();
    let current_fp = backup_key
        .as_ref()
        .map(libllm_backup::crypto::compute_kek_fingerprint);

    let display_order: Vec<usize> = (0..index.entries.len()).rev().collect();
    let rows: Vec<String> = display_order
        .iter()
        .map(|&i| {
            let entry = &index.entries[i];
            let time_col = format_relative(now, entry.created_at);
            let type_col = match entry.entry_type {
                libllm_backup::index::BackupType::Base => "Base",
                libllm_backup::index::BackupType::Diff => "Diff",
            };
            let size_col = format!("{:>8}", format_size(entry.plaintext_size));
            let status_col = if backup_key.is_none() {
                String::new()
            } else {
                let root = chain_root_for(&index, entry);
                match &root.kek_fingerprint {
                    Some(libllm_backup::index::FingerprintField::Known(fp))
                        if Some(fp) == current_fp.as_ref() =>
                    {
                        "current".to_string()
                    }
                    Some(libllm_backup::index::FingerprintField::Known(fp)) => {
                        format!("archived  fp:{}", &fp[..8.min(fp.len())])
                    }
                    Some(libllm_backup::index::FingerprintField::Unknown) => {
                        "archived  fp:unknown".to_string()
                    }
                    None => "archived  fp:unknown".to_string(),
                }
            };
            if status_col.is_empty() {
                format!("{time_col}  {type_col}  {size_col}")
            } else {
                format!("{time_col}  {type_col}  {size_col}  {status_col}")
            }
        })
        .collect();

    let chosen = crate::interactive::arrow_select("Select a backup to restore", &rows, 0)?;
    let Some(chosen) = chosen else {
        return Ok(());
    };

    let entry = &index.entries[display_order[chosen]];
    let root = chain_root_for(&index, entry);
    let status_detail = match &root.kek_fingerprint {
        Some(libllm_backup::index::FingerprintField::Known(fp))
            if Some(fp) == current_fp.as_ref() =>
        {
            "current passkey".to_string()
        }
        Some(libllm_backup::index::FingerprintField::Known(fp)) => {
            format!(
                "ARCHIVED -- passkey fingerprint: {fp}\n\
                 Restore will refuse unless you re-run non-interactively:\n\
                 libllm recover restore {} --archived-passkey <the-archived-passkey>",
                entry.id
            )
        }
        Some(libllm_backup::index::FingerprintField::Unknown) => format!(
            "ARCHIVED -- fingerprint unknown (rebuilt from a foreign backup).\n\
             Restore will refuse unless you re-run non-interactively:\n\
             libllm recover restore {} --archived-passkey <the-archived-passkey>",
            entry.id
        ),
        None => "unencrypted chain".to_string(),
    };

    println!();
    println!("Backup ID:        {}", entry.id);
    println!("Chain root:       {}", root.id);
    println!("Type:             {:?}", entry.entry_type);
    println!("Plaintext size:   {}", format_size(entry.plaintext_size));
    println!("Stored size:      {}", format_size(entry.stored_size));
    println!(
        "Created at (UTC): {}",
        entry.created_at.format("%Y-%m-%d %H:%M:%S")
    );
    println!("Status:           {status_detail}");
    println!();

    let archived = matches!(
        &root.kek_fingerprint,
        Some(libllm_backup::index::FingerprintField::Known(fp)) if Some(fp) != current_fp.as_ref()
    ) || matches!(
        &root.kek_fingerprint,
        Some(libllm_backup::index::FingerprintField::Unknown)
    );

    if archived {
        println!(
            "This chain is archived. Re-run non-interactively with \
             --archived-passkey to restore."
        );
        return Ok(());
    }

    let prompt = format!(
        "Restore to '{}'? This overwrites the live database.",
        entry.id
    );
    let Some(true) = crate::interactive::confirm(&prompt, false)? else {
        println!("Cancelled.");
        return Ok(());
    };

    let entry_type_str = entry.entry_type.to_string();
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "recover.restore",
        id = entry.id.as_str(),
        entry_type = entry_type_str.as_str(),
        plaintext_size = entry.plaintext_size,
        stored_size = entry.stored_size,
        encrypted = entry.encrypted,
        source = "interactive" ;
        { restore_to_point(data_dir, &entry.id, passkey, None)}
    )?;

    println!("Restore to '{}' completed successfully.", entry.id);
    Ok(())
}

fn chain_root_for<'a>(
    index: &'a libllm_backup::index::BackupIndex,
    entry: &'a libllm_backup::index::BackupEntry,
) -> &'a libllm_backup::index::BackupEntry {
    if entry.entry_type == libllm_backup::index::BackupType::Base {
        return entry;
    }
    entry
        .base_id
        .as_deref()
        .and_then(|bid| index.find_entry(bid))
        .unwrap_or(entry)
}

fn cmd_list(data_dir: &Path, passkey: Option<&str>) -> Result<()> {
    let index_path = data_dir.join("backups").join("index.json");
    let kek = libllm_backup::crypto::resolve_backup_key(data_dir, passkey)?;
    let index = open_index(&index_path, kek.as_ref())?;
    tracing::info!(
        result = "ok",
        entry_count = index.entries.len(),
        "recover.list"
    );

    if index.entries.is_empty() {
        println!("No backup points found.");
        return Ok(());
    }

    let current_fp = kek
        .as_ref()
        .map(libllm_backup::crypto::compute_kek_fingerprint);

    println!(
        "{:<20} {:<6} {:<12} {:<12} {:<10} {:<26} Status",
        "ID", "Type", "Plain Size", "Stored Size", "Encrypted", "Created"
    );
    println!("{}", "-".repeat(100));

    for entry in index.entries.iter().rev() {
        let root = chain_root_for(&index, entry);
        let status = match &root.kek_fingerprint {
            Some(libllm_backup::index::FingerprintField::Known(fp))
                if Some(fp) == current_fp.as_ref() =>
            {
                "current".to_string()
            }
            Some(libllm_backup::index::FingerprintField::Known(fp)) => format!("archived {fp}"),
            Some(libllm_backup::index::FingerprintField::Unknown) => "archived unknown".to_string(),
            None => "n/a".to_string(),
        };
        println!(
            "{:<20} {:<6} {:<12} {:<12} {:<10} {:<26} {}",
            entry.id,
            entry.entry_type,
            format_size(entry.plaintext_size),
            format_size(entry.stored_size),
            if entry.encrypted { "yes" } else { "no" },
            entry.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            status,
        );
    }

    Ok(())
}

fn cmd_verify(
    data_dir: &Path,
    passkey: Option<&str>,
    archived_passkey: Option<&str>,
    full: bool,
) -> Result<()> {
    let result = libllm_core::timed_result!(
        tracing::Level::INFO,
        "recover.verify",
        full = full ;
        { verify_chain(data_dir, passkey, archived_passkey, full)}
    )?;
    tracing::info!(
        phase = "summary",
        checked_count = result.checked_count,
        error_count = result.errors.len(),
        result = if result.errors.is_empty() {
            "ok"
        } else {
            "error"
        },
        "recover.verify"
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

fn cmd_restore(
    data_dir: &Path,
    passkey: Option<&str>,
    id: &str,
    yes: bool,
    archived_passkey: Option<&str>,
) -> Result<()> {
    let index_path = data_dir.join("backups").join("index.json");
    let kek = libllm_backup::crypto::resolve_backup_key(data_dir, passkey)?;
    let index = open_index(&index_path, kek.as_ref())?;

    let entry = index
        .find_entry(id)
        .with_context(|| format!("backup id not found: {id}"))?;

    let current_fp = kek
        .as_ref()
        .map(libllm_backup::crypto::compute_kek_fingerprint);
    let root = chain_root_for(&index, entry);
    let status = match &root.kek_fingerprint {
        Some(libllm_backup::index::FingerprintField::Known(fp))
            if Some(fp) == current_fp.as_ref() =>
        {
            "current".to_string()
        }
        Some(libllm_backup::index::FingerprintField::Known(fp)) => format!("archived {fp}"),
        Some(libllm_backup::index::FingerprintField::Unknown) => "archived unknown".to_string(),
        None => "n/a".to_string(),
    };

    println!("Restore target:");
    println!("  ID:          {}", entry.id);
    println!("  Type:        {}", entry.entry_type);
    println!("  Plain size:  {}", format_size(entry.plaintext_size));
    println!(
        "  Created:     {}",
        entry.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("  Status:      {status}");

    if !yes {
        print!("Continue? [y/N] ");
        io::stdout().flush().context("failed to flush stdout")?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read confirmation")?;

        if input.trim().to_lowercase() != "y" {
            println!("Aborted.");
            tracing::info!(
                phase = "aborted",
                id = id,
                result = "skipped",
                "recover.restore"
            );
            return Ok(());
        }
    }

    let entry_type_str = entry.entry_type.to_string();
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "recover.restore",
        id = id,
        entry_type = entry_type_str.as_str(),
        plaintext_size = entry.plaintext_size,
        stored_size = entry.stored_size,
        encrypted = entry.encrypted ;
        { restore_to_point(data_dir, id, passkey, archived_passkey)}
    )?;
    println!("Restore to '{id}' completed successfully.");
    Ok(())
}

fn cmd_rebuild_index(data_dir: &Path, passkey: Option<&str>, interactive: bool) -> Result<()> {
    libllm_core::timed_result!(tracing::Level::INFO, "recover.rebuild_index", ; {
        let backups_dir = data_dir.join("backups");

        if !backups_dir.exists() {
            bail!("backups directory does not exist: {}", backups_dir.display());
        }

        let (rebuilt, warnings) = libllm_core::timed_result!(
            tracing::Level::INFO,
            "recover.resolve_backup_index",
            has_passkey = passkey.is_some() ;
            { rebuild_index(&backups_dir, passkey) }
        )?;

        for warning in &warnings {
            eprintln!("Warning: {warning}");
        }

        let base_count = rebuilt
            .entries
            .iter()
            .filter(|e| matches!(e.entry_type, BackupType::Base))
            .count();
        let diff_count = rebuilt
            .entries
            .iter()
            .filter(|e| matches!(e.entry_type, BackupType::Diff))
            .count();
        let encrypted_any = rebuilt.entries.iter().any(|e| e.encrypted);
        tracing::info!(
            phase = "summary",
            file_count = rebuilt.entries.len(),
            base_count = base_count,
            diff_count = diff_count,
            encrypted = encrypted_any,
            "recover.rebuild_index"
        );
        let index_path = backups_dir.join("index.json");
        if index_path.exists() {
            let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S.%3fZ");
            let backup_path = backups_dir.join(format!("index.json.{stamp}.bak"));
            std::fs::copy(&index_path, &backup_path).with_context(|| {
                format!("failed to back up existing index to {}", backup_path.display())
            })?;
            tracing::info!(
                phase = "backup",
                backup_path = %backup_path.display(),
                "recover.rebuild_index"
            );

            let existing = load_index(&index_path)?;
            if !existing.entries.is_empty() && rebuilt.entries.len() < existing.entries.len() {
                let proceed = if interactive {
                    crate::interactive::confirm(
                        &format!(
                            "Existing index has {} entries; rebuild found {}. Replace anyway?",
                            existing.entries.len(),
                            rebuilt.entries.len()
                        ),
                        false,
                    )?
                    .unwrap_or(false)
                } else {
                    false
                };
                if !proceed {
                    tracing::warn!(
                        phase = "refused",
                        existing_count = existing.entries.len(),
                        rebuilt_count = rebuilt.entries.len(),
                        "recover.rebuild_index"
                    );
                    bail!(
                        "refusing to overwrite index ({} entries) with fewer ({}); a backup was saved to {}",
                        existing.entries.len(),
                        rebuilt.entries.len(),
                        backup_path.display()
                    );
                }
            }
        }
        libllm_backup::index::save_index(&index_path, &rebuilt)?;

        println!("Rebuilt index with {} entry/entries.", rebuilt.entries.len());
        Ok(())
    })
}
