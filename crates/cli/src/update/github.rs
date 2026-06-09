//! GitHub Releases API types, client construction, and release fetching.

use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::REPO;
use super::version::parse_version_tag;

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

pub(super) async fn fetch_releases(client: &reqwest::Client) -> Result<Vec<Release>> {
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

pub(super) async fn pick_branch(client: &reqwest::Client) -> Result<Option<String>> {
    tracing::debug!(phase = "start", "update.interactive");
    let releases = fetch_releases(client).await?;
    let entries = build_branch_list(&releases, super::CHANNEL, env!("CARGO_PKG_VERSION"));

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
}
