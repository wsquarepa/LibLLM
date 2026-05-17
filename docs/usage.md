# Usage Guide

## Interactive Chat

Start the TUI:

```sh
libllm
```

Type a message and press Enter. Press Alt+Enter for a newline. If a response is streaming, Esc cancels generation and keeps the partial response.

## One-Off Prompts

Use `-m` for script-friendly prompts:

```sh
libllm -m "Summarize this file" < document.txt
echo "Translate to French: hello world" | libllm -m -
```

Without `--data`, one-off prompts are ephemeral and are not saved.

## Persistent Scripted Conversations

Use a data directory to save and continue scripted sessions:

```sh
libllm -d ./project-data --no-encrypt -m "Explain quantum computing"
libllm -d ./project-data --no-encrypt --continue <session-id> -m "Now explain it to a 5-year-old"
```

The first command prints the session ID to stderr.

## Conversation Branching

LibLLM keeps alternate replies and edits as branches.

- `/retry` regenerates the last response.
- With an empty input box, press Up to select a previous user message, then Enter to edit it.
- Alt+Left and Alt+Right switch between sibling branches.
- `/branch` opens a branch picker at the current position.

## Keyboard Controls

| Key | Context | Action |
|---|---|---|
| Enter | Input | Send message |
| Alt+Enter | Input | Insert newline |
| Up | Empty input | Select previous user message |
| Enter | Message selected | Edit selected message |
| Left/Right | Message selected | Move through sibling branches |
| Tab | Global | Cycle focus between input, chat, and sidebar |
| Esc | Global | Return to input or cancel current navigation |
| Esc | Streaming | Cancel generation and keep partial text |
| Alt+Left/Alt+Right | Global | Switch conversation branches |
| Up/Down | Chat | Navigate messages |
| Enter | Chat | Edit selected message |
| Up/Down | Sidebar | Browse sessions |
| Delete | Sidebar | Delete selected session |
| Ctrl+F | Chat/Input | Open full-text search |
| Ctrl+F | Sidebar | Filter sessions by name |
| Ctrl+C | Global | Quit |

## Slash Commands

Type `/` in the input box to open the command picker.

| Command | Aliases | Description |
|---|---|---|
| `/clear` | `/new` | Start a new conversation |
| `/system` | | Select or edit the system prompt |
| `/retry` | | Regenerate the last response |
| `/continue` | `/cont` | Continue the last assistant response |
| `/branch` | | Browse branches at the current position |
| `/character` | | Select or manage character cards |
| `/chat` | | Edit scenario and group-chat settings |
| `/persona` | `/self`, `/user`, `/me` | Manage user personas |
| `/note` | `/an`, `/authornote` | View or edit the author's note |
| `/worldbook` | `/lore`, `/world`, `/lorebook` | Toggle worldbooks for the session |
| `/passkey` | `/password`, `/pass`, `/auth` | Set or change the encryption passkey |
| `/config` | | Open settings |
| `/theme [name]` | | Open theme settings or switch to `dark` / `light` |
| `/next [name]` | | Force the next group-chat turn |
| `/regex` | | Manage regex find/replace rules |
| `/export [html|md|jsonl]` | | Export the current branch |
| `/search [query]` | `/find` | Search stored messages |
| `/macro` | `/m` | Run a configured macro |
| `/report` | | Copy the active debug log to `./debug.log` |
| `/quit` | `/exit` | Exit |

## Characters And Personas

Character cards define the assistant character. Personas define the user identity.

Start roleplay mode from the CLI:

```sh
libllm -c character_name -p persona_name
```

Both `-c` and `-p` are required together. You can also manage characters and personas in the TUI with `/character` and `/persona`.

LibLLM supports JSON character cards and PNG cards with SillyTavern-compatible metadata.

## Group Chat

Attach two or more character cards to start a group chat:

```sh
libllm -c alice -c bob -p me --scenario "Alice and Bob are debating the merits of Rust"
```

If you omit `--scenario`, the TUI asks for one before the group chat starts.

Choose a turn-order mode with `--chat-mode`:

| Mode | Behavior |
|---|---|
| `action-value` | Default mode. Characters speak based on accumulated action values. |
| `round-robin` | Characters speak in attachment order. |
| `weighted-random` | Speakers are chosen using talkativeness weights. |
| `directed` | Characters speak only when you use `/next <name>`. |

Use `/chat` to edit the scenario, mode, and talkativeness sliders. Use `/next <name>` to force a specific character to respond.

## Side Characters

When a main character is attached, one user input can include temporary side-character lines:

```text
*I smile to the barkeeper.* "I'll have two beers, please."

[Barkeep]: "Coming right up."
```

Escape a literal bracketed header with a backslash:

```text
\[not a speaker]: this stays plain text
```

## Author's Note

Use `/note` to keep short steering text attached to the session. From the CLI:

```sh
libllm --note "Keep replies concise" --note-depth 3
libllm --note "Keep replies concise" --note-top
```

`--note-depth` controls how close to the latest messages the note is placed. `--note-top` pins it near the system prompt.

## Worldbooks

Worldbooks add keyword-triggered context. Use `/worldbook` to enable or disable worldbooks for the current session. Imported worldbooks can also be managed through `libllm import` and `libllm edit worldbook`.

## File Attachments

Use `@<path>` in a message to attach a file snapshot:

```text
Summarize @notes/project-plan.md
```

LibLLM stores the attached snapshot with the message, so retries and branch changes keep using the same file content.

File size limits and summary behavior are configured in `[files]` and `[summarization]`. See [Configuration](configuration.md).

## Search

Open search in the TUI with `/search` or Ctrl+F. Use `libllm search` from the CLI:

```sh
libllm search "redact pii"
libllm search "\"exact phrase\"" --json
libllm search "after:2025-12-01 before:2026-01-15" --full
```

Query forms:

| Form | Example |
|---|---|
| Terms | `redact pii` |
| Phrase | `"redact pii"` |
| Role filter | `role:user redact` |
| Date before | `before:2026-01-15` |
| Date after | `after:2025-12-01` |
| Session name | `session:feature` |
| Raw FTS5 | `m:redact OR pii` |

## Export

Export the current branch with `/export`:

```text
/export
/export html
/export md
/export jsonl
```

Exports are written to the current working directory as `export-<timestamp>.<ext>`.

## Macros

Define macros in `config.toml`:

```toml
[macros]
refactor = "Refactor the following code to be more idiomatic Rust: {{}}"
translate = "Translate from {{1}} to {{2}}: {{3..}}"
```

Run one in the TUI:

```text
/macro translate English French hello world
```

Placeholders:

| Syntax | Meaning |
|---|---|
| `{{}}` | All arguments |
| `{{1}}` | First argument |
| `{{N..M}}` | Arguments N through M |
| `{{N..}}` | Argument N and everything after |

## Regex Rules

Use `/regex` to manage ordered find/replace rules. Rules can target user, assistant, system, or summary text and can apply to display, prompt sending, prompt receiving, or export.
