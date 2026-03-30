# LibLLM

A keyboard-driven terminal chat client for local LLMs. LibLLM connects to any [llama.cpp](https://github.com/ggerganov/llama.cpp)-compatible API and gives you conversation branching, encrypted session persistence, character cards, and worldbooks -- all from the terminal.

Built for power users who run local models and want a fast, private chat interface with full control over conversation history.

**Why LibLLM?**

- **Branching conversations** -- retry or edit any message to fork the conversation, then navigate between branches like a tree
- **Encrypted by default** -- sessions, characters, and worldbooks are encrypted at rest with AES-256-GCM
- **Character cards and worldbooks** -- load SillyTavern-compatible cards and keyword-activated lore entries
- **Pipe-friendly CLI** -- send a single message with `libllm -m "prompt"` for scripting and automation

<!-- TODO: add screenshot or GIF of the TUI here -->
*Screenshot coming soon.*

## Table of contents

- [Quickstart](#quickstart)
- [Concepts](#concepts)
- [Common workflows](#common-workflows)
- [Installation](#installation)
- [CLI reference](#cli-reference)
- [TUI keyboard shortcuts](#tui-keyboard-shortcuts)
- [TUI commands](#tui-commands)
- [Configuration](#configuration)
- [Data directory](#data-directory)
- [Encryption](#encryption)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)
- [License](#license)

## Quickstart

**Prerequisites:** a running llama.cpp-compatible API server. LibLLM expects an OpenAI-compatible `/v1/chat/completions` endpoint. The default URL is `http://localhost:5001/v1`.

**1. Install**

Download a pre-built binary from the [nightly release](../../releases/tag/nightly) (Linux, macOS, Windows). Or build from source:

```sh
git clone https://github.com/wsquarepa/LibLLM.git
cd LibLLM
cargo build --release
# binary is at target/release/libllm
```

**2. Launch**

```sh
libllm
```

On first launch, LibLLM prompts you to set an encryption passkey. This passkey protects all your saved sessions, character cards, and worldbooks. You must set a passkey to continue (or use `--no-encrypt` to opt out).

**3. Chat**

Type a message and press Enter. The response streams in real-time. Your session is auto-saved after each exchange.

**Override the API URL** if your server runs on a different address:

```sh
libllm --api-url http://localhost:8080/v1
# or via environment variable
export LIBLLM_API_URL=http://localhost:8080/v1
```

## Concepts

### Conversation branching

Messages in LibLLM form a tree, not a flat list. When you use `/retry` to regenerate a response or `/edit` to rewrite a message, the new version becomes a sibling branch of the original. You can navigate between branches with Alt+Left/Right, and branch indicators like `[1/3]` appear at fork points.

This means you never lose a previous response -- you can always switch back to an earlier branch.

### Character cards

Character cards define an AI persona with a name, description, personality, and scenario. LibLLM supports JSON and PNG formats (SillyTavern-compatible `tEXt` chunk extraction). Drop a `.json` or `.png` card into `~/.local/share/libllm/characters/` or use the `/character` command to import one. Template variables `{{char}}` and `{{user}}` are substituted automatically.

### Worldbooks

Worldbooks (lorebooks) provide keyword-activated context injection. Each entry has a set of trigger keywords; when those keywords appear in the conversation, the entry's content is injected into the prompt. This lets you build persistent lore, facts, or instructions that activate only when relevant.

### Encryption

By default, LibLLM encrypts all sessions, character cards, and worldbooks at rest using AES-256-GCM with an Argon2id-derived key. You set your passkey on first launch, and it is required each time you start the TUI.

To skip encryption entirely, use `--no-encrypt` (sessions saved as plaintext JSON, no passkey prompt) or `-s session.json` (explicit plaintext session file).

There is no passkey recovery mechanism. If you forget your passkey, encrypted data cannot be decrypted.

## Common workflows

### Start the TUI

```sh
libllm
```

### Send a one-off message from a script

```sh
libllm -m "Summarize this file" < document.txt
# or
echo "Translate to French: hello world" | libllm -m -
```

### Load a character card

```sh
# By name (from ~/.local/share/libllm/characters/)
libllm -c character_name

# By file path
libllm -c /path/to/card.png
```

Or use the `/character` command inside the TUI to browse and import cards.

### Toggle worldbooks

Use the `/worldbook` command inside the TUI to enable or disable worldbooks for the current session. Worldbooks are loaded from `~/.local/share/libllm/worldinfo/`.

### Use plaintext mode

```sh
# Disable encryption for all sessions
libllm --no-encrypt

# Use a specific plaintext session file
libllm -s my_session.json
```

### Switch branches during a conversation

- `/retry` to regenerate the last response (creates a new branch)
- `/edit` to rewrite a previous message (creates a new branch)
- Alt+Left / Alt+Right to switch between sibling branches
- `/branch` to browse all branches at the current position

### Override sampling parameters

```sh
libllm --temperature 0.5 --top-p 0.9 --max-tokens 512
```

### Provide passkey non-interactively

```sh
LIBLLM_PASSKEY=mypasskey libllm
# or
libllm --passkey mypasskey
```

## Installation

### Quick install (Linux / macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/wsquarepa/LibLLM/master/install.sh | sh
```

This downloads the latest nightly binary for your platform and installs it to `~/.local/bin`. Set `INSTALL_DIR` to override the install location. For private repositories, set `GITHUB_TOKEN` or `GH_TOKEN` before running.

### Update

```sh
libllm update
```

Re-running the install script on a system that already has libllm will automatically run `libllm update` instead.

### From nightly release (recommended)

Pre-built binaries for Linux (x86_64, aarch64), macOS (x86_64, aarch64), and Windows (x86_64, aarch64) are published on every push to `master` as a [nightly release](../../releases/tag/nightly).

There are no stable releases yet. Nightly is the recommended install method.

### From source

Requires [Rust](https://rustup.rs/) (stable toolchain).

```sh
git clone https://github.com/wsquarepa/LibLLM.git
cd LibLLM
cargo build --release
# binary is at target/release/libllm
```

## CLI reference

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
| `--tls-skip-verify` | Skip TLS certificate verification |

### Subcommands

```sh
# Update to the latest nightly build
libllm update

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

## TUI commands

Type `/` in the input to open the command picker. Tab or Space to autocomplete, Enter to execute.

| Command | Aliases | Description |
|---|---|---|
| `/clear` | `/new` | Clear conversation history |
| `/save` | | Save session to file |
| `/load` | | Load session from file |
| `/system` | | Select or edit system prompt |
| `/retry` | | Regenerate last response (new branch) |
| `/branch` | | Browse branches at current position |
| `/character` | | Select a character or import a card |
| `/persona` | `/self`, `/user`, `/me` | Manage user personas |
| `/worldbook` | `/lore`, `/world`, `/lorebook` | Toggle worldbooks for this session |
| `/passkey` | `/password`, `/pass`, `/auth` | Set or change encryption passkey |
| `/config` | | Open configuration dialog |
| `/quit` | `/exit` | Exit the chat |

## Configuration

Configuration is stored at `~/.local/share/libllm/config.toml`. Edit it directly or use the `/config` TUI command.

```toml
api_url = "http://localhost:5001/v1"
template = "chatml"
worldbooks = ["fantasy-lore", "tech-terms"]
tls_skip_verify = false

[sampling]
temperature = 0.8
top_k = 40
top_p = 0.95
min_p = 0.05
repeat_last_n = 64
repeat_penalty = 1.0
max_tokens = -1
```

System prompts and user personas are managed as separate encrypted files via the `/system` and `/persona` TUI commands, not in `config.toml`.

## Data directory

```
~/.local/share/libllm/
  config.toml              # API URL, template, sampling defaults
  .salt                    # 16-byte random salt (generated on first run)
  .key_check               # Passkey verification fingerprint
  index.json               # Metadata cache for fast listing
  sessions/
    *.session              # AES-256-GCM encrypted session files
  characters/
    *.character            # Encrypted character cards
    *.json                 # Plaintext character cards (auto-encrypted on next run)
    *.png                  # PNG cards with embedded JSON (auto-imported on startup)
  worldinfo/
    *.worldbook            # Encrypted worldbook files
    *.json                 # Plaintext worldbooks (auto-normalized on next run)
  system/
    assistant.prompt       # Builtin system prompt (encrypted)
    roleplay.prompt        # Builtin system prompt (encrypted)
    *.prompt / *.json      # Custom system prompts (JSON auto-encrypted)
  personas/
    *.persona / *.json     # User personas (JSON auto-encrypted)
```

## Encryption

Sessions, character cards, and worldbooks are encrypted at rest using **AES-256-GCM**. The encryption key is derived from your passkey using **Argon2id** (64 MB memory, 3 iterations) with a per-installation random salt.

Encrypted file format: `LLMS` magic (4 bytes) + version (1 byte) + nonce (12 bytes) + ciphertext.

Each file gets a unique random nonce. The passkey can be changed at any time via `/passkey`, which re-encrypts all stored files.

To opt out of encryption, use `--no-encrypt` or `-s <path>` for plaintext sessions.

## Troubleshooting

### Cannot connect to API

LibLLM expects a running llama.cpp-compatible server at the configured URL (default `http://localhost:5001/v1`). Verify:

- The server is running and listening on the expected port.
- The URL matches (check `--api-url`, `LIBLLM_API_URL`, or `api_url` in `config.toml`).
- The server exposes an OpenAI-compatible `/v1/chat/completions` endpoint.

### Forgot passkey

There is no passkey recovery. If you forget your passkey, encrypted sessions, characters, and worldbooks cannot be decrypted. You can start fresh by deleting the data directory (`~/.local/share/libllm/`) or use `--no-encrypt` to start without encryption.

### Sessions appear missing

Sessions are tied to the encryption passkey. If you enter the wrong passkey, previously saved sessions will not appear in the sidebar. Re-launch with the correct passkey.

### Character or worldbook not showing up

- PNG cards are auto-imported on startup. If you added a PNG while the TUI was running, restart it.
- JSON files are auto-encrypted on next launch. Ensure the file is valid JSON and placed in the correct directory (`characters/` or `worldinfo/`).

### `--no-encrypt` vs `-s`

- `--no-encrypt` disables encryption globally. Sessions are saved as plaintext JSON in the default data directory. No passkey prompt.
- `-s session.json` loads and saves a specific plaintext file. Useful for one-off sessions or backward compatibility.

### TLS / self-signed certificate errors

Use `--tls-skip-verify` to bypass certificate verification when connecting to a server with a self-signed certificate.

## Contributing

Bug reports and feature requests: [GitHub Issues](../../issues)

To build from source:

```sh
cargo build
```

There is no test suite yet. Verify changes with `cargo build` and manual testing.

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
