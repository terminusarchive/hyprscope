#!/bin/sh
# Build the release binary. Pass "run" to start it afterwards.
set -e
cd "$(dirname "$0")"
cargo build --release
echo "built: $(pwd)/target/release/hyprscope"
if [ "$1" = "run" ]; then
    exec ./target/release/hyprscope
fi
