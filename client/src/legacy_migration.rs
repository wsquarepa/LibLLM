//! Legacy file-based storage detection and migration utility download.

use anyhow::{Context, Result};
use std::io::{self, IsTerminal, Write};
use libllm::config;
use libllm::debug_log;
use crate::update;

const LEGACY_DIRS: [&str; 5] = ["sessions", "characters", "worldinfo", "system", "personas"];

pub async fn check_and_run_migration(no_encrypt: bool, passkey: Option<&str>) -> Result<()> {
    let data_dir = config::data_dir();
    let db_path = data_dir.join("data.db");
    let db_exists = db_path.exists();

    if db_exists {
        debug_log::log_kv(
            "legacy.migration",
            &[
                debug_log::field("phase", "check"),
                debug_log::field("result", "skipped"),
                debug_log::field("reason", "db_present"),
                debug_log::field("db_exists", true),
            ],
        );
        return Ok(());
    }

    let legacy_dirs_found = LEGACY_DIRS
        .iter()
        .filter(|dir| {
            let path = data_dir.join(dir);
            path.is_dir()
                && std::fs::read_dir(&path)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false)
        })
        .count();
    let has_legacy = legacy_dirs_found > 0;

    debug_log::log_kv(
        "legacy.migration",
        &[
            debug_log::field("phase", "check"),
            debug_log::field("db_exists", db_exists),
            debug_log::field("has_legacy", has_legacy),
            debug_log::field("legacy_dirs_found", legacy_dirs_found),
        ],
    );

    if !has_legacy {
        debug_log::log_kv(
            "legacy.migration",
            &[
                debug_log::field("phase", "skipped"),
                debug_log::field("result", "skipped"),
                debug_log::field("reason", "no_legacy_data"),
            ],
        );
        return Ok(());
    }

    eprintln!("Legacy file-based data detected. Migration to SQLite is required.");

    let migrate_name = if cfg!(target_os = "windows") {
        "migrate.exe"
    } else {
        "migrate"
    };

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_owned()));

    let migrate_path = exe_dir
        .as_ref()
        .map(|d| d.join(migrate_name))
        .filter(|p| p.exists());

    debug_log::log_kv(
        "legacy.migration",
        &[
            debug_log::field("phase", "locate_utility"),
            debug_log::field("found", migrate_path.is_some()),
            debug_log::field("channel", update::CHANNEL),
        ],
    );

    let (migrate_path, was_downloaded) = if let Some(path) = migrate_path {
        (path, false)
    } else {
        eprintln!("Migration utility not found.");

        let channel = update::CHANNEL;
        if channel == "unknown" {
            anyhow::bail!(
                "Legacy data directory needs migration but no '{}' binary found.\n\
                 Download it from: https://github.com/{}/releases/tag/legacy-migrate",
                migrate_name,
                update::REPO,
            );
        }

        let stdin = io::stdin();
        if !stdin.is_terminal() {
            anyhow::bail!(
                "Legacy data directory needs migration. Download the migration utility from\n\
                 https://github.com/{}/releases/tag/legacy-migrate or run in an interactive terminal.",
                update::REPO,
            );
        }

        eprint!("Download migration utility from GitHub? [Y/n] ");
        io::stderr().flush()?;
        let mut answer = String::new();
        stdin.read_line(&mut answer)?;
        let accepted = answer.trim().is_empty() || answer.trim().eq_ignore_ascii_case("y");
        debug_log::log_kv(
            "legacy.migration",
            &[
                debug_log::field("phase", "prompt_download"),
                debug_log::field(
                    "result",
                    if accepted { "accepted" } else { "declined" },
                ),
            ],
        );
        if !accepted {
            anyhow::bail!("Migration required. Cannot continue without migrating data.");
        }

        let dest = exe_dir
            .as_ref()
            .map(|d| d.join(migrate_name))
            .unwrap_or_else(|| std::path::PathBuf::from(migrate_name));

        download_migrate_binary(&dest).await?;
        (dest, true)
    };

    eprintln!("Running migration...");
    let mut cmd = std::process::Command::new(&migrate_path);
    cmd.arg("-d").arg(&data_dir);

    if no_encrypt {
        cmd.arg("--no-encrypt");
    }
    if let Some(passkey) = passkey {
        cmd.arg("--passkey").arg(passkey);
    }

    let status = debug_log::timed_result(
        "legacy.migration.run",
        &[
            debug_log::field("path", migrate_path.display()),
            debug_log::field("no_encrypt", no_encrypt),
            debug_log::field("has_passkey", passkey.is_some()),
        ],
        || cmd.status().context("failed to run migration utility"),
    )?;

    if was_downloaded {
        let removed = std::fs::remove_file(&migrate_path).is_ok();
        debug_log::log_kv(
            "legacy.migration",
            &[
                debug_log::field("phase", "cleanup"),
                debug_log::field("was_downloaded", true),
                debug_log::field("removed", removed),
            ],
        );
    }

    debug_log::log_kv(
        "legacy.migration.run",
        &[
            debug_log::field("phase", "exit"),
            debug_log::field(
                "exit_code",
                status.code().map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned()),
            ),
            debug_log::field(
                "result",
                if status.success() { "ok" } else { "error" },
            ),
        ],
    );

    if !status.success() {
        anyhow::bail!(
            "Migration failed with exit code: {}",
            status.code().unwrap_or(-1)
        );
    }

    eprintln!("Migration complete.");
    Ok(())
}

async fn download_migrate_binary(dest: &std::path::Path) -> Result<()> {
    let start = std::time::Instant::now();
    let client = update::build_client()?;

    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/legacy-migrate",
        update::REPO,
    );
    let release = update::fetch_release(&client, &url).await?;

    let expected_name = if cfg!(target_os = "windows") {
        format!("migrate-{}.exe", update::TARGET)
    } else {
        format!("migrate-{}", update::TARGET)
    };

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == expected_name)
        .context(format!(
            "no migration utility found for this platform ({}) in the legacy-migrate release",
            update::TARGET,
        ))?;

    eprintln!("Downloading {}...", asset.name);

    let resp = client
        .get(&asset.url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("failed to download migration utility")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        debug_log::log_kv(
            "legacy.migration.download",
            &[
                debug_log::field("asset", &asset.name),
                debug_log::field("result", "error"),
                debug_log::field("status", status.as_u16()),
                debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
            ],
        );
        anyhow::bail!("download failed with status {status}");
    }

    let bytes = resp.bytes().await.context("failed to read download")?;
    std::fs::write(dest, &bytes).context("failed to write migration utility")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .context("failed to set executable permissions")?;
    }

    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    debug_log::log_kv(
        "legacy.migration.download",
        &[
            debug_log::field("asset", &asset.name),
            debug_log::field("result", "ok"),
            debug_log::field("bytes", bytes.len()),
            debug_log::field("dest", dest.display()),
            debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
        ],
    );

    eprintln!("Saved to {}", dest.display());
    Ok(())
}
