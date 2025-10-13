# --save Option Implementation Summary

This document summarizes the implementation of the `--save` option for the minimize-git-diff-llm tool.

## What was implemented

### 1. Command-line argument parsing
- Added support for the `--save` flag using `std::env::args()`
- When `--save` is provided, the tool switches to "save mode" instead of minimizing output
- Added optional `--save-path <path>` argument to specify a custom output directory
  - If not provided, defaults to `llm/diff`
  - Path is relative to the current working directory

### 2. Project-specific folders (NEW)
- Each project now gets its own subfolder under the base output directory
- Project identifier is determined by:
  1. Git repository name (from `git rev-parse --show-toplevel`)
  2. Falls back to current directory name
  3. Ultimate fallback: "default-project"
- This prevents different concurrent project workflows from overlapping
- Example structure: `llm/diff/project-name/chunk_aa.diff`

### 3. REVIEW.md file generation (NEW)
- Automatically creates a `REVIEW.md` file in the current working directory
- Tracks all diff chunks with metadata for review workflow
- Format includes:
  - Guidelines section explaining review process
  - For each changed file:
    - File path as heading
    - `meta:hash`: Hash of the diff chunk content
    - `meta:diff_chunk`: Relative path to the chunk file
    - `meta:status`: Review status (default: "pending")
    - Placeholder for review comments
- Supports review workflow tracking and status updates

### 4. Chunk suffix generation
- Implemented `generate_chunk_suffix()` function that generates suffixes in the pattern:
  - `aa` to `zz` for files 0-675 (26×26 = 676 combinations)
  - `0000` onwards for files 676+ (numeric suffix with 4 digits)
- This provides support for up to 10,676 files (676 + 10,000)

### 5. Directory management
- Creates output directory for chunks with project-specific subfolder
- Default path: `llm/diff/<project-name>/`
- Custom path: `<custom-path>/<project-name>/`
- If the directory already exists, it is completely removed and recreated
- This ensures clean output on each run
- Path is relative to the current working directory

### 6. Diff chunk splitting
- Each file change in the git diff is saved as a separate chunk file
- Chunks are named `chunk_<suffix>.diff` where suffix is generated as described above
- Each chunk contains the full diff for a single file, including:
  - The `diff --git` header
  - All metadata lines (file mode, index, etc.)
  - All content changes

### 7. Testing
- Added `test_generate_chunk_suffix()` to verify suffix generation logic
- Updated `test_save_diff_chunks()` to verify project-specific folder structure
- Updated `test_save_diff_chunks_custom_path()` to verify custom path functionality
- Added `test_parse_save_path()` to verify argument parsing
- Added `test_get_project_identifier()` to verify project identification
- Added `test_compute_file_hash()` to verify hash computation
- Added `test_review_md_format()` to verify REVIEW.md generation
- All 17 tests pass (10 original + 7 new/updated tests)
- Tested with edge cases:
  - Multiple files with project-specific folders
  - Custom output paths with project separation
  - REVIEW.md format and content
  - Concurrent projects not overlapping

### 8. Demo script
- Existing `demo_save_option.sh` script demonstrates the workflow
- Shows the output chunks and their contents
- Note: Script may need updating to reflect new project-specific folder structure

## Usage

### Basic usage (default path)
```bash
git diff | minimize-git-diff-llm --save
```
Output structure:
```
llm/diff/<project-name>/
  ├── chunk_aa.diff
  ├── chunk_ab.diff
  └── ...
REVIEW.md (in current directory)
```

### With custom path
```bash
git diff | minimize-git-diff-llm --save --save-path my/custom/output
```
Output structure:
```
my/custom/output/<project-name>/
  ├── chunk_aa.diff
  ├── chunk_ab.diff
  └── ...
REVIEW.md (in current directory)
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
  - Project-specific subfolder is automatically added
  - Parent directories are created automatically
  - Existing directory is removed and recreated for clean output

## Output structure

After running with `--save`, you'll find:
```
llm/
└── diff/
    └── <project-name>/
        ├── chunk_aa.diff    # First file's changes
        ├── chunk_ab.diff    # Second file's changes
        ├── chunk_ac.diff    # Third file's changes
        └── ...
REVIEW.md                     # Review tracking file (in current directory)
```

## REVIEW.md format

Example REVIEW.md content:
```markdown
# Code Review Tracking

This file tracks the review status of code changes.

## Guidelines
- Update `meta:status` after reviewing each file
- Status values: `pending`, `reviewed@YYYY-MM-DD`, `outdated`
- Add review comments in the placeholder section below each file

---

## file1.txt
- meta:hash: 6f8895699da1b8b3
- meta:diff_chunk: chunk_aa.diff
- meta:status: pending

<!-- Review comments go here -->

---

## file2.txt
- meta:hash: 7ee4ccfbb1c2bf6a
- meta:diff_chunk: chunk_ab.diff
- meta:status: pending

<!-- Review comments go here -->

---
```

### Review workflow
1. Run `minimize-git-diff-llm --save` to generate chunks and REVIEW.md
2. Review each chunk file referenced in REVIEW.md
3. Update `meta:status` field for each file after review:
   - `pending` → `reviewed@2025-10-13` (after review)
   - `pending` → `outdated` (if changes are superseded)
4. Add review comments in the placeholder section
5. Use REVIEW.md to track progress across multiple review sessions

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
- `src/main.rs`: Added save functionality with project-specific paths, REVIEW.md generation, and argument parsing
- `.gitignore`: Added `llm/` directory to prevent committing output

### Dependencies
No new dependencies were added. The implementation uses only:
- `std::env` for argument parsing and environment access
- `std::fs` for file system operations
- `std::io` for I/O operations
- `std::path::Path` for path handling
- `std::process::Command` for executing git commands
- `std::collections::hash_map::DefaultHasher` for computing file hashes
- `std::hash::{Hash, Hasher}` for hashing functionality

### Code quality
- All tests pass (17/17)
- Clippy passes with no warnings (`-D warnings`)
- Code is formatted with `cargo fmt`

## Benefits of project-specific folders

1. **Concurrent workflows**: Multiple CI/CD workflows can run simultaneously without conflicts
2. **Multi-project development**: Work on multiple projects without cleanup between switches
3. **Clear organization**: Easy to identify which chunks belong to which project
4. **No data loss**: Different projects never overwrite each other's chunks

## Review workflow integration

The REVIEW.md file enables:
1. **Systematic review**: Track review progress for each file
2. **Status tracking**: Know which files have been reviewed
3. **Comments**: Add review feedback directly in the tracking file
4. **History**: Keep review history with dated status updates
5. **Team coordination**: Share review status across team members

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
ls -la llm/diff/test_repo/
cat llm/diff/test_repo/chunk_aa.diff
cat REVIEW.md
```

## Performance

The implementation is efficient:
- Parses the diff only once
- Writes chunks incrementally
- Computes hashes on-the-fly
- No unnecessary memory allocations
- Works well with large diffs (tested with 680 files)
- Project identification uses fast git command (falls back to directory name)
