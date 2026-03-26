# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

LibLLM is a Rust TUI/CLI chat client for the llama.cpp completions API. It supports single-message mode (`-m`), an interactive REPL (`--repl`), and a full terminal UI (default), with tree-structured conversation history, encrypted session persistence, and branch navigation.

## Build and Run

```sh
cargo build
cargo run -- --help
cargo run -- -m "Hello"              # single message, ephemeral (no passkey)
cargo run                            # TUI mode, prompts for passkey
cargo run -- --repl                  # REPL mode, prompts for passkey
cargo run -- -s session.json         # plaintext mode, bypasses encryption
cargo run -- --no-encrypt            # auto-save without encryption
cargo run -- --template chatml       # use ChatML prompt template
cargo run -- --temperature 0.5       # override sampling params
LIBLLM_PASSKEY=foo cargo run         # passkey via env var (for scripting)
```

The API URL defaults to `http://localhost:5001/v1` and can be overridden via `--api-url`, `LIBLLM_API_URL` env var, or config file.

## Data Directory

```
~/.local/share/libllm/
├── config.toml              # API URL, template, sampling defaults (NOT encrypted)
├── .salt                    # 16-byte random salt (generated on first run)
└── sessions/
    └── *.session            # AES-256-GCM encrypted session files
```

Old config at `~/.config/libllm/config.toml` is auto-migrated on first run.

## Architecture

The codebase uses Rust 2024 edition with async (tokio) and streaming HTTP (reqwest + futures-util).

- **`cli`** -- Clap-derived argument parsing with sampling flags, `--repl`, `--no-encrypt`, `--passkey`
- **`client`** -- `ApiClient` with two streaming modes: `impl Write` (REPL/single-msg) and `mpsc::Sender<StreamToken>` (TUI)
- **`commands`** -- Shared command registry for `/help` and TUI command picker
- **`config`** -- TOML config at `~/.local/share/libllm/config.toml`, data/sessions directory management, migration from old config path
- **`context`** -- `ContextManager` for token estimation and pure `truncated_path`
- **`crypto`** -- AES-256-GCM encryption/decryption, Argon2id key derivation, salt management
- **`prompt`** -- `Template` enum (Llama2, ChatML, Mistral, Phi, Raw)
- **`sampling`** -- `SamplingParams` and `SamplingOverrides` with `with_overrides` merge
- **`session`** -- `MessageTree` (arena-based branching), `SaveMode` enum (None/Plaintext/Encrypted), encrypted save/load, session listing with previews
- **`interactive`** -- Legacy REPL mode (`--repl`)
- **`tui`** -- Full ratatui terminal UI with sidebar, scrollable chat, multi-line input, command picker, status bar, streaming, and branch navigation

### Encryption

Sessions are encrypted with AES-256-GCM. A single salt (`.salt` file) is created on first run. The user's passkey + salt derive one key via Argon2id at startup. Each session file gets a unique random nonce. The `-s` flag bypasses encryption for backward compatibility.

### Conversation Branching

Messages form a tree (`MessageTree`). `/retry` and `/edit` create sibling branches. `branch_path()` walks from head to root. `/branch next|prev|list|<id>` and Alt+Left/Right navigate branches. Branch indicators `[1/3]` show at branch points.
