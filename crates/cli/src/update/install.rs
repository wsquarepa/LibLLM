//! Binary download, atomic installation, and user confirmation prompts.

use std::io::{self, IsTerminal, Write};
use std::time::Instant;

use anyhow::{Context, Result};

use super::github::{Asset, Release, fetch_release, fetch_releases};
use super::version::parse_version_tag;
use super::{CHANNEL, REPO, TARGET};

fn parse_release_hash(body: &str) -> Option<&str> {
    let start = body.find("- [")?;
    let after = &body[start + "- [".len()..];
    let end = after.find("](")?;
    let hash = &after[..end];
    (hash.len() >= 7 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

pub(super) fn find_asset(release: &Release) -> Result<&Asset> {
    let expected_name = if cfg!(target_os = "windows") {
        format!("libllm-{TARGET}.exe")
    } else {
        format!("libllm-{TARGET}")
    };

    release
        .assets
        .iter()
        .find(|a| a.name == expected_name)
        .context(format!(
            "no asset found for this platform ({TARGET}) in the release"
        ))
}

pub(super) async fn download_and_replace(client: &reqwest::Client, asset: &Asset) -> Result<()> {
    let start = Instant::now();
    let download_resp = client
        .get(&asset.url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("failed to download binary")?;
    if !download_resp.status().is_success() {
        let status = download_resp.status();
        let body = download_resp.text().await.unwrap_or_default();
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::warn!(
            asset = asset.name.as_str(),
            result = "error",
            status = status.as_u16(),
            body_bytes = body.len(),
            elapsed_ms = elapsed_ms,
            "update.download"
        );
        anyhow::bail!("download failed with status {status}: {body}");
    }

    let bytes = download_resp
        .bytes()
        .await
        .context("failed to read download body")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    tracing::info!(
        asset = asset.name.as_str(),
        result = "ok",
        bytes = bytes.len(),
        elapsed_ms = elapsed_ms,
        "update.download"
    );

    let install_start = Instant::now();
    let exe_path =
        std::env::current_exe().context("failed to determine current executable path")?;
    let tmp_path = crate::paths::append_suffix(&exe_path, ".tmp");
    let old_path = crate::paths::append_suffix(&exe_path, ".old");

    std::fs::write(&tmp_path, &bytes).context("failed to write temporary file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .context("failed to set executable permissions")?;
    }

    if old_path.exists() {
        let _ = std::fs::remove_file(&old_path);
    }
    std::fs::rename(&exe_path, &old_path).context("failed to move current binary aside")?;
    if let Err(e) = std::fs::rename(&tmp_path, &exe_path) {
        let _ = std::fs::rename(&old_path, &exe_path);
        let install_elapsed = install_start.elapsed().as_secs_f64() * 1000.0;
        tracing::error!(phase = "rollback", result = "error", exe_path = %exe_path.display(), elapsed_ms = install_elapsed, err = %e, "update.install");
        return Err(e).context("failed to install new binary");
    }
    let _ = std::fs::remove_file(&old_path);

    let install_elapsed = install_start.elapsed().as_secs_f64() * 1000.0;
    tracing::info!(phase = "install", result = "ok", exe_path = %exe_path.display(), elapsed_ms = install_elapsed, "update.install");

    Ok(())
}

pub(super) async fn update_stable(client: &reqwest::Client) -> Result<()> {
    let releases = fetch_releases(client).await?;
    let mut stable: Vec<(semver::Version, Release)> = releases
        .into_iter()
        .filter(|r| !r.prerelease)
        .filter_map(|r| parse_version_tag(&r.tag_name).map(|v| (v, r)))
        .collect();
    stable.sort_by(|a, b| b.0.cmp_precedence(&a.0));

    let (_, release) = stable
        .into_iter()
        .next()
        .context("no stable releases published")?;

    let asset = find_asset(&release)?;

    if let Some(body) = &release.body
        && let Some(remote_hash) = parse_release_hash(body)
    {
        let current_hash = env!("LIBLLM_COMMIT", "unknown");
        if current_hash != "unknown" && current_hash == remote_hash {
            tracing::info!(
                channel = "stable",
                tag = release.tag_name.as_str(),
                result = "skipped",
                reason = "up_to_date",
                "update.check"
            );
            println!(
                "Already up to date ({} commit {current_hash}).",
                release.tag_name
            );
            return Ok(());
        }
    }

    let expected_name = &asset.name;
    println!("Downloading {expected_name}...");
    download_and_replace(client, asset).await?;

    let hash_display = release
        .body
        .as_deref()
        .and_then(parse_release_hash)
        .unwrap_or("unknown");
    println!("Updated to {} (commit {hash_display}).", release.tag_name);
    Ok(())
}

pub(super) async fn update_to_tag(client: &reqwest::Client, tag: &str) -> Result<()> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{tag}");
    let release = fetch_release(client, &url).await?;
    let asset = find_asset(&release)?;

    let installed_tag = if parse_version_tag(tag).is_some() {
        Some(format!("v{}", env!("CARGO_PKG_VERSION")))
    } else if CHANNEL == tag {
        Some(CHANNEL.to_string())
    } else {
        None
    };

    if installed_tag.as_deref() == Some(tag)
        && let Some(body) = &release.body
        && let Some(remote_hash) = parse_release_hash(body)
    {
        let current_hash = env!("LIBLLM_COMMIT", "unknown");
        if current_hash != "unknown" && current_hash == remote_hash {
            tracing::info!(
                tag = tag,
                result = "skipped",
                reason = "up_to_date",
                "update.check"
            );
            println!("Already up to date on '{tag}' (commit {current_hash}).");
            return Ok(());
        }
    }

    let expected_name = &asset.name;
    println!("Downloading {expected_name}...");
    download_and_replace(client, asset).await?;

    let hash_display = release
        .body
        .as_deref()
        .and_then(parse_release_hash)
        .unwrap_or("unknown");
    if parse_version_tag(tag).is_some() {
        println!("Switched to {tag} (commit {hash_display}).");
    } else {
        println!("Switched to branch '{tag}' (commit {hash_display}).");
    }
    Ok(())
}

enum Confirmation {
    YesFlag,
    NonInteractive,
    Answered(bool),
}

/// Honors `yes` without prompting, refuses when stdin is not a terminal, otherwise prints
/// `warning` to stderr, asks "Continue? [y/N]", and reads one line. Errors only when stderr
/// cannot be flushed or stdin cannot be read.
fn ask_confirmation(yes: bool, warning: &str) -> Result<Confirmation> {
    if yes {
        return Ok(Confirmation::YesFlag);
    }
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return Ok(Confirmation::NonInteractive);
    }
    eprintln!("{warning}");
    eprint!("\nContinue? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    stdin.read_line(&mut answer)?;
    Ok(Confirmation::Answered(
        answer.trim().eq_ignore_ascii_case("y"),
    ))
}

pub(super) fn confirm_channel_switch(target: &str, yes: bool) -> Result<bool> {
    let warning = format!(
        "WARNING: You are currently on '{CHANNEL}'.\n\
         Switching to '{target}' may cause issues if your current build introduced\n\
         data format changes that '{target}' does not yet support.\n\
         Your data directory could become unreadable."
    );
    match ask_confirmation(yes, &warning)? {
        Confirmation::YesFlag => {
            tracing::info!(
                from = CHANNEL,
                to = target,
                result = "confirmed",
                reason = "yes_flag",
                "update.channel_switch"
            );
            Ok(true)
        }
        Confirmation::NonInteractive => {
            tracing::warn!(
                from = CHANNEL,
                to = target,
                result = "error",
                reason = "non_interactive",
                "update.channel_switch"
            );
            anyhow::bail!(
                "Currently on '{CHANNEL}'. \
                 Switching channels in a non-interactive terminal requires --yes."
            );
        }
        Confirmation::Answered(confirmed) => {
            tracing::info!(
                from = CHANNEL,
                to = target,
                result = if confirmed { "confirmed" } else { "declined" },
                "update.channel_switch"
            );
            Ok(confirmed)
        }
    }
}

pub(super) fn confirm_downgrade(target: &str, yes: bool) -> Result<bool> {
    let from = concat!("v", env!("CARGO_PKG_VERSION"));
    let warning = format!(
        "WARNING: Downgrading from {from} to {target}.\n\
         Older builds may not understand data written by newer ones; \
         your data directory could become unreadable."
    );
    match ask_confirmation(yes, &warning)? {
        Confirmation::YesFlag => {
            tracing::info!(
                from = from,
                to = target,
                result = "confirmed",
                reason = "yes_flag",
                "update.downgrade"
            );
            Ok(true)
        }
        Confirmation::NonInteractive => {
            tracing::warn!(
                from = from,
                to = target,
                result = "error",
                reason = "non_interactive",
                "update.downgrade"
            );
            anyhow::bail!(
                "Downgrading to '{target}' in a non-interactive terminal requires --yes."
            );
        }
        Confirmation::Answered(confirmed) => {
            tracing::info!(
                from = from,
                to = target,
                result = if confirmed { "confirmed" } else { "declined" },
                "update.downgrade"
            );
            Ok(confirmed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_release_hash;

    #[test]
    fn parse_release_hash_reads_first_bulleted_link() {
        let body = "Changes this release:\n\n\
                    - [f7be246](https://github.com/x/y/commit/f7be2465) feat: thing\n\
                    - [acfc73e](https://github.com/x/y/commit/acfc73ef) chore: other\n";
        assert_eq!(parse_release_hash(body), Some("f7be246"));
    }

    #[test]
    fn parse_release_hash_rejects_non_hex_bracket_content() {
        let body = "- [not-a-hash](https://example.com) nope\n";
        assert_eq!(parse_release_hash(body), None);
    }

    #[test]
    fn parse_release_hash_returns_none_for_old_commit_prefix_format() {
        let body = "Commit `abcdef1` at `2026-01-01`\n\n```\nmsg\n```\n";
        assert_eq!(parse_release_hash(body), None);
    }

    #[test]
    fn parse_release_hash_returns_none_for_empty_body() {
        assert_eq!(parse_release_hash(""), None);
    }
}
