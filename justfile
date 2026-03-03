install-local:
    cargo build --release
    mkdir -p ~/.local/bin
    mv -f target/release/agpod ~/.local/bin/agpod
