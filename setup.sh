#!/bin/bash
# Setup script for Zureshot Tauri project

cd "$(dirname "$0")"

# Remove old Rust files from src/ (now only frontend code should be there)
rm -f src/capture.rs src/writer.rs src/main.rs

# Install frontend dependencies
if command -v pnpm &> /dev/null; then
    pnpm install
elif command -v npm &> /dev/null; then
    npm install
else
    echo "Please install pnpm or npm first"
    exit 1
fi

echo "Setup complete! Run 'pnpm tauri dev' to start development"
