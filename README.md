# agpod

A powerful agent helper tool with features including git diff minimization for LLM context, reducing token usage while preserving essential information.

## Features

- **Large file handling**: Files with significant changes are summarized with metadata only
- **Keyword extraction**: Extracts relevant keywords from JSON, code files, and other text formats
- **Empty line reduction**: Removes excessive consecutive empty lines while preserving structure
- **Comprehensive diff support**: Handles added, deleted, modified, and renamed files
- **Save mode**: Split diffs into separate chunk files for review workflows
- **Review tracking**: Automatic REVIEW.md generation with file hashes and status tracking
- **Project isolation**: Project-specific folders prevent conflicts in concurrent workflows

## Installation

### Quick install (recommended)

Install the latest release directly to `/usr/local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/towry/agpod/main/install.sh | bash
```

Or download and run the install script manually:

```bash
wget https://raw.githubusercontent.com/towry/agpod/main/install.sh
chmod +x install.sh
./install.sh
```

**Requirements:**
- macOS with Apple Silicon (M1/M2/M3)
- `curl` or `wget`
- `sudo` access (for installing to `/usr/local/bin`)

### From source

```bash
git clone https://github.com/towry/agpod.git
cd agpod
cargo build --release
```

The binary will be available at `target/release/agpod`.

## Usage

### Basic usage

Minimize a git diff (reads from stdin):

```bash
git diff | agpod diff
```

### With staged changes

```bash
git diff --cached | agpod diff
```

### Compare specific commits

```bash
git diff HEAD~1 HEAD | agpod diff
```

### Save mode for review workflows

Split diff into separate chunk files with review tracking:

```bash
# Save chunks to default location (llm/diff/<project-name>/)
git diff | agpod diff --save

# Save chunks to custom location
git diff | agpod diff --save --save-path custom/path
```

Output format (machine-readable, to stdout):
```
generated: llm/diff/<project-name>/
REVIEW.md: /path/to/working/directory/REVIEW.md
```

This creates:
- Individual diff chunk files in `<path>/<project-name>/chunk_*.diff`
- A `REVIEW.md` file in the same directory for tracking review progress

The `REVIEW.md` file includes:
- File hashes for change tracking
- Chunk file references
- Review status fields (pending, reviewed@date, outdated)
- Placeholders for review comments

See [SAVE_OPTION_SUMMARY.md](SAVE_OPTION_SUMMARY.md) for detailed documentation.

### Integration with other tools

```bash
# Copy minimized diff to clipboard (macOS)
git diff | agpod diff | pbcopy

# Save to file
git diff | agpod diff > minimized_diff.txt
```

## Strategy

The tool applies the following minimization strategy:

1. **Large files** (>100 changes or >500 total lines): Shows only:
   - File name and path
   - Change type (added/deleted/modified/renamed)
   - Number of content lines
   - Extracted keywords (for readable files like JSON, code)

2. **Regular files**: Preserves the full diff but:
   - Removes excessive empty lines (limits to 2 consecutive)
   - Maintains proper context for LLM understanding

3. **Keyword extraction**: For supported file types:
   - **JSON**: Top-level keys
   - **Code files** (.rs, .py, .js, .ts, .java, .cpp): Function/class/struct names
   - Limits to 10 most relevant keywords

## Example

### Input
```diff
diff --git a/large_config.json b/large_config.json
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/large_config.json
@@ -0,0 +1,150 @@
+{
+  "database": {...},
+  "api_endpoints": {...},
+  // ... 150+ lines of JSON
+}
```

### Output
```
Large file change: large_config.json
Change type: added  
Content lines: 161
Keywords: database, api_endpoints, cache, logging, security
```

## Supported File Types

- **Text files**: .txt, .md, .log, .csv
- **Code files**: .rs, .py, .js, .ts, .java, .cpp, .c, .h, .go, .php, .rb, .sh
- **Config files**: .json, .yaml, .yml, .toml, .xml
- **Web files**: .html, .css

## License

MIT License - see [LICENSE](LICENSE) file for details.


