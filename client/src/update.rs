//! Self-update mechanism via GitHub release downloads.

use std::io::{self, IsTerminal, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;

pub const REPO: &str = "wsquarepa/LibLLM";
pub const CHANNEL: &str = env!("LIBLLM_CHANNEL");

pub const TARGET: &str = const {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
};

#[derive(Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub prerelease: bool,
}

#[derive(Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// One row in the interactive branch picker.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BranchEntry {
    pub name: String,
    pub current: bool,
}

/// Build the branch picker list from prerelease tag names.
///
/// Rules:
/// - `stable` is always the first entry.
/// - Prereleases are included in input order, except tags equal to
///   `"stable"` or `"master"`. Per release CI, `stable` is built from
///   every push to `master`, so a `master` prerelease tag would be a
///   duplicate of `stable`; the `stable` exclusion guards against a
///   stable tag being incorrectly marked prerelease.
/// - The `current` flag is set on whichever entry matches `channel`;
///   when `channel == "master"`, it is set on the `stable` entry.
pub fn build_branch_list(prerelease_tags: &[String], channel: &str) -> Vec<BranchEntry> {
    let current_is_stable = channel == "stable" || channel == "master";
    let mut out = vec![BranchEntry {
        name: "stable".to_string(),
        current: current_is_stable,
    }];
    for tag in prerelease_tags {
        if tag == "stable" || tag == "master" {
            continue;
        }
        out.push(BranchEntry {
            name: tag.clone(),
            current: tag == channel,
        });
    }
    out
}

fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok()
        .filter(|t| !t.is_empty())
}

pub fn build_client() -> Result<reqwest::Client> {
    libllm::crypto_provider::install_default_crypto_provider();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("libllm-updater"),
    );

    if let Some(token) = github_token() {
        let value = format!("Bearer {token}");
        headers.insert(
            reqwest::header::AUTHORIZATION,
            value.parse().context("invalid token")?,
        );
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("failed to build HTTP client")
}

fn parse_release_hash(body: &str) -> Option<&str> {
    let start = body.find("- [")?;
    let after = &body[start + "- [".len()..];
    let end = after.find("](")?;
    let hash = &after[..end];
    (hash.len() >= 7 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

fn current_exe_path() -> Result<std::path::PathBuf> {
    std::env::current_exe().context("failed to determine current executable path")
}

pub async fn fetch_release(client: &reqwest::Client, url: &str) -> Result<Release> {
    let start = Instant::now();
    let resp = match client
        .get(url)
        .send()
        .await
        .context("failed to fetch release info")
    {
        Ok(resp) => resp,
        Err(err) => {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::warn!(url = url, result = "error", elapsed_ms = elapsed_ms, err = %err, "update.fetch_release");
            return Err(err);
        }
    };

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::UNAUTHORIZED {
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::warn!(url = url, result = "error", status = status.as_u16(), has_token = github_token().is_some(), elapsed_ms = elapsed_ms, "update.fetch_release");
        if github_token().is_none() {
            anyhow::bail!(
                "GitHub API returned {status}. \
                 If the repository is private, set GITHUB_TOKEN or GH_TOKEN."
            );
        }
        anyhow::bail!("GitHub API returned {status}. Check that your token has repository access.");
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::warn!(url = url, result = "error", status = status.as_u16(), body_bytes = body.len(), elapsed_ms = elapsed_ms, "update.fetch_release");
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let release: Result<Release> = resp.json().await.context("failed to parse release JSON");
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    match &release {
        Ok(release) => tracing::info!(url = url, result = "ok", tag = release.tag_name.as_str(), asset_count = release.assets.len(), elapsed_ms = elapsed_ms, "update.fetch_release"),
        Err(err) => tracing::warn!(url = url, result = "error", elapsed_ms = elapsed_ms, err = %err, "update.fetch_release"),
    }
    release
}

fn find_asset(release: &Release) -> Result<&Asset> {
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

async fn download_and_replace(client: &reqwest::Client, asset: &Asset) -> Result<()> {
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
        tracing::warn!(asset = asset.name.as_str(), result = "error", status = status.as_u16(), body_bytes = body.len(), elapsed_ms = elapsed_ms, "update.download");
        anyhow::bail!("download failed with status {status}: {body}");
    }

    let bytes = download_resp
        .bytes()
        .await
        .context("failed to read download body")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    tracing::info!(asset = asset.name.as_str(), result = "ok", bytes = bytes.len(), elapsed_ms = elapsed_ms, "update.download");

    let install_start = Instant::now();
    let exe_path = current_exe_path()?;
    let tmp_path = exe_path.with_extension("tmp");
    let old_path = exe_path.with_extension("old");

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

async fn update_stable(client: &reqwest::Client) -> Result<()> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/stable");
    let release = fetch_release(client, &url).await?;
    let asset = find_asset(&release)?;

    if let Some(body) = &release.body
        && let Some(remote_hash) = parse_release_hash(body) {
            let current_hash = env!("LIBLLM_COMMIT", "unknown");
            if current_hash != "unknown" && current_hash == remote_hash {
                tracing::info!(channel = "stable", result = "skipped", reason = "up_to_date", "update.check");
                println!("Already up to date (commit {current_hash}).");
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
    println!("Updated to stable (commit {hash_display}).");
    Ok(())
}

async fn update_branch(client: &reqwest::Client, branch: &str) -> Result<()> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{branch}");
    let release = fetch_release(client, &url).await?;
    let asset = find_asset(&release)?;

    if CHANNEL == branch
        && let Some(body) = &release.body
            && let Some(remote_hash) = parse_release_hash(body) {
                let current_hash = env!("LIBLLM_COMMIT", "unknown");
                if current_hash != "unknown" && current_hash == remote_hash {
                    tracing::info!(channel = branch, result = "skipped", reason = "up_to_date", "update.check");
                    println!("Already up to date on '{branch}' (commit {current_hash}).");
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
    println!("Switched to branch '{branch}' (commit {hash_display}).");
    Ok(())
}

fn confirm_channel_switch(target: &str, yes: bool) -> Result<bool> {
    if yes {
        tracing::info!(from = CHANNEL, to = target, result = "confirmed", reason = "yes_flag", "update.channel_switch");
        return Ok(true);
    }

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        tracing::warn!(from = CHANNEL, to = target, result = "error", reason = "non_interactive", "update.channel_switch");
        anyhow::bail!(
            "Currently on '{CHANNEL}'. \
             Switching channels in a non-interactive terminal requires --yes."
        );
    }

    eprintln!("WARNING: You are currently on '{CHANNEL}'.");
    eprintln!(
        "Switching to '{target}' may cause issues if your current build introduced\n\
         data format changes that '{target}' does not yet support.\n\
         Your data directory could become unreadable."
    );
    eprint!("\nContinue? [y/N] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    stdin.read_line(&mut answer)?;
    let confirmed = answer.trim().eq_ignore_ascii_case("y");
    tracing::info!(from = CHANNEL, to = target, result = if confirmed { "confirmed" } else { "declined" }, "update.channel_switch");
    Ok(confirmed)
}

async fn fetch_prerelease_tags(client: &reqwest::Client) -> Result<Vec<String>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let resp = client
        .get(&url)
        .send()
        .await
        .context("failed to fetch releases")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(result = "error", status = status.as_u16(), body_bytes = body.len(), "update.fetch_prereleases");
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let releases: Vec<Release> = resp.json().await.context("failed to parse releases")?;

    let tags: Vec<String> = releases
        .into_iter()
        .filter(|r| r.prerelease)
        .map(|r| r.tag_name)
        .collect();

    tracing::info!(tag_count = tags.len(), result = "ok", "update.fetch_prereleases");

    Ok(tags)
}

async fn pick_branch(client: &reqwest::Client) -> Result<Option<String>> {
    tracing::debug!(phase = "start", "update.interactive");
    let tags = fetch_prerelease_tags(client).await?;
    let entries = build_branch_list(&tags, CHANNEL);

    let rows: Vec<String> = entries
        .iter()
        .map(|entry| {
            if entry.current {
                format!("{} (current)", entry.name)
            } else {
                entry.name.clone()
            }
        })
        .collect();

    let Some(index) = crate::interactive::select("Select a release channel:", &rows)? else {
        tracing::debug!(phase = "cancelled", "update.interactive");
        return Ok(None);
    };

    let selected = entries[index].name.clone();
    tracing::debug!(phase = "branch_selected", branch = selected.as_str(), "update.interactive");
    Ok(Some(selected))
}

pub async fn run(branch: Option<String>, yes: bool) -> Result<()> {
    if CHANNEL == "unknown" {
        tracing::warn!(phase = "start", result = "error", reason = "not_installed", "update.run");
        anyhow::bail!("This build was not installed from a release. Use install.sh to install.");
    }

    tracing::info!(phase = "start", channel = CHANNEL, target = branch.as_deref().unwrap_or("stable"), interactive = crate::interactive::is_interactive(), "update.run");

    let client = build_client()?;

    let resolved = match branch {
        Some(name) => Some(name),
        None if crate::interactive::is_interactive() => match pick_branch(&client).await? {
            Some(name) => Some(name),
            None => return Ok(()),
        },
        None => None,
    };

    let target = resolved.as_deref().unwrap_or("stable");
    if CHANNEL != target && !confirm_channel_switch(target, yes)? {
        tracing::info!(phase = "cancel", reason = "channel_switch_declined", "update.run");
        println!("Cancelled.");
        return Ok(());
    }

    match resolved.as_deref() {
        Some("stable") | None => update_stable(&client).await,
        Some(name) => update_branch(&client, name).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_prepends_stable_and_marks_it_current_for_stable_channel() {
        let tags = vec!["feat/foo".to_string(), "bugfix/bar".to_string()];
        let result = build_branch_list(&tags, "stable");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "stable");
        assert!(result[0].current);
        assert_eq!(result[1].name, "feat/foo");
        assert!(!result[1].current);
        assert_eq!(result[2].name, "bugfix/bar");
        assert!(!result[2].current);
    }

    #[test]
    fn filter_excludes_stable_tag_from_prereleases() {
        let tags = vec!["stable".to_string(), "feat/foo".to_string()];
        let result = build_branch_list(&tags, "stable");
        assert_eq!(result.iter().filter(|b| b.name == "stable").count(), 1);
    }

    #[test]
    fn filter_excludes_master_tag_from_prereleases() {
        let tags = vec!["master".to_string(), "feat/foo".to_string()];
        let result = build_branch_list(&tags, "stable");
        assert_eq!(result.iter().filter(|b| b.name == "master").count(), 0);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_marks_master_channel_as_stable_current() {
        let tags = vec!["feat/foo".to_string()];
        let result = build_branch_list(&tags, "master");
        assert_eq!(result[0].name, "stable");
        assert!(result[0].current, "stable should be marked current when CHANNEL=master");
        assert!(!result[1].current);
    }

    #[test]
    fn filter_marks_selected_branch_current() {
        let tags = vec!["feat/foo".to_string(), "bugfix/bar".to_string()];
        let result = build_branch_list(&tags, "feat/foo");
        assert!(!result[0].current);
        assert_eq!(result[1].name, "feat/foo");
        assert!(result[1].current);
        assert!(!result[2].current);
    }

    #[test]
    fn filter_preserves_api_order_of_prereleases() {
        let tags = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        let result = build_branch_list(&tags, "stable");
        assert_eq!(result[1].name, "c");
        assert_eq!(result[2].name, "a");
        assert_eq!(result[3].name, "b");
    }

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
