use anyhow::{Context, Result};
use std::io::{self, IsTerminal, Write};
use libllm::config;
use crate::update;

const LEGACY_DIRS: [&str; 5] = ["sessions", "characters", "worldinfo", "system", "personas"];

pub async fn check_and_run_migration(no_encrypt: bool, passkey: Option<&str>) -> Result<()> {
    let data_dir = config::data_dir();
    let db_path = data_dir.join("data.db");

    if db_path.exists() {
        return Ok(());
    }

    let has_legacy = LEGACY_DIRS.iter().any(|dir| {
        let path = data_dir.join(dir);
        path.is_dir()
            && std::fs::read_dir(&path)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
    });

    if !has_legacy {
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

    let (migrate_path, was_downloaded) = if let Some(path) = migrate_path {
        (path, false)
    } else {
        eprintln!("Migration utility not found.");

        let channel = update::CHANNEL;
        if channel == "unknown" {
            anyhow::bail!(
                "Legacy data directory needs migration but no '{}' binary found.\n\
                 Build it with: cargo build -p migrate\n\
                 Then run: migrate -d {}",
                migrate_name,
                data_dir.display()
            );
        }

        let stdin = io::stdin();
        if !stdin.is_terminal() {
            anyhow::bail!(
                "Legacy data directory needs migration. Download the migration utility from\n\
                 https://github.com/{}/releases/tag/{} or run in an interactive terminal.",
                update::REPO,
                channel
            );
        }

        eprint!("Download migration utility from GitHub? [Y/n] ");
        io::stderr().flush()?;
        let mut answer = String::new();
        stdin.read_line(&mut answer)?;
        if !answer.trim().is_empty() && !answer.trim().eq_ignore_ascii_case("y") {
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

    let status = cmd.status().context("failed to run migration utility")?;

    if was_downloaded {
        let _ = std::fs::remove_file(&migrate_path);
    }

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
    let channel = update::CHANNEL;
    let client = update::build_client()?;

    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        update::REPO,
        channel
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
            "no migration utility found for this platform ({}) in the {} release",
            update::TARGET,
            channel
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

    eprintln!("Saved to {}", dest.display());
    Ok(())
}
