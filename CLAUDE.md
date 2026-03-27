# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

LibLLM is a Rust TUI/CLI chat client for the llama.cpp completions API. It supports single-message mode (`-m`) and a full terminal UI (default), with tree-structured conversation history, encrypted session persistence, branch navigation, character cards, and worldbook/lorebook support.

## Build and Run

```sh
cargo build
cargo run -- --help
cargo run -- -m "Hello"              # single message, ephemeral (no passkey)
cargo run                            # TUI mode, prompts for passkey
cargo run -- -s session.json         # plaintext mode, bypasses encryption
cargo run -- --no-encrypt            # auto-save without encryption
cargo run -- --template chatml       # use ChatML prompt template
cargo run -- --temperature 0.5       # override sampling params
cargo run -- -c character_name       # load a character card
LIBLLM_PASSKEY=foo cargo run         # passkey via env var (for scripting)
```

The API URL defaults to `http://localhost:5001/v1` and can be overridden via `--api-url`, `LIBLLM_API_URL` env var, or config file.

No tests exist -- verify changes with `cargo build` and manual testing. CI builds on push to master and on PRs via GitHub Actions (`.github/workflows/build.yml`).

## Data Directory

```
~/.local/share/libllm/
├── config.toml              # API URL, template, sampling defaults (NOT encrypted)
├── .salt                    # 16-byte random salt (generated on first run)
├── sessions/
│   └── *.session            # AES-256-GCM encrypted session files
├── characters/
│   └── *.json / *.png       # Character cards (PNG with embedded JSON supported)
└── worldinfo/
    └── *.json               # Worldbook/lorebook files
```

Old config at `~/.config/libllm/config.toml` is auto-migrated on first run.

## Architecture

The codebase uses Rust 2024 edition with async (tokio) and streaming HTTP (reqwest + futures-util).

- **`cli`** -- Clap-derived argument parsing with sampling flags, `--no-encrypt`, `--passkey`, `-c` for character cards
- **`client`** -- `ApiClient` with two streaming modes: `impl Write` (single-msg) and `mpsc::Sender<StreamToken>` (TUI)
- **`commands`** -- Shared command registry for `/help` and TUI command picker; includes `resolve_alias()` and `matching_commands()`
- **`config`** -- TOML config at `~/.local/share/libllm/config.toml`, data/sessions/characters/worldinfo directory management, migration from old config path
- **`context`** -- `ContextManager` for token estimation and pure `truncated_path`
- **`crypto`** -- AES-256-GCM encryption/decryption, Argon2id key derivation, salt management. Encrypted file format: magic "LLMS" (4 bytes) + version (1 byte) + nonce (12 bytes) + ciphertext
- **`character`** -- `CharacterCard` parsing from JSON and PNG (base64 text chunk extraction). Supports old (top-level) and new (nested `data` object) formats. Auto-imports PNG cards on startup
- **`worldinfo`** -- `WorldBook` with entry scanning by keyword match. `scan_entries()` activates entries whose keys appear in message text. `normalize_worldbooks()` converts legacy field names
- **`prompt`** -- `Template` enum (Llama2, ChatML, Mistral, Phi, Raw)
- **`sampling`** -- `SamplingParams` and `SamplingOverrides` with `with_overrides` merge
- **`session`** -- `MessageTree` (arena-based branching with `Vec<Node>` + `NodeId`), `SaveMode` enum (None/Plaintext/Encrypted/PendingPasskey), encrypted save/load, session listing with previews. Supports legacy flat session format migration
- **`tui`** -- Full ratatui terminal UI:
  - `mod.rs` -- App state, Focus enum (Input/Chat/Sidebar/dialogs), async event loop with 16ms tick, layout (sidebar 32 cols | chat + status)
  - `business.rs` -- `build_effective_system_prompt()`, worldbook entry injection, `{{char}}`/`{{user}}` template variable substitution
  - `commands.rs` -- Slash command dispatch, streaming via channel, session auto-save
  - `input.rs` -- Keyboard handling, tree navigation (`switch_sibling`, `navigate_up`, `navigate_down`), command picker with Tab
  - `render.rs` -- Styled text parsing (bold/italic markdown), chat rendering, status bar with branch indicators
  - `dialogs/` -- Modal dialogs: passkey, branch selector, character picker, message editor, system prompt editor, worldbook toggle list, config editor

### Encryption

Sessions are encrypted with AES-256-GCM. A single salt (`.salt` file) is created on first run. The user's passkey + salt derive one key via Argon2id at startup. Each session file gets a unique random nonce. The `-s` flag bypasses encryption for backward compatibility.

### Render Debug Logging

Dev builds support `--debug <out_path>` for render performance diagnostics:
```sh
cargo run -- --debug log.txt
```
The `--debug` flag and all associated logging are behind `#[cfg(debug_assertions)]` -- automatically available in dev builds, compiled out of release builds.

When modifying the rendering pipeline (`tui/render.rs`, `tui/mod.rs::render_frame`, dialog render functions), add corresponding `debug_log::timed()` or `debug_log::log()` calls gated behind `#[cfg(debug_assertions)]` to maintain diagnostics coverage.

### Conversation Branching

Messages form a tree (`MessageTree`). `/retry` and `/edit` create sibling branches. `branch_path()` walks from head to root. `/branch next|prev|list|<id>` and Alt+Left/Right navigate branches. Branch indicators `[1/3]` show at branch points.
