#!/bin/bash
set -euo pipefail

# ============================================================
# Voltrus PLC — Build, Sign, Notarize, DMG
# ============================================================
# Usage:
#   ./scripts/build-dmg.sh              # Build + sign + DMG (no notarize)
#   ./scripts/build-dmg.sh --dev         # Dev build (skip license check)
#   NOTARIZE=1 ./scripts/build-dmg.sh   # Build + sign + notarize + DMG
#   SKIP_BUILD=1 ./scripts/build-dmg.sh # Skip cargo tauri build

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
APP_NAME="Voltrus PLC"
BUILD_DIR="$MONOREPO_DIR/target/release/bundle"
APP_PATH="$BUILD_DIR/macos/$APP_NAME.app"

SKIP_BUILD="${SKIP_BUILD:-0}"
NOTARIZE="${NOTARIZE:-0}"
DEV_MODE=0

for arg in "$@"; do
  case "$arg" in
    --dev)      DEV_MODE=1 ;;
    --notarize) NOTARIZE=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
  esac
done

# Colors
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

# Get version from tauri.conf.json
VERSION=$(grep '"version"' "$PROJECT_DIR/src-tauri/tauri.conf.json" | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/')
DMG_NAME="${APP_NAME// /_}_${VERSION}_aarch64.dmg"
DMG_PATH="$BUILD_DIR/dmg/$DMG_NAME"

# ============================================================
# 1. BUILD
# ============================================================
step "Step 1/4: Building $APP_NAME $VERSION"

if [[ "$SKIP_BUILD" == "1" ]]; then
  warn "Skipping build (SKIP_BUILD=1)"
  if [[ ! -d "$APP_PATH" ]]; then
    err "App not found at $APP_PATH — cannot skip build"
    exit 1
  fi
else
  info "Cleaning frontend dist..."
  rm -rf "$PROJECT_DIR/dist"
  info "Running cargo tauri build..."
  cd "$PROJECT_DIR"
  pnpm tauri build 2>&1 | tail -20
  if [[ ! -d "$APP_PATH" ]]; then
    err "Build failed — app not found at $APP_PATH"
    exit 1
  fi
  ok "Build complete: $APP_PATH"
fi

# ============================================================
# 2. CODE SIGN
# ============================================================
step "Step 2/4: Code Signing"

if [[ -z "${CODESIGN_IDENTITY:-}" ]]; then
  CODESIGN_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
    | awk '/Developer ID Application: Dzulfikar/ { print $2; exit }')
  if [[ -z "$CODESIGN_IDENTITY" ]]; then
    CODESIGN_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
      | awk '/Apple Development:/ { print $2; exit }')
    if [[ -n "$CODESIGN_IDENTITY" ]]; then
      warn "Using Apple Development certificate (not for production distribution)"
    fi
  fi
  if [[ -z "$CODESIGN_IDENTITY" ]]; then
    err "No signing identity found. Set CODESIGN_IDENTITY."
    exit 1
  fi
fi

info "Signing with: $(security find-identity -v -p codesigning 2>/dev/null | grep "$CODESIGN_IDENTITY" | head -1)"
codesign --deep --force --verify --verbose \
  --sign "$CODESIGN_IDENTITY" \
  --options runtime \
  --entitlements "$PROJECT_DIR/src-tauri/entitlements.plist" \
  "$APP_PATH"
ok "Signed successfully"

# ============================================================
# 3. NOTARIZE
# ============================================================
step "Step 3/4: Notarization"

CERT_NAME=$(security find-identity -v -p codesigning 2>/dev/null | grep "$CODESIGN_IDENTITY" | head -1)
if [[ "$CERT_NAME" =~ "Apple Development" ]]; then
  warn "Skipping notarization (development certificate)"
  NOTARIZE=0
fi

if [[ "$NOTARIZE" == "1" ]]; then
  NOTARY_PROFILE="${NOTARY_KEYCHAIN_PROFILE:-waqtu-notary}"

  info "Zipping for notarization..."
  ZIP_PATH="$BUILD_DIR/${APP_NAME// /_}-$VERSION.zip"
  rm -f "$ZIP_PATH"
  ditto -c -k --keepParent "$APP_PATH" "$ZIP_PATH"

  info "Submitting to Apple notarization..."
  xcrun notarytool submit "$ZIP_PATH" --keychain-profile "$NOTARY_PROFILE" --wait

  info "Stapling notarization ticket..."
  xcrun stapler staple "$APP_PATH"

  info "Verifying Gatekeeper acceptance..."
  spctl --assess --verbose "$APP_PATH" || true
  ok "Notarized and stapled"
else
  warn "Skipping notarization (set NOTARIZE=1 to enable)"
fi

# ============================================================
# 4. CREATE DMG
# ============================================================
step "Step 4/4: Creating DMG"

DMG_DIR="$BUILD_DIR/dmg-staging"
rm -rf "$DMG_DIR"
mkdir -p "$DMG_DIR"
cp -R "$APP_PATH" "$DMG_DIR/"
ln -sf /Applications "$DMG_DIR/Applications"

mkdir -p "$(dirname "$DMG_PATH")"
rm -f "$DMG_PATH"
hdiutil create \
  -volname "$APP_NAME" \
  -srcfolder "$DMG_DIR" \
  -ov -format UDZO \
  "$DMG_PATH"

rm -rf "$DMG_DIR"

FILE_SIZE=$(stat -f%z "$DMG_PATH" 2>/dev/null || stat -c%s "$DMG_PATH")
FILE_SIZE_MB=$(echo "scale=1; $FILE_SIZE / 1048576" | bc)
ok "DMG created: $DMG_PATH (${FILE_SIZE_MB} MB)"

# ============================================================
# DONE
# ============================================================
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║    $APP_NAME $VERSION — Build Complete         ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  DMG:  ${BLUE}$DMG_PATH${NC}"
echo -e "  Size: ${BLUE}${FILE_SIZE_MB} MB${NC}"
echo ""
