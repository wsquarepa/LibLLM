# RULES.md

The canonical source for this file is `.agents/RULES.md`. Both `CLAUDE.md` and `AGENTS.md` in the repo root are symlinks to it.

## What This Is

Read `README.md` for a full project overview, CLI reference, data directory layout, encryption details, and configuration guide. This file covers only what an agent needs beyond that.

LibLLM is a Rust TUI/CLI chat client for the llama.cpp completions API. It is a Cargo workspace with three crates: `libllm` (shared library), `client` (main binary with TUI/CLI), and `backup` (backup and recovery library).

## Build and Test

```sh
cargo build --workspace
cargo test --workspace
```

CI runs `cargo test --workspace` on all pushes and PRs. Run tests locally before submitting changes.

### Test suites

Integration tests live in `client/tests/` across six files: `core_data`, `content_management`, `request_pipeline`, `infrastructure`, `tui_business`, `smoke`. Unit tests live in `libllm/src/db/` sub-modules. Shared helpers are in `client/tests/common/mod.rs`.

### Verifying test results

`cargo test --workspace` runs multiple binaries. Some may report `0 tests`. Do not use `tail` to check results. Instead:

```sh
cargo test --workspace 2>&1 | grep -E "^test result:"
```

Every line must show `0 failed`.

### OnceLock constraint

`config::set_data_dir()` uses `OnceLock` and can only be called once per process. Only `client/tests/infrastructure.rs` owns this call -- other test files must pass explicit paths instead of relying on `data_dir()`.

## Architecture Gotchas

These are non-obvious patterns that cannot be inferred from a quick code read.

### CLI Override System

CLI flags that overlap with `/config` fields are tracked in `CliOverrides` (in `client/src/cli.rs`). Overridden fields display in red in the `/config` dialog and cannot be edited. The `-r` flag forces `/system` read-only; `-p` forces `/persona` read-only. Both show content in red.

### Statusbar

The statusbar default info line (model, template, tokens, branch) is sacred -- always visible unless a temporary message is active. Temporary messages use `App::set_status()` with `StatusLevel` (Info/Warning/Error) and auto-clear after 5 seconds. Do not add hints that duplicate info already visible in borders or obvious UI state.

### Theme colors

All colors in `tui/render.rs` must read from `app.theme` -- no hardcoded color constants.

### Diagnostics authoring

When modifying instrumented paths (startup, session I/O, rendering), maintain diagnostics coverage with `debug_log::log_kv()`, `debug_log::timed_kv()`, or `debug_log::timed_result()`. Timing data feeds the `--timings` report; do not write inline elapsed lines to the debug log.

### Conversation tree

Messages form a tree (`MessageTree` in `libllm/src/session.rs`) using an arena (`Vec<Node>` + `NodeId`). `/retry` and `/edit` create sibling branches. `branch_path()` walks from head to root.
