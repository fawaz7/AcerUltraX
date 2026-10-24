#!/bin/sh
# Everything that must pass before a commit: formatting, lints, all tests. The reference tests run when the
# Acer files and Windows recordings are available (ACERULTRAX_VENDOR_DIR, ACERULTRAX_GOLDEN_DIR; see
# docs/TESTING.md), and skip themselves otherwise.
set -eu
cd "$(dirname "$0")/.."
: "${ACERULTRAX_VENDOR_DIR:=$HOME/.local/share/acerultrax/dts}"
[ -d "$ACERULTRAX_VENDOR_DIR" ] && export ACERULTRAX_VENDOR_DIR
[ -d "$ACERULTRAX_VENDOR_DIR/spatial" ] && export ACERULTRAX_SPATIAL_DIR="${ACERULTRAX_SPATIAL_DIR:-$ACERULTRAX_VENDOR_DIR/spatial}"
cargo fmt --all --check
cargo clippy -j4 --workspace --all-targets -- -D warnings
cargo test -j4 --release --workspace
echo "All checks passed."
