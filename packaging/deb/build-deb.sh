#!/bin/sh
# Build the .deb from a release build (cargo build --release first). Run on the oldest supported Debian/Ubuntu
# (Ubuntu 24.04) so the binaries run on newer ones too. Output: dist/acerultrax_<version>_amd64.deb
set -eu
cd "$(dirname "$0")/../.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT
PKG="$ROOT/pkg"

for b in acerultrax acerultrax-daemon acerultrax-cli; do
    install -Dm755 "target/release/$b" "$PKG/usr/bin/$b"
done
install -Dm644 packaging/acerultrax.service "$PKG/usr/lib/systemd/user/acerultrax.service"
install -Dm644 packaging/io.github.fawaz7.AcerUltraX.desktop "$PKG/usr/share/applications/io.github.fawaz7.AcerUltraX.desktop"
(cd data/icons && find hicolor -type f -exec install -Dm644 {} "$PKG/usr/share/icons/{}" \;)
install -Dm644 LICENSE "$PKG/usr/share/doc/acerultrax/copyright"
install -Dm644 README.md "$PKG/usr/share/doc/acerultrax/README.md"

# Library dependencies as dpkg sees them, plus what the service needs at run time.
mkdir -p "$ROOT/src/debian"
printf 'Source: acerultrax\n\nPackage: acerultrax\nArchitecture: amd64\n' > "$ROOT/src/debian/control"
SHLIBS=$(cd "$ROOT/src" && dpkg-shlibdeps -O "$PKG"/usr/bin/* 2>/dev/null | sed -n 's/^shlibs:Depends=//p')

mkdir -p "$PKG/DEBIAN"
cat > "$PKG/DEBIAN/control" <<EOF
Package: acerultrax
Version: $VERSION
Architecture: amd64
Maintainer: Fawaz Al-Ghzawi <ghazawe1@gmail.com>
Depends: $SHLIBS, pipewire, wireplumber
Recommends: pipewire-pulse
Section: sound
Priority: optional
Homepage: https://github.com/fawaz7/AcerUltraX
Installed-Size: $(du -sk "$PKG/usr" | cut -f1)
Description: Your Acer laptop's Windows sound (DTS:X Ultra) on Linux
 AcerUltraX runs Acer's DTS:X Ultra sound natively on Linux, without Wine:
 Acer's sound modes for the laptop's speakers and headphones, per-device
 profiles, headphone models, spatial sound and an equaliser. On first start
 it downloads Acer's own files for the laptop from Acer.
EOF
mkdir -p dist
dpkg-deb --root-owner-group --build "$PKG" "dist/acerultrax_${VERSION}_amd64.deb"
echo "dist/acerultrax_${VERSION}_amd64.deb"
