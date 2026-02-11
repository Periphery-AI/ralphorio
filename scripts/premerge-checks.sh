#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo test --manifest-path sim-core/Cargo.toml
cargo test --manifest-path worker/Cargo.toml
cargo check --manifest-path worker/Cargo.toml
cargo check --manifest-path game-client/Cargo.toml --target wasm32-unknown-unknown
npm run build:web
