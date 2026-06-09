//! Self-update mechanism via GitHub release downloads.
//!
//! Entry point is [`run`]. [`build_client`] and [`fetch_release`] are also used
//! by the legacy-migration path.

mod github;
mod install;
mod version;

pub use github::{Asset, BranchEntry, Release, build_branch_list, build_client, fetch_release};

use anyhow::Result;

use install::{confirm_channel_switch, confirm_downgrade, update_stable, update_to_tag};
use version::{is_stable_target, normalize_tag, should_warn_downgrade};

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
        None if crate::interactive::is_interactive() => match github::pick_branch(&client).await? {
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
