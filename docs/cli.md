# CLI Reference

Run `libllm --help` for the exact help text for your installed version.

## Top-Level Options

| Option | Description |
|---|---|
| `-V`, `--version` | Print version and exit |
| `-d`, `--data <PATH>` | Use a specific data directory |
| `--continue <SESSION_ID>` | Continue a saved session by ID; requires `-m` and `-d` |
| `--passkey <PASSKEY>` | Passkey for encrypted data; also supports `LIBLLM_PASSKEY` |
| `--no-encrypt` | Use plaintext local data; requires `-d` |
| `-m`, `--message <TEXT>` | Send one message and exit; use `-` to read stdin |
| `-r`, `--system-prompt <TEXT>` | Override the system prompt |
| `-p`, `--persona <NAME>` | User persona; requires `-c` |
| `-c`, `--character <NAME_OR_PATH>` | Character card name or file path; repeat for group chats |
| `--note <TEXT>` | Override the session author's note |
| `--note-depth <N>` | Place the author's note near the Nth message from the end; requires `--note` |
| `--note-top` | Pin the author's note near the system prompt; requires `--note` |
| `--api-url <URL>` | API base URL without `/completions`; also supports `LIBLLM_API_URL` |
| `-t`, `--template <NAME>` | Instruct preset, such as `Mistral V3-Tekken`, `Llama 3 Instruct`, `ChatML`, `Phi`, `Alpaca`, or `Raw` |
| `--temperature <N>` | Sampling temperature |
| `--top-k <N>` | Top-K sampling |
| `--top-p <N>` | Top-P sampling |
| `--min-p <N>` | Min-P sampling |
| `--repeat-last-n <N>` | Repeat penalty window |
| `--repeat-penalty <N>` | Repeat penalty strength |
| `--max-tokens <N>` | Maximum tokens to generate; `-1` means unlimited |
| `--chat-mode <MODE>` | Group-chat mode: `action-value`, `round-robin`, `weighted-random`, or `directed` |
| `--talkativeness <VALUES>` | Per-character weights, such as `alice=0.7,bob=0.3` |
| `--scenario <TEXT>` | Set the session scenario |
| `--tls-skip-verify` | Disable TLS certificate verification |
| `--no-summarize` | Disable auto-summarization for this run |
| `--auth-type <TYPE>` | API auth type: `none`, `basic`, `bearer`, `header`, or `query` |
| `--auth-basic-username <NAME>` | Basic auth username |
| `--auth-header-name <NAME>` | Header auth field name |
| `--auth-query-name <NAME>` | Query auth parameter name |
| `--debug <PATH>` | Write a debug log to a specific path |
| `--log-filter <FILTER>` | Set the debug log filter; requires `--debug` |
| `--timings [PATH]` | Write a timings report; defaults to `./timings.log` when no path is given |
| `--cleanup` | Remove LibLLM temporary debug logs and exit |

Secret auth values come from environment variables so they do not appear in shell history or process listings:

```sh
LIBLLM_AUTH_BASIC_PASSWORD=secret libllm --auth-type basic --auth-basic-username alice
LIBLLM_AUTH_BEARER_TOKEN=sk-... libllm --auth-type bearer
LIBLLM_AUTH_HEADER_VALUE=secret libllm --auth-type header --auth-header-name X-Api-Key
LIBLLM_AUTH_QUERY_VALUE=secret libllm --auth-type query --auth-query-name api_key
```

CLI flags override matching `config.toml` values for that run. In the TUI, overridden config fields are shown as locked.

## One-Off And Persistent Messages

```sh
libllm -m "hello"
libllm -m - < prompt.txt
libllm -d ./data --no-encrypt -m "save this session"
libllm -d ./data --no-encrypt --continue <session-id> -m "continue it"
```

Without `--data`, `-m` does not save the session.

## Subcommands

### `update`

```sh
libllm update
libllm update stable
libllm update preview
libllm update feature/branch
libllm update -y feature/branch
```

`libllm update` opens an interactive picker on a TTY and updates stable in non-interactive shells.

### `recover`

```sh
libllm recover
libllm recover list
libllm recover restore <id>
libllm recover restore --yes <id>
libllm recover verify
libllm recover verify --full
libllm recover rebuild-index
```

Use `LIBLLM_ARCHIVED_PASSKEY` or `--archived-passkey` when verifying or restoring archived encrypted backups that were created with a different passkey.

### `edit`

```sh
libllm edit character <name>
libllm edit worldbook <name>
```

Opens the item in `$EDITOR`.

### `import`

```sh
libllm import card.json
libllm import card.png
libllm import --type persona persona.txt
libllm import --type prompt system.txt
libllm import card.json lore.json card2.png
```

Supported forced types are `character`, `char`, `worldbook`, `wb`, `book`, `persona`, `prompt`, and `system-prompt`.

### `search`

```sh
libllm search "redact pii"
libllm search "role:user redact" --limit 50
libllm search "\"exact phrase\"" --json
libllm search "after:2025-12-01 before:2026-01-15" --full
```

Options:

- `--limit <N>`: maximum result count, default `200`.
- `--json`: emit a JSON array.
- `--full`: print full message content instead of snippets.

### `db`

Use `libllm db` for direct database inspection and repair.

```sh
libllm db sql "SELECT slug, name FROM personas;"
libllm db sql --write "UPDATE personas SET name = 'Me' WHERE slug = 'me';"
libllm db shell
libllm db shell --write
libllm db dump backup.db
libllm db dump --yes backup.db
libllm db import edited.db
libllm db import --yes edited.db
```

`sql` and `shell` are read-only unless you pass `--write`. `dump` writes a decrypted SQLite copy. `import` replaces the live database from a plaintext SQLite file and creates a backup first.

`db shell` supports dot-commands such as `.help`, `.tables`, `.schema`, and `.read`.
