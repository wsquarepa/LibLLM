use std::process::Command;

fn main() {
    let hash = git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let dirty = match Command::new("git").args(["diff-index", "--quiet", "HEAD", "--"]).status() {
        Ok(status) if status.success() => "",
        Ok(_) => "+dirty",
        Err(_) => "",
    };
    let channel = std::env::var("LIBLLM_CHANNEL").unwrap_or_else(|_| "unknown".to_owned());

    let descriptor = match (channel.as_str(), hash.as_str()) {
        ("stable", sha) => format!("+{sha}{dirty}"),
        ("unknown", _) => format!("-dev{dirty}"),
        (_, sha) => format!("-{sha}{dirty}"),
    };

    println!("cargo:rustc-env=LIBLLM_COMMIT={hash}");
    println!("cargo:rustc-env=LIBLLM_GIT_DIRTY={dirty}");
    println!("cargo:rustc-env=LIBLLM_CHANNEL={channel}");
    println!("cargo:rustc-env=LIBLLM_VERSION_DESCRIPTOR={descriptor}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-env-changed=LIBLLM_CHANNEL");
}

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}
