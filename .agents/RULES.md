# RULES.md
This file contains details to **any programming agent** about what this repository is and what code style guidelines to follow.

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
├── .key_check               # Passkey verification fingerprint
├── index.meta               # Encrypted metadata cache for fast session/character/worldbook listing
├── sessions/
│   └── *.session            # AES-256-GCM encrypted session files
├── characters/
│   └── *.character / *.json / *.png  # Character cards (PNG auto-imported, JSON auto-encrypted)
├── worldinfo/
│   └── *.worldbook / *.json # Worldbook files (JSON auto-encrypted)
├── system/
│   ├── assistant.prompt     # Builtin system prompt
│   ├── roleplay.prompt      # Builtin system prompt
│   └── *.prompt / *.json    # Custom system prompts (JSON auto-encrypted)
└── personas/
    └── *.persona / *.json   # User personas (JSON auto-encrypted)
```

Old config at `~/.config/libllm/config.toml` is auto-migrated on first run. System prompts and personas previously stored in `config.toml` are auto-migrated to their respective directories.

## Architecture

The codebase uses Rust 2024 edition with async (tokio) and streaming HTTP (reqwest + futures-util).

- **`cli`** -- Clap-derived argument parsing with sampling flags, `--no-encrypt`, `--passkey`, `-c` for character cards
- **`client`** -- `ApiClient` with two streaming modes: `impl Write` (single-msg) and `mpsc::Sender<StreamToken>` (TUI)
- **`commands`** -- Shared command registry for `/help` and TUI command picker; includes `resolve_alias()` and `matching_commands()`
- **`config`** -- TOML config at `~/.local/share/libllm/config.toml`, data/sessions/characters/worldinfo/system/personas directory management, migration from old config path
- **`context`** -- `ContextManager` for token estimation and pure `truncated_path`
- **`crypto`** -- AES-256-GCM encryption/decryption, Argon2id key derivation, salt management. Encrypted file format: magic "LLMS" (4 bytes) + version (1 byte) + nonce (12 bytes) + ciphertext
- **`character`** -- `CharacterCard` parsing from JSON and PNG (base64 text chunk extraction). Supports old (top-level) and new (nested `data` object) formats. Auto-imports PNG cards on startup
- **`worldinfo`** -- `WorldBook` with entry scanning by keyword match. `scan_entries()` activates entries whose keys appear in message text. `normalize_worldbooks()` converts legacy field names
- **`prompt`** -- `Template` enum (Llama2, ChatML, Mistral, Phi, Raw)
- **`sampling`** -- `SamplingParams` and `SamplingOverrides` with `with_overrides` merge
- **`session`** -- `MessageTree` (arena-based branching with `Vec<Node>` + `NodeId`), `SaveMode` enum (None/Plaintext/Encrypted/PendingPasskey), encrypted save/load, session listing with previews. Supports legacy flat session format migration
- **`system_prompt`** -- File-based system prompt management. Two hardcoded builtins (`assistant`, `roleplay`) are auto-created if missing. Custom prompts stored as encrypted `.prompt` files. Handles migration from old `config.toml` fields
- **`persona`** -- File-based user persona management (`name` + `persona` text). Stored as encrypted `.persona` files. Migrates from old `config.toml` `user_name`/`user_persona` fields
- **`index`** -- `MetadataIndex` for fast session/character/worldbook listing. Caches display names, message counts, and previews in encrypted `index.meta` to avoid decrypting every file on startup
- **`migration`** -- Centralized migration orchestration. Runs all migrations (config path, system prompts, personas, worldbook normalization, plaintext encryption) on startup with warning reporting
- **`tui`** -- Full ratatui terminal UI:
  - `mod.rs` -- App state, Focus enum (Input/Chat/Sidebar/dialogs), async event loop with 16ms tick, layout (sidebar 32 cols | chat + status)
  - `business.rs` -- `build_effective_system_prompt()`, worldbook entry injection, `{{char}}`/`{{user}}` template variable substitution
  - `commands.rs` -- Slash command dispatch, streaming via channel, session auto-save
  - `input.rs` -- Keyboard handling, tree navigation (`switch_sibling`, `navigate_up`, `navigate_down`), command picker with Tab
  - `render.rs` -- Styled text parsing (bold/italic markdown), chat rendering, status bar with branch indicators
  - `maintenance.rs` -- Background maintenance tasks (PNG import, plaintext encryption, worldbook normalization, builtin prompt setup) spawned on startup and after passkey unlock
  - `dialogs/` -- Modal dialogs: passkey, branch selector, character picker, persona editor, system prompt selector, message editor, worldbook toggle list, delete confirmation, config editor, API error

### Encryption

Sessions are encrypted with AES-256-GCM. A single salt (`.salt` file) is created on first run. The user's passkey + salt derive one key via Argon2id at startup. Each session file gets a unique random nonce. The `-s` flag bypasses encryption for backward compatibility.

### Diagnostics

All builds write a debug log automatically. By default LibLLM creates the log in the OS temp directory under a unique `libllm-debug-*.log` filename. `--debug <out_path>` overrides that location with an explicit path.

`--timings[=<out_path>]` writes a timings report at shutdown. `--timings` with no value writes `./timings.log`.

`--cleanup` removes LibLLM-managed temporary debug logs and exits.

The TUI `/report` command copies the currently active debug log to `./debug.log` and refuses to overwrite an existing file.

When modifying instrumented paths such as startup, session I/O, metadata hydration, unlock flow, or rendering, maintain diagnostics coverage with `debug_log::log_kv()`, `debug_log::timed_kv()`, or `debug_log::timed_result()`. Immediate debug logs should describe subsystem behavior; timing data should feed the `--timings` report rather than writing inline elapsed lines to the debug log.

### Statusbar

The statusbar shows persistent info (model, template, tokens, branch) by default. Temporary messages use `App::set_status()` with a `StatusLevel` (Info/Warning/Error) and auto-clear after 5 seconds. Do not use the statusbar for hints that duplicate information already visible in block borders or obvious UI state changes. The statusbar default info line is sacred -- it should always be visible unless a temporary message is actively displayed.

### Conversation Branching

Messages form a tree (`MessageTree`). `/retry` and `/edit` create sibling branches. `branch_path()` walks from head to root. `/branch next|prev|list|<id>` and Alt+Left/Right navigate branches. Branch indicators `[1/3]` show at branch points.
