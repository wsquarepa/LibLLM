# Troubleshooting

## Cannot Connect To The API

LibLLM expects a llama.cpp-compatible text completions server at the configured base URL. The default is `http://localhost:5001/v1`, and LibLLM sends requests to `/v1/completions`.

Check:

- The server is running.
- The port and host match `--api-url`, `LIBLLM_API_URL`, or `api_url` in `config.toml`.
- The server supports `/v1/completions`, not only `/v1/chat/completions`.
- TLS settings are correct if you use HTTPS.

For a self-signed HTTPS server, use:

```sh
libllm --tls-skip-verify
```

## Forgot Passkey

There is no passkey recovery. Without the passkey, encrypted data cannot be opened.

You can start over with a new data directory:

```sh
libllm -d ./new-data --no-encrypt
```

If you intentionally want a fresh default profile, move the old default data directory out of the way first so you still have a copy.

## Sessions Appear Missing

Make sure you are using the same data directory and encryption mode as before:

```sh
libllm -d ./project-data
libllm -d ./project-data --no-encrypt
```

Encrypted sessions also require the same passkey.

## Plaintext And Encrypted Modes Are Mixed

LibLLM rejects unsafe combinations:

- `--no-encrypt` on an encrypted data directory.
- `--passkey` on a plaintext data directory.
- Creating a new encrypted salt beside an existing database without `.salt`.

Use a different `--data` directory if you want a separate encryption mode.

## A Character, Persona, Worldbook, Or Session Is Stuck

Use `libllm db` to inspect or repair rows directly:

```sh
libllm db sql "SELECT slug, name FROM personas;"
libllm db shell --write
```

Before making manual edits, create or verify a backup:

```sh
libllm recover list
libllm db dump backup.db
```

## Update Problems

Check the installed version:

```sh
libllm --version
```

Try updating a known channel:

```sh
libllm update stable
libllm update preview
```

For private repository access, set `GITHUB_TOKEN` or `GH_TOKEN`.

## Recover From A Backup

```sh
libllm recover list
libllm recover verify
libllm recover restore <id>
```

For encrypted backups from an older passkey, use `--archived-passkey` or `LIBLLM_ARCHIVED_PASSKEY`.

## Debug Logs

Create a debug log for a run:

```sh
libllm --debug ./libllm-debug.log
libllm -m "hello" --debug ./single-run.log
```

Set the log filter:

```sh
libllm --debug ./libllm-debug.log --log-filter debug
LIBLLM_LOG=info,libllm::db=debug libllm --debug ./libllm-debug.log
```

`LIBLLM_LOG` is ignored unless `--debug` is set. Inside the TUI, `/report` copies the active debug log to `./debug.log`.

Clean up temporary logs:

```sh
libllm --cleanup
```

## Dev Build Requires `--data`

A local development build may print:

```text
You are running a dev build. Use --data/-d to specify a data directory.
```

Run it with an explicit data directory:

```sh
./target/release/client --data ./dev-data
./target/debug/client --data ./dev-data --no-encrypt
```
