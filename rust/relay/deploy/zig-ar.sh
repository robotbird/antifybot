#!/usr/bin/env sh
exec "${ANTIFY_ZIG:?Set ANTIFY_ZIG to the local Zig executable}" ar "$@"
