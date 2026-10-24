#!/bin/sh
# Screenshot a page of the app (built from this checkout) without touching the running AcerUltraX: it starts in a
# private D-Bus session, saves the window, and quits.
#
#   scripts/snapshot.sh about /tmp/about.png        pages: sound profiles devices settings about
#   ACERULTRAX_CHANNEL=preview scripts/snapshot.sh about out.png
# XDG_DATA_HOME can point it at another setup (e.g. one imported for another laptop).
set -eu
cd "$(dirname "$0")/.."
page=${1:?page}; out=${2:?output.png}
cargo build -q --release -j4 -p acerultrax
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac
timeout 40 dbus-run-session -- env ACERULTRAX_SNAPSHOT="$out" ACERULTRAX_PAGE="$page" \
    ACERULTRAX_SNAPSHOT_DELAY="${ACERULTRAX_SNAPSHOT_DELAY:-3000}" target/release/acerultrax >/dev/null 2>&1 || true
[ -s "$out" ] && echo "$out" || { echo "no screenshot written" >&2; exit 1; }
