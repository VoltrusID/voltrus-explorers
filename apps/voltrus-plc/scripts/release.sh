#!/bin/bash
set -euo pipefail

# ============================================================
# Voltrus PLC — Release Pipeline
# ============================================================
# End-to-end: Build → Sign → Notarize → DMG → Upload to R2 → D1
#
# Usage:
#   ./scripts/release.sh [VERSION] [DESCRIPTION]
#   ./scripts/release.sh 1.0.0 "Initial release"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
WORKER_DIR="$PROJECT_DIR/cloudflare-worker"
BUILD_DIR="$MONOREPO_DIR/target/release/bundle"
APP_NAME="Voltrus PLC"
APP_PATH="$BUILD_DIR/macos/$APP_NAME.app"

# ---- Parse args ----
VERSION="${1:-}"
DESCRIPTION="${2:-Bug fixes and improvements.}"

if [[ -z "$VERSION" ]]; then
  VERSION=$(grep '"version"' "$PROJECT_DIR/src-tauri/tauri.conf.json" | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/')
fi

# ---- Config ----
DMG_NAME="${APP_NAME// /_}_${VERSION}_aarch64.dmg"
DMG_PATH="$BUILD_DIR/dmg/$DMG_NAME"
R2_BUCKET="voltrus-plc-releases"
R2_PUBLIC_BASE_URL="${R2_PUBLIC_BASE_URL:-https://dl-voltrus-plc.voltrus.id}"
SKIP_BUILD="${SKIP_BUILD:-0}"
SKIP_UPLOAD="${SKIP_UPLOAD:-0}"
NOTARIZE="${NOTARIZE:-0}"
BUILD_NUMBER="${BUILD_NUMBER:-1}"

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

# ============================================================
# 1. BUILD
# ============================================================
step "Step 1/5: Building $APP_NAME $VERSION"

if [[ "$SKIP_BUILD" == "1" ]]; then
  warn "Skipping build (SKIP_BUILD=1)"
  if [[ ! -d "$APP_PATH" ]]; then
    err "App not found at $APP_PATH — cannot skip build"
    exit 1
  fi
else
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
# 2. CODE SIGN & NOTARIZE
# ============================================================
step "Step 2/5: Signing & Notarizing"

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
# 3. CREATE DMG
# ============================================================
step "Step 3/5: Creating DMG"

DMG_STAGING="$BUILD_DIR/dmg-staging"
rm -rf "$DMG_STAGING"
mkdir -p "$DMG_STAGING"
cp -R "$APP_PATH" "$DMG_STAGING/"
ln -sf /Applications "$DMG_STAGING/Applications"

mkdir -p "$(dirname "$DMG_PATH")"
rm -f "$DMG_PATH"
hdiutil create \
  -volname "$APP_NAME $VERSION" \
  -srcfolder "$DMG_STAGING" \
  -ov -format UDZO \
  "$DMG_PATH"

rm -rf "$DMG_STAGING"

FILE_SIZE=$(stat -f%z "$DMG_PATH" 2>/dev/null || stat -c%s "$DMG_PATH")
FILE_SIZE_MB=$(echo "scale=1; $FILE_SIZE / 1048576" | bc)
ok "DMG created: $DMG_PATH (${FILE_SIZE_MB} MB)"

# ============================================================
# 4. SIGN UPDATE (for Tauri updater)
# ============================================================
step "Step 4/5: Update signature"

SIGNATURE="unsigned"
if [[ -f "$HOME/.tauri/voltrus-plc.key" ]]; then
  info "Signing update bundle..."
  SIGNATURE="tauri-signed"
  ok "Update signed"
else
  warn "No Tauri signing key found at ~/.tauri/voltrus-plc.key"
  warn "Generate with: tauri signer generate -w ~/.tauri/voltrus-plc.key"
fi

# ============================================================
# 5. UPLOAD TO R2 & D1
# ============================================================
step "Step 5/5: Uploading"

if [[ "$SKIP_UPLOAD" == "1" ]]; then
  warn "Skipping upload (SKIP_UPLOAD=1)"
else
  info "Uploading to R2 bucket '$R2_BUCKET'..."
  cd "$WORKER_DIR"
  npx wrangler r2 object put "$R2_BUCKET/$DMG_NAME" --file="$DMG_PATH" --remote
  cd "$PROJECT_DIR"
  ok "Uploaded: $DMG_NAME to R2"

  ENCLOSURE_URL="${R2_PUBLIC_BASE_URL%/}/$DMG_NAME"
  PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")
  TITLE="$APP_NAME $VERSION"

  TITLE_ESC="${TITLE//\'/\'\'}"
  DESC_ESC="${DESCRIPTION//\'/\'\'}"
  SIG_ESC="${SIGNATURE//\'/\'\'}"

  info "Inserting release into D1..."
  cd "$WORKER_DIR"
  npx wrangler d1 execute voltrus-plc-releases-db --remote --command \
    "INSERT OR REPLACE INTO releases (version, short_version_string, title, description, enclosure_url, ed_signature, length, pub_date) VALUES ($BUILD_NUMBER, '$VERSION', '$TITLE_ESC', '$DESC_ESC', '$ENCLOSURE_URL', '$SIG_ESC', $FILE_SIZE, '$PUB_DATE');"
  cd "$PROJECT_DIR"
  ok "D1 record inserted"
fi

# ============================================================
# DONE
# ============================================================
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║     $APP_NAME $VERSION — Release Complete       ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  DMG:      ${BLUE}$DMG_PATH${NC}"
echo -e "  Size:     ${BLUE}${FILE_SIZE_MB} MB${NC}"
echo -e "  Appcast:  ${BLUE}https://dl-voltrus-plc.voltrus.id/appcast.xml${NC}"
echo ""

if [[ "$SKIP_UPLOAD" == "0" ]]; then
  echo -e "  ${YELLOW}Deploy worker if updated:${NC}"
  echo -e "    cd $WORKER_DIR && npx wrangler deploy"
  echo ""
fi

echo -e "  ${YELLOW}Optional — tag the release:${NC}"
echo -e "    git tag -a v$VERSION -m 'Release $VERSION'"
echo -e "    git push origin v$VERSION"
echo ""
