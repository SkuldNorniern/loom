#!/bin/sh
set -e
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown --target-dir target
rm -rf dist
mkdir -p dist
wasm-bindgen --target web --no-typescript \
    --out-dir dist target/wasm32-unknown-unknown/release/game_client.wasm
