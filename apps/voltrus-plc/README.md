# Voltrus PLC

Native macOS IEC 61131-3 PLC development tool. Write Structured Text (and FBD /
LD / SFC), compile, run on the built-in softPLC, and deploy to real targets —
ESP32-S3 or Linux runtimes.

Part of the [MacTools](../../README.md) monorepo. See [ROADMAP.md](./ROADMAP.md)
for the product vision and feature status.

## Develop

```bash
# from monorepo root
pnpm install

# run the IDE (macOS)
pnpm --filter voltrus-plc tauri dev
```

- Frontend: React 19 + TypeScript + Vite (dev server on port **1440**)
- Backend: Rust (Tauri v2), shared crates in `../../crates/` (`mt-stc`
  compiler, `mt-plc-runtime` VM, `mt-grpc` Linux runtime)

## Test

```bash
cd apps/voltrus-plc
pnpm test          # Vitest unit tests (store + transpilers)
pnpm test:e2e      # Playwright smoke tests (plain Chromium, port 1440)

# from monorepo root
cargo test -p mt-stc -p mt-plc-runtime -p mt-plc-types -p mt-grpc
cargo check -p voltrus_plc
```

## Release

```bash
cd apps/voltrus-plc

./scripts/release-local.sh                    # macOS ARM + Intel + Windows → R2 + D1
./scripts/release-local.sh --target mac       # both macOS archs only
./scripts/release-local.sh --bump             # patch-bump version first
./scripts/release-local.sh --bump=minor
SKIP_UPLOAD=1 ./scripts/release-local.sh      # build only
SKIP_BUILD=1 ./scripts/release-local.sh       # upload existing builds
./scripts/build-dmg.sh                        # macOS DMG only
./scripts/build-appstore.sh                   # .pkg for App Store Connect
./scripts/build-linux.sh                      # Linux build (Docker)
```

Downloads are served by the Cloudflare Worker `apps/voltrus-plc/cloudflare-worker` in the private VoltrusID/voltrus-dl-workers repo
(R2 + D1, `dl-voltrus-plc.voltrus.id`), which also handles Polar.sh license
activation/validation/deactivation and the Tauri v2 auto-update endpoint.

Worker secrets (`wrangler secret put`): `POLAR_ACCESS_TOKEN`, `POLAR_ORG_ID`.
Optional `POLAR_BENEFIT_ID` var scopes activation to this product only.

## Targets

| Target | Transport | Deploy |
|--------|-----------|--------|
| Local softPLC | embedded VM | Build → Run |
| ESP32-S3 | HTTP + WebSocket | Build & Deploy (`http://<ip>/api/deploy`) |
| Linux runtime (RevPi, RPi, IPC) | gRPC | Build & Deploy (`http://<ip>:50051`, bincode payload) |
