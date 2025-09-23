# minimize-git-diff-llm

CLI tool to minimize git diff content for LLM context, reducing token usage while preserving essential information. Written in Rust as a single binary with minimal dependencies.

Always reference these instructions first and fallback to search or bash commands only when you encounter unexpected information that does not match the info here.

## Working Effectively

- Bootstrap, build, and test the repository:
  - Rust toolchain is pre-installed: `cargo 1.89.0`, `rustc 1.89.0`
  - `cargo check` -- verifies compilation. Takes ~8 seconds. NEVER CANCEL. Set timeout to 180+ seconds.
  - `cargo test` -- runs all unit tests. Takes ~6 seconds. NEVER CANCEL. Set timeout to 120+ seconds.
  - `cargo build --release` -- creates optimized binary. Takes ~9 seconds. NEVER CANCEL. Set timeout to 300+ seconds.
  - `cargo clippy -- -D warnings` -- linting check. Takes ~1 second. Set timeout to 60+ seconds.
  - `cargo fmt` -- formats code. Takes <1 second. Set timeout to 30+ seconds.

- Run the CLI tool:
  - ALWAYS build first with `cargo build --release`
  - Binary location: `./target/release/minimize-git-diff-llm`
  - Usage: `git diff | ./target/release/minimize-git-diff-llm`
  - Usage: `git diff --cached | ./target/release/minimize-git-diff-llm`
  - Usage: `cat diff_file.txt | ./target/release/minimize-git-diff-llm`

- Dependencies:
  - Only one external dependency: `regex = "1.0"` (specified in Cargo.toml)
  - No additional system dependencies required
  - Works on any system with Rust installed

## Validation

- Always manually validate any new code changes by running through complete scenarios.
- ALWAYS run through at least one complete end-to-end scenario after making changes:
  1. Build the application: `cargo build --release`
  2. Create test diff: `git init && git add . && git diff --cached | ./target/release/minimize-git-diff-llm`
  3. Test large file handling: Use test file with 400+ lines, verify it shows summary only
  4. Test deleted file handling: Verify it shows "Deleted file: filename" only
  5. Test small diff handling: Verify it preserves content for small changes

- Test scenarios to validate after changes:
  - **Small file test**: Create 3-line diff, verify full content is preserved
  - **Large file test**: Use `test_data/large_config.json` (431 lines), verify shows "Large file change" summary only
  - **Deleted file test**: Create deleted file diff, verify shows "Deleted file: filename" only
  - **Empty input test**: Test with empty input, verify no errors

- Always run `cargo fmt` and `cargo clippy -- -D warnings` before you are done or the CI (.github/workflows/release.yml) will fail.

- Test file functionality examples:
  ```bash
  # Small file test - should preserve content
  echo 'diff --git a/test.txt b/test.txt
  new file mode 100644
  index 0000000..1234567
  --- /dev/null
  +++ b/test.txt
  @@ -0,0 +1,3 @@
  +Line 1
  +Line 2
  +Line 3' | ./target/release/minimize-git-diff-llm
  
  # Large file test - should show summary only
  wc -l test_data/large_config.json  # Shows 431 lines
  
  # Deleted file test - should show summary only
  echo 'diff --git a/oldfile.txt b/oldfile.txt
  deleted file mode 100644
  index 1234567..0000000
  --- a/oldfile.txt
  +++ /dev/null
  @@ -1,5 +0,0 @@
  -Line 1
  -Line 2' | ./target/release/minimize-git-diff-llm
  # Expected output: "Deleted file: oldfile.txt"
  ```

## Common tasks

The following are outputs from frequently run commands. Reference them instead of viewing, searching, or running bash commands to save time.

### Repository structure
```
.
├── .git/
├── .github/
│   └── workflows/
│       └── release.yml     # CI/CD pipeline for macOS releases
├── .gitignore             # Standard Rust gitignore
├── Cargo.lock            # Dependency lock file
├── Cargo.toml            # Project manifest with regex dependency
├── LICENSE               # MIT license
├── README.md            # Project documentation
├── src/
│   └── main.rs          # Single source file with all logic
└── test_data/
    └── large_config.json # 431-line test file for large diff testing
```

### Cargo.toml contents
```toml
[package]
name = "minimize-git-diff-llm"
version = "0.1.0"
edition = "2021"
authors = ["Development Team"]
description = "CLI tool to minimize git diff content for LLM context"
license = "MIT"

[[bin]]
name = "minimize-git-diff-llm"
path = "src/main.rs"

[dependencies]
regex = "1.0"
```

### Key source code structure
- `main()` - Entry point, calls `process_git_diff()`
- `process_git_diff()` - Reads stdin, calls `minimize_diff()`, prints result
- `minimize_diff()` - Main logic that processes diff content
- `parse_git_diff()` - Parses git diff format into structured data
- `FileChange` struct - Represents a single file change
- `ChangeType` enum - Added, Deleted, Modified, Renamed
- Large file detection: >100 changes OR >500 total lines
- 10 comprehensive unit tests covering all scenarios

### Application behavior
- **Large files** (>100 changes or >500 total lines): Shows metadata summary only:
  ```
  Large file change: path/to/file.ext
  Change type: added
  Content lines: 437
  ```
- **Deleted files**: Shows single line summary:
  ```
  Deleted file: path/to/file.ext
  ```
- **Regular files**: Preserves full diff but removes excessive empty lines (limits to 2 consecutive)
- **Empty input**: Returns empty string, no errors

### CI/CD Pipeline
- Located in `.github/workflows/release.yml`
- Builds for macOS Apple Silicon (aarch64-apple-darwin)
- Runs: `cargo test`, `cargo clippy -- -D warnings`, `cargo build --release`
- Creates releases on push to main branch
- Build takes ~45 minutes in CI, test takes ~15 minutes

### Build times and timeouts
- `cargo check`: ~8 seconds (set timeout 180+ seconds)
- `cargo test`: ~6 seconds (set timeout 120+ seconds) 
- `cargo build --release`: ~9 seconds (set timeout 300+ seconds)
- `cargo clippy`: ~1 second (set timeout 60+ seconds)
- `cargo fmt`: <1 second (set timeout 30+ seconds)

### Common errors and troubleshooting
- Code not formatted: Run `cargo fmt` 
- Clippy warnings: Run `cargo clippy -- -D warnings` and fix reported issues
- Tests failing: Check that test data files exist and changes don't break core logic
- Binary not found: Ensure you ran `cargo build --release` first
- Permission denied: The binary may need execute permissions in some environments

### Testing strategy
- 10 unit tests cover all major scenarios
- Tests use realistic data from `test_data/large_config.json`
- Test coverage includes: empty input, small files, large files, deleted files, modified files, renamed files, multiple files, edge cases
- Always run full test suite with `cargo test` after changes
- Test file has 431 lines and generates proper large file behavior when used in diffs