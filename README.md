# minimize-git-diff-llm

CLI tool to minimize git diff content for LLM context, reducing token usage while preserving essential information.

## Features

- **Large file handling**: Files with significant changes are summarized with metadata only
- **Keyword extraction**: Extracts relevant keywords from JSON, code files, and other text formats
- **Empty line reduction**: Removes excessive consecutive empty lines while preserving structure
- **Comprehensive diff support**: Handles added, deleted, modified, and renamed files

## Installation

### From source

```bash
git clone https://github.com/towry/minimize-git-diff-llm.git
cd minimize-git-diff-llm
cargo build --release
```

The binary will be available at `target/release/minimize-git-diff-llm`.

## Usage

### Basic usage

```bash
git diff | minimize-git-diff-llm
```

### With staged changes

```bash
git diff --cached | minimize-git-diff-llm
```

### Compare specific commits

```bash
git diff HEAD~1 HEAD | minimize-git-diff-llm
```

### Integration with other tools

```bash
# Copy minimized diff to clipboard (macOS)
git diff | minimize-git-diff-llm | pbcopy

# Save to file
git diff | minimize-git-diff-llm > minimized_diff.txt
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
