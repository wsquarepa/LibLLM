# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

LibLLM is a Rust CLI chat client for the llama.cpp completions API. It supports both single-message mode (`-m`) and an interactive REPL, with optional JSON-based session persistence.

## Build and Run

```sh
cargo build
cargo run -- --help
cargo run -- -m "Hello"              # single message mode
cargo run -- -s session.json         # interactive mode with session persistence
echo "prompt" | cargo run -- -m -    # pipe stdin as message
cargo run -- --template chatml       # use ChatML prompt template
cargo run -- --temperature 0.5       # override sampling params
```

The API URL defaults to `http://localhost:5001/v1` and can be overridden via `--api-url`, `LIBLLM_API_URL` env var, or `~/.config/libllm/config.toml`.

## Architecture

The codebase uses Rust 2024 edition with async (tokio) and streaming HTTP (reqwest + futures-util).

- **`cli`** -- Clap-derived argument parsing (`Args` struct) with sampling parameter flags
- **`client`** -- `ApiClient` wrapping reqwest; handles SSE streaming from `/completions` and model fetching from `/models`
- **`config`** -- TOML config file support (`~/.config/libllm/config.toml`) for persistent defaults
- **`context`** -- `ContextManager` for token estimation and automatic history truncation
- **`prompt`** -- `PromptTemplate` trait with implementations: Llama2, ChatML, Mistral, Phi, Raw
- **`render`** -- Markdown rendering via termimad for `/render` command
- **`sampling`** -- `SamplingParams` struct for generation parameters (temperature, top_k, top_p, etc.)
- **`session`** -- Structured message history (`Vec<Message>` with role/content/timestamp); backward-compatible with legacy single-string sessions
- **`interactive`** -- REPL loop with slash commands (/help, /clear, /save, /load, /model, /system, /retry, /edit, /history, /render, /quit)

Flow: `main` parses args, loads config, resolves template + sampling params, loads/creates session, then either sends a single completion (message mode) or enters the interactive REPL. The prompt template renders structured messages into the format expected by the model.
