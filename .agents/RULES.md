# RULES.md
This file contains details to **any programming agent** about what this repository is and what code style guidelines to follow.

## What This Is

LibLLM is a Rust TUI/CLI chat client for the llama.cpp completions API. It supports single-message mode (`-m`) and a full terminal UI (default), with tree-structured conversation history, encrypted session persistence (SQLite + SQLCipher), branch navigation, character cards, and worldbook/lorebook support.

## Project Structure

The project is a Cargo workspace with three crates:

```
LibLLM/
  Cargo.toml                    # workspace root
  libllm-core/                  # shared library (domain structs, database, config, crypto)
  libllm/                       # main binary (TUI, CLI, self-update)
  libllm-migrate/               # one-time migration binary (legacy file-based data -> SQLite)
```

- **`libllm-core`** -- shared library used by both binaries. Contains domain structs, SQLite database module, config, key derivation, presets, API client, export, and debug logging.
- **`libllm`** -- main binary with TUI, CLI argument parsing, and self-update.
- **`libllm-migrate`** -- one-time migration tool that converts legacy file-based data directories to the SQLite database format. Ships with its own AES-256-GCM decryption for reading old encrypted files.

## Build and Run

```sh
cargo build --workspace
cargo run -p libllm -- --help
cargo run -p libllm -- -m "Hello"                         # single message, ephemeral
cargo run -p libllm                                       # TUI mode, prompts for passkey
cargo run -p libllm -- -d ./data --no-encrypt             # custom data dir, plaintext
cargo run -p libllm -- -d ./data --no-encrypt -m "Hello"  # persistent single-shot
cargo run -p libllm -- -d ./data --no-encrypt -m "Follow up" --continue <uuid>
cargo run -p libllm -- --template chatml                  # use ChatML instruct preset
cargo run -p libllm -- --temperature 0.5                  # override sampling params
cargo run -p libllm -- -c character_name -p persona_name  # roleplay mode (requires both)
cargo run -p libllm -- -r "You are a helpful assistant"   # override system prompt
cargo run -p libllm -- edit character my_char             # edit character in $EDITOR
cargo run -p libllm -- edit worldbook my_book             # edit worldbook in $EDITOR
cargo run -p libllm -- import card.json                   # auto-detect and import (character or worldbook)
cargo run -p libllm -- import card.png                    # import PNG character card
cargo run -p libllm -- import --type persona note.txt     # import persona from .txt
cargo run -p libllm -- import --type prompt sys.txt       # import system prompt from .txt
cargo run -p libllm -- import a.json b.png c.json         # batch import multiple files
cargo run -p libllm -- update                             # update to stable (or stay on current channel)
cargo run -p libllm -- update feature/branch              # switch to a branch build
cargo run -p libllm -- update --list                      # list available branch builds
cargo run -p libllm -- update --yes                       # skip channel-switch confirmation
LIBLLM_PASSKEY=foo cargo run -p libllm -- -d ./data       # passkey via env var
```

The API URL defaults to `http://localhost:5001/v1` and can be overridden via `--api-url`, `LIBLLM_API_URL` env var, or config file.

CI runs `cargo test --workspace` before building on push to any branch (`.github/workflows/build.yml`) and on PRs (`.github/workflows/check.yml`). Pushing to master creates a `stable` release; pushing to other branches creates pre-releases tagged with the branch name. Run tests locally with `cargo test --workspace` before submitting changes.

## Testing

Integration tests live in `libllm/tests/` and are organized into six suites:

```sh
cargo test --workspace                  # run all tests
cargo test -p libllm --test core_data         # session and tree tests
cargo test -p libllm --test content_management # characters, worldbooks, prompts, personas
cargo test -p libllm --test request_pipeline  # preset rendering, sampling, context truncation
cargo test -p libllm --test infrastructure    # config, migrations
cargo test -p libllm --test tui_business      # template vars, command registry, business logic
cargo test -p libllm --test smoke             # end-to-end smoke tests
```

Unit tests for the database module live in `libllm-core/src/db/` (each sub-module has `#[cfg(test)]` tests).

Shared test helpers are in `libllm/tests/common/mod.rs` (temp dirs, key derivation, fixture builders).

`config::set_data_dir()` uses `OnceLock` and can only be called once per process. Only `libllm/tests/infrastructure.rs` owns this call -- other test files must pass explicit paths instead of relying on `data_dir()`.

### Verifying test results

`cargo test --workspace` runs multiple test binaries (unit tests, integration suites, doctests). Some binaries may report `0 tests` if they have no matching tests. **Do not use `tail` to check results** -- it only shows the last binary's output, which may be an empty suite. Instead, grep for all result lines:

```sh
cargo test --workspace 2>&1 | grep -E "^test result:"
```

Every line must show `0 failed`. If any line shows failures, the full output is needed to diagnose which tests failed.

## Data Directory

The default data directory is `~/.local/share/libllm/`. A custom path can be specified with `--data/-d`, which uses the given path directly (no subdirectory created).

```
<data_dir>/
├── config.toml              # API URL, template, sampling defaults (NOT encrypted)
├── data.db                  # SQLite database (SQLCipher-encrypted or plain)
├── .salt                    # 16-byte random salt (generated on first run)
├── .key_check               # Passkey verification fingerprint
└── presets/
    ├── instruct/            # Instruct presets (Mistral V3-Tekken, Llama 3, ChatML, Phi, Alpaca)
    ├── reasoning/           # Reasoning presets (DeepSeek)
    └── template/            # Context template presets (Default)
```

All sessions, characters, worldbooks, system prompts, and personas are stored in `data.db`. The database schema is versioned and auto-migrated on startup.

Old config at `~/.config/libllm/config.toml` is auto-migrated on first run.

### Migrating from legacy file-based storage

If upgrading from a version that used per-file storage (`sessions/`, `characters/`, etc.), run:

```sh
libllm-migrate -d <data_dir>
# or for unencrypted directories:
libllm-migrate -d <data_dir> --no-encrypt
```

This creates a `.7z` backup, imports all data into `data.db`, and removes the old directories.

## Architecture

The codebase uses Rust 2024 edition with async (tokio) and streaming HTTP (reqwest + futures-util). The project is a Cargo workspace with three crates.

### libllm-core (shared library)

- **`db`** -- SQLite database module (via `rusqlite` with SQLCipher). Contains `Database` struct with connection management, versioned schema migrations, and CRUD operations for all data types (sessions, messages, characters, worldbooks, system prompts, personas)
- **`session`** -- `MessageTree` (arena-based branching with `Vec<Node>` + `NodeId`), `SaveMode` enum (None/Database/PendingPasskey), `Session` struct with tree, model, template, and metadata
- **`character`** -- `CharacterCard` struct and parsing from JSON and PNG (base64 text chunk extraction). Supports old (top-level) and new (nested `data` object) formats
- **`worldinfo`** -- `WorldBook` with entry scanning by keyword match. `scan_entries()` activates entries whose keys appear in message text
- **`system_prompt`** -- `SystemPromptFile` struct with builtin name constants (`BUILTIN_ASSISTANT`, `BUILTIN_ROLEPLAY`) and content
- **`persona`** -- `PersonaFile` struct (`name` + `persona` text)
- **`template`** -- `{{char}}`/`{{user}}` template variable substitution
- **`config`** -- TOML config at `<data_dir>/config.toml`, data directory management. `data_dir()` supports a `OnceLock`-based override set via `set_data_dir()` for the `--data` flag
- **`crypto`** -- Argon2id key derivation, salt management, passkey verification. `DerivedKey` struct provides the key for SQLCipher's `PRAGMA key`
- **`client`** -- `ApiClient` with two streaming modes: `impl Write` (single-msg) and `mpsc::Sender<StreamToken>` (TUI)
- **`commands`** -- Shared command registry for `/help` and TUI command picker
- **`preset`** -- Three preset types loaded from JSON files in `<data_dir>/presets/`: `InstructPreset` (prompt formatting), `ReasoningPreset` (thinking block support), `ContextPreset` (context template variables)
- **`sampling`** -- `SamplingParams` and `SamplingOverrides` with `with_overrides` merge
- **`context`** -- `ContextManager` for token estimation and pure `truncated_path`
- **`export`** -- Conversation branch export to HTML (styled, responsive dark/light), Markdown, or JSONL (SillyTavern-compatible format with metadata header)
- **`migration`** -- Legacy config path migration (`~/.config/libllm/` to `~/.local/share/libllm/`)
- **`debug_log`** -- Structured diagnostic logging and timing instrumentation

### libllm (main binary)

- **`cli`** -- Clap-derived argument parsing with `CliOverrides` struct for tracking which config fields are overridden by CLI flags. Flags `-c` and `-p` are mutually required (roleplay mode). `--no-encrypt` and `--passkey` require `--data/-d`. Subcommands: `edit` (open character/worldbook in `$EDITOR`), `import` (import characters/worldbooks/personas/system prompts from files with auto-detection), `update` (self-update with optional branch target, `--list`, `--yes`)
- **`update`** -- Self-update via GitHub releases. Supports stable and branch channels with interactive branch selection, channel-switch confirmation, and cross-platform binary replacement
- **`tui`** -- Full ratatui terminal UI:
  - `mod.rs` -- App state (holds `Database`), Focus enum, async event loop with 16ms tick, layout (sidebar 32 cols | chat + status). Stores `CliOverrides` for enforcing read-only UI on CLI-overridden fields
  - `business.rs` -- `build_effective_system_prompt()`, worldbook entry injection, `config_locked_fields()` for determining which `/config` fields are CLI-locked
  - `clipboard.rs` -- Clipboard integration for copy/paste
  - `commands.rs` -- Slash command dispatch, streaming via channel, session auto-save via database
  - `input.rs` -- Keyboard handling, tree navigation (`switch_sibling`, `navigate_up`, `navigate_down`), command picker with Tab
  - `render.rs` -- Styled text parsing (bold/italic markdown), chat rendering, status bar with branch indicators. All colors read from `app.theme` (no hardcoded color constants)
  - `theme.rs` -- `Theme` struct with 25 color fields, built-in presets (dark, light), `parse_color()` for named/hex/indexed colors, `resolve_theme()` merges preset + config overrides
  - `maintenance.rs` -- Startup tasks (ensure builtin system prompts in database)
  - `dialogs/` -- Modal dialogs: `passkey`, `set_passkey`, `branch`, `character`, `persona`, `system_prompt`, `edit`, `worldbook`, `preset`, `delete_confirm`, `api_error`. All dialogs use database CRUD directly

### libllm-migrate (migration binary)

- **`legacy`** -- Reads old AES-256-GCM encrypted files (sessions, characters, worldbooks, personas, system prompts) from legacy data directories
- **`backup`** -- Creates `.7z` backup archive of legacy files before migration
- **`main`** -- CLI entry point: parse args, derive key, create backup, import into SQLite, delete old files

### CLI Override System

CLI flags that overlap with `/config` fields (api-url, template, sampling params, tls-skip-verify) are tracked in a `CliOverrides` struct passed to the TUI. Overridden fields:
- Display in red in the `/config` dialog and cannot be edited
- Are excluded from config.toml writes (preserving the on-disk values)
- Take priority when `apply_config()` reloads settings

The `-r` (system prompt) flag forces `/system` into a read-only viewer. The `-p` (persona) flag forces `/persona` into a read-only viewer. Both show content in red with editing disabled.

### Encryption

All data is stored in a single SQLite database (`data.db`) encrypted with **SQLCipher** (AES-256). The encryption key is derived from the user's passkey using **Argon2id** (64 MB memory, 3 iterations) with a per-installation random salt. The derived key is passed to SQLCipher via `PRAGMA key`.

When using `--data/-d`, the encryption mode must be consistent with the directory: `--passkey` is rejected on unencrypted data directories, and `--no-encrypt` is rejected on encrypted ones. New directories allow either mode for first-time setup.

The passkey can be changed at any time via `/passkey`, which uses SQLCipher's `PRAGMA rekey` to re-encrypt the database.

### Diagnostics

Debug logging is off by default. Enable it via `debug_log = true` in config or `--debug <out_path>` on the command line. When enabled, LibLLM creates the log in the OS temp directory under a unique `libllm-debug-*.log` filename. `--debug <out_path>` overrides that location with an explicit path.

`--timings[=<out_path>]` writes a timings report at shutdown. `--timings` with no value writes `./timings.log`.

`--cleanup` removes LibLLM-managed temporary debug logs and exits.

The TUI `/report` command copies the currently active debug log to `./debug.log` and refuses to overwrite an existing file.

When modifying instrumented paths such as startup, session I/O, or rendering, maintain diagnostics coverage with `debug_log::log_kv()`, `debug_log::timed_kv()`, or `debug_log::timed_result()`. Immediate debug logs should describe subsystem behavior; timing data should feed the `--timings` report rather than writing inline elapsed lines to the debug log.

### Statusbar

The statusbar shows persistent info (model, template, tokens, branch) by default. Temporary messages use `App::set_status()` with a `StatusLevel` (Info/Warning/Error) and auto-clear after 5 seconds. Do not use the statusbar for hints that duplicate information already visible in block borders or obvious UI state changes. The statusbar default info line is sacred -- it should always be visible unless a temporary message is actively displayed.

### Conversation Branching

Messages form a tree (`MessageTree`). `/retry` and `/edit` create sibling branches. `branch_path()` walks from head to root. `/branch next|prev|list|<id>` and Alt+Left/Right navigate branches. Branch indicators `[1/3]` show at branch points.
