#!/usr/bin/env bash
set -euo pipefail
args=()
for arg in "$@"; do
    case "$arg" in
        --target=x86_64-unknown-linux-musl) ;; # cc-rs uses Rust's target spelling.
        *) args+=("$arg") ;;
    esac
done
exec "${ANTIFY_ZIG:?Set ANTIFY_ZIG to the local Zig executable}" cc -target x86_64-linux-musl "${args[@]}"
