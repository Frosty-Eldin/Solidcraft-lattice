#!/usr/bin/env sh
# Builds dist/lattice.zip: the SolidCraft plugin package (plugin.json + lattice.wasm component).
# One-time setup: rustup target add wasm32-unknown-unknown && cargo install wasm-tools
set -eu
cd "$(dirname "$0")"
# Tests: cargo test --release
cargo build --release --target wasm32-unknown-unknown
rm -rf dist && mkdir -p dist/lattice
wasm-tools component new target/wasm32-unknown-unknown/release/solidcraft_lattice.wasm -o dist/lattice/lattice.wasm
wasm-tools validate dist/lattice/lattice.wasm
cp plugin.json dist/lattice/plugin.json
(cd dist && zip -qr lattice.zip lattice)
echo "Built $(pwd)/dist/lattice.zip"
