#!/usr/bin/env bash
# Build on the developer machine; deploy only the resulting static Linux binary.
set -euo pipefail
script_dir="$(cd "$(dirname "$0")" && pwd)"
rust_dir="$(cd "$script_dir/../.." && pwd)"
export ANTIFY_ZIG="${ANTIFY_ZIG:?Set ANTIFY_ZIG to the local Zig executable}"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$script_dir/zig-cc.sh"
export CC_x86_64_unknown_linux_musl="$script_dir/zig-cc.sh"
export AR_x86_64_unknown_linux_musl="$script_dir/zig-ar.sh"
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-self-contained=no"
export ZIG_GLOBAL_CACHE_DIR="$rust_dir/../.local/relay/zig-cache"
export ZIG_LOCAL_CACHE_DIR="$rust_dir/../.local/relay/zig-local-cache"
cd "$rust_dir"
cargo build --locked --offline --release -p antify-relay --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/antify-relay
shasum -a 256 target/x86_64-unknown-linux-musl/release/antify-relay
