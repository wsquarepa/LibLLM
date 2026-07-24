//! Repository automation. Run via the `cargo xtask` alias.
//!
//! Commands:
//! - `cargo xtask ci` — the full verification suite (fmt, clippy, test, doc),
//!   teed to a single timestamped log under the temp dir.
//! - `cargo xtask release X.Y.Z` — bump the workspace version in `Cargo.toml`.
//! - `cargo xtask scenario <file> [--bless]` — replay a `.scenario` file.

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use chrono::Utc;
use rand::Rng;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("release") => release(args.get(1).map(String::as_str)),
        Some("scenario") => scenario(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask <ci|release X.Y.Z|scenario <file> [--bless]>");
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

/// Runs the full verification suite, teeing every step to the terminal and to a single
/// timestamped log so a failed run can be inspected afterward. Stops at the first
/// failing step.
fn ci() -> Result<(), String> {
    let log_path = ci_log_path();
    let file = create_ci_log(&log_path)?;
    let log = Arc::new(Mutex::new(file));
    println!("xtask ci: logging to {}", log_path.display());

    run_step(&log, &log_path, &["fmt", "--all", "--check"])?;
    run_step(
        &log,
        &log_path,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_step(
        &log,
        &log_path,
        &[
            "clippy",
            "-p",
            "libllm-tui",
            "--all-targets",
            "--features",
            "test-support",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_step(&log, &log_path, &["test", "--workspace"])?;
    run_step(
        &log,
        &log_path,
        &["test", "-p", "libllm-tui", "--features", "test-support"],
    )?;
    run_step(&log, &log_path, &["doc", "--workspace", "--no-deps"])?;

    println!("xtask ci: all checks passed (log: {})", log_path.display());
    Ok(())
}

/// `<tempdir>/libllm-ci-YYYYMMDD-HHMMSSmmm-<6 hex>.log`. The millisecond timestamp plus
/// a random suffix keeps every run's log distinct, so repeated or concurrent runs never
/// clobber each other.
fn ci_log_path() -> PathBuf {
    let mut suffix_bytes = [0u8; 3];
    rand::rng().fill_bytes(&mut suffix_bytes);
    let suffix: String = suffix_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let stamp = Utc::now().format("%Y%m%d-%H%M%S%3f");
    env::temp_dir().join(format!("libllm-ci-{stamp}-{suffix}.log"))
}

/// Creates the CI log at `path` exclusively (`O_CREAT|O_EXCL`) with owner-only mode
/// 0600 on Unix, so other local users cannot read run output and an existing path
/// (including a symlink) cannot be overwritten.
fn create_ci_log(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))
}

/// Spawns `cargo <args>` in the workspace root, streaming its stdout and stderr to both
/// this process's terminal and the shared log. Errors if the command cannot be spawned
/// or exits non-zero.
fn run_step(log: &Arc<Mutex<File>>, log_path: &Path, args: &[&str]) -> Result<(), String> {
    let program = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let banner = format!("\n===== cargo {} =====\n", args.join(" "));
    print!("{banner}");
    io::stdout().flush().ok();
    write_log(log, banner.as_bytes())?;

    let mut child = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn `cargo {}`: {err}", args.join(" ")))?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out_pump = pump(stdout, Box::new(io::stdout()), Arc::clone(log));
    let err_pump = pump(stderr, Box::new(io::stderr()), Arc::clone(log));

    let status = child
        .wait()
        .map_err(|err| format!("failed to wait on `cargo {}`: {err}", args.join(" ")))?;
    out_pump.join().expect("stdout pump thread panicked")?;
    err_pump.join().expect("stderr pump thread panicked")?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`cargo {}` failed; see {}",
            args.join(" "),
            log_path.display()
        ))
    }
}

/// Tees one child stream, line by line, to `sink` (the terminal) and to the shared log.
/// A terminal write that fails (e.g. a closed pipe) is non-fatal; a failed log write is
/// propagated, because the log is the authoritative record of the run.
fn pump(
    reader: impl Read + Send + 'static,
    mut sink: Box<dyn Write + Send>,
    log: Arc<Mutex<File>>,
) -> thread::JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = reader
                .read_until(b'\n', &mut line)
                .map_err(|err| format!("failed reading command output: {err}"))?;
            if read == 0 {
                return Ok(());
            }
            sink.write_all(&line).ok();
            sink.flush().ok();
            write_log(&log, &line)?;
        }
    })
}

fn write_log(log: &Arc<Mutex<File>>, bytes: &[u8]) -> Result<(), String> {
    let mut file = log.lock().map_err(|_| "ci log mutex poisoned".to_owned())?;
    file.write_all(bytes)
        .map_err(|err| format!("failed writing to ci log: {err}"))
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

fn scenario(rest: &[String]) -> Result<(), String> {
    let program = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut cargo = std::process::Command::new(program);
    cargo.args([
        "run",
        "-p",
        "libllm-tui",
        "--features",
        "test-support",
        "--example",
        "scenario_runner",
        "--",
    ]);
    cargo.args(rest);
    let status = cargo
        .status()
        .map_err(|e| format!("failed to spawn cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("scenario run failed".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{ci_log_path, create_ci_log, replace_workspace_version, validate_version};

    #[test]
    fn ci_log_path_is_a_timestamped_temp_log() {
        let path = ci_log_path();
        assert!(path.starts_with(std::env::temp_dir()));
        let name = path.file_name().unwrap().to_str().unwrap();
        let stem = name
            .strip_prefix("libllm-ci-")
            .and_then(|rest| rest.strip_suffix(".log"))
            .expect("filename has the `libllm-ci-<date>-<time>-<rand>.log` shape");
        let parts: Vec<&str> = stem.split('-').collect();
        assert_eq!(parts.len(), 3, "expected date-time-suffix, got {stem}");
        assert_eq!(parts[0].len(), 8, "YYYYMMDD");
        assert_eq!(parts[1].len(), 9, "HHMMSSmmm");
        assert_eq!(parts[2].len(), 6, "random hex suffix");
        assert!(
            parts
                .iter()
                .all(|part| part.bytes().all(|byte| byte.is_ascii_hexdigit()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn create_ci_log_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;

        let path = ci_log_path();
        let _file = create_ci_log(&path).expect("create_ci_log should succeed on a fresh path");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            mode & 0o777,
            0o600,
            "ci log must be owner read/write only"
        );
    }

    #[test]
    fn create_ci_log_rejects_existing_path() {
        let path = ci_log_path();
        let _first = create_ci_log(&path).expect("first create should succeed");
        let err = create_ci_log(&path).expect_err("second create must refuse an existing path");
        let _ = std::fs::remove_file(&path);
        assert!(
            err.contains("failed to create"),
            "unexpected error: {err}"
        );
    }

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
