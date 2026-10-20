#!/bin/sh
# Rebuild the icons: SVGs from generate.py, PNGs with rsvg-convert. Run from anywhere.
set -e
cd "$(dirname "$0")"
ID=io.github.fawaz7.AcerUltraX
python3 -I generate.py svg
mkdir -p hicolor/scalable/apps hicolor/symbolic/apps tray
mv svg/app.svg "hicolor/scalable/apps/$ID.svg"
mv svg/tray-solid.svg "hicolor/symbolic/apps/$ID-symbolic.svg"
mv svg/tray-facets.svg svg/tray-badge.svg tray/
mv svg/mark.svg mark.svg
for s in 16 24 32; do
    mkdir -p "hicolor/${s}x$s/apps"
    rsvg-convert -w $s -h $s svg/app-small.svg -o "hicolor/${s}x$s/apps/$ID.png"
done
for s in 48 64 128 256 512; do
    mkdir -p "hicolor/${s}x$s/apps"
    rsvg-convert -w $s -h $s "hicolor/scalable/apps/$ID.svg" -o "hicolor/${s}x$s/apps/$ID.png"
done
rm -r svg
# In-app brand mark (embedded in the app) and GitHub's social preview.
rsvg-convert -w 96 -h 96 mark.svg -o ../../crates/gui/src/mark.png
rsvg-convert social-preview.svg -o social-preview.png
