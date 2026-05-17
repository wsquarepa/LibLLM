# Configuration

Configuration lives in `<data_dir>/config.toml`. The default data directory is `~/.local/share/libllm/`.

Open settings from the TUI with `/config`, or edit the file directly while LibLLM is not running.

## Data Directory

Use `--data` / `-d` to choose a directory:

```sh
libllm -d ./project-data
libllm -d ./project-data --no-encrypt
```

Useful files and directories:

```text
<data_dir>/
  config.toml
  data.db
  .salt
  backups/
  presets/
```

`config.toml` stores settings. `data.db` stores conversations and user content. `.salt` is required for encrypted data. `backups/` stores recovery points when backups are enabled.

## Encryption

Encryption is enabled by default for saved TUI sessions. On first launch, LibLLM asks you to set a passkey.

Use plaintext storage only when you explicitly want it:

```sh
libllm -d ./data --no-encrypt
```

Important behavior:

- There is no passkey recovery.
- `--no-encrypt` and `--passkey` require `--data`.
- A data directory must keep the same encryption mode. LibLLM rejects `--no-encrypt` on encrypted data and rejects `--passkey` on plaintext data.
- Change the passkey from the TUI with `/passkey`.

## Example Config

```toml
api_url = "http://localhost:5001/v1"
instruct_preset = "Mistral V3-Tekken"
reasoning_preset = "OFF"
template_preset = "Default"
worldbooks = ["fantasy-lore", "tech-terms"]
tls_skip_verify = false
theme = "dark"

[sampling]
temperature = 0.8
top_k = 40
top_p = 0.95
min_p = 0.05
repeat_last_n = 64
repeat_penalty = 1.0
max_tokens = -1

[auth]
type = "none"

[summarization]
enabled = true
context_size = 131072
trigger_percent = 90
keep_last = 4

[files]
enabled = true
per_file_bytes = 524288
per_message_bytes = 4194304
summarize_mode = "eager"

[backup]
enabled = true
keep_all_days = 7
keep_daily_days = 30
keep_weekly_days = 90
rebase_threshold_percent = 50
rebase_hard_ceiling = 10
```

## API URL

Set the base URL without `/completions`:

```toml
api_url = "http://localhost:5001/v1"
```

Override it for one run:

```sh
libllm --api-url http://localhost:8080/v1
LIBLLM_API_URL=http://localhost:8080/v1 libllm
```

## Authentication

Configure API authentication in `/config`, in `[auth]`, or with CLI flags plus secret environment variables.

```toml
[auth]
type = "bearer"
token = "sk-..."
```

CLI examples:

```sh
LIBLLM_AUTH_BEARER_TOKEN=sk-... libllm --auth-type bearer
LIBLLM_AUTH_BASIC_PASSWORD=secret libllm --auth-type basic --auth-basic-username alice
LIBLLM_AUTH_HEADER_VALUE=secret libllm --auth-type header --auth-header-name X-Api-Key
LIBLLM_AUTH_QUERY_VALUE=secret libllm --auth-type query --auth-query-name api_key
```

## Presets

- `instruct_preset`: prompt format for the model. Default is `Mistral V3-Tekken`.
- `reasoning_preset`: optional reasoning wrapper. `OFF` disables it.
- `template_preset`: context template for character/persona formatting. Default is `Default`.

Built-in instruct presets include `Mistral V3-Tekken`, `Llama 3 Instruct`, `ChatML`, `Phi`, and `Alpaca`.

## Sampling

Set defaults under `[sampling]` or override them per run:

```sh
libllm --temperature 0.5 --top-p 0.9 --max-tokens 512
```

CLI sampling flags take priority over config values for that run.

## Summarization

Auto-summarization keeps long sessions usable as they approach the configured context limit.

```toml
[summarization]
enabled = true
context_size = 131072
trigger_percent = 90
keep_last = 4
# api_url = "http://localhost:5001/v1"
# prompt = "Summarize the conversation..."
```

Disable for one run:

```sh
libllm --no-summarize
```

Or disable in config:

```toml
[summarization]
enabled = false
```

## File Attachments

File attachments use `@<path>` tokens in messages. Configure limits under `[files]`:

```toml
[files]
enabled = true
per_file_bytes = 524288
per_message_bytes = 4194304
summarize_mode = "eager"
summary_prompt = "Summarize this file. Focus on facts useful for answering questions."
```

`summarize_mode` can be:

- `eager`: start summaries when files are attached.
- `lazy`: summarize only when needed.

## Backups

Backups are enabled by default.

```toml
[backup]
enabled = true
keep_all_days = 7
keep_daily_days = 30
keep_weekly_days = 90
rebase_threshold_percent = 50
rebase_hard_ceiling = 10
```

Use `libllm recover` to list, verify, and restore backup points.

## Themes

Switch themes in the TUI:

```text
/theme dark
/theme light
```

Set the default:

```toml
theme = "dark"
```

Override individual colors with `[theme_colors]`:

```toml
[theme_colors]
border_focused = "cyan"
file_reference_fg = "blue"
token_band_ok = "green"
token_band_warn = "yellow"
token_band_over = "red"
```

Color values can be named colors, hex values like `#RRGGBB`, or indexed terminal colors like `indexed(236)`.

## Macros

```toml
[macros]
refactor = "Refactor the following code to be more idiomatic Rust: {{}}"
compare = "Compare {{1}} with {{2}}"
translate = "Translate from {{1}} to {{2}}: {{3..}}"
```

Run macros with `/macro <name> <args...>`.

## Regex Rules

Most users should manage regex rules with `/regex`. If you edit `config.toml` directly, rules live under `[[regex]]` entries:

```toml
[[regex]]
name = "hide-thoughts"
pattern = "(?s)<think>.*?</think>"
replacement = ""
scope = ["display", "export"]
target = ["assistant"]
enabled = true
```
