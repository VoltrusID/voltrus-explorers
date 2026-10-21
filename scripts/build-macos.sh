#!/bin/bash
# Universal macOS build of one app, signed for either the Mac App Store or direct download.
#
#   scripts/build-macos.sh <app> appstore [--upload]   # .pkg for App Store Connect (validated, or uploaded)
#   scripts/build-macos.sh <app> dmg [--notarize]      # Developer ID .dmg (optionally notarized + stapled)
#   scripts/build-macos.sh <app> unsigned              # compile + bundle only (CI on pull requests)
#
# <app> is a directory under apps/ (bacnet-explorer, modbus-poll, dnp3-explorer, s7-explorer). Output goes to build/<app>/<mode>/.
#
# Signing identities come from the keychain search list (CI imports them into a temporary keychain).
#   appstore: "Apple Distribution" + "3rd Party Mac Developer Installer", and MAS_PROFILE = path to a
#             MAC_APP_STORE provisioning profile for the app's bundle id (scripts/asc-profile.py).
#   dmg:      "Developer ID Application".
# --upload, --notarize and the App Store validation (when ASC_KEY_ID is set) use the App Store Connect API key:
# ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH.
# BUILD_NUMBER (CFBundleVersion) defaults to the Unix time, the scheme the existing App Store builds use.
set -euo pipefail

APP="${1:?usage: $0 <app> appstore|dmg|unsigned [--upload|--notarize]}"
MODE="${2:?usage: $0 <app> appstore|dmg|unsigned [--upload|--notarize]}"
FLAG="${3:-}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_DIR="$ROOT/apps/$APP"
CONF="$APP_DIR/src-tauri/tauri.conf.json"
[[ -f "$CONF" ]] || { echo "no such app: $APP" >&2; exit 1; }
case "$MODE/$FLAG" in
  appstore/|appstore/--upload|dmg/|dmg/--notarize|unsigned/) ;;
  *) echo "invalid mode/flag: $MODE $FLAG" >&2; exit 1 ;;
esac

conf() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))$1)" "$CONF"; }
PRODUCT="$(conf "['productName']")"
BUNDLE_ID="$(conf "['identifier']")"
VERSION="$(conf "['version']")"
BUILD_NUMBER="${BUILD_NUMBER:-$(date +%s)}"
OUT="$ROOT/build/$APP/$MODE"
NAME="${PRODUCT// /}-$VERSION"

identity() { # SHA-1 of the first valid identity whose name contains $1 ($2: optional -p policy)
  security find-identity -v ${2:+-p "$2"} | awk -v n="$1" 'index($0, n) { print $2; exit }'
}
identity_name() { # full name of that identity (productbuild wants a name, not a hash)
  security find-identity -v | awk -F'"' -v n="$1" 'index($0, n) { print $2; exit }'
}

echo "==> $PRODUCT $VERSION ($BUILD_NUMBER), $MODE"
rm -rf "$OUT" && mkdir -p "$OUT"
# Only the App Store build sells Pro (StoreKit); source, DMG and CI builds have everything unlocked.
FEATURES=()
[[ "$MODE" == appstore ]] && FEATURES=(--features store)
# ${a[@]+...}: macOS /bin/bash 3.2 treats an empty array as unbound under `set -u`.
( cd "$APP_DIR" && bun run tauri build --target universal-apple-darwin --bundles app --ci ${FEATURES[@]+"${FEATURES[@]}"} )
cp -R "$ROOT/target/universal-apple-darwin/release/bundle/macos/$PRODUCT.app" "$OUT/"
BUNDLE="$OUT/$PRODUCT.app"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$BUNDLE/Contents/Info.plist"
ARCHS="$(lipo -archs "$BUNDLE/Contents/MacOS/"*)"
[[ "$ARCHS" == *x86_64* && "$ARCHS" == *arm64* ]] || { echo "not a universal binary: $ARCHS" >&2; exit 1; }

case "$MODE" in
unsigned)
  echo "==> $BUNDLE"
  ;;

appstore)
  : "${MAS_PROFILE:?MAS_PROFILE must point to a MAC_APP_STORE provisioning profile}"
  APP_ID="$(identity "Apple Distribution" codesigning)"
  PKG_ID="$(identity_name "3rd Party Mac Developer Installer")"
  [[ -n "$APP_ID" && -n "$PKG_ID" ]] || { echo "Apple Distribution / Mac Installer identity missing" >&2; exit 1; }

  # macOS codesign does not take entitlements from the profile: application-identifier and team-identifier
  # must be declared and match the profile exactly, or App Store Connect rejects the build (error 90886).
  PROFILE_PLIST="$OUT/profile.plist"
  security cms -D -i "$MAS_PROFILE" > "$PROFILE_PLIST"
  AID="$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.application-identifier" "$PROFILE_PLIST")"
  TEAM="$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.developer.team-identifier" "$PROFILE_PLIST")"
  [[ "$AID" == "$TEAM.$BUNDLE_ID" ]] || { echo "profile is for $AID, expected $TEAM.$BUNDLE_ID" >&2; exit 1; }
  ENT="$OUT/entitlements-appstore.plist"
  cp "$APP_DIR/src-tauri/entitlements.plist" "$ENT"
  /usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $AID" "$ENT"
  /usr/libexec/PlistBuddy -c "Add :com.apple.developer.team-identifier string $TEAM" "$ENT"

  cp "$MAS_PROFILE" "$BUNDLE/Contents/embedded.provisionprofile"
  xattr -cr "$BUNDLE"
  codesign --force --options runtime --timestamp --entitlements "$ENT" --sign "$APP_ID" "$BUNDLE"
  codesign --verify --strict --verbose=2 "$BUNDLE"
  PKG="$OUT/$NAME.pkg"
  productbuild --component "$BUNDLE" /Applications --sign "$PKG_ID" "$PKG"
  echo "==> $PKG"
  # altool reads the key from ~/.appstoreconnect/private_keys/AuthKey_<id>.p8
  if [[ "$FLAG" == --upload ]]; then
    xcrun altool --upload-app --type macos --file "$PKG" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
  elif [[ -n "${ASC_KEY_ID:-}" ]]; then
    xcrun altool --validate-app --type macos --file "$PKG" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
  fi
  ;;

dmg)
  APP_ID="$(identity "Developer ID Application" codesigning)"
  [[ -n "$APP_ID" ]] || { echo "Developer ID Application identity missing" >&2; exit 1; }
  xattr -cr "$BUNDLE"
  codesign --force --options runtime --timestamp --entitlements "$APP_DIR/src-tauri/entitlements.plist" --sign "$APP_ID" "$BUNDLE"
  codesign --verify --strict --verbose=2 "$BUNDLE"
  DMG="$OUT/$NAME-universal.dmg"
  STAGING="$(mktemp -d)"
  cp -R "$BUNDLE" "$STAGING/"
  ln -s /Applications "$STAGING/Applications"
  hdiutil create -volname "$PRODUCT" -srcfolder "$STAGING" -ov -format UDZO "$DMG"
  rm -rf "$STAGING"
  codesign --force --timestamp --sign "$APP_ID" "$DMG"
  if [[ "$FLAG" == --notarize ]]; then
    xcrun notarytool submit "$DMG" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait
    xcrun stapler staple "$DMG"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
  fi
  echo "==> $DMG"
  ;;
esac
