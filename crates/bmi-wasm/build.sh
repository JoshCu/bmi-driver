#!/usr/bin/env bash
# Build the bmi-wasm adapter module (and optionally the test side module).
# Requires emsdk (https://github.com/emscripten-core/emsdk) and the Rust
# wasm32-unknown-emscripten target (rustup target add wasm32-unknown-emscripten).
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v emcc >/dev/null 2>&1; then
    EMSDK_DIR="${EMSDK:-$HOME/emsdk}"
    if [ -f "$EMSDK_DIR/emsdk_env.sh" ]; then
        # shellcheck disable=SC1091
        source "$EMSDK_DIR/emsdk_env.sh" >/dev/null 2>&1
    else
        echo "error: emcc not found and no emsdk at $EMSDK_DIR" >&2
        exit 1
    fi
fi

cargo build --release

mkdir -p pkg
# Rename the glue to .mjs so Node treats it as an ES module (built with -sEXPORT_ES6).
cp target/wasm32-unknown-emscripten/release/bmi.js pkg/bmi.mjs
cp target/wasm32-unknown-emscripten/release/bmi.wasm pkg/bmi.wasm

echo "built pkg/bmi.mjs + pkg/bmi.wasm"

if [ "${1:-}" = "--test" ]; then
    emcc -O2 -sSIDE_MODULE=2 -o test/model.wasm test/model.c
    echo "built test/model.wasm (side module)"
    node test/run.mjs
fi
