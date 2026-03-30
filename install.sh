#!/bin/sh
set -e

REPO="wsquarepa/LibLLM"
TAG="nightly"
BINARY_NAME="libllm"

main() {
    if command -v "$BINARY_NAME" >/dev/null 2>&1; then
        echo "libllm is already installed. Running 'libllm update' instead."
        exec "$BINARY_NAME" update
    fi

    detect_platform
    resolve_install_dir
    download_binary
    install_binary
    print_success
}

detect_platform() {
    OS=$(uname -s)
    ARCH=$(uname -m)

    case "$OS" in
        Linux)  OS_TARGET="unknown-linux-gnu" ;;
        Darwin) OS_TARGET="apple-darwin" ;;
        *)      echo "Error: unsupported operating system: $OS" >&2; exit 1 ;;
    esac

    case "$ARCH" in
        x86_64)         ARCH_TARGET="x86_64" ;;
        aarch64|arm64)  ARCH_TARGET="aarch64" ;;
        *)              echo "Error: unsupported architecture: $ARCH" >&2; exit 1 ;;
    esac

    TARGET="${ARCH_TARGET}-${OS_TARGET}"
    ASSET_NAME="${BINARY_NAME}-${TARGET}"
}

resolve_install_dir() {
    if [ -n "$INSTALL_DIR" ]; then
        BIN_DIR="$INSTALL_DIR"
    elif [ "$(id -u)" = "0" ]; then
        BIN_DIR="/usr/local/bin"
    else
        BIN_DIR="$HOME/.local/bin"
    fi

    mkdir -p "$BIN_DIR"
}

auth_header() {
    TOKEN="${GITHUB_TOKEN:-$GH_TOKEN}"
    if [ -n "$TOKEN" ]; then
        echo "Authorization: Bearer $TOKEN"
    fi
}

download_binary() {
    API_URL="https://api.github.com/repos/${REPO}/releases/tags/${TAG}"

    if command -v curl >/dev/null 2>&1; then
        FETCHER="curl"
    elif command -v wget >/dev/null 2>&1; then
        FETCHER="wget"
    else
        echo "Error: curl or wget is required." >&2
        exit 1
    fi

    AUTH=$(auth_header)

    if [ "$FETCHER" = "curl" ]; then
        HTTP_CODE=$(curl -sL -w "%{http_code}" -o /dev/null ${AUTH:+-H "$AUTH"} "$API_URL")
    else
        HTTP_CODE=$(wget -q --server-response -O /dev/null ${AUTH:+--header="$AUTH"} "$API_URL" 2>&1 | awk '/HTTP\//{print $2}' | tail -1)
    fi

    if [ "$HTTP_CODE" = "404" ] || [ "$HTTP_CODE" = "401" ]; then
        if [ -z "${GITHUB_TOKEN:-$GH_TOKEN}" ]; then
            echo "Error: GitHub API returned $HTTP_CODE." >&2
            echo "If the repository is private, set GITHUB_TOKEN or GH_TOKEN." >&2
        else
            echo "Error: GitHub API returned $HTTP_CODE. Check that your token has repository access." >&2
        fi
        exit 1
    fi

    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET_NAME}"

    TMPFILE=$(mktemp)
    trap 'rm -f "$TMPFILE"' EXIT

    echo "Downloading ${ASSET_NAME}..."

    if [ "$FETCHER" = "curl" ]; then
        curl -fSL ${AUTH:+-H "$AUTH"} -o "$TMPFILE" "$DOWNLOAD_URL"
    else
        wget -q ${AUTH:+--header="$AUTH"} -O "$TMPFILE" "$DOWNLOAD_URL"
    fi
}

install_binary() {
    chmod +x "$TMPFILE"
    mv "$TMPFILE" "${BIN_DIR}/${BINARY_NAME}"
    trap - EXIT
}

print_success() {
    echo "Installed libllm to ${BIN_DIR}/${BINARY_NAME}"

    case ":$PATH:" in
        *":${BIN_DIR}:"*) ;;
        *)
            echo ""
            echo "Add ${BIN_DIR} to your PATH:"
            echo "  export PATH=\"${BIN_DIR}:\$PATH\""
            ;;
    esac
}

main
