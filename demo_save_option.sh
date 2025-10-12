#!/bin/bash
# Demo script for --save option
# This script demonstrates how to use the --save option to split git diffs into chunks

set -e

# Save the original directory
ORIGINAL_DIR="$(pwd)"

echo "==================================="
echo "Demo: --save option"
echo "==================================="
echo ""

# Create a temporary demo repository
DEMO_DIR=$(mktemp -d)
cd "$DEMO_DIR"

echo "1. Creating demo repository in: $DEMO_DIR"
git init
git config user.email "demo@example.com"
git config user.name "Demo User"

# Create some initial files
echo "2. Creating initial files..."
cat > file1.txt << 'EOF'
This is file 1
Line 2
Line 3
EOF

cat > file2.txt << 'EOF'
This is file 2
Content here
More content
EOF

cat > config.json << 'EOF'
{
  "name": "demo",
  "version": "1.0.0"
}
EOF

git add .
git commit -m "Initial commit"

# Make changes
echo "3. Making changes to files..."
cat > file1.txt << 'EOF'
This is file 1 - MODIFIED
Line 2 updated
Line 3
New line 4
EOF

cat > new_file.py << 'EOF'
#!/usr/bin/env python3

def hello():
    print("Hello, World!")

if __name__ == "__main__":
    hello()
EOF

rm file2.txt
git add -A

# Show the diff
echo ""
echo "4. Generated diff (3 file changes):"
echo "-----------------------------------"
git diff --cached | head -20
echo "... (truncated)"
echo ""

# Use --save option
echo "5. Running: git diff --cached | minimize-git-diff-llm --save"

# Try to find the binary
if command -v minimize-git-diff-llm &> /dev/null; then
    git diff --cached | minimize-git-diff-llm --save
elif [ -f "$ORIGINAL_DIR/target/release/minimize-git-diff-llm" ]; then
    git diff --cached | "$ORIGINAL_DIR/target/release/minimize-git-diff-llm" --save
else
    echo "Error: minimize-git-diff-llm not found. Please build it first with: cargo build --release"
    exit 1
fi

# Show results
echo ""
echo "6. Created chunks in llm/diff/:"
echo "-----------------------------------"
ls -lh llm/diff/
echo ""

echo "7. Content of chunk_aa.diff (file1.txt changes):"
echo "-----------------------------------"
cat llm/diff/chunk_aa.diff
echo ""

echo "8. Content of chunk_ab.diff (file2.txt deletion):"
echo "-----------------------------------"
cat llm/diff/chunk_ab.diff
echo ""

echo "9. Content of chunk_ac.diff (new_file.py addition):"
echo "-----------------------------------"
cat llm/diff/chunk_ac.diff
echo ""

echo "==================================="
echo "Demo completed successfully!"
echo "==================================="
echo ""
echo "The demo repository is at: $DEMO_DIR"
echo "You can explore the llm/diff/ folder to see the chunks."
echo ""
echo "To clean up, run: rm -rf $DEMO_DIR"
