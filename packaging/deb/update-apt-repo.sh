#!/bin/sh
# Add .deb files to the apt repository in <repo-dir> (a flat repository, served by GitHub Pages) and sign it with
# the default GPG key. Usage: update-apt-repo.sh <repo-dir> <file.deb>...
#
# Users add it with:
#   deb [signed-by=/usr/share/keyrings/acerultrax.gpg] https://fawaz7.github.io/AcerUltraX/apt ./
set -eu
REPO=$1
shift
mkdir -p "$REPO/pool"
cp "$@" "$REPO/pool/"
cd "$REPO"
apt-ftparchive packages pool > Packages
gzip -9kf Packages
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=AcerUltraX \
    -o APT::FTPArchive::Release::Label=AcerUltraX \
    -o APT::FTPArchive::Release::Suite=stable \
    -o APT::FTPArchive::Release::Architectures=amd64 \
    -o "APT::FTPArchive::Release::Description=AcerUltraX: your Acer laptop's Windows sound on Linux" \
    release . > Release
rm -f InRelease Release.gpg
gpg --batch --yes --clearsign -o InRelease Release
gpg --batch --yes -abs -o Release.gpg Release
gpg --batch --yes --export -o acerultrax.gpg
gpg --batch --yes --armor --export -o acerultrax.asc
