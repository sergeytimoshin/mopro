#!/bin/sh
set -eu
# mcl-rust does not forward CXXFLAGS. Match Rust's shared-memory features.
for arg in "$@"; do
    case "$arg" in
        --target=wasm32-*) exec "${MCL_CLANGXX:-clang++}" -matomics -mbulk-memory "$@" ;;
    esac
done
exec "${MCL_CLANGXX:-clang++}" "$@"
