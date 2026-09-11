#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 2 ]]; then
    echo "Usage: bash build-msm.sh arkworks|mcl|check OUTPUT_DIR" >&2
    exit 2
fi
case "$1" in
    arkworks) features=() ;;
    mcl) features=(--features mcl-msm) ;;
    check) features=(--features mcl-msm,msm-check) ;;
    *) echo "Unknown MSM variant: $1" >&2; exit 2 ;;
esac
mkdir -p "$2"
output=$(cd "$2" && pwd)
cd "$(dirname "${BASH_SOURCE[0]}")"
# Templates are extracted without executable bits. Copy the compiler wrapper to
# an executable temporary file rather than requiring changes to CLI extraction.
wrapper=$(mktemp)
trap 'rm -f "$wrapper"' EXIT
cp build-support/clang-mcl.sh "$wrapper"
chmod +x "$wrapper"
export CXX="$wrapper"
export AR="${MCL_LLVM_AR:-llvm-ar}"
rustup run nightly-2025-11-15 "${MOPRO_WASM_PACK:-wasm-pack}" build \
    --target web --release --out-name gnark_kernel --out-dir "$output" \
    -- --locked --no-default-features "${features[@]}"
cp LICENSE-APACHE LICENSE-MIT "$output/"
if [[ "$1" != arkworks ]]; then cp LICENSE-MCL "$output/"; fi
: > "$output/.npmignore"
