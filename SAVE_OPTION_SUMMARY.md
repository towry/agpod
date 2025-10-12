# --save Option Implementation Summary

This document summarizes the implementation of the `--save` option for the minimize-git-diff-llm tool.

## What was implemented

### 1. Command-line argument parsing
- Added support for the `--save` flag using `std::env::args()`
- When `--save` is provided, the tool switches to "save mode" instead of minimizing output
- Added optional `--save-path <path>` argument to specify a custom output directory
  - If not provided, defaults to `llm/diff`
  - Path is relative to the current working directory

### 2. Chunk suffix generation
- Implemented `generate_chunk_suffix()` function that generates suffixes in the pattern:
  - `aa` to `zz` for files 0-675 (26×26 = 676 combinations)
  - `0000` onwards for files 676+ (numeric suffix with 4 digits)
- This provides support for up to 10,676 files (676 + 10,000)

### 3. Directory management
- Creates output directory for chunks (default: `llm/diff`, or custom path via `--save-path`)
- If the directory already exists, it is completely removed and recreated
- This ensures clean output on each run
- Path is relative to the current working directory

### 4. Diff chunk splitting
- Each file change in the git diff is saved as a separate chunk file
- Chunks are named `chunk_<suffix>.diff` where suffix is generated as described above
- Each chunk contains the full diff for a single file, including:
  - The `diff --git` header
  - All metadata lines (file mode, index, etc.)
  - All content changes

### 5. Testing
- Added `test_generate_chunk_suffix()` to verify suffix generation logic
- Added `test_save_diff_chunks()` to verify the complete save functionality with default path
- Added `test_save_diff_chunks_custom_path()` to verify custom path functionality
- Added `test_parse_save_path()` to verify argument parsing
- All 14 tests pass (10 original + 4 new tests)
- Tested with edge cases:
  - 3 files (aa, ab, ac)
  - 30 files (up to bd)
  - 680 files (testing zz → 0000 transition)
  - Custom output paths (relative and nested directories)

### 6. Demo script
- Created `demo_save_option.sh` for easy demonstration
- Script creates a temporary git repository with sample changes
- Demonstrates the complete workflow of using `--save`
- Shows the output chunks and their contents

## Usage

### Basic usage (default path)
```bash
git diff | minimize-git-diff-llm --save
```

### With custom path
```bash
git diff | minimize-git-diff-llm --save --save-path my/custom/output
```

### With staged changes
```bash
git diff --cached | minimize-git-diff-llm --save
```

### Compare specific commits
```bash
git diff HEAD~1 HEAD | minimize-git-diff-llm --save --save-path diffs/comparison
```

## Command-line options

- `--save`: Enable save mode (splits diff into chunks)
- `--save-path <path>`: Specify custom output directory (optional, defaults to `llm/diff`)
  - Path is relative to current working directory
  - Parent directories are created automatically
  - Existing directory is removed and recreated for clean output

## Output structure

After running with `--save`, you'll find:
```
llm/
└── diff/
    ├── chunk_aa.diff    # First file's changes
    ├── chunk_ab.diff    # Second file's changes
    ├── chunk_ac.diff    # Third file's changes
    └── ...
```

## Example chunk content

Each chunk file contains the complete diff for a single file:

```diff
diff --git a/file1.txt b/file1.txt
index d787615..94babbe 100644
--- a/file1.txt
+++ b/file1.txt
@@ -1,3 +1,4 @@
-This is file 1
-Line 2
+This is file 1 - MODIFIED
+Line 2 updated
 Line 3
+New line 4
```

## Technical details

### Files modified
- `src/main.rs`: Added save functionality and argument parsing
- `.gitignore`: Added `llm/` directory to prevent committing output
- `demo_save_option.sh`: Created demo script

### Dependencies
No new dependencies were added. The implementation uses only:
- `std::env` for argument parsing
- `std::fs` for file system operations
- `std::io` for I/O operations
- `std::path::Path` for path handling

### Code quality
- All tests pass (12/12)
- Clippy passes with no warnings (`-D warnings`)
- Code is formatted with `cargo fmt`

## Testing the implementation

Run the demo script:
```bash
./demo_save_option.sh
```

Or test manually:
```bash
# Build the project
cargo build --release

# Create a test repository
mkdir test_repo && cd test_repo
git init
git config user.email "test@example.com"
git config user.name "Test User"

# Create and commit some files
echo "content" > file1.txt
git add . && git commit -m "initial"

# Make changes
echo "changed" >> file1.txt
git add .

# Use --save option
git diff --cached | /path/to/minimize-git-diff-llm --save

# Check results
ls -la llm/diff/
cat llm/diff/chunk_aa.diff
```

## Performance

The implementation is efficient:
- Parses the diff only once
- Writes chunks incrementally
- No unnecessary memory allocations
- Works well with large diffs (tested with 680 files)
