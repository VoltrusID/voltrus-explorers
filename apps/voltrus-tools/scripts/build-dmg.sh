#!/usr/bin/env bash
# Direct-download build of Voltrus Tools: universal (arm64 + x86_64), `license`
# feature (14-day trial on this Mac, then a licence key from the Voltrus
# console), hardened runtime without the App Sandbox, Developer ID signed,
# notarized and stapled into a DMG for dl-voltrus-tools.voltrus.id. Release
# steps: docs/release.md ("Direct download").
#
#   scripts/build-dmg.sh bundle <out dir>                      # compile, lipo, bundle: <out>/Voltrus Tools.app (unsigned)
#   scripts/build-dmg.sh package <app> <out dir> [--notarize]  # sign the app, pack <out>/Voltrus-Tools-<version>-universal.dmg,
#                                                              # sign it, and notarize and staple it
#
# `bundle` needs no secrets (CI runs it before any identity is on the runner).
# `package` needs what scripts/build-macos.sh dmg needs: the "Developer ID
# Application" identity in the keychain and, for --notarize, the App Store
# Connect API key (ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH). BUILD_NUMBER
# (CFBundleVersion) defaults to the Unix time.
#
# The DMG's name must match /voltrus[ _.-]*tools/i: the download worker serves
# only matching files.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_ID=com.joyodigitama.voltrus-tools
PRODUCT="Voltrus Tools"
usage="usage: $0 bundle <out dir> | package <app> <out dir> [--notarize]"

identity() { # SHA-1 of the first valid identity whose name contains $1 ($2: optional -p policy)
  security find-identity -v ${2:+-p "$2"} | awk -v n="$1" 'index($0, n) { print $2; exit }'
}

case "${1:-}" in
bundle)
  out="${2:?$usage}"
  mkdir -p "$out"
  out="$(cd "$out" && pwd)"
  targets=(aarch64-apple-darwin x86_64-apple-darwin)
  binaries=()
  for target in "${targets[@]}"; do
    (cd "$here" && cargo build --locked --release -p vt-app --features license --target "$target")
    binaries+=("$here/target/$target/release/voltrus-tools")
  done
  lipo -create -output "$out/voltrus-tools" "${binaries[@]}"
  archs="$(lipo -archs "$out/voltrus-tools")"
  [[ "$archs" == *x86_64* && "$archs" == *arm64* ]] || { echo "not a universal binary: $archs" >&2; exit 1; }

  "$here/scripts/bundle-macos.sh" "$out/voltrus-tools" "$out"
  rm -f "$out/voltrus-tools" "$out"/Voltrus-Tools-macos-*.zip
  app="$out/$PRODUCT.app"
  plist="$app/Contents/Info.plist"
  put() { # key type value: set, or add when missing
    /usr/libexec/PlistBuddy -c "Set :$1 $3" "$plist" 2>/dev/null \
      || /usr/libexec/PlistBuddy -c "Add :$1 $2 $3" "$plist"
  }
  # The App Store's bundle id: the same app, so the Keychain items and the
  # data folder name stay the app's own.
  put CFBundleIdentifier string "$BUNDLE_ID"
  put CFBundleVersion string "${BUILD_NUMBER:-$(date +%s)}"
  # Info.plist carries NSLocalNetworkUsageDescription and
  # NSBluetoothAlwaysUsageDescription: without the sandbox they are what macOS
  # shows when the app first reaches the network or Bluetooth.
  for key in NSLocalNetworkUsageDescription NSBluetoothAlwaysUsageDescription; do
    /usr/libexec/PlistBuddy -c "Print :$key" "$plist" >/dev/null || { echo "Info.plist lacks $key" >&2; exit 1; }
  done
  # Ad-hoc again after the Info.plist changes, so the unsigned bundle still
  # launches for a smoke test; `package` replaces it.
  codesign --force --sign - "$app"
  echo "==> $app ($archs)"
  ;;

package)
  app="${2:?$usage}"
  out="${3:?$usage}"
  flag="${4:-}"
  [[ -z "$flag" || "$flag" == --notarize ]] || { echo "$usage" >&2; exit 1; }
  mkdir -p "$out"
  plist="$app/Contents/Info.plist"
  [[ "$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$plist")" == "$BUNDLE_ID" ]] \
    || { echo "$app is not a direct-download bundle (scripts/build-dmg.sh bundle)" >&2; exit 1; }
  APP_ID="$(identity "Developer ID Application" codesigning)"
  [[ -n "$APP_ID" ]] || { echo "Developer ID Application identity missing" >&2; exit 1; }

  xattr -cr "$app"
  codesign --force --options runtime --timestamp \
    --entitlements "$here/packaging/entitlements.dmg.plist" --sign "$APP_ID" "$app"
  codesign --verify --strict --verbose=2 "$app"
  codesign -d --entitlements - "$app"

  version="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$plist")"
  dmg="$out/Voltrus-Tools-$version-universal.dmg"
  shopt -s nocasematch
  [[ "$(basename "$dmg")" =~ voltrus[\ _.-]*tools ]] || { echo "the worker would not serve $dmg" >&2; exit 1; }
  shopt -u nocasematch
  staging="$(mktemp -d)"
  cp -R "$app" "$staging/"
  ln -s /Applications "$staging/Applications"
  # hdiutil sizes the image from -srcfolder and sometimes guesses too small for a large binary ("No space left on
  # device" inside the new volume): give it the folder size plus 20% and 20 MB.
  dmg_mb=$(( $(du -sm "$staging" | cut -f1) * 12 / 10 + 20 ))
  hdiutil create -volname "$PRODUCT" -srcfolder "$staging" -size "${dmg_mb}m" -ov -format UDZO "$dmg"
  rm -rf "$staging"
  codesign --force --timestamp --sign "$APP_ID" "$dmg"
  if [[ "$flag" == --notarize ]]; then
    : "${ASC_KEY_PATH:?ASC_KEY_PATH must point to the App Store Connect API key (.p8)}"
    xcrun notarytool submit "$dmg" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
  fi
  echo "==> $dmg"
  ;;

*)
  echo "$usage" >&2
  exit 1
  ;;
esac
