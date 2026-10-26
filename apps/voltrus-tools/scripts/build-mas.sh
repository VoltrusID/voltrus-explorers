#!/usr/bin/env bash
# Mac App Store build of Voltrus Tools: universal (arm64 + x86_64), `store`
# feature (StoreKit 2 trial and unlock), App Sandbox, signed into a .pkg for
# App Store Connect. Release steps: docs/release.md.
#
#   scripts/build-mas.sh bundle <out dir>                   # compile, lipo, bundle: <out>/Voltrus Tools.app (unsigned)
#   scripts/build-mas.sh package <app> <out dir> [--upload] # sign the app, pack <out>/VoltrusTools-<version>.pkg,
#                                                           # validate it with App Store Connect (or upload it)
#
# `bundle` needs no secrets: CI builds it on every push so the store variant
# can't rot. `package` needs what scripts/build-macos.sh appstore needs: the
# "Apple Distribution" and "3rd Party Mac Developer Installer" identities in
# the keychain, MAS_PROFILE (a MAC_APP_STORE profile for the bundle id, from
# scripts/asc-profile.py) and, to validate or upload, ASC_KEY_ID and
# ASC_ISSUER_ID (altool reads ~/.appstoreconnect/private_keys/AuthKey_<id>.p8).
# BUILD_NUMBER (CFBundleVersion) defaults to the Unix time, as for the Tauri
# apps.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_ID=com.joyodigitama.voltrus-tools
usage="usage: $0 bundle <out dir> | package <app> <out dir> [--upload]"

identity() { # SHA-1 of the first valid identity whose name contains $1 ($2: optional -p policy)
  security find-identity -v ${2:+-p "$2"} | awk -v n="$1" 'index($0, n) { print $2; exit }'
}
identity_name() { # full name of that identity (productbuild wants a name, not a hash)
  security find-identity -v | awk -F'"' -v n="$1" 'index($0, n) { print $2; exit }'
}

case "${1:-}" in
bundle)
  out="${2:?$usage}"
  mkdir -p "$out"
  out="$(cd "$out" && pwd)"
  targets=(aarch64-apple-darwin x86_64-apple-darwin)
  binaries=()
  for target in "${targets[@]}"; do
    (cd "$here" && cargo build --locked --release -p vt-app --features store --target "$target")
    binaries+=("$here/target/$target/release/voltrus-tools")
  done
  lipo -create -output "$out/voltrus-tools" "${binaries[@]}"
  archs="$(lipo -archs "$out/voltrus-tools")"
  [[ "$archs" == *x86_64* && "$archs" == *arm64* ]] || { echo "not a universal binary: $archs" >&2; exit 1; }

  "$here/scripts/bundle-macos.sh" "$out/voltrus-tools" "$out"
  rm -f "$out/voltrus-tools" "$out"/Voltrus-Tools-macos-*.zip
  app="$out/Voltrus Tools.app"
  plist="$app/Contents/Info.plist"
  put() { # key type value: set, or add when missing
    /usr/libexec/PlistBuddy -c "Set :$1 $3" "$plist" 2>/dev/null \
      || /usr/libexec/PlistBuddy -c "Add :$1 $2 $3" "$plist"
  }
  put CFBundleIdentifier string "$BUNDLE_ID"
  put CFBundleVersion string "${BUILD_NUMBER:-$(date +%s)}"
  put NSBluetoothAlwaysUsageDescription string \
    "Voltrus Tools connects to the Bluetooth LE devices you choose to inspect."
  # Ad-hoc again after the Info.plist changes, so the unsigned bundle still
  # launches for a smoke test; `package` replaces it.
  codesign --force --sign - "$app"
  version="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$plist")"
  (cd "$out" && ditto -c -k --keepParent "Voltrus Tools.app" "Voltrus-Tools-$version-mas-unsigned.zip")
  echo "==> $app ($archs)"
  ;;

package)
  app="${2:?$usage}"
  out="${3:?$usage}"
  flag="${4:-}"
  [[ -z "$flag" || "$flag" == --upload ]] || { echo "$usage" >&2; exit 1; }
  : "${MAS_PROFILE:?MAS_PROFILE must point to a MAC_APP_STORE provisioning profile for $BUNDLE_ID}"
  mkdir -p "$out"
  APP_ID="$(identity "Apple Distribution" codesigning)"
  PKG_ID="$(identity_name "3rd Party Mac Developer Installer")"
  [[ -n "$APP_ID" && -n "$PKG_ID" ]] || { echo "Apple Distribution / Mac Installer identity missing" >&2; exit 1; }
  plist="$app/Contents/Info.plist"
  [[ "$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$plist")" == "$BUNDLE_ID" ]] \
    || { echo "$app is not a store bundle (scripts/build-mas.sh bundle)" >&2; exit 1; }

  # codesign doesn't take entitlements from the profile: the application and
  # team identifiers must be declared and match it (App Store Connect error
  # 90886), as in scripts/build-macos.sh.
  profile_plist="$out/profile.plist"
  security cms -D -i "$MAS_PROFILE" > "$profile_plist"
  aid="$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.application-identifier" "$profile_plist")"
  team="$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.developer.team-identifier" "$profile_plist")"
  [[ "$aid" == "$team.$BUNDLE_ID" ]] || { echo "profile is for $aid, expected $team.$BUNDLE_ID" >&2; exit 1; }
  ent="$out/entitlements.plist"
  cp "$here/packaging/entitlements.mas.plist" "$ent"
  /usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $aid" "$ent"
  /usr/libexec/PlistBuddy -c "Add :com.apple.developer.team-identifier string $team" "$ent"

  cp "$MAS_PROFILE" "$app/Contents/embedded.provisionprofile"
  xattr -cr "$app"
  codesign --force --options runtime --timestamp --entitlements "$ent" --sign "$APP_ID" "$app"
  codesign --verify --strict --verbose=2 "$app"
  codesign -d --entitlements - "$app"
  version="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$plist")"
  pkg="$out/VoltrusTools-$version.pkg"
  productbuild --component "$app" /Applications --sign "$PKG_ID" "$pkg"
  echo "==> $pkg"
  if [[ "$flag" == --upload ]]; then
    xcrun altool --upload-app --type macos --file "$pkg" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
  elif [[ -n "${ASC_KEY_ID:-}" ]]; then
    xcrun altool --validate-app --type macos --file "$pkg" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
  fi
  ;;

*)
  echo "$usage" >&2
  exit 1
  ;;
esac
