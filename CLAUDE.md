# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

LibLLM is a Rust TUI/CLI chat client for the llama.cpp completions API. It supports single-message mode (`-m`), an interactive REPL (`--repl`), and a full terminal UI (default), with tree-structured conversation history and JSON session persistence.

## Build and Run

```sh
cargo build
cargo run -- --help
cargo run -- -m "Hello"              # single message mode
cargo run -- -s session.json         # TUI mode with session persistence
cargo run -- --repl -s session.json  # legacy REPL mode
cargo run -- --template chatml       # use ChatML prompt template
cargo run -- --temperature 0.5       # override sampling params
echo "prompt" | cargo run -- -m -    # pipe stdin as message
```

The API URL defaults to `http://localhost:5001/v1` and can be overridden via `--api-url`, `LIBLLM_API_URL` env var, or `~/.config/libllm/config.toml`.

## Architecture

The codebase uses Rust 2024 edition with async (tokio) and streaming HTTP (reqwest + futures-util).

- **`cli`** -- Clap-derived argument parsing with sampling flags and `--repl` mode switch
- **`client`** -- `ApiClient` with two streaming modes: `impl Write` (REPL/single-msg) and `mpsc::Sender<StreamToken>` (TUI)
- **`config`** -- TOML config file support (`~/.config/libllm/config.toml`)
- **`context`** -- `ContextManager` for token estimation and pure `truncated_path` (returns a view, never mutates the tree)
- **`prompt`** -- `Template` enum (Llama2, ChatML, Mistral, Phi, Raw) with `render(&[&Message])` producing the full prompt string
- **`sampling`** -- `SamplingParams` (concrete) and `SamplingOverrides` (partial) with `with_overrides` merge
- **`session`** -- Tree-structured message history via `MessageTree` (arena of `Node`s with parent/children links, head pointer). Backward-compatible loading of flat `Vec<Message>` and legacy `prompt_history` sessions.
- **`interactive`** -- Legacy REPL mode with rustyline and slash commands (activated via `--repl`)
- **`tui`** -- Full ratatui terminal UI (default mode) with sidebar, scrollable chat, multi-line input, status bar, real-time streaming, and branch navigation (Alt+Left/Right)

### Conversation Branching

Messages form a tree (arena in `MessageTree`). `/retry` and `/edit` create sibling branches instead of destroying history. `branch_path()` walks from the current head to root to produce the active conversation thread. `/branch next|prev|list|<id>` navigates branches. The TUI shows `[1/3]`-style indicators at branch points.
