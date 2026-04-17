//! Diagnostics: banner, tracing subscriber, file log layer, timing aggregation.

mod banner;
mod format;
mod subscriber;
mod sysinfo_snapshot;
mod timings;

pub use banner::{render, BannerContext, BuildInfo, RuntimeInfo};
pub use format::FileLayer;
pub use sysinfo_snapshot::{SystemInfo, TerminalInfo, collect_system, collect_terminal};
pub use timings::{TimingCollector, TimingLayer};
