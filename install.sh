#!/bin/bash

# maria Simulator Installation Script
# Automatically detects and installs the latest stable release
n
# Strict mode - exit on error
set -e
n
# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color
n
# Function to print colored output
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to detect the current platform
detect_platform() {
    local arch="$(uname -m)"
    local os="$(uname -s)"

    case "${arch}" in
        x86_64) ARCH="x86_64" ;;
        arm64) ARCH="aarch64" ;;
        *) print_error "Unsupported architecture: ${arch}"; exit 1 ;;
    esac

    case "${os}" in
        Linux*) OS="unknown-linux-gnu" ;;
        Darwin*) OS="apple-darwin" ;;
        *) print_error "Unsupported OS: ${os}"; exit 1 ;;
    esac

    echo "${ARCH}-${OS}"
}

# Function to fetch the latest release version from GitHub API
fetch_latest_version() {
    local repo="Yoriyoi-drop/maria"
    
    if command -v curl >/dev/null 2>&1; then
        VERSION=$(curl -s "https://api.github.com/repos/${repo}/releases/latest" | grep -o '"tag_name": "v[^"]*' | cut -d'"' -f4)
    elif command -v wget >/dev/null 2>&1; then
        VERSION=$(wget -qO- "https://api.github.com/repos/${repo}/releases/latest" | grep -o '"tag_name": "v[^"]*' | cut -d'"' -f4)
    else
        print_error "Neither curl nor wget found. Please install one of them."
        exit 1
    fi

    if [[ "$VERSION" == v* ]]; then
        echo "${VERSION#v}"
    else
        print_error "Failed to fetch version from GitHub API"
        exit 1
    fi
}

# Function to install the latest stable release
install_latest() {
    local version="$1"
    local platform="$2"
    
    print_info "Installing Maria v${version} for ${platform}"
    
    # Download URL
    local download_url="https://github.com/Yoriyoi-drop/maria/releases/download/v${version}/maria-${platform}"
    
    # Check if we have the correct binary name
    if ! curl -s -f "${download_url}" >/dev/null 2>&1; then
        # Try alternative naming
        download_url="https://github.com/Yoriyoi-drop/maria/releases/download/v${version}/maria"
    fi
    
    # Download the binary
    print_info "Downloading from ${download_url}"
    
    if command -v curl >/dev/null 2>&1; then
        curl -L -o /usr/local/bin/maria "${download_url}" 2>/dev/null
    elif command -v wget >/dev/null 2>&1; then
        wget -O /usr/local/bin/maria "${download_url}"
    fi

    # Verify download
    if [[ ! -f "/usr/local/bin/maria" ]]; then
        print_error "Download failed. Please check your internet connection."
        exit 1
    fi

    # Make it executable
    chmod +x /usr/local/bin/maria

    # Verify installation
    if command -v /usr/local/bin/maria >/dev/null 2>&1; then
        local installed_version="$($/usr/local/bin/maria --version 2>/dev/null || echo 'unknown')"
        print_info "Maria v${installed_version} installed successfully!"
        
        # Add to PATH if not already in PATH
        if ! command -v maria >/dev/null 2>&1; then
            # Add to user's PATH
            if [[ "$(uname)" == "Darwin" ]]; then
                echo "export PATH=\$PATH:/usr/local/bin" >> ~/.zshrc
                echo "Added /usr/local/bin to PATH in ~/.zshrc"
            else
                echo "export PATH=\$PATH:/usr/local/bin" >> ~/.bashrc
                echo "Added /usr/local/bin to PATH in ~/.bashrc"
            fi
            print_warn "Please restart your shell or run 'source ~/.bashrc' to update PATH"
        fi
    else
        print_error "Installation failed. Please check permissions."
        exit 1
    fi
}

# Function to build from source
build_from_source() {
    print_info "Building Maria from source..."
    
    # Check if Rust is installed
    if ! command -v cargo >/dev/null 2>&1; then
        print_error "Rust/cargo not found. Please install Rust: https://rustup.rs/"
        exit 1
    fi
    
    # Check if we are in the maria repository
    if [[ ! -f "Cargo.toml" ]] || [[ ! -d "crates" ]]; then
        print_error "This does not appear to be the Maria repository. Please clone it first."
        exit 1
    fi
    
    # Build release version
    cargo build --release
    
    # Install binary
    if [[ -f "target/release/maria" ]]; then
        cp target/release/maria /usr/local/bin/
        chmod +x /usr/local/bin/maria
        print_info "Maria built and installed from source!"
    else
        print_error "Build failed. Check the Cargo output above."
        exit 1
    fi
}

# Main installation logic
main() {
    echo "=== Maria Simulator Installer ==="
    echo
    
    # Detect platform
    local platform="$(detect_platform)"
    print_info "Detected platform: ${platform}"
    
    # Check if we're in the Maria source directory
    if [[ -f "Cargo.toml" ]] && [[ -d "crates" ]] && [[ -d ".maria" ]]; then
        print_info "Maria source detected. Building from source..."
        build_from_source
        exit 0
    fi
    
    # Check if user wants to install latest stable release
    print_info "This will install the latest stable release from GitHub."
    
    # Fetch latest version
    print_info "Fetching latest release version..."
    local version="$(fetch_latest_version)"
    
    # Confirm installation
    echo
    print_warn "This will install Maria v${version} to /usr/local/bin/"
    read -p "Do you want to continue? (y/N): " -n 1 -r
    echo
    
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        install_latest "${version}" "${platform}"
    else
        print_info "Installation cancelled."
        exit 0
    fi
}

# Run main function with all arguments passed
main "$@"