all:
  just --list

install-agpod-local:
    cargo build -p agpod -p agpod-case-server -p agpod-mcp
    mkdir -p ~/.local/bin
    cp -f target/debug/agpod ~/.local/bin/agpod
    cp -f target/debug/agpod-case-server ~/.local/bin/agpod-case-server
    cp -f target/debug/agpod-mcp ~/.local/bin/agpod-mcp
