#!/bin/sh
set -e

REPO="wsquarepa/LibLLM"
BINARY_NAME="libllm"

main() {
    detect_existing_install
    detect_fetcher
    select_channel
    detect_platform
    resolve_install_dir
    download_binary
    install_binary
    print_success
}

select_channel() {
    printf "Select release channel:\n"
    printf "  1) stable  - latest stable release\n"
    printf "  2) branch  - pick a development branch\n"

    while true; do
        printf "Choice [1/2]: "
        read -r choice < /dev/tty
        case "$choice" in
            1) CHANNEL="stable"; break ;;
            2) select_branch; break ;;
            *) printf "Invalid choice. Enter 1 or 2.\n" ;;
        esac
    done
}

select_branch() {
    AUTH=$(auth_header)

    RELEASES_JSON=$(mktemp)
    trap 'rm -f "$RELEASES_JSON"' EXIT

    API_URL="https://api.github.com/repos/${REPO}/releases?per_page=100"
    if [ "$FETCHER" = "curl" ]; then
        curl -sL ${AUTH:+-H "$AUTH"} -o "$RELEASES_JSON" "$API_URL"
    else
        wget -q ${AUTH:+--header="$AUTH"} -O "$RELEASES_JSON" "$API_URL"
    fi

    BRANCHES=$(grep -o '"tag_name": *"[^"]*"' "$RELEASES_JSON" \
        | sed 's/"tag_name": *"//;s/"//' \
        | grep -v '^stable$')
    rm -f "$RELEASES_JSON"

    if [ -z "$BRANCHES" ]; then
        printf "No branch builds available. Falling back to stable.\n"
        CHANNEL="stable"
        return
    fi

    printf "\nAvailable branches:\n"
    i=1
    for b in $BRANCHES; do
        printf "  %d) %s\n" "$i" "$b"
        i=$((i + 1))
    done

    while true; do
        printf "Select branch number: "
        read -r num < /dev/tty
        SELECTED=$(echo "$BRANCHES" | sed -n "${num}p")
        if [ -n "$SELECTED" ]; then
            CHANNEL="$SELECTED"
            break
        fi
        printf "Invalid selection.\n"
    done
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

detect_existing_install() {
    EXISTING_BINARY=$(command -v "$BINARY_NAME" 2>/dev/null || true)
}

resolve_install_dir() {
    if [ -n "$INSTALL_DIR" ]; then
        BIN_DIR="$INSTALL_DIR"
    elif [ -n "$EXISTING_BINARY" ]; then
        BIN_DIR=${EXISTING_BINARY%/*}
        echo "libllm is already installed at ${EXISTING_BINARY}. Reinstalling in place."
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

detect_fetcher() {
    if command -v curl >/dev/null 2>&1; then
        FETCHER="curl"
    elif command -v wget >/dev/null 2>&1; then
        FETCHER="wget"
    else
        echo "Error: curl or wget is required." >&2
        exit 1
    fi
}

download_binary() {
    if [ "$CHANNEL" = "stable" ]; then
        API_URL="https://api.github.com/repos/${REPO}/releases/tags/stable"
    else
        API_URL="https://api.github.com/repos/${REPO}/releases/tags/${CHANNEL}"
    fi

    AUTH=$(auth_header)

    RELEASE_JSON=$(mktemp)
    trap 'rm -f "$RELEASE_JSON"' EXIT

    if [ "$FETCHER" = "curl" ]; then
        HTTP_CODE=$(curl -sL -w "%{http_code}" -o "$RELEASE_JSON" ${AUTH:+-H "$AUTH"} "$API_URL")
    else
        HTTP_CODE=$(wget -q --server-response -O "$RELEASE_JSON" ${AUTH:+--header="$AUTH"} "$API_URL" 2>&1 | awk '/HTTP\//{print $2}' | tail -1)
    fi

    if [ "$HTTP_CODE" != "200" ]; then
        if [ -z "${GITHUB_TOKEN:-$GH_TOKEN}" ]; then
            echo "Error: GitHub API returned $HTTP_CODE." >&2
            echo "If the repository is private, set GITHUB_TOKEN or GH_TOKEN." >&2
        else
            echo "Error: GitHub API returned $HTTP_CODE. Check that your token has repository access." >&2
        fi
        rm -f "$RELEASE_JSON"
        exit 1
    fi

    ASSET_API_URL=$(grep -B3 "\"name\": *\"${ASSET_NAME}\"" "$RELEASE_JSON" \
        | grep -o "https://api.github.com/repos/${REPO}/releases/assets/[0-9]*" \
        | head -1)

    rm -f "$RELEASE_JSON"

    if [ -z "$ASSET_API_URL" ]; then
        echo "Error: no release asset found for ${ASSET_NAME}." >&2
        echo "Available platforms can be checked at: https://github.com/${REPO}/releases" >&2
        exit 1
    fi

    TMPFILE=$(mktemp)
    trap 'rm -f "$TMPFILE"' EXIT

    echo "Downloading ${ASSET_NAME} (${CHANNEL})..."

    if [ "$FETCHER" = "curl" ]; then
        curl -fSL -H "Accept: application/octet-stream" ${AUTH:+-H "$AUTH"} -o "$TMPFILE" "$ASSET_API_URL"
    else
        wget -q --header="Accept: application/octet-stream" ${AUTH:+--header="$AUTH"} -O "$TMPFILE" "$ASSET_API_URL"
    fi
}

install_binary() {
    chmod +x "$TMPFILE"
    mv "$TMPFILE" "${BIN_DIR}/${BINARY_NAME}"
    trap - EXIT
}

print_success() {
    echo "Installed libllm (${CHANNEL}) to ${BIN_DIR}/${BINARY_NAME}"

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
