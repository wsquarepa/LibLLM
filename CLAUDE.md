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
```

The API URL defaults to `http://localhost:5001/v1` and can be overridden via `--api-url` or `LIBLLM_API_URL` env var.

## Architecture

The codebase uses Rust 2024 edition with async (tokio) and streaming HTTP (reqwest + futures-util).

- **`cli`** -- Clap-derived argument parsing (`Args` struct)
- **`client`** -- `ApiClient` wrapping reqwest; handles SSE streaming from the `/completions` endpoint and model name fetching from `/models`
- **`prompt`** -- `PromptTemplate` trait abstracting chat template formatting; currently implements `Llama2Template` with `[INST]`/`[/INST]` tags
- **`session`** -- JSON serialization of conversation state (`Session` struct with `prompt_history`); gracefully handles legacy plain-text session files
- **`interactive`** -- REPL loop using rustyline, wiring together client, session, and prompt template

Flow: `main` parses args, loads/creates session, seeds BOS tokens via the template, then either sends a single completion (message mode) or enters the interactive loop. Both paths append the raw prompt text to `session.prompt_history` as a growing string.
