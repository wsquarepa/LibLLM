# RULES.md

The canonical source for this file is `.agents/RULES.md`. Both `CLAUDE.md` and `AGENTS.md` in the repo root are symlinks to it.

## What This Is

LibLLM is a Rust TUI/CLI chat client for the llama.cpp completions API, organized as a Cargo workspace under `crates/`. Read `README.md` for the user-facing overview and `.agents/STANDARDS.md` for crate roles, boundaries, and coding conventions. Domain vocabulary is in `CONTEXT.md`; durable decisions are in `docs/adr/`; the release procedure is in `docs/releasing.md`.

## Build and Test

`cargo xtask ci` is the one command for the full verification suite. It runs, in order and stopping at the first failure: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p libllm-tui --all-targets --features test-support -- -D warnings`, `cargo test --workspace`, `cargo test -p libllm-tui --features test-support`, and `cargo doc --workspace --no-deps`. CI runs the same checks on every push and PR.

Every step is teed live to your terminal and to a single timestamped log (`<tempdir>/libllm-ci-<date>-<time>-<rand>.log`). The path is printed at the start (`xtask ci: logging to …`) and again on success. Re-running the suite just to see its output burns 1-to-5 minutes of CPU; re-read that log instead.

### Builds take time

`cargo xtask ci` typically takes 1 to 5+ minutes. A clean run produces no `error:` or `warning:` lines and exits 0.

**Controller agents (the main conversation):** back `cargo xtask ci` (and any long build) with `run_in_background: true`, then **end your turn**. Do not emit further tool calls or text until the completion notification fires. Do not poll, re-run, or start a second run while one is in flight; duplicate runs burn CPU and block on lock contention.

**Dispatched subagents:** do **not** background it. Run it synchronously in the foreground with `timeout: 600000` (Bash's maximum) and read the output inline. If it ends up backgrounded by accident, stop and report BLOCKED.

### Verifying results

Grep the log; do not `tail` it. Below, `<log>` is the printed path.

Every test binary must show `0 failed` (some legitimately report `0 tests`):

```sh
grep -E "^test result:" <log>
```

When a line shows `FAILED`, the failing test's full stdout is already in the log; do not re-run a narrower `cargo test -p ...` to see it again:

```sh
grep -B2 -A20 -E "FAILED|^---- .* stdout ----" <log>
```

Clippy is clean when this is empty:

```sh
grep -E "^error|^warning:" <log>
```

Only re-run `cargo xtask ci` after editing code to fix a failure.

### Test suites

Integration tests live in `crates/cli/tests/` across sixteen files: `author_note_injection`, `business_logic`, `cli`, `configuration`, `content`, `danger_subprocess`, `danger_tab`, `db_subcommand`, `file_summary`, `group_chats`, `import_subcommand`, `persistence`, `recover_subcommand`, `regex_rules`, `template_detection`, `tokenization`. Keep this list in sync when adding or removing a test binary. Shared helpers are in `crates/cli/tests/common/mod.rs`; each binary compiles its own copy of `mod common;` and uses a different subset, so the declaration carries `#[expect(dead_code, reason = "...")]`, never `#[allow]`.

`db_subcommand`, `import_subcommand`, and `recover_subcommand` spawn the compiled `libllm` binary via `common::client_bin()` to test the CLI contract end-to-end (exit codes, stderr/stdout split, env-var passkey, `--no-encrypt` data dirs). Use `.output()`, not `.status()`, so stderr is captured in failure messages. The `update` subcommand needs network access and the `edit` subcommand needs an `$EDITOR` mock; neither is subprocess-tested.

### OnceLock constraint

`config::set_data_dir()` uses `OnceLock` and can only be called once per process. Each integration-test binary is a separate process, so the rule applies per-binary: the first call in a binary uses `.unwrap()`; subsequent calls in other tests of the same binary use `.ok()` to tolerate "already set". When in doubt, pass an explicit path through your call chain instead of relying on `data_dir()`.

## Commit messages

Keep commits to a single subject line: `type(scope): summary`. No body, no bullet points. If a change cannot be summarized in one line, split it into multiple commits.

## Agent skills

### Issue tracker

Issues and specs live as markdown files under `.scratch/<feature>/` in this repo. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles use their default names (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` at the repo root is the glossary and `docs/adr/` holds decisions. See `docs/agents/domain.md`.
