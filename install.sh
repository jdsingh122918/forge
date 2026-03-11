#!/usr/bin/env sh
set -eu

# Forge installer — downloads pre-built binary from GitHub Releases.
# Usage: curl -sSf https://raw.githubusercontent.com/jdsingh122918/forge/main/install.sh | sh
# Pin a version: FORGE_VERSION=v0.2.0 curl ... | sh

REPO="jdsingh122918/forge"
INSTALL_DIR="${HOME}/.forge/bin"

main() {
    detect_platform
    get_version
    download_and_verify
    install_binary
    configure_path
    print_success
}

detect_platform() {
    OS=$(uname -s)
    ARCH=$(uname -m)

    case "$OS" in
        Darwin) ;;
        Linux) ;;
        *)
            echo "Error: Unsupported OS: $OS"
            echo "Forge supports macOS and Linux."
            exit 1
            ;;
    esac

    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        arm64|aarch64) ARCH="aarch64" ;;
        *)
            echo "Error: Unsupported architecture: $ARCH"
            exit 1
            ;;
    esac

    case "${OS}-${ARCH}" in
        Darwin-x86_64)  TARGET="x86_64-apple-darwin" ;;
        Darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
        Linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
        Linux-aarch64)  TARGET="aarch64-unknown-linux-gnu" ;;
        *)
            echo "Error: Unsupported platform: ${OS}-${ARCH}"
            exit 1
            ;;
    esac

    echo "Detected platform: ${TARGET}"
}

get_version() {
    if [ -n "${FORGE_VERSION:-}" ]; then
        VERSION="$FORGE_VERSION"
        echo "Installing pinned version: ${VERSION}"
    else
        echo "Fetching latest release..."
        VERSION=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

        if [ -z "$VERSION" ]; then
            echo "Error: Could not determine latest version."
            echo "Check https://github.com/${REPO}/releases"
            exit 1
        fi
        echo "Latest version: ${VERSION}"
    fi
}

download_and_verify() {
    ARCHIVE="forge-${TARGET}.tar.gz"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
    CHECKSUMS_URL="https://github.com/${REPO}/releases/download/${VERSION}/sha256sums.txt"

    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    echo "Downloading ${ARCHIVE}..."
    curl -sSfL -o "${TMPDIR}/${ARCHIVE}" "$URL" || {
        echo "Error: Failed to download ${URL}"
        echo "Check that version ${VERSION} exists at https://github.com/${REPO}/releases"
        exit 1
    }

    echo "Downloading checksums..."
    curl -sSfL -o "${TMPDIR}/sha256sums.txt" "$CHECKSUMS_URL" || {
        echo "Error: Failed to download checksums."
        exit 1
    }

    echo "Verifying checksum..."
    EXPECTED=$(grep "${ARCHIVE}" "${TMPDIR}/sha256sums.txt" | awk '{print $1}')
    if [ -z "$EXPECTED" ]; then
        echo "Error: Archive not found in checksums file."
        exit 1
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        ACTUAL=$(sha256sum "${TMPDIR}/${ARCHIVE}" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        ACTUAL=$(shasum -a 256 "${TMPDIR}/${ARCHIVE}" | awk '{print $1}')
    else
        echo "Warning: No sha256sum or shasum found, skipping verification."
        ACTUAL="$EXPECTED"
    fi

    if [ "$EXPECTED" != "$ACTUAL" ]; then
        echo "Error: Checksum verification failed!"
        echo "  Expected: ${EXPECTED}"
        echo "  Actual:   ${ACTUAL}"
        exit 1
    fi
    echo "Checksum verified."

    echo "Extracting..."
    tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}"
}

install_binary() {
    mkdir -p "$INSTALL_DIR"
    mv "${TMPDIR}/forge" "${INSTALL_DIR}/forge"
    chmod +x "${INSTALL_DIR}/forge"
    echo "Installed to ${INSTALL_DIR}/forge"
}

configure_path() {
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) return ;; # Already in PATH
    esac

    EXPORT_LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""
    SHELL_NAME=$(basename "${SHELL:-/bin/sh}")

    case "$SHELL_NAME" in
        zsh)
            RC_FILE="${HOME}/.zshrc"
            ;;
        bash)
            # macOS uses .bash_profile for login shells
            if [ "$(uname -s)" = "Darwin" ] && [ -f "${HOME}/.bash_profile" ]; then
                RC_FILE="${HOME}/.bash_profile"
            else
                RC_FILE="${HOME}/.bashrc"
            fi
            ;;
        fish)
            FISH_CONFIG="${HOME}/.config/fish/config.fish"
            RC_FILE="$FISH_CONFIG"
            if [ -f "$FISH_CONFIG" ] && ! grep -q "${INSTALL_DIR}" "$FISH_CONFIG" 2>/dev/null; then
                echo "fish_add_path ${INSTALL_DIR}" >> "$FISH_CONFIG"
                echo "Added ${INSTALL_DIR} to ${FISH_CONFIG}"
            fi
            return
            ;;
        *)
            RC_FILE="${HOME}/.profile"
            ;;
    esac

    if [ -f "$RC_FILE" ] && grep -qF "$INSTALL_DIR" "$RC_FILE" 2>/dev/null; then
        return # Already configured
    fi

    echo "" >> "$RC_FILE"
    echo "# Added by Forge installer" >> "$RC_FILE"
    echo "$EXPORT_LINE" >> "$RC_FILE"
    echo "Added ${INSTALL_DIR} to PATH in ${RC_FILE}"
}

print_success() {
    VERSION_NUM=$("${INSTALL_DIR}/forge" --version 2>/dev/null | awk '{print $2}' || echo "${VERSION}")
    echo ""
    echo "Forge ${VERSION_NUM} installed successfully!"
    echo ""
    echo "To get started, run:"
    echo "  source ${RC_FILE:-~/.profile}  # or open a new terminal"
    echo "  forge --help"
    echo ""
}

main
