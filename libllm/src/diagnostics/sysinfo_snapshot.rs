//! Deterministic `SystemInfo` / `TerminalInfo` snapshots consumed by the banner.

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

pub struct SystemInfo {
    pub host: String,
    pub os_name: String,
    pub os_version: String,
    pub kernel: String,
    pub arch: String,
    pub family: String,
    pub cpu_brand: String,
    pub logical_cpus: usize,
    pub total_memory_bytes: u64,
}

pub struct TerminalInfo {
    pub term: String,
    pub columns: Option<u16>,
    pub rows: Option<u16>,
    pub shell: String,
    pub locale: String,
}

pub(super) fn collect_system() -> SystemInfo {
    let kind = RefreshKind::nothing()
        .with_cpu(CpuRefreshKind::nothing())
        .with_memory(MemoryRefreshKind::nothing().with_ram());
    let system = System::new_with_specifics(kind);
    let cpu_brand = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    SystemInfo {
        host: System::host_name().unwrap_or_else(|| "unknown".to_owned()),
        os_name: System::name().unwrap_or_else(|| "unknown".to_owned()),
        os_version: System::os_version().unwrap_or_else(|| "unknown".to_owned()),
        kernel: System::kernel_version().unwrap_or_else(|| "unknown".to_owned()),
        arch: std::env::consts::ARCH.to_owned(),
        family: std::env::consts::FAMILY.to_owned(),
        cpu_brand,
        logical_cpus: system.cpus().len(),
        total_memory_bytes: system.total_memory(),
    }
}

pub(super) fn collect_terminal() -> TerminalInfo {
    let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_owned());
    let (columns, rows) = terminal_size();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_owned());
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "unknown".to_owned());
    TerminalInfo {
        term,
        columns,
        rows,
        shell,
        locale,
    }
}

fn terminal_size() -> (Option<u16>, Option<u16>) {
    let cols = std::env::var("COLUMNS").ok().and_then(|s| s.parse().ok());
    let rows = std::env::var("LINES").ok().and_then(|s| s.parse().ok());
    (cols, rows)
}
