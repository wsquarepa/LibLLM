//! Self-update mechanism via GitHub release downloads.

use std::io::{self, IsTerminal, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use libllm::debug_log;
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

fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok()
        .filter(|t| !t.is_empty())
}

pub fn build_client() -> Result<reqwest::Client> {
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
    let after_tick = body.strip_prefix("Commit `")?;
    let end = after_tick.find('`')?;
    Some(&after_tick[..end])
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
            debug_log::log_kv(
                "update.fetch_release",
                &[
                    debug_log::field("url", url),
                    debug_log::field("result", "error"),
                    debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
                    debug_log::field("error", &err),
                ],
            );
            return Err(err);
        }
    };

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::UNAUTHORIZED {
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        debug_log::log_kv(
            "update.fetch_release",
            &[
                debug_log::field("url", url),
                debug_log::field("result", "error"),
                debug_log::field("status", status.as_u16()),
                debug_log::field("has_token", github_token().is_some()),
                debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
            ],
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
        debug_log::log_kv(
            "update.fetch_release",
            &[
                debug_log::field("url", url),
                debug_log::field("result", "error"),
                debug_log::field("status", status.as_u16()),
                debug_log::field("body_bytes", body.len()),
                debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
            ],
        );
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let release: Result<Release> = resp.json().await.context("failed to parse release JSON");
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    match &release {
        Ok(release) => debug_log::log_kv(
            "update.fetch_release",
            &[
                debug_log::field("url", url),
                debug_log::field("result", "ok"),
                debug_log::field("tag", &release.tag_name),
                debug_log::field("asset_count", release.assets.len()),
                debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
            ],
        ),
        Err(err) => debug_log::log_kv(
            "update.fetch_release",
            &[
                debug_log::field("url", url),
                debug_log::field("result", "error"),
                debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
                debug_log::field("error", err),
            ],
        ),
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
        debug_log::log_kv(
            "update.download",
            &[
                debug_log::field("asset", &asset.name),
                debug_log::field("result", "error"),
                debug_log::field("status", status.as_u16()),
                debug_log::field("body_bytes", body.len()),
                debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
            ],
        );
        anyhow::bail!("download failed with status {status}: {body}");
    }

    let bytes = download_resp
        .bytes()
        .await
        .context("failed to read download body")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    debug_log::log_kv(
        "update.download",
        &[
            debug_log::field("asset", &asset.name),
            debug_log::field("result", "ok"),
            debug_log::field("bytes", bytes.len()),
            debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
        ],
    );

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
        debug_log::log_kv(
            "update.install",
            &[
                debug_log::field("phase", "rollback"),
                debug_log::field("result", "error"),
                debug_log::field("exe_path", exe_path.display()),
                debug_log::field("elapsed_ms", format!("{install_elapsed:.3}")),
                debug_log::field("error", &e),
            ],
        );
        return Err(e).context("failed to install new binary");
    }
    let _ = std::fs::remove_file(&old_path);

    let install_elapsed = install_start.elapsed().as_secs_f64() * 1000.0;
    debug_log::log_kv(
        "update.install",
        &[
            debug_log::field("phase", "install"),
            debug_log::field("result", "ok"),
            debug_log::field("exe_path", exe_path.display()),
            debug_log::field("elapsed_ms", format!("{install_elapsed:.3}")),
        ],
    );

    Ok(())
}

async fn update_stable(client: &reqwest::Client) -> Result<()> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/stable");
    let release = fetch_release(client, &url).await?;
    let asset = find_asset(&release)?;

    if let Some(body) = &release.body {
        if let Some(remote_hash) = parse_release_hash(body) {
            let current_hash = env!("LIBLLM_COMMIT", "unknown");
            if current_hash != "unknown" && current_hash == remote_hash {
                debug_log::log_kv(
                    "update.check",
                    &[
                        debug_log::field("channel", "stable"),
                        debug_log::field("result", "skipped"),
                        debug_log::field("reason", "up_to_date"),
                    ],
                );
                println!("Already up to date (commit {current_hash}).");
                return Ok(());
            }
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

    if CHANNEL == branch {
        if let Some(body) = &release.body {
            if let Some(remote_hash) = parse_release_hash(body) {
                let current_hash = env!("LIBLLM_COMMIT", "unknown");
                if current_hash != "unknown" && current_hash == remote_hash {
                    debug_log::log_kv(
                        "update.check",
                        &[
                            debug_log::field("channel", branch),
                            debug_log::field("result", "skipped"),
                            debug_log::field("reason", "up_to_date"),
                        ],
                    );
                    println!("Already up to date on '{branch}' (commit {current_hash}).");
                    return Ok(());
                }
            }
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
        debug_log::log_kv(
            "update.channel_switch",
            &[
                debug_log::field("from", CHANNEL),
                debug_log::field("to", target),
                debug_log::field("result", "confirmed"),
                debug_log::field("reason", "yes_flag"),
            ],
        );
        return Ok(true);
    }

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        debug_log::log_kv(
            "update.channel_switch",
            &[
                debug_log::field("from", CHANNEL),
                debug_log::field("to", target),
                debug_log::field("result", "error"),
                debug_log::field("reason", "non_interactive"),
            ],
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
    debug_log::log_kv(
        "update.channel_switch",
        &[
            debug_log::field("from", CHANNEL),
            debug_log::field("to", target),
            debug_log::field(
                "result",
                if confirmed { "confirmed" } else { "declined" },
            ),
        ],
    );
    Ok(confirmed)
}

async fn list_branches(client: &reqwest::Client) -> Result<()> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let resp = client
        .get(&url)
        .send()
        .await
        .context("failed to fetch releases")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let releases: Vec<Release> = resp.json().await.context("failed to parse releases")?;
    let branches: Vec<&str> = releases
        .iter()
        .filter(|r| r.prerelease)
        .map(|r| r.tag_name.as_str())
        .collect();

    debug_log::log_kv(
        "update.list_branches",
        &[
            debug_log::field("branch_count", branches.len()),
            debug_log::field("result", "ok"),
        ],
    );

    if branches.is_empty() {
        println!("No branch builds available.");
        return Ok(());
    }

    let stdin = io::stdin();
    if stdin.is_terminal() {
        for (i, name) in branches.iter().enumerate() {
            let marker = if *name == CHANNEL { " (current)" } else { "" };
            println!("  {}) {name}{marker}", i + 1);
        }
        eprint!("\nSelect a branch (or press Enter to cancel): ");
        io::stderr().flush()?;

        let mut input = String::new();
        stdin.read_line(&mut input)?;
        let input = input.trim();
        if input.is_empty() {
            return Ok(());
        }

        let index: usize = input
            .parse::<usize>()
            .ok()
            .filter(|&n| n >= 1 && n <= branches.len())
            .context("invalid selection")?;

        let selected = branches[index - 1];
        update_branch(client, selected).await
    } else {
        for name in &branches {
            println!("{name}");
        }
        Ok(())
    }
}

pub async fn run(branch: Option<String>, list: bool, yes: bool) -> Result<()> {
    if CHANNEL == "unknown" {
        debug_log::log_kv(
            "update.run",
            &[
                debug_log::field("phase", "start"),
                debug_log::field("result", "error"),
                debug_log::field("reason", "not_installed"),
            ],
        );
        anyhow::bail!("This build was not installed from a release. Use install.sh to install.");
    }

    debug_log::log_kv(
        "update.run",
        &[
            debug_log::field("phase", "start"),
            debug_log::field("channel", CHANNEL),
            debug_log::field("target", branch.as_deref().unwrap_or("stable")),
            debug_log::field("list", list),
        ],
    );

    let client = build_client()?;

    if list {
        return list_branches(&client).await;
    }

    let target = branch.as_deref().unwrap_or("stable");
    if CHANNEL != target {
        if !confirm_channel_switch(target, yes)? {
            debug_log::log_kv(
                "update.run",
                &[
                    debug_log::field("phase", "cancel"),
                    debug_log::field("reason", "channel_switch_declined"),
                ],
            );
            println!("Cancelled.");
            return Ok(());
        }
    }

    match branch {
        Some(name) => update_branch(&client, &name).await,
        None => update_stable(&client).await,
    }
}
