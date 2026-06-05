//! Self-update mechanism via GitHub release downloads.

use std::io::{self, IsTerminal, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::paths::append_suffix;

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

fn parse_version_tag(s: &str) -> Option<semver::Version> {
    s.strip_prefix('v')
        .and_then(|rest| semver::Version::parse(rest).ok())
}

fn normalize_tag(s: &str) -> String {
    if s.starts_with('v') {
        return s.to_string();
    }
    if semver::Version::parse(s).is_ok() {
        return format!("v{s}");
    }
    s.to_string()
}

fn is_stable_target(s: &str) -> bool {
    s == "stable" || parse_version_tag(s).is_some()
}

fn should_warn_downgrade(channel: &str, target: &str, current_version: &str) -> bool {
    if channel != "stable" {
        return false;
    }
    let Some(target_ver) = parse_version_tag(target) else {
        return false;
    };
    let Ok(current_ver) = semver::Version::parse(current_version) else {
        return false;
    };
    target_ver.cmp_precedence(&current_ver) == std::cmp::Ordering::Less
}

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

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BranchEntry {
    pub tag: String,
    pub display: String,
    pub current: bool,
}

pub fn build_branch_list(
    releases: &[Release],
    channel: &str,
    current_version: &str,
) -> Vec<BranchEntry> {
    let mut stable: Vec<(semver::Version, &Release)> = releases
        .iter()
        .filter(|r| !r.prerelease)
        .filter_map(|r| parse_version_tag(&r.tag_name).map(|v| (v, r)))
        .collect();
    stable.sort_by(|a, b| b.0.cmp_precedence(&a.0));

    let running: Option<semver::Version> = if channel == "stable" {
        semver::Version::parse(current_version).ok()
    } else {
        None
    };

    let mut out: Vec<BranchEntry> = Vec::new();
    let mut stable_iter = stable.iter();

    if let Some((version, release)) = stable_iter.next() {
        let is_current = running.as_ref().is_some_and(|r| r == version);
        out.push(BranchEntry {
            tag: release.tag_name.clone(),
            display: release.tag_name.clone(),
            current: is_current,
        });
    }

    if let Some(preview) = releases
        .iter()
        .find(|r| r.prerelease && r.tag_name == "preview")
    {
        out.push(BranchEntry {
            tag: preview.tag_name.clone(),
            display: preview.tag_name.clone(),
            current: channel == "preview",
        });
    }

    for release in releases.iter() {
        if !release.prerelease {
            continue;
        }
        if release.tag_name == "stable"
            || release.tag_name == "preview"
            || parse_version_tag(&release.tag_name).is_some()
        {
            continue;
        }
        out.push(BranchEntry {
            tag: release.tag_name.clone(),
            display: release.tag_name.clone(),
            current: release.tag_name == channel,
        });
    }

    for (version, release) in stable_iter {
        let is_current = running.as_ref().is_some_and(|r| r == version);
        out.push(BranchEntry {
            tag: release.tag_name.clone(),
            display: release.tag_name.clone(),
            current: is_current,
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
    libllm_protocol::crypto_provider::install_default_crypto_provider();
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
        tracing::warn!(
            url = url,
            result = "error",
            status = status.as_u16(),
            has_token = github_token().is_some(),
            elapsed_ms = elapsed_ms,
            "update.fetch_release"
        );
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
        tracing::warn!(
            url = url,
            result = "error",
            status = status.as_u16(),
            body_bytes = body.len(),
            elapsed_ms = elapsed_ms,
            "update.fetch_release"
        );
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let release: Result<Release> = resp.json().await.context("failed to parse release JSON");
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    match &release {
        Ok(release) => tracing::info!(
            url = url,
            result = "ok",
            tag = release.tag_name.as_str(),
            asset_count = release.assets.len(),
            elapsed_ms = elapsed_ms,
            "update.fetch_release"
        ),
        Err(err) => {
            tracing::warn!(url = url, result = "error", elapsed_ms = elapsed_ms, err = %err, "update.fetch_release")
        }
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
    let exe_path = current_exe_path()?;
    let tmp_path = append_suffix(&exe_path, ".tmp");
    let old_path = append_suffix(&exe_path, ".old");

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

async fn update_to_tag(client: &reqwest::Client, tag: &str) -> Result<()> {
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

fn confirm_channel_switch(target: &str, yes: bool) -> Result<bool> {
    if yes {
        tracing::info!(
            from = CHANNEL,
            to = target,
            result = "confirmed",
            reason = "yes_flag",
            "update.channel_switch"
        );
        return Ok(true);
    }

    let stdin = io::stdin();
    if !stdin.is_terminal() {
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
    tracing::info!(
        from = CHANNEL,
        to = target,
        result = if confirmed { "confirmed" } else { "declined" },
        "update.channel_switch"
    );
    Ok(confirmed)
}

fn confirm_downgrade(target: &str, yes: bool) -> Result<bool> {
    let from = concat!("v", env!("CARGO_PKG_VERSION"));
    if yes {
        tracing::info!(
            from = from,
            to = target,
            result = "confirmed",
            reason = "yes_flag",
            "update.downgrade"
        );
        return Ok(true);
    }

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        tracing::warn!(
            from = from,
            to = target,
            result = "error",
            reason = "non_interactive",
            "update.downgrade"
        );
        anyhow::bail!("Downgrading to '{target}' in a non-interactive terminal requires --yes.");
    }

    eprintln!(
        "WARNING: Downgrading from {from} to {target}.\n\
         Older builds may not understand data written by newer ones; \
         your data directory could become unreadable."
    );
    eprint!("\nContinue? [y/N] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    stdin.read_line(&mut answer)?;
    let confirmed = answer.trim().eq_ignore_ascii_case("y");
    tracing::info!(
        from = from,
        to = target,
        result = if confirmed { "confirmed" } else { "declined" },
        "update.downgrade"
    );
    Ok(confirmed)
}

async fn fetch_releases(client: &reqwest::Client) -> Result<Vec<Release>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let resp = client
        .get(&url)
        .send()
        .await
        .context("failed to fetch releases")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(
            result = "error",
            status = status.as_u16(),
            body_bytes = body.len(),
            "update.fetch_releases"
        );
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let releases: Vec<Release> = resp.json().await.context("failed to parse releases")?;
    tracing::info!(
        release_count = releases.len(),
        result = "ok",
        "update.fetch_releases"
    );

    Ok(releases)
}

async fn pick_branch(client: &reqwest::Client) -> Result<Option<String>> {
    tracing::debug!(phase = "start", "update.interactive");
    let releases = fetch_releases(client).await?;
    let entries = build_branch_list(&releases, CHANNEL, env!("CARGO_PKG_VERSION"));

    if entries.is_empty() {
        anyhow::bail!("No releases available to install.");
    }

    let rows: Vec<String> = entries
        .iter()
        .map(|entry| {
            if entry.current {
                format!("{} (current)", entry.display)
            } else {
                entry.display.clone()
            }
        })
        .collect();

    let default = entries.iter().position(|e| e.current).unwrap_or(0);
    let Some(index) =
        crate::interactive::arrow_select("Select a release channel:", &rows, default)?
    else {
        tracing::debug!(phase = "cancelled", "update.interactive");
        return Ok(None);
    };

    let selected = entries[index].tag.clone();
    tracing::debug!(
        phase = "branch_selected",
        branch = selected.as_str(),
        "update.interactive"
    );
    Ok(Some(selected))
}

pub async fn run(branch: Option<String>, yes: bool) -> Result<()> {
    if CHANNEL == "unknown" {
        tracing::warn!(
            phase = "start",
            result = "error",
            reason = "not_installed",
            "update.run"
        );
        anyhow::bail!("This build was not installed from a release. Use install.sh to install.");
    }

    tracing::info!(
        phase = "start",
        channel = CHANNEL,
        target = branch.as_deref().unwrap_or("stable"),
        interactive = crate::interactive::is_interactive(),
        "update.run"
    );

    let client = build_client()?;

    let resolved = match branch {
        Some(name) => Some(name),
        None if crate::interactive::is_interactive() => match pick_branch(&client).await? {
            Some(name) => Some(name),
            None => return Ok(()),
        },
        None => None,
    };

    let target_raw = resolved.as_deref().unwrap_or("stable");
    let target = normalize_tag(target_raw);

    let target_is_stable = is_stable_target(&target);
    let source_is_stable = CHANNEL == "stable";
    let switching_channels = source_is_stable != target_is_stable
        || (!source_is_stable && !target_is_stable && CHANNEL != target);

    if switching_channels && !confirm_channel_switch(&target, yes)? {
        tracing::info!(
            phase = "cancel",
            reason = "channel_switch_declined",
            "update.run"
        );
        println!("Cancelled.");
        return Ok(());
    }

    if should_warn_downgrade(CHANNEL, &target, env!("CARGO_PKG_VERSION"))
        && !confirm_downgrade(&target, yes)?
    {
        tracing::info!(
            phase = "cancel",
            reason = "downgrade_declined",
            "update.run"
        );
        println!("Cancelled.");
        return Ok(());
    }

    if target == "stable" {
        update_stable(&client).await
    } else {
        update_to_tag(&client, &target).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, prerelease: bool) -> Release {
        Release {
            tag_name: tag.to_string(),
            body: None,
            assets: Vec::new(),
            prerelease,
        }
    }

    #[test]
    fn build_list_puts_highest_semver_first_with_bare_tag() {
        let releases = vec![
            rel("v2.4.0", false),
            rel("v2.6.0", false),
            rel("v2.5.0", false),
            rel("feat/foo", true),
        ];
        let list = build_branch_list(&releases, "stable", "2.6.0");
        assert_eq!(list[0].tag, "v2.6.0");
        assert_eq!(list[0].display, "v2.6.0");
        assert!(list[0].current);
    }

    #[test]
    fn build_list_orders_highest_then_preview_then_branches_then_older_stables() {
        let releases = vec![
            rel("v2.4.0", false),
            rel("v2.6.0", false),
            rel("v2.5.0", false),
            rel("feat/foo", true),
            rel("preview", true),
            rel("feat/bar", true),
        ];
        let list = build_branch_list(&releases, "stable", "2.6.0");
        let names: Vec<&str> = list.iter().map(|e| e.tag.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "v2.6.0", "preview", "feat/foo", "feat/bar", "v2.5.0", "v2.4.0"
            ]
        );
    }

    #[test]
    fn build_list_marks_preview_current_on_preview_channel() {
        let releases = vec![rel("v2.6.0", false), rel("preview", true)];
        let list = build_branch_list(&releases, "preview", "2.6.0");
        let preview = list.iter().find(|e| e.tag == "preview").unwrap();
        assert!(preview.current);
    }

    #[test]
    fn build_list_omits_preview_section_when_not_published() {
        let releases = vec![rel("v2.6.0", false), rel("feat/foo", true)];
        let list = build_branch_list(&releases, "stable", "2.6.0");
        assert!(!list.iter().any(|e| e.tag == "preview"));
    }

    #[test]
    fn build_list_keeps_all_stable_versions_sorted_desc() {
        let releases = vec![
            rel("v2.4.0", false),
            rel("v2.6.0", false),
            rel("v2.1.0", false),
            rel("v2.5.0", false),
            rel("v2.2.0", false),
            rel("v2.3.0", false),
        ];
        let list = build_branch_list(&releases, "stable", "2.6.0");
        let stable_rows: Vec<&str> = list
            .iter()
            .filter(|e| e.tag.starts_with('v'))
            .map(|e| e.tag.as_str())
            .collect();
        assert_eq!(
            stable_rows,
            vec!["v2.6.0", "v2.5.0", "v2.4.0", "v2.3.0", "v2.2.0", "v2.1.0"]
        );
    }

    #[test]
    fn build_list_marks_current_on_running_version_not_latest() {
        let releases = vec![
            rel("v2.6.0", false),
            rel("v2.5.0", false),
            rel("v2.4.0", false),
        ];
        let list = build_branch_list(&releases, "stable", "2.4.0");
        let v260 = list.iter().find(|e| e.tag == "v2.6.0").unwrap();
        let v240 = list.iter().find(|e| e.tag == "v2.4.0").unwrap();
        assert!(!v260.current);
        assert!(v240.current);
    }

    #[test]
    fn build_list_marks_running_version_current_even_when_far_below_latest() {
        let releases = vec![
            rel("v2.10.0", false),
            rel("v2.9.0", false),
            rel("v2.8.0", false),
            rel("v2.7.0", false),
            rel("v2.6.0", false),
            rel("v2.5.0", false),
        ];
        let list = build_branch_list(&releases, "stable", "2.5.0");
        let v250 = list.iter().find(|e| e.tag == "v2.5.0").unwrap();
        assert!(v250.current);
        assert_eq!(list.iter().filter(|e| e.current).count(), 1);
    }

    #[test]
    fn build_list_appends_prereleases_after_semver_in_api_order() {
        let releases = vec![
            rel("v2.6.0", false),
            rel("feat/foo", true),
            rel("bugfix/bar", true),
        ];
        let list = build_branch_list(&releases, "stable", "2.6.0");
        let names: Vec<&str> = list.iter().map(|e| e.tag.as_str()).collect();
        assert_eq!(names, vec!["v2.6.0", "feat/foo", "bugfix/bar"]);
    }

    #[test]
    fn build_list_excludes_v_tags_from_prerelease_section() {
        let releases = vec![
            rel("v2.6.0", false),
            rel("v2.0.0-beta", true),
            rel("feat/foo", true),
        ];
        let list = build_branch_list(&releases, "stable", "2.6.0");
        assert!(!list.iter().any(|e| e.tag == "v2.0.0-beta"));
        assert!(list.iter().any(|e| e.tag == "feat/foo"));
    }

    #[test]
    fn build_list_marks_branch_channel_current_on_matching_prerelease() {
        let releases = vec![
            rel("v2.6.0", false),
            rel("feat/foo", true),
            rel("feat/bar", true),
        ];
        let list = build_branch_list(&releases, "feat/foo", "2.6.0");
        let foo = list.iter().find(|e| e.tag == "feat/foo").unwrap();
        let bar = list.iter().find(|e| e.tag == "feat/bar").unwrap();
        assert!(foo.current);
        assert!(!bar.current);
    }

    #[test]
    fn build_list_excludes_literal_stable_tag_from_prereleases() {
        let releases = vec![
            rel("v2.6.0", false),
            rel("stable", true),
            rel("feat/foo", true),
        ];
        let list = build_branch_list(&releases, "stable", "2.6.0");
        assert!(!list.iter().any(|e| e.tag == "stable"));
        assert!(list.iter().any(|e| e.tag == "feat/foo"));
    }

    #[test]
    fn build_list_returns_empty_when_no_releases() {
        let list = build_branch_list(&[], "stable", "2.6.0");
        assert!(list.is_empty());
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

    #[test]
    fn parse_version_tag_strips_v_prefix() {
        let v = parse_version_tag("v2.6.0").unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 6);
        assert_eq!(v.patch, 0);
    }

    #[test]
    fn parse_version_tag_rejects_branch_names() {
        assert!(parse_version_tag("feat/foo").is_none());
        assert!(parse_version_tag("vfoo").is_none());
        assert!(parse_version_tag("2.6.0").is_none());
        assert!(parse_version_tag("stable").is_none());
    }

    #[test]
    fn normalize_tag_adds_v_to_bare_semver() {
        assert_eq!(normalize_tag("2.4.0"), "v2.4.0");
    }

    #[test]
    fn normalize_tag_preserves_v_prefixed_semver() {
        assert_eq!(normalize_tag("v2.4.0"), "v2.4.0");
    }

    #[test]
    fn normalize_tag_passes_through_non_semver() {
        assert_eq!(normalize_tag("feat/foo"), "feat/foo");
        assert_eq!(normalize_tag("stable"), "stable");
    }

    #[test]
    fn is_stable_target_recognizes_stable_and_v_tags() {
        assert!(is_stable_target("stable"));
        assert!(is_stable_target("v2.6.0"));
        assert!(is_stable_target("v0.0.1"));
    }

    #[test]
    fn is_stable_target_rejects_branches() {
        assert!(!is_stable_target("feat/foo"));
        assert!(!is_stable_target("master"));
        assert!(!is_stable_target("v2.6"));
        assert!(!is_stable_target("2.6.0"));
    }

    #[test]
    fn warn_downgrade_fires_when_target_older() {
        assert!(should_warn_downgrade("stable", "v2.4.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_same() {
        assert!(!should_warn_downgrade("stable", "v2.6.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_newer() {
        assert!(!should_warn_downgrade("stable", "v2.7.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_source_is_branch() {
        assert!(!should_warn_downgrade("feat/foo", "v2.4.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_is_branch() {
        assert!(!should_warn_downgrade("stable", "feat/foo", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_is_literal_stable() {
        assert!(!should_warn_downgrade("stable", "stable", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_current_version_is_unparseable() {
        assert!(!should_warn_downgrade("stable", "v2.4.0", "unknown"));
    }
}
