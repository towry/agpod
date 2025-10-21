# Workspace Structure

agpod has been refactored into a Rust workspace with multiple crates for better modularity and distribution.

## Crates

### agpod-core
**Location**: `crates/agpod-core/`  
**Purpose**: Core configuration and utilities  
**Dependencies**: `serde`, `toml`, `dirs`

Contains the shared configuration system used by all features:
- Configuration file parsing and management
- XDG config directory support
- Version tracking for configuration schema

### agpod-diff
**Location**: `crates/agpod-diff/`  
**Purpose**: Git diff minimization for LLM context optimization  
**Dependencies**: `agpod-core`, `regex`, `chrono`, `anyhow`

Provides diff processing functionality:
- Git diff parsing and minimization
- Large file detection and summarization
- Diff chunk saving with review tracking
- Token usage optimization

### agpod-kiro
**Location**: `crates/agpod-kiro/`  
**Purpose**: PR draft workflow management  
**Dependencies**: `agpod-core`, `clap`, `minijinja`, `dialoguer`, and others

Implements the Kiro workflow:
- PR draft creation and management
- Template rendering system
- Git integration
- Plugin system for branch naming
- Interactive CLI interface

### agpod
**Location**: `crates/agpod/`  
**Purpose**: Main CLI binary  
**Dependencies**: `agpod-core`, `agpod-diff`, `agpod-kiro`, `clap`

The CLI application that integrates all features:
- Command-line argument parsing
- Subcommand routing (`diff`, `kiro`)
- User-facing interface

## Benefits

1. **Modularity**: Each crate has a clear, focused responsibility
2. **Reusability**: Library crates can be used independently
3. **Distribution**: Individual crates can be published to crates.io
4. **Maintainability**: Easier to understand and modify individual components
5. **Testing**: Tests are organized by crate functionality

## Building

All crates are built together as a workspace:

```bash
# Build all crates
cargo build

# Build release version
cargo build --release

# Test all crates
cargo test

# Run clippy on all crates
cargo clippy --all-targets --all-features -- -D warnings

# Format all crates
cargo fmt --all
```

## Directory Structure

```
agpod/
├── Cargo.toml                 # Workspace root
├── Cargo.lock                 # Shared dependency lock
├── crates/
│   ├── agpod-core/           # Configuration library
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs
│   ├── agpod-diff/           # Diff processing library
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── processor.rs
│   │       ├── save.rs
│   │       ├── tests.rs
│   │       └── types.rs
│   ├── agpod-kiro/           # Kiro workflow library
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── cli.rs
│   │       ├── commands.rs
│   │       ├── config.rs
│   │       ├── error.rs
│   │       ├── git.rs
│   │       ├── plugin.rs
│   │       ├── slug.rs
│   │       └── template.rs
│   └── agpod/                # Binary crate
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
├── examples/                  # Example configs and templates
├── test_data/                # Test fixtures
└── target/                   # Build output (gitignored)
```

## Workspace Configuration

The root `Cargo.toml` defines:

- **Workspace members**: All four crates
- **Shared package metadata**: version, edition, authors, license
- **Shared dependencies**: Common dependencies with consistent versions
- **Resolver**: Version 2 for better dependency resolution

## CI/CD

The existing CI/CD workflows automatically work with the workspace:

- **CI**: Runs `cargo test`, `cargo clippy`, `cargo fmt` on all crates
- **Release**: Builds the `agpod` binary from the workspace
- **Build artifacts**: Creates release binaries for distribution

No changes needed to workflows as cargo commands operate on workspaces by default.
