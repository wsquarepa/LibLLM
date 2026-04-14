use std::path::Path;

use anyhow::{Context, Result};

use crate::update;

pub async fn run(data_dir: &Path, passkey: Option<&str>, args: &[String]) -> Result<()> {
    let binary_name = if cfg!(target_os = "windows") {
        "recover.exe"
    } else {
        "recover"
    };

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_owned()));

    let local_path = exe_dir
        .as_ref()
        .map(|d| d.join(binary_name))
        .filter(|p| p.exists());

    let (recover_path, was_downloaded) = match local_path {
        Some(path) => (path, false),
        None => {
            let dest = exe_dir
                .as_ref()
                .map(|d| d.join(binary_name))
                .unwrap_or_else(|| std::path::PathBuf::from(binary_name));

            download_recover_binary(&dest).await?;
            (dest, true)
        }
    };

    let mut cmd = std::process::Command::new(&recover_path);
    cmd.arg("-d").arg(data_dir);

    if let Some(passkey) = passkey {
        cmd.arg("--passkey").arg(passkey);
    }

    cmd.args(args);

    let status = cmd.status().context("failed to run recovery utility")?;

    if was_downloaded {
        let _ = std::fs::remove_file(&recover_path);
    }

    if !status.success() {
        anyhow::bail!(
            "recovery utility exited with code: {}",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}

async fn download_recover_binary(dest: &Path) -> Result<()> {
    let channel = update::CHANNEL;
    if channel == "unknown" {
        anyhow::bail!(
            "Recovery utility not available for local builds.\n\
             Build it with: cargo build -p backup\n\
             Then run: cargo run -p backup --bin libllm-recover -- --help"
        );
    }

    let client = update::build_client()?;

    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        update::REPO,
        channel
    );
    let release = update::fetch_release(&client, &url).await?;

    let expected_name = if cfg!(target_os = "windows") {
        format!("recover-{}.exe", update::TARGET)
    } else {
        format!("recover-{}", update::TARGET)
    };

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == expected_name)
        .context(format!(
            "no recovery utility found for this platform ({}) in the {} release",
            update::TARGET,
            channel
        ))?;

    eprintln!("Downloading {}...", asset.name);

    let resp = client
        .get(&asset.url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("failed to download recovery utility")?;

    if !resp.status().is_success() {
        let status = resp.status();
        anyhow::bail!("download failed with status {status}");
    }

    let bytes = resp.bytes().await.context("failed to read download")?;
    std::fs::write(dest, &bytes).context("failed to write recovery utility")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .context("failed to set executable permissions")?;
    }

    eprintln!("Saved to {}", dest.display());
    Ok(())
}
