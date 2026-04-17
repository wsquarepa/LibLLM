//! Version string rendered by `--version` and the diagnostics banner.

pub const LONG: &str = concat!(
    "LibLLM version ",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("LIBLLM_VERSION_DESCRIPTOR"),
    ")"
);
