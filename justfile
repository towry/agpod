all:
  just --list

install-agpod-local:
    cargo build -p agpod
    mkdir -p ~/.local/bin
    cp -f target/debug/agpod ~/.local/bin/agpod
    (cd internal/agpod-mcp && go build -o ~/.local/bin/agpod-mcp ./cmd/agpod-mcp)
