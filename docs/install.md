# Installation

## Install Script

```sh
curl -fsSL https://raw.githubusercontent.com/wsquarepa/LibLLM/master/install.sh | sh
```

The script supports Linux and macOS. It asks for a release channel:

- `stable`: latest stable release.
- `preview`: bleeding-edge build from `master`.
- `branch`: prerelease build for a development branch.

By default it installs to an existing `libllm` location if one is found, `/usr/local/bin` when run as root, or `~/.local/bin` otherwise.

Set `INSTALL_DIR` to choose a directory:

```sh
INSTALL_DIR="$HOME/bin" sh -c "$(curl -fsSL https://raw.githubusercontent.com/wsquarepa/LibLLM/master/install.sh)"
```

For private release access, set `GITHUB_TOKEN` or `GH_TOKEN` before running the script.

## Release Downloads

Prebuilt release assets are published for:

- Linux: `x86_64`, `aarch64`
- macOS: `x86_64`, `aarch64`
- Windows: `x86_64`, `aarch64`

Download the matching `libllm-<target>` asset from [Releases](https://github.com/wsquarepa/LibLLM/releases), make it executable on Unix-like systems, and place it somewhere on your `PATH`.

## Build From Source

Requires the stable Rust toolchain from [rustup](https://rustup.rs/).

```sh
git clone https://github.com/wsquarepa/LibLLM.git
cd LibLLM
cargo build --release --workspace
```

The local Cargo binary is `target/release/client`. You can run it directly:

```sh
./target/release/client --help
```

To use the command name `libllm`, install or copy that binary under the name `libllm` somewhere on your `PATH`.

## Updating

```sh
libllm update
libllm update stable
libllm update preview
libllm update feature/branch
libllm update -y feature/branch
```

`libllm update` opens a channel picker when run from an interactive terminal. In non-interactive shells it updates to stable.

Switching channels may change application behavior or data expectations. Use `--yes` / `-y` only when you already know you want that channel.
