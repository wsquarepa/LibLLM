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

### Builds take time -- run them in the background

`cargo build --workspace` and `cargo test --workspace` typically take 1 to 5+ minutes on a cold build. Do **not** run them synchronously in the foreground -- start them with `run_in_background: true` (or equivalent), then wait patiently for the completion notification before proceeding. Never poll, re-run, or kick off a second build while one is in flight; duplicate builds burn CPU and block the first one on lock contention.

After the background task completes, read the output file to check for errors and warnings. A clean run produces no `error:` or `warning:` lines.

### Test suites

Integration tests live in `client/tests/` across six files: `core_data`, `content_management`, `request_pipeline`, `infrastructure`, `tui_business`, `smoke`. Unit tests live in `libllm/src/db/` sub-modules. Shared helpers are in `client/tests/common/mod.rs`.

### Verifying test results

`cargo test --workspace` runs multiple binaries. Some may report `0 tests`. Do not use `tail` to check results. Instead:

```sh
cargo test --workspace 2>&1 | grep -E "^test result:"
```

Every line must show `0 failed`.

### No warning suppression

Never silence compiler warnings with `#[allow(...)]` attributes, `#![allow(...)]` inner attributes, `RUSTFLAGS=-Awarnings`, or any equivalent mechanism. Fix the underlying code instead.

- Dead code → delete it.
- Unreachable expression → restructure control flow so the path is reachable, or remove the dead branch.
- Unused import → delete it.
- Unused variable → delete it or use it.

The workspace enforces this via `[workspace.lints.clippy] allow_attributes = "deny"` in the root `Cargo.toml`; `cargo clippy --workspace --all-targets` fails if any `#[allow(...)]` is present. The `clippy_passes_workspace_wide` test in `client/tests/lints.rs` runs clippy under `cargo test --workspace`, so the gate is part of the normal test cycle.

`#[expect(lint, reason = "...")]` is permissible for documented structural cases that are not real bugs. It is self-verifying: if the underlying warning stops firing, `expect` itself warns, forcing a follow-up cleanup. Example: each `client/tests/*.rs` binary compiles its own copy of `mod common;` and uses a different subset of the helpers, which makes `dead_code` fire legitimately per-binary. The fix is `#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]`, not `#[allow]`. Any `#[expect]` must carry a `reason` explaining the structural cause.

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
