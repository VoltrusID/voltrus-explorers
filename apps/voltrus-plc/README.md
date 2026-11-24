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

## Siemens S7: online and import

**Online tab** (desktop app): talks classic S7comm over ISO-on-TCP, port 102.

- *Accessible devices* scans a subnet (up to a /22) for hosts answering on port 102, and reads each one's identity.
- *Go online* shows the CPU's module, order number and firmware, and its operating mode. RUN and STOP each ask for confirmation.
- The watch table monitors absolute addresses (`MW10`, `DB1.DBD4`, `Q0.1`, `T5`) in IEC display types and writes modify values. 64-bit types are read and written as two DWords.
- It also reads the diagnostic buffer and the block list.

This works with S7-300/400. S7-1200/1500 work only with PUT/GET allowed and an access level that permits it. Optimized DBs have no addresses and cannot be reached. Program download to S7-1200/1500 is not offered.

**Import** (toolbar):
- PLC tag tables (XML export or `.xlsx`) become a `PROGRAM <Table>_Tags` of located variables.
- DB/UDT sources (`.db`, `.udt`) become STRUCT types plus a `PROGRAM <Source>_Data` that declares each DB, with standard-access offsets computed.
- SCL sources (`.scl`) are converted to ST.

Tags with absolute addresses, and members of non-optimized DBs once you enter the DB number, can be added to the Online watch table by name. What does not convert is listed in the dialog and the log.

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
