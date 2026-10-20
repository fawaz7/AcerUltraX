#!/bin/sh
# Build the no-package-manager download from a release build (cargo build --release first).
# Output: dist/acerultrax-<version>-x86_64-linux.tar.gz
set -eu
cd "$(dirname "$0")/../.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
NAME="acerultrax-$VERSION-x86_64-linux"
ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT
D="$ROOT/$NAME"

for b in acerultrax acerultrax-daemon acerultrax-cli; do
    install -Dm755 "target/release/$b" "$D/bin/$b"
done
install -Dm644 packaging/io.github.fawaz7.AcerUltraX.desktop "$D/share/applications/io.github.fawaz7.AcerUltraX.desktop"
(cd data/icons && find hicolor -type f -exec install -Dm644 {} "$D/share/icons/{}" \;)
install -Dm755 packaging/tarball/install.sh "$D/install.sh"
install -Dm644 LICENSE "$D/LICENSE"
install -Dm644 README.md "$D/README.md"
mkdir -p dist
tar -C "$ROOT" -czf "dist/$NAME.tar.gz" "$NAME"
echo "dist/$NAME.tar.gz"
