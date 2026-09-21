#!/bin/bash
# Mivon RTL Simulator Installation Script
# Version: 0.3.0
# Auto-updates from GitHub releases when new patches are available

set -euo pipefail

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }
print_step() { echo -e "${CYAN}[STEP]${NC} $1"; }

# Configuration
REPO="mivonsim/mivon"
BINARY_NAME="mivon"
INSTALL_DIR="${MIVON_INSTALL_DIR:-/usr/local/bin}"
VERSION_FILE="${MIVON_VERSION_FILE:-/tmp/mivon-version.txt}"
RELEASES_URL="https://api.github.com/repos/${REPO}/releases/latest"
RAW_URL="https://github.com/${REPO}/releases/download"

# Detect OS and architecture
detect_platform() {
    local os arch
    
    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="macos" ;;
        *) print_error "Unsupported OS: $(uname -s)"; exit 1 ;;
    esac
    
    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) print_error "Unsupported architecture: $(uname -m)"; exit 1 ;;
    esac
    
    echo "${arch}-${os}-gnu"
}

# Get latest release version from GitHub
get_latest_version() {
    local platform=$1
    local version
    
    version=$(curl -sL "${RELEASES_URL}" | grep -o '"tag_name": "v[^"]*"' | cut -d'"' -f4 2>/dev/null || echo "")
    
    if [ -z "$version" ]; then
        print_warn "Could not fetch latest version from GitHub API"
        return 1
    fi
    
    echo "$version"
}

# Check if update is available
check_for_updates() {
    local current_version=$1
    local latest_version=$2
    
    if [ "$current_version" != "$latest_version" ]; then
        print_info "New version available: $latest_version (current: $current_version)"
        return 0
    else
        print_info "Mivon is up to date (v$current_version)"
        return 1
    fi
}

# Download and install binary
install_binary() {
    local version=$1
    local platform=$2
    local tmpfile="/tmp/mivon-${version}-${platform}"
    
    print_step "Downloading Mivon v${version} for ${platform}..."
    
    if ! curl -fsSL "${RAW_URL}/${version}/mivon" -o "${tmpfile}"; then
        print_error "Failed to download binary"
        exit 1
    fi

    # Verifikasi SHA-256 terhadap release resmi: jangan pasang binary yang
    # checksum-nya tidak cocok (fail-closed).
    if ! curl -fsSL "${RAW_URL}/${version}/mivon.sha256" -o "${tmpfile}.sha256"; then
        print_error "Failed to download checksum (${version}/mivon.sha256)"
        rm -f "${tmpfile}"
        exit 1
    fi
    expected=$(awk '{print $1}' "${tmpfile}.sha256")
    actual=$(sha256sum "${tmpfile}" | awk '{print $1}')
    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        print_error "Checksum mismatch! expected ${expected:-?}, got ${actual}"
        rm -f "${tmpfile}" "${tmpfile}.sha256"
        exit 1
    fi
    print_info "Checksum SHA-256 verified"
    rm -f "${tmpfile}.sha256"

    chmod +x "${tmpfile}"
    
    print_step "Installing to ${INSTALL_DIR}..."
    if [ -w "${INSTALL_DIR}" ]; then
        mv "${tmpfile}" "${INSTALL_DIR}/${BINARY_NAME}"
    else
        print_warn "Need sudo to install to ${INSTALL_DIR}"
        sudo mv "${tmpfile}" "${INSTALL_DIR}/${BINARY_NAME}"
    fi
    
    print_info "Mivon v${version} installed successfully!"
}

# Verify installation
verify_installation() {
    if command -v mivon &> /dev/null; then
        print_info "Mivon binary verified:"
        mivon --version 2>/dev/null || mivon --help | head -1
    else
        print_error "Mivon binary not found in PATH"
        print_info "Add ${INSTALL_DIR} to your PATH:"
        print_info "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        exit 1
    fi
}

# Self-update function
self_update() {
    local current_version
    current_version=$(grep -E '^version =' Cargo.toml 2>/dev/null | cut -d'"' -f2 || echo "0.0.0")
    
    print_step "Checking for updates (current: v${current_version})..."
    
    local latest_version
    latest_version=$(get_latest_version "$(detect_platform)" 2>/dev/null || echo "")
    
    if [ -z "$latest_version" ]; then
        print_warn "Could not determine latest version, staying on v${current_version}"
        return 0
    fi
    
    if check_for_updates "$current_version" "$latest_version"; then
        echo "$latest_version" > "$VERSION_FILE"
        install_binary "$latest_version" "$(detect_platform)"
    fi
}

# Main installation
main() {
    echo "=========================================="
    echo "  Mivon RTL Simulator Installation"
    echo "=========================================="
    echo ""
    
    # If --update flag, just self-update
    if [ "${1:-}" = "--update" ] || [ "${1:-}" = "-u" ]; then
        self_update
        verify_installation
        exit 0
    fi
    
    # Check if Mivon is already installed
    if command -v mivon &> /dev/null; then
        print_info "Mivon is already installed"
        mivon --version 2>/dev/null || true
        
        read -rp "Would you like to check for updates? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            self_update
        fi
    else
        print_step "Installing Mivon..."
        local platform
        platform=$(detect_platform)
        
        local version
        version=$(get_latest_version "$platform" 2>/dev/null || echo "v0.3.0")
        
        install_binary "$version" "$platform"
    fi
    
    echo ""
    verify_installation
    echo ""
    print_info "Installation complete!"
    print_info "Run 'mivon --help' to get started"
}

main "$@"