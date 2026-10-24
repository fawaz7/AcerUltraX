#!/bin/sh
# Release the tested `preview` branch as version X.Y.Z: bump, check, move main to it, tag, push, and confirm the
# GitHub release and the apt repository have it. Write the CHANGELOG.md entry ("## X.Y.Z (date)") and
# packaging/RELEASE_NOTES.md first; the tag's workflow publishes those notes.
#
#   scripts/release.sh 0.1.4
set -eu
cd "$(dirname "$0")/.."
v=${1:?version, e.g. 0.1.4}
echo "$v" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "version must be X.Y.Z" >&2; exit 1; }
[ "$(git branch --show-current)" = preview ] || { echo "run it on the preview branch" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit or stash your changes first" >&2; exit 1; }
grep -q "^## $v " CHANGELOG.md || { echo "CHANGELOG.md has no '## $v (date)' entry" >&2; exit 1; }
grep -q "AcerUltraX $v" packaging/RELEASE_NOTES.md || { echo "RELEASE_NOTES.md does not start with 'AcerUltraX $v'" >&2; exit 1; }
git fetch -q origin
git merge-base --is-ancestor origin/main HEAD || { echo "preview is not based on the latest main" >&2; exit 1; }

sed -i "s/^version = \".*\"/version = \"$v\"/" Cargo.toml
sed -i "s/^pkgver=.*/pkgver=$v/" packaging/aur/PKGBUILD
cargo build -q --release -j4   # updates Cargo.lock
scripts/check.sh
git add Cargo.toml Cargo.lock packaging/aur/PKGBUILD
git commit -q -m "v$v" ${RELEASE_TRAILER:+-m "$RELEASE_TRAILER"}  # optional trailer, e.g. Co-Authored-By

git switch -q main
git merge -q --ff-only preview
git push -q origin main
git tag "v$v"
git push -q origin "v$v"
git switch -q preview

echo "Pushed v$v; waiting for the release workflow…"
sleep 10
run=$(gh run list --workflow release.yml --branch "v$v" --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" --exit-status >/dev/null
gh release view "v$v" --json assets --jq '.assets[].name'
# GitHub Pages can take a minute to serve the new apt index.
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
    curl -s https://fawaz7.github.io/AcerUltraX/apt/Packages | grep -q "^Version: $v$" && { echo "apt repository has $v"; exit 0; }
    sleep 15
done
echo "apt repository does not list $v yet: check the gh-pages branch" >&2
exit 1
