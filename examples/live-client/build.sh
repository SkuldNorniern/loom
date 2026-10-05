#!/bin/sh
set -e
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
rm -rf dist
mkdir -p dist
wasm-bindgen --target web --no-typescript \
    --out-dir dist target/wasm32-unknown-unknown/release/live_client.wasm
