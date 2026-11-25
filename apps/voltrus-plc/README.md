# Voltrus PLC

Native macOS IEC 61131-3 PLC development tool. Write Structured Text (and FBD /
LD / SFC), compile, run on the built-in softPLC, and deploy to real targets —
ESP32-S3 or Linux runtimes.

Part of the [MacTools](../../README.md) monorepo. See [ROADMAP.md](./ROADMAP.md)
for the product vision and feature status.

## Develop

```bash
# from monorepo root
bun install

# run the IDE
cd apps/voltrus-plc && bun run build:wasm && bunx tauri dev
```

- Frontend: React 19 + TypeScript + Vite (dev server on port **1440**)
- Backend: Rust (Tauri v2), shared crates in `../../crates/` (`mt-stc`
  compiler, `mt-plc-runtime` VM, `mt-grpc` Linux runtime)

## Test

```bash
cd apps/voltrus-plc
bun run test        # Vitest unit tests (store + transpilers)
bun run test:e2e      # Playwright smoke tests (plain Chromium, port 1440)

# from monorepo root
cargo test -p mt-stc -p mt-plc-runtime -p mt-plc-types -p mt-grpc
cargo check -p voltrus_plc
```

## Siemens S7: online and import

Talks classic S7comm over ISO-on-TCP, port 102 (desktop app). The CPU appears under **Devices** in the left rail:

- **Online & diagnostics** (a full editor tab): connection and *Accessible devices* (scans a subnet up to a /22 for hosts on port 102 and reads each identity); general CPU data (module, order number, firmware, serial); the diagnostic buffer with event details; the blocks in the CPU; and the operating mode, where RUN and STOP each ask for confirmation.
- **Watch tables** (`.s7watch`, saved with the project, as many as you like): rows take an absolute address (`MW10`, `DB1.DBD4`, `Q0.1`, `T5`) or a tag name from the project's tag tables. *Monitor all* reads them in IEC display types. *Modify now* writes the ticked values once; *Modify permanently* writes them on every monitor cycle. That is not a force: the program can overwrite the values between writes, and S7-1200/1500 accept no force jobs over PUT/GET. 64-bit types are read and written as two DWords.
- **PLC tags and DBs** (`.s7tags`): imported tag tables and DB layouts as tables, with monitoring and *Add to watch table*. DB members get addresses once the DB number is entered.
- Online mode shows in the window: *Go online* / *Go offline* in the toolbar, an orange line under the title bar and an "Online" pill with the CPU's mode, and status marks in the device tree.

This works with S7-300/400. S7-1200/1500 work only with PUT/GET allowed and an access level that permits it. Optimized DBs have no addresses and cannot be reached. Program download to S7-1200/1500 is not offered.

**Import** (toolbar):
- PLC tag tables (XML export or `.xlsx`) become a `PROGRAM <Table>_Tags` of located variables and a device tag table.
- DB/UDT sources (`.db`, `.udt`) become STRUCT types plus a `PROGRAM <Source>_Data` that declares each DB, with standard-access offsets computed, and a device tag table per DB.
- SCL sources (`.scl`) are converted to ST.

Tags with absolute addresses, and members of non-optimized DBs once you enter the DB number, can go into a new watch table straight away. What does not convert is listed in the dialog and the log.

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
