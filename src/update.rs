use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

const REPO: &str = "wsquarepa/LibLLM";
const TAG: &str = "nightly";

const TARGET: &str = const {
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
struct Release {
    body: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}

fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok()
        .filter(|t| !t.is_empty())
}

fn build_client() -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        "application/vnd.github+json"
            .parse()
            .expect("valid header value"),
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        "libllm-updater".parse().expect("valid header value"),
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

fn current_exe_path() -> Result<PathBuf> {
    std::env::current_exe().context("failed to determine current executable path")
}

pub async fn run() -> Result<()> {
    let client = build_client()?;

    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{TAG}");
    let resp = client
        .get(&url)
        .send()
        .await
        .context("failed to fetch release info")?;

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::UNAUTHORIZED {
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
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let release: Release = resp.json().await.context("failed to parse release JSON")?;

    let expected_name = if cfg!(target_os = "windows") {
        format!("libllm-{TARGET}.exe")
    } else {
        format!("libllm-{TARGET}")
    };

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == expected_name)
        .context(format!(
            "no asset found for this platform ({TARGET}) in the nightly release"
        ))?;
    if let Some(remote_hash) = parse_release_hash(&release.body) {
        let current_hash = env!("LIBLLM_COMMIT", "unknown");
        if current_hash != "unknown" && current_hash == remote_hash {
            println!("Already up to date (commit {current_hash}).");
            return Ok(());
        }
    }

    println!("Downloading {expected_name}...");

    let download_resp = client
        .get(&asset.url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("failed to download binary")?;
    if !download_resp.status().is_success() {
        let status = download_resp.status();
        anyhow::bail!("download failed with status {status}");
    }

    let bytes = download_resp
        .bytes()
        .await
        .context("failed to read download body")?;

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
        return Err(e).context("failed to install new binary");
    }
    let _ = std::fs::remove_file(&old_path);

    let hash_display = parse_release_hash(&release.body).unwrap_or("unknown");
    println!("Updated to nightly (commit {hash_display}).");
    Ok(())
}
