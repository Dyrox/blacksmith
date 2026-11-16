#!/bin/sh
# Packs Blacksmith.app into a disk image: the app next to a link to
# /Applications, so installing is one drag.
#   sh tools/dmg.sh dist/Blacksmith.app dist/Blacksmith.dmg
set -eu

APP=$1
OUT=$2
NAME=Blacksmith

WORK=$(mktemp -d)
trap 'hdiutil detach "$WORK/mnt" -quiet 2>/dev/null || true; rm -rf "$WORK"' EXIT

mkdir "$WORK/stage"
ditto "$APP" "$WORK/stage/$NAME.app"
ln -s /Applications "$WORK/stage/Applications"

hdiutil create -quiet -srcfolder "$WORK/stage" -volname "$NAME" -fs HFS+ -format UDRW "$WORK/rw.dmg"
hdiutil attach -quiet -noautoopen -nobrowse -mountpoint "$WORK/mnt" "$WORK/rw.dmg"

# The Finder window: icons side by side, app on the left. Cosmetic only — if
# Finder can't be scripted (no GUI session), the image still works.
osascript <<OSA || echo "dmg: window layout skipped"
tell application "Finder"
  set d to POSIX file "$WORK/mnt" as alias
  open d
  set w to container window of d
  set current view of w to icon view
  set toolbar visible of w to false
  set statusbar visible of w to false
  set bounds of w to {200, 200, 740, 520}
  set o to icon view options of w
  set arrangement of o to not arranged
  set icon size of o to 112
  set position of item "$NAME.app" of d to {140, 150}
  set position of item "Applications" of d to {400, 150}
  update d without registering applications
  delay 1
  close w
end tell
OSA

sync
hdiutil detach -quiet "$WORK/mnt"
rm -f "$OUT"
hdiutil convert -quiet "$WORK/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$OUT"
echo "$OUT"
