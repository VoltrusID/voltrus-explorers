#!/bin/bash
set -euo pipefail

# ============================================================
# BACnet Explorer — iOS build via Tauri v2 mobile
# ============================================================
# Builds the iOS app (iPhone and iPad) on macOS with Xcode. On
# first run it generates src-tauri/gen/apple (gitignored),
# injects icons, and archives the app. The device control panel
# window is desktop-only: on iOS the UI shows it as a sheet.
#
# Usage:
#   ./scripts/build-ios.sh                 # device build + IPA export
#   ./scripts/build-ios.sh --open          # open Xcode for manual Archive/Distribute
#   ./scripts/build-ios.sh --no-sign       # unsigned build (no certs needed)
#   ./scripts/build-ios.sh --skip-init     # skip init/icon steps
#   ./scripts/build-ios.sh --multicast     # add the multicast entitlement
#
# Prerequisites (macOS only):
#   1. Xcode with the iOS SDK
#   2. rustup target add aarch64-apple-ios
#   3. Signing team for com.joyodigitama.bacnet-explorer
#      (tauri.conf.json bundle.iOS.developmentTeam, override with
#      APPLE_DEVELOPMENT_TEAM env)
#
# Notes:
#   - src-tauri/Info.ios.plist is merged into the generated iOS
#     Info.plist (NSLocalNetworkUsageDescription for device access,
#     Who-Is scanning, COV and the simulator).
#   - Who-Is scanning needs Apple's multicast entitlement
#     (com.apple.developer.networking.multicast, see
#     src-tauri/entitlements.ios.plist and docs/ios-multicast.md).
#     It is added to the generated entitlements only when IOS_MULTICAST=1
#     (or --multicast): a provisioning profile without it makes the
#     archive fail to sign. release.yml sets it when the profile has it.
#   - Upload the exported IPA via Transporter or
#     `xcrun altool --upload-app --type ios`.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ICON_SRC="$PROJECT_DIR/src-tauri/icons/icon-1024.png"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { echo -e "${BLUE}[*]${NC} $1"; }
ok()    { echo -e "${GREEN}[✓]${NC} $1"; }
warn()  { echo -e "${YELLOW}[!]${NC} $1"; }
err()   { echo -e "${RED}[✗]${NC} $1"; }
step()  { echo -e "\n${BLUE}━━━ $1 ━━━${NC}"; }

OPEN_XCODE=""
NO_SIGN=""
SKIP_INIT=""
MULTICAST="${IOS_MULTICAST:-}"
BUILD_NUMBER=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --open)         OPEN_XCODE=1 ;;
    --no-sign)      NO_SIGN=1 ;;
    --skip-init)    SKIP_INIT=1 ;;
    --multicast)    MULTICAST=1 ;;
    --build-number) BUILD_NUMBER="$2"; shift ;;
    *) err "Unknown option: $1"; usage_exit=1 ;;
  esac
  shift
done
if [[ -n "${usage_exit:-}" ]]; then
  echo "Usage: $0 [--open] [--no-sign] [--skip-init] [--multicast] [--build-number N]"
  exit 1
fi

# ============================================================
# 1. Environment checks
# ============================================================
step "Step 1/3: Environment checks"

if [[ "$(uname -s)" != "Darwin" ]]; then
  err "iOS builds must run on macOS with Xcode installed."
  exit 1
fi
# Runner services don't source .zprofile — expose brew (and its packages:
# xcodegen, pod, idevicesyslog) to cargo-mobile2's spawned commands.
if [[ -x /opt/homebrew/bin/brew ]]; then
  eval "$(/opt/homebrew/bin/brew shellenv)"
elif [[ -x /usr/local/bin/brew ]]; then
  eval "$(/usr/local/bin/brew shellenv)"
fi
if ! command -v xcodebuild >/dev/null 2>&1; then
  err "xcodebuild not found — install Xcode and run: xcode-select --install"
  exit 1
fi
if ! xcodebuild -showsdks 2>/dev/null | grep -q "iphoneos"; then
  err "iOS SDK not found — open Xcode → Settings → Platforms and install the iOS platform."
  exit 1
fi

TEAM_ID="${APPLE_DEVELOPMENT_TEAM:-F6Q6UFUK69}"
export APPLE_DEVELOPMENT_TEAM="$TEAM_ID"

# Tauri CLI runs through a package script (bun workspace).
if command -v bun >/dev/null 2>&1; then
  TAURI=(bun run tauri)
elif command -v pnpm >/dev/null 2>&1; then
  TAURI=(pnpm tauri)
else
  err "Neither bun nor pnpm found — install one to run the Tauri CLI."
  exit 1
fi

if ! rustup target list --installed 2>/dev/null | grep -q "aarch64-apple-ios"; then
  warn "Rust target aarch64-apple-ios not installed — adding it."
  rustup target add aarch64-apple-ios
fi

if [[ ! -d "$PROJECT_DIR/src-tauri/gen/apple" ]] && ! command -v xcodegen >/dev/null 2>&1; then
  err "XcodeGen is required to generate the iOS project: brew install xcodegen (or copy the release binary to ~/.local/bin)"
  exit 1
fi

if [[ ! -f "$ICON_SRC" ]]; then
  err "Icon source missing: $ICON_SRC"
  exit 1
fi

ok "Environment ready (team $TEAM_ID, runner: ${TAURI[0]})"

# ============================================================
# 2. Project generation + icons
# ============================================================
if [[ -z "$SKIP_INIT" ]]; then
  step "Step 2/3: Xcode project + icons"

  if [[ ! -d "$PROJECT_DIR/src-tauri/gen/apple" ]]; then
    info "Generating Xcode project (first run)..."
    ( cd "$PROJECT_DIR" && "${TAURI[@]}" ios init )
    ok "Project generated: src-tauri/gen/apple"
  else
    info "Xcode project exists — skipping init."
  fi
  info "Generating app icons into the Xcode asset catalog..."
  ( cd "$PROJECT_DIR" && "${TAURI[@]}" icon "$ICON_SRC" )
  ok "Icons generated"
else
  info "Skipping init/icon steps (--skip-init)."
fi

# ============================================================
# 2b. Entitlements
# ============================================================
if [[ -z "$SKIP_INIT" || -n "$MULTICAST" ]]; then
  ENTITLEMENTS_FILE="$(find "$PROJECT_DIR/src-tauri/gen/apple" -name '*.entitlements' -not -path '*/build/*' 2>/dev/null | head -1 || true)"
  PLIST_BUDDY=/usr/libexec/PlistBuddy
  if [[ -z "$ENTITLEMENTS_FILE" ]]; then
    if [[ -n "$MULTICAST" ]]; then
      err "No .entitlements file in src-tauri/gen/apple (generated by 'tauri ios init'); cannot add the multicast entitlement."
      exit 1
    fi
  else
    # Start from a clean slate so reruns and a profile that lost the capability both work.
    "$PLIST_BUDDY" -c "Delete :com.apple.developer.networking.multicast" "$ENTITLEMENTS_FILE" >/dev/null 2>&1 || true
    if [[ -n "$MULTICAST" ]]; then
      "$PLIST_BUDDY" -c "Merge $PROJECT_DIR/src-tauri/entitlements.ios.plist" "$ENTITLEMENTS_FILE"
      ok "Multicast networking entitlement added (Who-Is scanning)"
    fi
  fi
  if [[ -z "$MULTICAST" ]]; then
    warn "Building without the multicast entitlement: Who-Is scanning finds nothing; connecting by IP works."
  fi
fi

# ============================================================
# 3. Build
# ============================================================
step "Step 3/3: Building iOS app (aarch64, App Store export)"

# iOS ships only through the App Store, so it always sells Pro (StoreKit).
BUILD_ARGS=(--target aarch64 --export-method app-store-connect --ci --features store)
if [[ -n "$OPEN_XCODE" ]]; then BUILD_ARGS+=(--open); fi
if [[ -n "$NO_SIGN" ]]; then BUILD_ARGS+=(--no-sign); fi
if [[ -n "$BUILD_NUMBER" ]]; then BUILD_ARGS+=(--build-number "$BUILD_NUMBER"); fi

( cd "$PROJECT_DIR" && "${TAURI[@]}" ios build "${BUILD_ARGS[@]}" )

IPA_PATH="$(find "$PROJECT_DIR/src-tauri/gen/apple/build" -name '*.ipa' 2>/dev/null | head -1 || true)"

if [[ -n "$IPA_PATH" ]]; then
  if ! unzip -l "$IPA_PATH" 2>/dev/null | grep -q "Payload/[^/]*\.app/"; then
    err "IPA is malformed — no .app bundle inside Payload/ (known Xcode 26 archive quirk)."
    err "Inspect the xcarchive: $PROJECT_DIR/src-tauri/gen/apple/build/"
    find "$PROJECT_DIR/src-tauri/gen/apple/build" -maxdepth 5 -print >&2
    grep -n "PRODUCT_NAME\|PRODUCT_BUNDLE_IDENTIFIER" "$PROJECT_DIR/src-tauri/gen/apple/project.yml" >&2 || true
    unzip -l "$IPA_PATH" >&2 || true
    exit 1
  fi
  ok "IPA structure valid: $IPA_PATH"
fi

# ============================================================
# 4. Result
# ============================================================

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║   iOS Build Ready                            ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
if [[ -n "$IPA_PATH" ]]; then
  echo -e "  IPA:      ${BLUE}$IPA_PATH${NC}"
else
  echo -e "  Archive:  ${BLUE}$PROJECT_DIR/src-tauri/gen/apple/build/${NC}"
fi
echo ""
echo -e "  ${YELLOW}Upload options:${NC}"
echo ""
echo "  Option 1: Transporter app (recommended)"
echo "    1. Open Transporter from /Applications"
echo "    2. Drag the .ipa file into Transporter"
echo "    3. Click 'Deliver'"
echo ""
echo "  Option 2: xcrun altool"
if [[ -n "$IPA_PATH" ]]; then
  echo "    xcrun altool --upload-app --type ios -f \"$IPA_PATH\" --apiKey <KEY_ID> --apiIssuer <ISSUER_ID>"
fi
echo ""
echo -e "  ${YELLOW}App Store Connect checklist:${NC}"
echo "    1. Bundle ID: com.joyodigitama.bacnet-explorer (the Mac app already uses it; add the iOS platform to the app record)"
echo "    2. Screenshots: 6.7\" (1290x2796) + 6.5\" (1284x2778) — capture from simulator"
echo "    3. Review notes: the built-in simulator (Pro) gives a device to browse, read and write on-device"
echo "       (Simulation > Connect to simulator). Scanning needs the multicast entitlement; connecting by IP"
echo "       address always works. The local-network permission prompt appears on first network use."
echo ""
