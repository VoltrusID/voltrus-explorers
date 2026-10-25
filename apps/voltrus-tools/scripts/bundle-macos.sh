#!/usr/bin/env bash
# Wrap the release binary in an app bundle and zip it.
# Usage: scripts/bundle-macos.sh <binary> <output dir>
set -euo pipefail

binary="$1"
out="$2"
here="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/Cargo.toml" | head -1)"
app="$out/Voltrus Tools.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/voltrus-tools"

iconset="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$here/packaging/AppIcon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$here/packaging/AppIcon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

sed "s/@VERSION@/$version/g" "$here/packaging/Info.plist" > "$app/Contents/Info.plist"

# Ad-hoc signature: required to launch on Apple silicon. Release builds get a
# Developer ID signature and notarization instead.
codesign --force --deep --sign - "$app"

(cd "$out" && ditto -c -k --keepParent "Voltrus Tools.app" "Voltrus-Tools-macos-arm64.zip")
echo "Bundled $app ($version)"
