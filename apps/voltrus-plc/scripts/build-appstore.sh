#!/bin/bash
set -euo pipefail

# ============================================================
# Voltrus PLC — App Store Build via Xcode Wrapper
# ============================================================
# Creates a minimal Xcode project that embeds the Tauri-built
# binary for App Store submission.
#
# Usage: ./scripts/build-appstore.sh
#
# Prerequisites:
#   1. Apple Distribution certificate installed
#   2. App registered in App Store Connect
#   3. pnpm tauri build already completed (creates release binary)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
WRAPPER_DIR="$PROJECT_DIR/xcode-wrapper"
APP_NAME="Voltrus PLC"
BUNDLE_ID="com.joyodigitama.voltrus-plc"
TAURI_BINARY="$MONOREPO_DIR/target/release/voltrus_plc"
BUILD_DIR="$PROJECT_DIR/build-appstore"

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
BUILD_NUMBER="${BUILD_NUMBER:-3}"

# ============================================================
# 1. Verify Tauri binary exists
# ============================================================
step "Step 1/5: Verifying Tauri binary"

if [[ ! -f "$TAURI_BINARY" ]]; then
  err "Tauri binary not found at $TAURI_BINARY"
  err "Run 'pnpm tauri build' first"
  exit 1
fi
ok "Binary found: $TAURI_BINARY"

# ============================================================
# 2. Build .app bundle manually
# ============================================================
step "Step 2/5: Creating App Store .app bundle"

APP_BUNDLE="$BUILD_DIR/$APP_NAME.app"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

# Create .app bundle structure
mkdir -p "$APP_BUNDLE/Contents/MacOS"
mkdir -p "$APP_BUNDLE/Contents/Resources"

# Embed provisioning profile if available
PROVISION_PROFILE=""
for candidate in \
  "$PROJECT_DIR/provisioning/com.joyodigitama.voltrus-plc.provisionprofile" \
  "$PROJECT_DIR/provisioning/voltrus_plc.provisionprofile" \
  "$HOME/Downloads"/*.provisionprofile \
  "$HOME/Library/MobileDevice/Provisioning Profiles"/*.mobileprovision; do
  if [[ -f "$candidate" ]]; then
    PROVISION_PROFILE="$candidate"
    break
  fi
done

if [[ -n "$PROVISION_PROFILE" ]]; then
  info "Embedding provisioning profile: $(basename "$PROVISION_PROFILE")"
  cp "$PROVISION_PROFILE" "$APP_BUNDLE/Contents/embedded.provisionprofile"
  ok "Provisioning profile embedded"
else
  warn "No provisioning profile found. Place one at:"
  warn "  $PROJECT_DIR/provisioning/com.joyodigitama.voltrus-plc.provisionprofile"
  warn "  (or in ~/Downloads/)"
fi

# Copy binary
cp "$TAURI_BINARY" "$APP_BUNDLE/Contents/MacOS/voltrus_plc"
chmod +x "$APP_BUNDLE/Contents/MacOS/voltrus_plc"

# Copy Tauri assets from existing bundle
TAURI_APP="$MONOREPO_DIR/target/release/bundle/macos/$APP_NAME.app"
if [[ -d "$TAURI_APP" ]]; then
  if [[ -d "$TAURI_APP/Contents/Resources" ]]; then
    cp -R "$TAURI_APP/Contents/Resources/"* "$APP_BUNDLE/Contents/Resources/" 2>/dev/null || true
  fi
  ok "Copied resources from Tauri bundle"
fi

# Copy icon.icns directly from source
ICONS_DIR="$PROJECT_DIR/src-tauri/icons"
if [[ -f "$ICONS_DIR/icon.icns" ]]; then
  cp "$ICONS_DIR/icon.icns" "$APP_BUNDLE/Contents/Resources/icon.icns"
fi
ok "Icon copied"

# Create Info.plist for App Store
cat > "$APP_BUNDLE/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>voltrus_plc</string>
    <key>CFBundleIconFile</key>
    <string>icon</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>Voltrus PLC</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${BUILD_NUMBER}</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHumanReadableCopyright</key>
    <string>Copyright © 2026 JoyoDigitama. All rights reserved.</string>
</dict>
</plist>
PLIST

# Get Team ID from the Apple Distribution cert
TEAM_ID=$(security find-identity -v -p codesigning 2>/dev/null \
  | grep "Apple Distribution: Dzulfikar" \
  | sed -n 's/.*(\([^)]*\)).*/\1/p' \
  | head -1)

# Create entitlements for App Store
cat > "$BUILD_DIR/entitlements-appstore.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.application-identifier</key>
    <string>${TEAM_ID}.${BUNDLE_ID}</string>
    <key>com.apple.developer.team-identifier</key>
    <string>${TEAM_ID}</string>
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.network.server</key>
    <true/>
    <key>com.apple.security.device.serial</key>
    <true/>
    <key>com.apple.security.files.user-selected.read-only</key>
    <true/>
</dict>
</plist>
PLIST

ok "App bundle created: $APP_BUNDLE"

# ============================================================
# 3. Code sign for App Store
# ============================================================
step "Step 3/5: Code Signing (App Store)"

DIST_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
  | awk '/Apple Distribution: Dzulfikar/ { print $2; exit }')

if [[ -z "$DIST_IDENTITY" ]]; then
  DIST_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
    | awk '/3rd Party Mac Developer Application/ { print $2; exit }')
fi

if [[ -z "$DIST_IDENTITY" ]]; then
  err "Apple Distribution certificate not found."
  err "Install it from developer.apple.com → Certificates, Identifiers & Profiles"
  exit 1
fi

# Remove quarantine attributes
info "Stripping quarantine attributes..."
xattr -cr "$APP_BUNDLE" 2>/dev/null || true

info "Signing with: $(security find-identity -v -p codesigning 2>/dev/null | grep "$DIST_IDENTITY" | head -1)"
codesign --deep --force --verify --verbose \
  --sign "$DIST_IDENTITY" \
  --identifier "$BUNDLE_ID" \
  --entitlements "$BUILD_DIR/entitlements-appstore.plist" \
  --options runtime \
  "$APP_BUNDLE"
ok "Signed for App Store"

# ============================================================
# 4. Create .pkg for App Store upload
# ============================================================
step "Step 4/5: Creating PKG"

INSTALLER_IDENTITY=$(security find-identity -v 2>/dev/null \
  | awk '/3rd Party Mac Developer Installer/ { print $2; exit }')

if [[ -z "$INSTALLER_IDENTITY" ]]; then
  INSTALLER_IDENTITY=$(security find-identity -v 2>/dev/null \
    | awk '/Developer ID Installer/ { print $2; exit }')
fi

PKG_PATH="$BUILD_DIR/VoltrusPLC-${VERSION}.pkg"

if [[ -n "$INSTALLER_IDENTITY" ]]; then
  info "Creating signed PKG via productbuild..."
  productbuild --component "$APP_BUNDLE" /Applications \
    --sign "$INSTALLER_IDENTITY" \
    "$PKG_PATH" 2>&1
else
  warn "Installer identity not found, creating unsigned PKG via productbuild"
  productbuild --component "$APP_BUNDLE" /Applications \
    "$PKG_PATH" 2>&1
fi

if [[ -f "$PKG_PATH" ]]; then
  FILE_SIZE=$(stat -f%z "$PKG_PATH" 2>/dev/null || stat -c%s "$PKG_PATH")
  FILE_SIZE_MB=$(echo "scale=1; $FILE_SIZE / 1048576" | bc)
  ok "PKG created: $PKG_PATH (${FILE_SIZE_MB} MB)"
else
  warn "PKG creation failed. You can upload the .app directly via Transporter."
fi

# ============================================================
# 5. Upload instructions
# ============================================================
step "Step 5/5: Upload to App Store Connect"

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║   App Store Build Ready                      ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  Bundle:   ${BLUE}$APP_BUNDLE${NC}"
if [[ -f "$PKG_PATH" ]]; then
echo -e "  PKG:      ${BLUE}$PKG_PATH${NC}"
fi
echo ""
echo -e "  ${YELLOW}Upload options:${NC}"
echo ""
echo "  Option 1: Transporter app (recommended)"
echo "    1. Open Transporter from /Applications"
echo "    2. Drag the .pkg file into Transporter"
echo "    3. Click 'Deliver'"
echo ""
echo "  Option 2: xcrun altool"
if [[ -f "$PKG_PATH" ]]; then
echo "    xcrun altool --upload-app --type macos --file \"$PKG_PATH\" \\"
echo "      --apiKey YOUR_KEY --apiIssuer YOUR_ISSUER"
fi
echo ""
echo -e "  ${YELLOW}App Store Connect checklist:${NC}"
echo "    1. App name: Voltrus PLC"
echo "    2. Bundle ID: $BUNDLE_ID"
echo "    3. Price: \$29.99"
echo "    4. Category: Developer Tools"
echo "    5. Screenshots: 1280x800 or 1440x900"
echo "    6. Privacy policy URL: https://voltrus.id/voltrus-plc/privacy.html"
echo "    7. Support URL: https://voltrus.id/voltrus-plc/support.html"
echo "    8. Description: IEC 61131-3 Structured Text editor with softPLC runtime for macOS."
echo ""
