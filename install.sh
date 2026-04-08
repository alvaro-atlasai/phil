#!/bin/sh
# Phil installer — curl -fsSL https://raw.githubusercontent.com/alvaro-atlasai/phil/main/install.sh | sh
#
# Installs phil and any2mcp to /usr/local/bin (or ~/.local/bin if no write access).

set -eu

REPO="alvaro-atlasai/phil"
INSTALL_DIR="/usr/local/bin"
TMPDIR_BASE="${TMPDIR:-/tmp}"

# Colors (disabled if not a terminal)
if [ -t 1 ]; then
    BOLD='\033[1m'
    GREEN='\033[32m'
    RED='\033[31m'
    RESET='\033[0m'
else
    BOLD='' GREEN='' RED='' RESET=''
fi

info()  { printf "${GREEN}▸${RESET} %s\n" "$1"; }
error() { printf "${RED}✗${RESET} %s\n" "$1" >&2; exit 1; }

# Detect OS and arch
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Darwin) OS_NAME="darwin" ;;
        Linux)  OS_NAME="linux" ;;
        *)      error "Unsupported OS: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64)   ARCH_NAME="amd64" ;;
        arm64|aarch64)  ARCH_NAME="arm64" ;;
        *)              error "Unsupported architecture: $ARCH" ;;
    esac

    PLATFORM="${OS_NAME}-${ARCH_NAME}"
}

# Get latest release tag
get_latest_version() {
    if command -v curl >/dev/null 2>&1; then
        VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//')"
    elif command -v wget >/dev/null 2>&1; then
        VERSION="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//')"
    else
        error "curl or wget is required"
    fi

    [ -z "$VERSION" ] && error "Could not determine latest version. Check https://github.com/${REPO}/releases"
}

download() {
    URL="https://github.com/${REPO}/releases/download/${VERSION}/phil-${PLATFORM}.tar.gz"
    TMPDIR="$(mktemp -d "${TMPDIR_BASE}/phil-install.XXXXXX")"
    trap 'rm -rf "$TMPDIR"' EXIT

    info "Downloading phil ${VERSION} for ${PLATFORM}..."
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$URL" -o "${TMPDIR}/phil.tar.gz"
    else
        wget -qO "${TMPDIR}/phil.tar.gz" "$URL"
    fi

    tar xzf "${TMPDIR}/phil.tar.gz" -C "$TMPDIR"
}

install() {
    # Fall back to ~/.local/bin if /usr/local/bin isn't writable
    if [ ! -w "$INSTALL_DIR" ]; then
        INSTALL_DIR="${HOME}/.local/bin"
        mkdir -p "$INSTALL_DIR"
    fi

    for bin in phil any2mcp; do
        if [ -f "${TMPDIR}/${bin}" ]; then
            cp "${TMPDIR}/${bin}" "${INSTALL_DIR}/${bin}"
            chmod +x "${INSTALL_DIR}/${bin}"
            info "Installed ${bin} → ${INSTALL_DIR}/${bin}"
        fi
    done

    # Check PATH
    case ":$PATH:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            printf "\n${BOLD}Add to your shell profile:${RESET}\n"
            printf "  export PATH=\"%s:\$PATH\"\n\n" "$INSTALL_DIR"
            ;;
    esac
}

verify() {
    if command -v phil >/dev/null 2>&1; then
        printf "\n${GREEN}✓${RESET} ${BOLD}phil ${VERSION} installed successfully${RESET}\n"
        printf "\n"
        printf "  Get started:\n"
        printf "    phil \"what is the capital of France?\"\n"
        printf "    echo 'hello world' | phil \"translate to Spanish\"\n"
        printf "    phil model ls\n"
        printf "    phil pack ls\n"
        printf "\n"
        printf "  First run downloads the Phi-4-mini model (~2.5GB).\n"
    else
        printf "\n${GREEN}✓${RESET} Binaries installed to ${INSTALL_DIR}\n"
        printf "  You may need to restart your shell or add ${INSTALL_DIR} to PATH.\n"
    fi
}

main() {
    printf "\n${BOLD}Phil Installer${RESET}\n\n"
    detect_platform
    get_latest_version
    download
    install
    verify
}

main
