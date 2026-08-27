#!/bin/sh
# Rasterise icon-src.svg into the Android launcher mipmaps that cargo-apk packs
# (referenced as @mipmap/ic_launcher via Cargo.toml).  Re-run after editing the
# SVG.  Needs rsvg-convert (librsvg).  Vector is rendered natively at each
# density rather than upscaled, so every size stays crisp.
set -e
cd "$(dirname "$0")/.."

SVG=icon-src.svg

# Android density buckets → launcher icon px.
gen() { # <dir> <px>
    mkdir -p "res/mipmap-$1"
    rsvg-convert -w "$2" -h "$2" "$SVG" -o "res/mipmap-$1/ic_launcher.png"
    echo "  res/mipmap-$1/ic_launcher.png  (${2}px)"
}

echo "Generating launcher icons from $SVG:"
gen mdpi 48
gen hdpi 72
gen xhdpi 96
gen xxhdpi 144
gen xxxhdpi 192
echo "Done."
