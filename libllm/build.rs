use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());

    let channel = std::env::var("LIBLLM_CHANNEL").unwrap_or_else(|_| "unknown".to_owned());

    println!("cargo:rustc-env=LIBLLM_COMMIT={hash}");
    println!("cargo:rustc-env=LIBLLM_CHANNEL={channel}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");
    println!("cargo:rerun-if-env-changed=LIBLLM_CHANNEL");
}
