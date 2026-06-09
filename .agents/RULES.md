# RULES.md

The canonical source for this file is `.agents/RULES.md`. Both `CLAUDE.md` and `AGENTS.md` in the repo root are symlinks to it.

## What This Is

Read `README.md` for a full project overview, CLI reference, data directory layout, encryption details, and configuration guide. This file covers only what an agent needs beyond that.

LibLLM is a Rust TUI/CLI chat client for the llama.cpp completions API. It is a Cargo workspace whose members live under `crates/`:

- `libllm-core` (`crates/core`) — pure domain: session tree, character/persona/world-info, presets, file-ingestion pipeline, crypto, config types. No database, network, async runtime, or process/global state.
- `libllm-storage` (`crates/storage`) — SQLCipher database access, migrations, repositories, and FTS search. Depends on core.
- `libllm-protocol` (`crates/protocol`) — the llama.cpp HTTP API client, tokenizer, and summarization orchestration. Depends on core.
- `libllm-config` (`crates/config`) — process-boundary config: the data-directory resolution, the `DATA_DIR_OVERRIDE` global (with the `test-support` thread-local variant), and `config.toml` load/save. Note: config TYPES (`Config`, `Auth`, etc.) live in `libllm-core::config`; this crate holds the process-boundary FUNCTIONS (`data_dir`, `load`, `save`, `*_presets_dir`, ...). Depends on core.
- `libllm-diagnostics` (`crates/diagnostics`) — logging/diagnostics infrastructure: tracing-subscriber setup, the diagnostics global state, log-file management, the `--timings` report, and the startup banner's sysinfo collection. Depends on core. (The `timed_result!` macro stays in `libllm-core` because core itself uses it.)
- `libllm-tui` (`crates/tui`) — the TUI: rendering, dialogs, input handling, view state, and the `FileSummarizer` orchestrator. Depends on core/storage/protocol/config.
- `libllm-cli` (`crates/cli`) — argument parsing, command dispatch, subcommands, startup orchestration (`app::run`), and the thin `src/main.rs`. Produces the `libllm` binary (via `[[bin]] name = "libllm"`). Depends on `libllm-tui` and core/storage/protocol/config.
- `libllm-backup` (`crates/backup`) — backup and recovery library. Depends on core.

Dependencies flow inward: cli -> tui -> storage/protocol/config/diagnostics -> core. Core never depends on an outer crate. Application crates depend on the concrete library crates directly (`libllm_core`, `libllm_storage`, `libllm_protocol`, `libllm_config`, `libllm_diagnostics`) — there is no umbrella/facade crate.

## Build and Test

`cargo xtask ci` is the one command for the full verification suite. It runs, in order and stopping at the first failure: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p libllm-tui --all-targets --features test-support -- -D warnings`, `cargo test --workspace`, `cargo test -p libllm-tui --features test-support`, and `cargo doc --workspace --no-deps`.

Every step is teed live to your terminal and to a single timestamped log under the temp dir (`<tempdir>/libllm-ci-<date>-<time>-<rand>.log`, unique per run). The path is printed at the start (`xtask ci: logging to …`) and again on success. Re-running the suite just to see its output burns 1-to-5 minutes of CPU; re-read that log instead — it is the authoritative record of what went wrong and where.

CI runs the same checks on all pushes and PRs. Run `cargo xtask ci` locally before submitting changes.

### Builds take time

`cargo xtask ci` (like a cold `cargo build --workspace`) typically takes 1 to 5+ minutes. A clean run produces no `error:` or `warning:` lines and exits 0.

**Controller agents (the main conversation):** back `cargo xtask ci` (and any long build) with `run_in_background: true`, then **end your turn**. Do not emit further tool calls or text until the completion notification fires. "Waiting" means ending the turn, not polling the output file, checking status, starting unrelated work, or narrating progress. The notification is reliable; do not poll, re-run, or kick off a second run while one is in flight. Duplicate runs burn CPU and block the first one on lock contention.

**Dispatched subagents:** do **not** background it. Subagents do not reliably receive the completion notification, so the result is silently lost. Run it synchronously in the foreground with an explicit long timeout (e.g. `timeout: 600000` — 10 minutes — which is Bash's maximum). Block on the command and read the output inline. If it ends up backgrounded by accident, stop and report it as BLOCKED — do not try to work around by polling, sleeping, or launching a second run.

### Test suites

Integration tests live in `crates/cli/tests/` across sixteen files: `author_note_injection`, `business_logic`, `cli`, `configuration`, `content`, `danger_subprocess`, `danger_tab`, `db_subcommand`, `file_summary`, `group_chats`, `import_subcommand`, `persistence`, `recover_subcommand`, `regex_rules`, `template_detection`, `tokenization`. Keep this list in sync when adding or removing a test binary. Unit tests live in `crates/storage/src/db/` sub-modules and in `crates/cli/src/cli/db/{parser,format}.rs`. Shared helpers are in `crates/cli/tests/common/mod.rs`. Each integration test binary compiles its own copy of `mod common;` and uses a different subset of the helpers — use `#[expect(dead_code, reason = "...")]` on the `mod common;` declaration, never `#[allow]`.

**Subprocess integration tests:** Three test binaries (`db_subcommand`, `import_subcommand`, `recover_subcommand`) spawn the compiled `libllm` binary via `common::client_bin()` to exercise the CLI surface end-to-end (exit codes, stderr/stdout split, env-var passkey, `--no-encrypt` data dirs). Use this pattern when the contract being tested is the CLI itself — argument parsing, exit codes, confirmation prompts, multi-process safety. Use `.output()` (not `.status()`) so stderr is captured in failure messages. The `update` subcommand is deliberately not subprocess-tested because it depends on network access; the `edit` subcommand would need an `$EDITOR` mock and is also currently uncovered at this level.

### Verifying results

`cargo xtask ci` prints its log path (`xtask ci: logging to <path>`). That single file holds the fmt, clippy, test, and doc output in run order. Do not use `tail`; grep the log. Below, `<log>` is that path.

The test step runs multiple binaries, some of which legitimately report `0 tests`. Every result line must show `0 failed`:

```sh
grep -E "^test result:" <log>
```

When a line shows `FAILED`, investigate from the same log -- **do not re-run a narrower `cargo test -p ...` just to see the failure output again.** The full stdout of the failing test is already there:

```sh
grep -B2 -A20 -E "FAILED|^---- .* stdout ----" <log>
```

Clippy is clean when this is empty:

```sh
grep -E "^error|^warning:" <log>
```

Only re-run `cargo xtask ci` after you've edited code to fix a failure, not to re-read its output.

### No warning suppression

Never silence compiler warnings with `#[allow(...)]` attributes, `#![allow(...)]` inner attributes, `RUSTFLAGS=-Awarnings`, or any equivalent mechanism. Fix the underlying code instead.

- Dead code → delete it.
- Unreachable expression → restructure control flow so the path is reachable, or remove the dead branch.
- Unused import → delete it.
- Unused variable → delete it or use it.

The workspace enforces this via `[workspace.lints.clippy] allow_attributes = "deny"` in the root `Cargo.toml`; the clippy step of `cargo xtask ci` (`cargo clippy --workspace --all-targets -- -D warnings`) fails if any `#[allow(...)]` is present. A dedicated `clippy` job in `.github/workflows/{check,build}.yml` runs the same lint on every PR and push; `cargo xtask ci` runs it locally before you push. Read clippy results back from the ci log as shown under [Verifying results](#verifying-results) — do not re-run just to re-read its output.

`#[expect(lint, reason = "...")]` is permissible for documented structural cases that are not real bugs. It is self-verifying: if the underlying warning stops firing, `expect` itself warns, forcing a follow-up cleanup. Example: each `crates/cli/tests/*.rs` binary compiles its own copy of `mod common;` and uses a different subset of the helpers, which makes `dead_code` fire legitimately per-binary. The fix is `#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]`, not `#[allow]`. Any `#[expect]` must carry a `reason` explaining the structural cause.

### OnceLock constraint

`config::set_data_dir()` uses `OnceLock` and can only be called once per process. Each integration-test binary is a separate process, so the rule applies per-binary. Within a binary, the first call should use `.unwrap()` (it owns the OnceLock); subsequent calls in other tests of the same binary must use `.ok()` to tolerate "already set" without failing. Tests in unrelated binaries can each own their own first call. When in doubt, pass an explicit path through your call chain instead of relying on `data_dir()`.

## Commit messages

Keep commits to a single subject line: `type(scope): summary`. No body, no bullet points, no multi-paragraph prose. Context that doesn't fit in the subject belongs in the diff, the PR description, or the issue tracker -- not in the commit message. If a change genuinely cannot be summarized in one line, split it into multiple commits.

## Release process

Stable releases are tag-driven, not push-driven. A push to master runs tests and clippy only -- it does **not** produce a release. To cut a stable release:

1. Bump `workspace.package.version` in `Cargo.toml` and merge the bump into `master` (a `chore(release): bump workspace version to X.Y.Z` commit).
2. After the bump lands, push a matching tag: `git tag vX.Y.Z && git push origin vX.Y.Z`. The `v` prefix is required.

CI rejects mismatches between the tag (`vX.Y.Z`) and the Cargo workspace version (`X.Y.Z`). When the user asks to "bump version" or "cut a release", both steps are needed -- bumping the version alone produces no release.

Backports are handled automatically: if `vX.Y.Z` is older than the highest existing v-tag at push time, CI marks the new release `--latest=false` so the newer release stays current. Branch builds (nightly prereleases on every non-`master` branch push) are unaffected by this scheme.

The workflow refuses to build a branch named `stable` or one matching `vX.Y.Z`; those names are reserved for stable-channel releases.

## Architecture Gotchas

These are non-obvious patterns that cannot be inferred from a quick code read.

### CLI Override System

CLI flags that overlap with `/config` fields are tracked in `CliOverrides` (defined in `libllm-core`'s `config` module, re-exported via `libllm-cli`'s `cli` module; the clap-derived arg wrappers like `AuthKindArg` stay in `crates/cli/src/cli/mod.rs`). Overridden fields display in red in the `/config` dialog and cannot be edited. The `-r` flag forces `/system` read-only; `-p` forces `/persona` read-only. Both show content in red.

### Statusbar

The statusbar's default content -- the version/build status (left) and the keybind hints (right) -- is sacred: always visible unless a temporary message is active. Temporary messages use `App::set_status()` with `StatusLevel` (Info/Warning/Error) and auto-clear after 5 seconds; Info/Warning slide in over the hints on the right, Error takes over the whole bar centered. The model name and token count live in the chat block's bottom border, the per-message branch indicator (`[1/2]`) renders inline in the chat, and the input token estimate is an input-box title -- none of these are in the statusbar. Do not add hints that duplicate info already visible in borders or obvious UI state.

### Theme colors

All colors in `tui/render.rs` must read from `app.theme` -- no hardcoded color constants.

### Diagnostics authoring

Emit structured events with `tracing::trace!`, `tracing::debug!`, `tracing::info!`, `tracing::warn!`, or `tracing::error!`. Pick levels using the `libllm-diagnostics` rubric:

- `TRACE` — per-frame or per-keystroke events (render, input, layout).
- `DEBUG` — state transitions, config reads, background task lifecycle.
- `INFO` — DB operations, migrations, session save/load, API summaries.
- `WARN` — retries, degraded fallbacks, recoverable problems.
- `ERROR` — unrecoverable failures.

For timed blocks, use `tracing::info_span!("name", field = value).entered()` or the `libllm_core::timed_result!` macro (which records `elapsed_ms` and `result=ok|error` automatically). Span close feeds the `--timings` report; do not write elapsed fields manually.

Default filter is `info`. Users override via `--log-filter <DIRECTIVE>` (requires `--debug`) or `LIBLLM_LOG` (ignored unless `--debug` is set). Both take `env_logger`-style directives, e.g. `info,libllm_storage::db=debug,libllm_tui::render=off`.

### Conversation tree

Messages form a tree (`MessageTree` in `crates/core/src/session.rs`) using an arena (`Vec<Node>` + `NodeId`). `/retry` and `/edit` create sibling branches. `branch_path()` walks from head to root.

### Database migrations

Migrations live under `crates/storage/src/db/migrations/` — one file per version (`v1.rs`, `v2.rs`, ...), each exposing `pub(super) fn migrate(conn: &Connection) -> Result<()>`. `migrations/mod.rs` owns the `CURRENT_VERSION` constant, the `run_migrations` dispatch loop, the `stamp_version` / `apply_migration` helpers, and the cross-version tests. Adding a new migration is three touches:

1. Create `crates/storage/src/db/migrations/v{N}.rs` with the `pub(super) fn migrate` body.
2. Add `mod v{N};` to `migrations/mod.rs`.
3. Bump `CURRENT_VERSION = N` and append `if version < N { apply_migration(conn, N, v{N}::migrate)?; applied += 1; }` to `run_migrations`.

`apply_migration` runs each migration and stamps its version inside one transaction, so a crash mid-upgrade rolls back cleanly instead of leaving a half-applied schema. Individual `migrate` bodies therefore stay plain `&Connection` statements and must **not** open their own transaction.

Migrations run exactly once per process: `Database::open` (in `crates/storage/src/db/mod.rs`) calls `migrations::run_migrations(&conn)` on the main connection after applying the SQLCipher key. The `FileSummarizer`'s dedicated second connection (built in the `libllm-tui` crate, `crates/tui/src/file_summarizer.rs`) does **not** run migrations — it observes the already-migrated schema over SQLite's WAL file locking.

### `libllm db` subcommand group

`crates/cli/src/cli/db/` exposes `db {sql, shell, dump, import}` for direct database inspection and editing through the existing decryption pipeline. Read the README's "Direct database access" section for user-facing semantics. Implementation gotchas:

- `sql` and `shell` open with `PRAGMA query_only = ON` and only lift it when launched with `--write`. All SQL routes through `Database::execute_query` plus `Database::changes()` for the affected-row count when there are no result columns — this handles `INSERT ... RETURNING`, bare `VALUES`, and comment-leading SQL uniformly. Do not reintroduce a leading-keyword heuristic.
- `import` always invokes `libllm_backup::snapshot::create_snapshot` before swapping the database file. There is no `--no-backup` flag; this is intentional. The pre-swap backup is the recovery story for any failure between `build_replacement` and `fs::rename`.
- `dump` and `import` both call `wal_liveness_check` (in `cli/db/mod.rs`) which probes for `SQLITE_BUSY` via `BEGIN IMMEDIATE; ROLLBACK;` to refuse if another LibLLM process holds the database. The check early-returns when the database file does not exist (otherwise `Connection::open` would silently create an empty file).
- Tmp-path computation in `dump` appends `.tmp` to the user's path (it does not use `Path::with_extension`, which would replace any existing `.tmp` and collide with the destination).
- Schema-version compatibility is gated on `libllm_storage::db::CURRENT_VERSION` (re-exported from `db::migrations`). If you add a new migration, both the import gate and the migration runner read the same constant — see the "Database migrations" section above.
- Standard exit codes shared across the group: `1` generic, `2` user declined, `3` schema-version mismatch, `4` WAL-liveness failure (constants in `cli/db/mod.rs::exit`).
- The shell uses `rustyline` with a `DotCommandOutcome::{Continue, Quit}` enum (NOT `std::process::exit`) so `save_history` runs on clean exit. A statement whose first input line begins with whitespace is excluded from both on-disk and in-memory history (bash `HISTCONTROL=ignorespace`).

## TUI dialog keybindings

Every dialog handler under `crates/tui/src/dialogs/` MUST follow this contract. Diverging is a review-blocking issue; if a dialog cannot conform, document the exception in this section.

| Key             | Action                                                          |
|-----------------|-----------------------------------------------------------------|
| `Up` / `Down`   | Move field focus. Never alias to anything else.                 |
| `Left` / `Right`| Adjust the focused field value (toggle, slider, radio cycle).   |
| `Tab` / `BackTab`  | Switch dialog tabs (when present). Never alias Down or Enter.   |
| `Enter`         | Activate the focused field (open editor / picker / commit row). |
| `Space`         | Toggle a boolean field, or pick a row in multi-select lists.    |
| `Ctrl+S`        | Save the dialog. Equivalent to focusing and pressing `[Save]`.  |
| `Esc`           | Close the dialog. If the dialog is dirty, push `UnsavedWarning` instead of closing directly. |

### Exceptions (documented intentional divergences)

- `file_picker.rs`: `Tab` descends into the selected folder or accepts the file
  (matches shell tab-complete UX). `Up/Down/Esc` still follow the contract.
- `paged_list.rs` (inside an active search): `Tab` commits the filter (equivalent
  to `Enter`). Outside search mode the contract applies.
- `set_passkey.rs`: `Tab` toggles between the two password fields (no other
  navigation possible in a 2-field dialog).

### Dirty-check / unsaved-changes warning

Editor dialogs that mutate persistent data MUST track a dirty bit and route Esc
through `crates/tui/src/dialogs/unsaved_warning.rs` when dirty. The warning
offers `[Save & Close] [Discard] [Cancel]` and is the only place where an
ambiguous "Esc on a modified dialog" decision is made.
