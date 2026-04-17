use std::process::Command;

#[test]
fn clippy_passes_workspace_wide() {
    let output = Command::new(env!("CARGO"))
        .args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--quiet",
        ])
        .env("CARGO_TARGET_DIR", env!("CARGO_TARGET_TMPDIR"))
        .output()
        .expect("failed to spawn cargo clippy");
    assert!(
        output.status.success(),
        "cargo clippy reported violations:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
