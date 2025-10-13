#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
REPO="towry/minimize-git-diff-llm"
BINARY_NAME="minimize-git-diff-llm"
INSTALL_DIR="/usr/local/bin"

# Function to print colored output
print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to detect OS and architecture
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Darwin)
            case "$(uname -m)" in
                arm64|aarch64)
                    arch="aarch64-apple-darwin"  # Apple Silicon
                    ;;
                x86_64)
                    arch="x86_64-apple-darwin"  # Intel Mac
                    ;;
                *)
                    print_error "Unsupported Mac architecture: $(uname -m)"
                    exit 1
                    ;;
            esac
            ;;
        Linux)
            case "$(uname -m)" in
                x86_64|amd64)
                    arch="x86_64-unknown-linux-gnu"  # Linux x86_64
                    ;;
                *)
                    print_error "Unsupported Linux architecture: $(uname -m)"
                    exit 1
                    ;;
            esac
            ;;
        *)
            print_error "Unsupported operating system: $(uname -s)"
            print_info "This tool supports macOS (Apple Silicon & Intel) and Linux (x86_64)."
            exit 1
            ;;
    esac

    echo "$arch"
}

# Function to get latest release info
get_latest_release() {
    print_info "Fetching latest release information..."
    
    local api_url="https://api.github.com/repos/${REPO}/releases/latest"
    local release_info
    
    if command -v curl >/dev/null 2>&1; then
        release_info=$(curl -s "$api_url")
    elif command -v wget >/dev/null 2>&1; then
        release_info=$(wget -qO- "$api_url")
    else
        print_error "Neither curl nor wget is available. Please install one of them."
        exit 1
    fi
    
    if [ -z "$release_info" ]; then
        print_error "Failed to fetch release information."
        exit 1
    fi
    
    echo "$release_info"
}

# Function to extract download URL from release info
get_download_url() {
    local release_info="$1"
    local platform="$2"
    
    # Look for the asset with the correct platform in the name
    local download_url
    download_url=$(echo "$release_info" | grep -o '"browser_download_url": "[^"]*' | grep "${platform}" | head -1 | cut -d'"' -f4)
    
    if [ -z "$download_url" ]; then
        print_error "No release asset found for platform: $platform"
        print_info "Available assets:"
        echo "$release_info" | grep -o '"name": "[^"]*' | cut -d'"' -f4 | sed 's/^/  - /'
        exit 1
    fi
    
    echo "$download_url"
}

# Function to download and install
install_binary() {
    local download_url="$1"
    local temp_dir
    temp_dir=$(mktemp -d)
    local archive_name="$(basename "$download_url")"
    local archive_path="$temp_dir/$archive_name"
    
    print_info "Downloading from: $download_url"
    
    # Download the archive
    if command -v curl >/dev/null 2>&1; then
        curl -L -o "$archive_path" "$download_url"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$archive_path" "$download_url"
    else
        print_error "Neither curl nor wget is available."
        exit 1
    fi
    
    if [ ! -f "$archive_path" ]; then
        print_error "Failed to download the archive."
        exit 1
    fi
    
    print_info "Extracting archive..."
    cd "$temp_dir"
    
    # Extract based on file extension
    case "$archive_name" in
        *.tar.gz)
            tar -xzf "$archive_path"
            ;;
        *.zip)
            unzip -q "$archive_path"
            ;;
        *)
            print_error "Unsupported archive format: $archive_name"
            exit 1
            ;;
    esac
    
    # Find the binary
    local binary_path
    binary_path=$(find "$temp_dir" -name "$BINARY_NAME" -type f | head -1)
    
    if [ ! -f "$binary_path" ]; then
        print_error "Binary '$BINARY_NAME' not found in the archive."
        exit 1
    fi
    
    # Verify checksum if available
    local checksum_file
    checksum_file=$(find "$temp_dir" -name "*.sha256" | head -1)
    if [ -f "$checksum_file" ]; then
        print_info "Verifying checksum..."
        cd "$(dirname "$binary_path")"
        if ! shasum -a 256 -c "$checksum_file" 2>/dev/null; then
            print_warning "Checksum verification failed, but continuing with installation."
        else
            print_success "Checksum verification passed."
        fi
    fi
    
    # Make binary executable
    chmod +x "$binary_path"
    
    # Install binary
    print_info "Installing to $INSTALL_DIR..."
    
    if [ ! -w "$INSTALL_DIR" ]; then
        print_info "Installing with sudo (admin privileges required)..."
        sudo cp "$binary_path" "$INSTALL_DIR/"
    else
        cp "$binary_path" "$INSTALL_DIR/"
    fi
    
    # Cleanup
    rm -rf "$temp_dir"
    
    print_success "Installation completed!"
}

# Function to verify installation
verify_installation() {
    print_info "Verifying installation..."
    
    if command -v "$BINARY_NAME" >/dev/null 2>&1; then
        local version
        version=$("$BINARY_NAME" --version 2>/dev/null || echo "unknown")
        print_success "$BINARY_NAME is installed and available in PATH"
        print_info "You can now use: git diff | $BINARY_NAME"
    else
        print_warning "$BINARY_NAME is installed but not in PATH"
        print_info "You may need to restart your terminal or add $INSTALL_DIR to your PATH"
        print_info "Or use the full path: $INSTALL_DIR/$BINARY_NAME"
    fi
}

# Main installation process
main() {
    echo "🚀 Git Diff Minimizer Installer"
    echo "================================"
    echo
    
    # Check if already installed
    if command -v "$BINARY_NAME" >/dev/null 2>&1; then
        print_info "$BINARY_NAME is already installed. Upgrading to the latest version..."
    fi
    
    # Detect platform
    local platform
    platform=$(detect_platform)
    print_info "Detected platform: $platform"
    
    # Get latest release
    local release_info
    release_info=$(get_latest_release)
    
    # Extract version
    local version
    version=$(echo "$release_info" | grep -o '"tag_name": "[^"]*' | cut -d'"' -f4)
    print_info "Latest version: $version"
    
    # Get download URL
    local download_url
    download_url=$(get_download_url "$release_info" "$platform")
    
    # Install
    install_binary "$download_url"
    
    # Verify
    verify_installation
    
    echo
    print_success "🎉 Installation complete!"
    print_info "Run 'git diff | $BINARY_NAME' to get started"
}

# Check for help flag
if [[ "$1" == "-h" || "$1" == "--help" ]]; then
    echo "Git Diff Minimizer Installer"
    echo
    echo "Usage: $0 [OPTIONS]"
    echo
    echo "OPTIONS:"
    echo "  -h, --help    Show this help message"
    echo
    echo "This script downloads and installs the latest release of minimize-git-diff-llm"
    echo "to $INSTALL_DIR for multiple platforms."
    echo
    echo "Requirements:"
    echo "  - macOS (Apple Silicon or Intel) or Linux (x86_64)"
    echo "  - curl or wget"
    echo "  - tar"
    echo "  - sudo access (if $INSTALL_DIR is not writable)"
    echo
    exit 0
fi

# Run main function
main "$@"