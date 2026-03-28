# LibLLM

A Rust TUI and CLI chat client for the [llama.cpp](https://github.com/ggerganov/llama.cpp) completions API. Features a full terminal interface with tree-structured conversation branching, encrypted session persistence, character cards, and worldbook/lorebook support.

## Features

- **Terminal UI** with sidebar session management, real-time streaming, and keyboard-driven navigation
- **Conversation branching** -- retry or edit any message to create a new branch, navigate between branches with Alt+Left/Right
- **Encrypted sessions** -- AES-256-GCM encryption with Argon2id key derivation; all sessions, characters, and worldbooks are encrypted at rest
- **Character cards** -- import from JSON or PNG (SillyTavern-compatible `tEXt` chunk extraction), with `{{char}}`/`{{user}}` template variable substitution
- **Worldbooks/Lorebooks** -- keyword-activated context injection with scan depth, selective keys, and ordering control
- **Prompt templates** -- Llama 2, ChatML, Mistral, Phi, and Raw formats
- **Configurable sampling** -- temperature, top-k, top-p, min-p, repeat penalty, and max tokens
- **Single-message mode** -- pipe-friendly `libllm -m "prompt"` for scripting, supports stdin with `-m -`
- **Cross-platform** -- builds for Linux (x86_64, aarch64), macOS (x86_64, aarch64), and Windows (x86_64, aarch64)

## Installation

### From nightly release

Pre-built binaries are published on every push to `master` as a [nightly release](../../releases/tag/nightly).

### From source

Requires [Rust](https://rustup.rs/) (stable toolchain).

```sh
git clone https://github.com/wsquarepa/LibLLM.git
cd LibLLM
cargo build --release
# binary is at target/release/libllm
```

## Usage

LibLLM connects to a llama.cpp-compatible completions API (default `http://localhost:5001/v1`).

```sh
# Launch the TUI (prompts for encryption passkey)
libllm

# Single message mode (ephemeral, no session saved)
libllm -m "Explain quicksort"

# Read prompt from stdin
echo "Translate to French: hello" | libllm -m -

# Specify API URL and template
libllm --api-url http://localhost:8080/v1 --template chatml

# Override sampling parameters
libllm --temperature 0.5 --top-p 0.9 --max-tokens 512

# Load a character card
libllm -c character_name
libllm -c /path/to/card.png

# Plaintext session (no encryption)
libllm -s session.json
libllm --no-encrypt

# Passkey via environment variable
LIBLLM_PASSKEY=mypasskey libllm
```

### CLI reference

| Flag | Description |
|---|---|
| `-m`, `--message` | Send a single message and exit (`-` for stdin) |
| `-s`, `--session` | Explicit session file path (plaintext JSON, bypasses encryption) |
| `-c`, `--character` | Character card name or path to `.json`/`.png` file |
| `-t`, `--template` | Prompt template: `llama2`, `chatml`, `mistral`, `phi`, `raw` |
| `-p`, `--system-prompt` | Set the system prompt |
| `--api-url` | API base URL (env: `LIBLLM_API_URL`) |
| `--no-encrypt` | Disable session encryption |
| `--passkey` | Encryption passkey (env: `LIBLLM_PASSKEY`) |
| `--temperature` | Sampling temperature |
| `--top-k` | Top-K sampling |
| `--top-p` | Top-P (nucleus) sampling |
| `--min-p` | Min-P sampling |
| `--repeat-last-n` | Repeat penalty window size |
| `--repeat-penalty` | Repeat penalty strength |
| `--max-tokens` | Maximum tokens to generate (`-1` for unlimited) |

### Subcommands

```sh
# Edit a character card or worldbook in $EDITOR
libllm edit character <name>
libllm edit worldbook <name>
```

## TUI keyboard shortcuts

| Key | Context | Action |
|---|---|---|
| Enter | Input | Send message |
| Alt+Enter | Input | Insert newline |
| Up arrow | Input (empty) | Navigate to previous user message |
| Enter | Input (navigating) | Edit selected message |
| Tab | Global | Cycle focus: Input -> Chat -> Sidebar |
| Esc | Global | Return to input, cancel navigation |
| Alt+Left/Right | Global | Switch between conversation branches |
| Up/Down | Chat | Navigate between messages |
| Left/Right | Chat | Switch branch at current node |
| Enter | Chat | Open edit dialog for selected message |
| Up/Down | Sidebar | Browse sessions |
| Delete | Sidebar | Delete selected session |
| Esc | Streaming | Cancel generation |
| Ctrl+C | Global | Quit |

### TUI commands

Type `/` in the input to open the command picker. Tab or Space to autocomplete, Enter to execute.

| Command | Aliases | Description |
|---|---|---|
| `/clear` | `/new` | Clear conversation history |
| `/save` | | Save session to file |
| `/load` | | Load session from file |
| `/system` | | Set or show system prompt |
| `/retry` | | Regenerate last response (new branch) |
| `/branch` | | Browse branches at current position |
| `/character` | | Select or import a character card |
| `/self` | `/user`, `/me` | Set your name and persona |
| `/worldbook` | `/lore`, `/world`, `/lorebook` | Toggle worldbooks for this session |
| `/passkey` | `/password`, `/pass`, `/auth` | Set or change encryption passkey |
| `/config` | | Open configuration dialog |
| `/quit` | `/exit` | Exit the chat |

## Configuration

Configuration is stored at `~/.local/share/libllm/config.toml`. Edit it directly or use the `/config` TUI command.

```toml
api_url = "http://localhost:5001/v1"
template = "chatml"
system_prompt = "You are a helpful assistant."
roleplay_system_prompt = "Respond in character."
user_name = "Alice"
user_persona = "A curious software engineer."
worldbooks = ["fantasy-lore", "tech-terms"]

[sampling]
temperature = 0.8
top_k = 40
top_p = 0.95
min_p = 0.05
repeat_last_n = 64
repeat_penalty = 1.0
max_tokens = -1
```

## Data directory

```
~/.local/share/libllm/
  config.toml              # API URL, template, sampling defaults
  .salt                    # 16-byte random salt (generated on first run)
  .key_check               # Passkey verification fingerprint
  sessions/
    *.session              # AES-256-GCM encrypted session files
  characters/
    *.character            # Encrypted character cards
    *.json                 # Plaintext character cards (auto-encrypted on next run)
    *.png                  # PNG cards with embedded JSON (auto-imported on startup)
  worldinfo/
    *.worldbook            # Encrypted worldbook files
    *.json                 # Plaintext worldbooks (auto-normalized on next run)
```

## Encryption

Sessions, character cards, and worldbooks are encrypted at rest using **AES-256-GCM**. The encryption key is derived from your passkey using **Argon2id** (64 MB memory, 3 iterations) with a per-installation random salt.

Encrypted file format: `LLMS` magic (4 bytes) + version (1 byte) + nonce (12 bytes) + ciphertext.

Each file gets a unique random nonce. The passkey can be changed at any time via `/passkey`, which re-encrypts all stored files.

To opt out of encryption, use `--no-encrypt` or `-s <path>` for plaintext sessions.

## Building

```sh
cargo build              # debug build
cargo build --release    # optimized release build
cargo run -- --help      # run with arguments
```

Debug builds include a render performance logger:

```sh
cargo run -- --debug log.txt
```
