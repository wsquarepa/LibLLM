//! Banner rendering: produces the fixed-width ASCII header written before tracing installs.

use std::fmt::Write;

use crate::diagnostics::sysinfo_snapshot::{SystemInfo, TerminalInfo};

pub struct BuildInfo {
    pub version: &'static str,
    pub channel: &'static str,
    pub commit: &'static str,
    pub dirty: bool,
}

pub(super) struct RuntimeInfo {
    pub run_mode: String,
    pub pid: u32,
    pub executable: String,
    pub working_dir: String,
    pub cli_args: String,
    pub debug_log_path: String,
    pub timings_path: String,
    pub filter_directive: String,
    pub filter_source: String,
}

pub(super) struct BannerContext<'a> {
    pub build: &'a BuildInfo,
    pub system: &'a SystemInfo,
    pub terminal: &'a TerminalInfo,
    pub runtime: &'a RuntimeInfo,
    pub wall_clock: &'a str,
}

pub(super) fn render(ctx: &BannerContext<'_>) -> String {
    let mut out = String::with_capacity(2048);
    let border = "=".repeat(80);
    let subborder = "-".repeat(80);

    writeln!(&mut out, "{}", border).unwrap();
    writeln!(&mut out, "{}", header_line(ctx)).unwrap();
    writeln!(&mut out, "{}", border).unwrap();

    write_row(&mut out, "Run mode", &ctx.runtime.run_mode);
    write_row(&mut out, "PID", &ctx.runtime.pid.to_string());
    write_row(&mut out, "Executable", &ctx.runtime.executable);
    write_row(&mut out, "Working dir", &ctx.runtime.working_dir);
    write_row(&mut out, "CLI args", &ctx.runtime.cli_args);

    writeln!(&mut out, "{}", subborder).unwrap();
    write_row(&mut out, "Host", &ctx.system.host);
    write_row(
        &mut out,
        "OS",
        &format!(
            "{} (kernel {})",
            combined_os(&ctx.system.os_name, &ctx.system.os_version),
            ctx.system.kernel
        ),
    );
    write_row(&mut out, "Arch", &format!("{} ({})", ctx.system.arch, ctx.system.family));
    write_row(
        &mut out,
        "CPU",
        &format!("{} ({} logical cores)", ctx.system.cpu_brand, ctx.system.logical_cpus),
    );
    write_row(&mut out, "Memory", &format_memory(ctx.system.total_memory_bytes));

    writeln!(&mut out, "{}", subborder).unwrap();
    write_row(&mut out, "Terminal", &format_terminal(ctx.terminal));
    write_row(&mut out, "Shell", &ctx.terminal.shell);
    write_row(&mut out, "Locale", &ctx.terminal.locale);

    writeln!(&mut out, "{}", subborder).unwrap();
    write_row(&mut out, "Debug log", &ctx.runtime.debug_log_path);
    write_row(&mut out, "Timings", &ctx.runtime.timings_path);
    write_row(
        &mut out,
        "Filter",
        &format!("{}  (source: {})", ctx.runtime.filter_directive, ctx.runtime.filter_source),
    );

    writeln!(&mut out, "{}", border).unwrap();
    writeln!(&mut out, "Events (offsets are +hh:mm:ss.sss from run start):").unwrap();
    writeln!(&mut out).unwrap();
    out
}

fn header_line(ctx: &BannerContext<'_>) -> String {
    let descriptor = build_descriptor(ctx.build);
    let name = format!("LibLLM version {} ({})", ctx.build.version, descriptor);
    let wall = ctx.wall_clock;
    let gap = 80usize.saturating_sub(1 + name.len() + wall.len() + 1);
    format!(" {name}{}{wall} ", " ".repeat(gap))
}

fn build_descriptor(build: &BuildInfo) -> String {
    let base = match build.channel {
        "stable" => format!("+{}", build.commit),
        "unknown" => "-dev".to_owned(),
        _ => format!("-{}", build.commit),
    };
    if build.dirty {
        format!("{base}+dirty")
    } else {
        base
    }
}

fn write_row(out: &mut String, label: &str, value: &str) {
    writeln!(out, " {:<13} {}", label, value).unwrap();
}

fn combined_os(os_name: &str, os_version: &str) -> String {
    if os_version.is_empty() || os_version == "unknown" {
        os_name.to_owned()
    } else {
        format!("{os_name} {os_version}")
    }
}

fn format_memory(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!("{:.1} GiB total", bytes as f64 / GIB)
}

fn format_terminal(info: &TerminalInfo) -> String {
    match (info.columns, info.rows) {
        (Some(c), Some(r)) => format!("{}  ({} x {})", info.term, c, r),
        _ => info.term.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (BuildInfo, SystemInfo, TerminalInfo, RuntimeInfo) {
        let build = BuildInfo {
            version: "1.0.0",
            channel: "feat/log-rework",
            commit: "a1b2c3d",
            dirty: false,
        };
        let system = SystemInfo {
            host: "orion".to_owned(),
            os_name: "Artemis Linux".to_owned(),
            os_version: "2".to_owned(),
            kernel: "6.0.0-artemis".to_owned(),
            arch: "x86_64".to_owned(),
            family: "unix".to_owned(),
            cpu_brand: "Saturn V Core".to_owned(),
            logical_cpus: 16,
            total_memory_bytes: 68_719_476_736,
        };
        let terminal = TerminalInfo {
            term: "xterm-256color".to_owned(),
            columns: Some(158),
            rows: Some(42),
            shell: "/usr/bin/fish".to_owned(),
            locale: "en_US.UTF-8".to_owned(),
        };
        let runtime = RuntimeInfo {
            run_mode: "tui".to_owned(),
            pid: 48291,
            executable: "/home/astronaut/.cargo/bin/libllm".to_owned(),
            working_dir: "/home/astronaut/mission-control".to_owned(),
            cli_args: "tui --persona assistant".to_owned(),
            debug_log_path: "/home/astronaut/libllm-debug.log".to_owned(),
            timings_path: "disabled".to_owned(),
            filter_directive: "info".to_owned(),
            filter_source: "default".to_owned(),
        };
        (build, system, terminal, runtime)
    }

    #[test]
    fn renders_full_banner() {
        let (build, system, terminal, runtime) = fixture();
        let ctx = BannerContext {
            build: &build,
            system: &system,
            terminal: &terminal,
            runtime: &runtime,
            wall_clock: "2026-04-17 11:12:51",
        };
        let out = render(&ctx);
        assert!(out.lines().next().unwrap().starts_with("================"));
        assert!(out.contains("LibLLM version 1.0.0 (-a1b2c3d)"));
        assert!(out.contains("2026-04-17 11:12:51"));
        assert!(out.contains("Run mode      tui"));
        assert!(out.contains("CPU           Saturn V Core (16 logical cores)"));
        assert!(out.contains("Memory        64.0 GiB total"));
        assert!(out.contains("Terminal      xterm-256color  (158 x 42)"));
        assert!(out.contains("Filter        info  (source: default)"));
        assert!(out.ends_with("Events (offsets are +hh:mm:ss.sss from run start):\n\n"));
        for line in out.lines().take_while(|l| l.starts_with('=') || l.starts_with('-') || l.starts_with(' ')) {
            assert!(line.chars().count() <= 80, "banner line exceeds 80 cols: {:?}", line);
        }
    }

    #[test]
    fn stable_build_uses_plus_sha_descriptor() {
        let (mut build, system, terminal, runtime) = fixture();
        build.channel = "stable";
        build.dirty = false;
        let ctx = BannerContext {
            build: &build,
            system: &system,
            terminal: &terminal,
            runtime: &runtime,
            wall_clock: "2026-04-17 11:12:51",
        };
        let out = render(&ctx);
        assert!(out.contains("LibLLM version 1.0.0 (+a1b2c3d)"));
        assert!(!out.contains("master"));
    }

    #[test]
    fn dirty_stable_build_appends_dirty_marker() {
        let (mut build, system, terminal, runtime) = fixture();
        build.channel = "stable";
        build.commit = "deadbee";
        build.dirty = true;
        let ctx = BannerContext {
            build: &build,
            system: &system,
            terminal: &terminal,
            runtime: &runtime,
            wall_clock: "2026-04-17 11:12:51",
        };
        let out = render(&ctx);
        assert!(out.contains("LibLLM version 1.0.0 (+deadbee+dirty)"));
    }
}
