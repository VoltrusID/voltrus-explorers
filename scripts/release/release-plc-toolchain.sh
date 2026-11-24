#!/usr/bin/env bash
# Build and publish the free Voltrus PLC toolchain: the `voltrus-plc` CLI
# (crates/mt-plc-cli) and `voltrus-lsp` (crates/voltrus-lsp), for 5 platforms.
#
# Output: one archive per platform with both binaries at the archive root,
# uploaded to R2 bucket voltrus-plc-releases under toolchain/<version>/, plus
# toolchain/latest.json, which the VS Code extension and the landing page read:
#   https://dl-voltrus-plc.voltrus.id/releases/toolchain/latest.json
#
# Usage:
#   scripts/release/release-plc-toolchain.sh 0.1.0            # build + upload
#   SKIP_UPLOAD=1 scripts/release/release-plc-toolchain.sh 0.1.0
#   PLATFORMS="linux-x86_64" SKIP_UPLOAD=1 scripts/release/release-plc-toolchain.sh 0.1.0
#
# Needs: cargo-zigbuild + zig (Linux and macOS targets), cargo-xwin (Windows),
# rustup targets x86_64/aarch64-unknown-linux-musl, aarch64/x86_64-apple-darwin,
# x86_64-pc-windows-msvc. Builds into ~/.cache/voltrus-plc-toolchain-target
# because mbx garbage-collects the repo's target/.
set -euo pipefail

VERSION="${1:?usage: $0 <version>}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/dist/plc-toolchain/$VERSION"
export VOLTRUS_TOOLCHAIN_VERSION="$VERSION"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/voltrus-plc-toolchain-target}"
BUCKET="voltrus-plc-releases"
BASE_URL="https://dl-voltrus-plc.voltrus.id/releases/toolchain/$VERSION"
export CLOUDFLARE_ACCOUNT_ID="${CLOUDFLARE_ACCOUNT_ID:-b7e6f8457bd39fa37a7eea5249799754}"

ALL_PLATFORMS="linux-x86_64 linux-aarch64 macos-aarch64 macos-x86_64 windows-x86_64"
PLATFORMS="${PLATFORMS:-$ALL_PLATFORMS}"

triple() {
  case "$1" in
    linux-x86_64)   echo x86_64-unknown-linux-musl ;;
    linux-aarch64)  echo aarch64-unknown-linux-musl ;;
    macos-aarch64)  echo aarch64-apple-darwin ;;
    macos-x86_64)   echo x86_64-apple-darwin ;;
    windows-x86_64) echo x86_64-pc-windows-msvc ;;
    *) echo "unknown platform $1" >&2; exit 1 ;;
  esac
}

rm -rf "$OUT" && mkdir -p "$OUT"
cd "$ROOT"

for p in $PLATFORMS; do
  t="$(triple "$p")"
  echo "==> $p ($t)"
  if [[ "$p" == windows-* ]]; then
    cargo xwin build --release --target "$t" -p mt-plc-cli -p voltrus-lsp
    ext=".exe"
  else
    cargo zigbuild --release --target "$t" -p mt-plc-cli -p voltrus-lsp
    ext=""
  fi
  stage="$(mktemp -d)"
  cp "$CARGO_TARGET_DIR/$t/release/voltrus-plc$ext" "$CARGO_TARGET_DIR/$t/release/voltrus-lsp$ext" "$stage/"
  if [[ "$p" == windows-* ]]; then
    archive="voltrus-toolchain-$VERSION-$p.zip"
    (cd "$stage" && zip -q -9 "$OUT/$archive" voltrus-plc.exe voltrus-lsp.exe)
  else
    archive="voltrus-toolchain-$VERSION-$p.tar.gz"
    tar -C "$stage" -czf "$OUT/$archive" voltrus-plc voltrus-lsp
  fi
  rm -rf "$stage"
done

# Smoke-test the binary that runs on this machine.
host="linux-$(uname -m | sed 's/arm64/aarch64/')"
if [[ -f "$OUT/voltrus-toolchain-$VERSION-$host.tar.gz" ]]; then
  smoke="$(mktemp -d)"
  tar -C "$smoke" -xzf "$OUT/voltrus-toolchain-$VERSION-$host.tar.gz"
  "$smoke/voltrus-plc" --version
  "$smoke/voltrus-plc" test "$ROOT/apps/voltrus-plc/samples" >/dev/null && echo "smoke: sample tests pass"
  rm -rf "$smoke"
fi

# latest.json (the format apps/voltrus-plc-vscode/src/toolchain.ts parses).
node - "$OUT" "$VERSION" "$BASE_URL" <<'EOF'
const fs = require("fs"), path = require("path"), crypto = require("crypto");
const [out, version, base] = process.argv.slice(2);
const files = {};
for (const f of fs.readdirSync(out).filter((f) => /^voltrus-toolchain-/.test(f)).sort()) {
  const platform = f.replace(`voltrus-toolchain-${version}-`, "").replace(/\.(tar\.gz|zip)$/, "");
  const sha256 = crypto.createHash("sha256").update(fs.readFileSync(path.join(out, f))).digest("hex");
  files[platform] = { url: `${base}/${f}`, sha256 };
}
fs.writeFileSync(path.join(out, "latest.json"), JSON.stringify({ version, files }, null, 2) + "\n");
EOF
cat "$OUT/latest.json"

if [[ -n "${SKIP_UPLOAD:-}" ]]; then
  echo "SKIP_UPLOAD set; artifacts in $OUT"
  exit 0
fi

for f in "$OUT"/voltrus-toolchain-*; do
  npx wrangler r2 object put "$BUCKET/toolchain/$VERSION/$(basename "$f")" --file="$f" --remote
done
npx wrangler r2 object put "$BUCKET/toolchain/latest.json" --file="$OUT/latest.json" \
  --content-type=application/json --cache-control="public, max-age=300" --remote
echo "Published: https://dl-voltrus-plc.voltrus.id/releases/toolchain/latest.json"
