#!/bin/sh
# Renders assets/logo.svg into the app's icons. Run on macOS after changing the logo:
#   sh tools/icons.sh
# Needs: qlmanage (macOS QuickLook, renders the SVG like a browser), ImageMagick (`magick`),
# iconutil (Xcode command line tools). ImageMagick's own SVG renderer drops the gradients
# and the anvil, hence QuickLook.
set -eu
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

qlmanage -t -s 1024 -o "$tmp" assets/logo.svg >/dev/null 2>&1
# QuickLook paints a white page behind the SVG: cut the tile's rounded corners
# (rect 16..496 of 512, rx 112) back to transparent.
magick -size 1024x1024 xc:black -fill white -draw "roundrectangle 32,32 991,991 224,224" "$tmp/mask.png"
magick "$tmp/logo.svg.png" "$tmp/mask.png" -alpha off -compose CopyOpacity -composite "$tmp/full.png"

# macOS: Apple's icon grid keeps the tile at about 824/1024 with room around it.
set="$tmp/Blacksmith.iconset"
mkdir "$set"
magick "$tmp/full.png" -resize 824x824 -background none -gravity center -extent 1024x1024 "$tmp/mac.png"
for s in 16 32 128 256 512; do
  magick "$tmp/mac.png" -resize ${s}x${s} "$set/icon_${s}x${s}.png"
  magick "$tmp/mac.png" -resize $((s * 2))x$((s * 2)) "$set/icon_${s}x${s}@2x.png"
done
iconutil -c icns -o assets/Blacksmith.icns "$set"

# Windows: the .exe's icon (embedded by build.rs), several sizes.
magick "$tmp/full.png" -define icon:auto-resize=256,128,64,48,32,24,16 assets/icon.ico

# The window icon, raw RGBA (no image decoder needed at run time).
magick "$tmp/full.png" -resize 64x64 -depth 8 rgba:assets/icon-64.rgba
echo "icons written: assets/Blacksmith.icns assets/icon.ico assets/icon-64.rgba"
