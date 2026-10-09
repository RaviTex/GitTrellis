#!/usr/bin/env bash
# Create a demo repository for manual testing.
# Usage: ./scripts/make-demo-repo.sh [target-dir]
set -euo pipefail

target="${1:-/tmp/gittrellis-demo}"

if [ -e "$target" ]; then
    echo "refusing to overwrite existing path: $target" >&2
    exit 1
fi

git init -q -b main "$target"
git -C "$target" config user.name "Ada Lovelace"
git -C "$target" config user.email "ada@example.com"

cat > "$target/README.md" << 'EOF'
# Demo Project

A sample repository for trying out GitTrellis.
EOF

cat > "$target/app.py" << 'EOF'
def greet(name):
    return f"Hello, {name}!"

if __name__ == "__main__":
    print(greet("world"))
EOF

git -C "$target" add .
git -C "$target" commit -qm "Initial commit"

cat > "$target/app.py" << 'EOF'
def greet(name):
    return f"Hello, {name}!"

def farewell(name):
    return f"Goodbye, {name}!"

if __name__ == "__main__":
    print(greet("world"))
EOF
git -C "$target" add app.py
git -C "$target" commit -qm "feat: add farewell function"

git -C "$target" switch -qc feature/docs
cat >> "$target/README.md" << 'EOF'

## Usage

Run `python app.py`.
EOF
git -C "$target" add README.md
git -C "$target" commit -qm "docs: add usage section"
git -C "$target" switch -q main

cat >> "$target/README.md" << 'EOF'

## Uncommitted note

This line is not committed yet.
EOF
echo "scratch notes" > "$target/notes.txt"

echo "demo repository ready: $target"
git -C "$target" status --short
