//! Repository automation. Run via the `cargo xtask` alias.
//!
//! Commands:
//! - `cargo xtask ci` — the full verification suite (fmt, clippy, test, doc).
//! - `cargo xtask release X.Y.Z` — bump the workspace version in `Cargo.toml`.

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("release") => release(args.get(1).map(String::as_str)),
        _ => {
            eprintln!("usage: cargo xtask <ci|release X.Y.Z>");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The xtask manifest is always at `<workspace_root>/xtask`, so the parent of
/// `CARGO_MANIFEST_DIR` is the workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest dir always has a workspace-root parent")
        .to_path_buf()
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let program = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let status = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .status()
        .map_err(|err| format!("failed to spawn `cargo {}`: {err}", args.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

fn ci() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    cargo(&["doc", "--workspace", "--no-deps"])?;
    Ok(())
}

fn release(version: Option<&str>) -> Result<(), String> {
    let version = version.ok_or("release requires a version, e.g. `cargo xtask release 3.2.0`")?;
    validate_version(version)?;
    let manifest = workspace_root().join("Cargo.toml");
    let contents = std::fs::read_to_string(&manifest)
        .map_err(|err| format!("failed to read {}: {err}", manifest.display()))?;
    let updated = replace_workspace_version(&contents, version)?;
    std::fs::write(&manifest, updated)
        .map_err(|err| format!("failed to write {}: {err}", manifest.display()))?;
    println!("Bumped workspace version to {version}.");
    println!("Next: commit the bump, then `git tag v{version} && git push origin v{version}`.");
    Ok(())
}

fn validate_version(version: &str) -> Result<(), String> {
    let parts: Vec<&str> = version.split('.').collect();
    let well_formed = parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    if well_formed {
        Ok(())
    } else {
        Err(format!(
            "version must be X.Y.Z with numeric parts, got `{version}`"
        ))
    }
}

fn replace_workspace_version(contents: &str, version: &str) -> Result<String, String> {
    let mut out = String::with_capacity(contents.len());
    let mut in_workspace_package = false;
    let mut replaced = false;
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_workspace_package = trimmed.starts_with("[workspace.package]");
        }
        if in_workspace_package
            && !replaced
            && trimmed.starts_with("version")
            && trimmed.contains('=')
        {
            out.push_str(&format!("version = \"{version}\"\n"));
            replaced = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if replaced {
        Ok(out)
    } else {
        Err("could not find a version key under [workspace.package]".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{replace_workspace_version, validate_version};

    #[test]
    fn rewrites_only_the_workspace_package_version() {
        let input = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nversion = \"1.0.0\"\nedition = \"2024\"\n\n[workspace.dependencies]\nserde = \"1\"\n";
        let out = replace_workspace_version(input, "2.3.4").unwrap();
        assert!(out.contains("version = \"2.3.4\""));
        assert!(out.contains("edition = \"2024\""));
        assert!(out.contains("serde = \"1\""));
    }

    #[test]
    fn missing_workspace_package_version_is_an_error() {
        let input = "[workspace]\nmembers = [\"a\"]\n";
        assert!(replace_workspace_version(input, "2.3.4").is_err());
    }

    #[test]
    fn rejects_malformed_versions() {
        assert!(validate_version("1.2").is_err());
        assert!(validate_version("1.2.x").is_err());
        assert!(validate_version("1.2.3").is_ok());
    }
}
