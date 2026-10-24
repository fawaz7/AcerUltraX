#!/bin/sh
# Build the Nix package (flake) of this checkout inside a NixOS container, without installing Nix.
# The Nix store lives in the Docker volume `acerultrax-nix`, so later builds reuse it (remove it with
# `docker volume rm acerultrax-nix`).
#
#   scripts/nix-build.sh                 build .#acerultrax (all three programs) and run its tests
#   scripts/nix-build.sh acerultrax-headless
set -eu
cd "$(dirname "$0")/.."
target=${1:-acerultrax}
exec docker run --rm --memory=8g --cpus=4 \
    -v acerultrax-nix:/nix \
    -v "$PWD":/src:ro \
    nixos/nix:latest \
    sh -c "set -e
        echo 'experimental-features = nix-command flakes' >> /etc/nix/nix.conf
        echo 'max-jobs = 1' >> /etc/nix/nix.conf
        echo 'cores = 4' >> /etc/nix/nix.conf
        mkdir /tmp/src && tar -C /src --exclude=./target --exclude=./.git -c . | tar -C /tmp/src -x && cd /tmp/src
        nix build --print-build-logs path:.#$target
        echo; echo 'Built:'; ls -l result/bin result/lib/systemd/user 2>/dev/null
        echo 'Programs next to each other:'; ls result/bin"
