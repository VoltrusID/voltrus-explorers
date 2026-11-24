# Voltrus HMI

Drag-and-drop HMI (human-machine interface) designer for Voltrus PLCs. Build
operator dashboards with widgets bound to PLC variables, then export them as a
**single self-contained HTML file** that runs on any browser — no server-side
runtime, no install.

## What it does

- **Design**: 6 widget types (gauge, button, indicator, text, slider, chart),
  multi-screen projects, templates, undo/redo, copy/paste, z-order, zoom + snap.
- **Live binding**: connect to a PLC (ESP32 soft-PLC or RPi5 agent), browse
  variables, drag them onto widgets — previews show live values while you design.
- **Export**: one HTML file with the WS runtime, alarms, PIN access control,
  i18n, and kiosk mode baked in.
- **Deploy**: push the export straight to an ESP32 (`/api/deploy`, firmware
  `plc_ws_server.c`) or an RPi5 kiosk (`mt-hmi-agent`, port 8080).

## Architecture

```
Designer (this app, Tauri v2)
  └─ export_hmi → standalone .html
        └─ ws://{plc}/ws/plc  ← binary VAR_UPDATE frames (firmware plc_flatbuf)
           /api/variables      ← HTTP snapshot
           /api/deploy         ← push updated HTML to device
```

Wire type codes (firmware `plc_value_type_t`):
`BOOL=0 BYTE=1 INT=2 DINT=3 REAL=4 TIME=5 STRING=6 WORD=7 DWORD=8`

## Development

```bash
pnpm --filter voltrus-hmi tauri dev   # run the designer
pnpm --filter voltrus-hmi test        # vitest (store, parser, crypto)
cargo check -p voltrus_hmi            # Rust backend
cargo test -p voltrus_hmi             # export-escaping tests
```

## Release

```bash
cd apps/voltrus-hmi
./scripts/release-local.sh              # all platforms → R2 + D1
./scripts/release-local.sh --target aarch64
NOTARIZE=1 ./scripts/release-local.sh   # production
./scripts/build-appstore.sh             # App Store .pkg
```

`scripts/release.sh` is deprecated — use `release-local.sh`.

Update feed: `https://dl-voltrus-hmi.voltrus.id/update/{{target}}/{{arch}}/{{current_version}}`
(Cloudflare Worker in `cloudflare-worker/`).

## Security notes

- The exported HTML embeds stretched PIN hashes (SHA-256 × 5000, salted) and
  verifies them client-side with a pure-JS SHA-256 so it also works over plain
  HTTP. This is tamper-evident, not tamper-proof — anyone with file access can
  strip it. Enforce critical writes at the PLC/gateway, not in the panel.
- All user strings (labels, alarm messages, translations, hosts, colors, ids)
  are escaped/validated before they reach the generated HTML — covered by
  `cargo test -p voltrus_hmi`.

## Project files

`.vhmi` files are JSON (`version: "0.3"`): screens, plcIp, scadaUrl, alarms,
accessConfig, translations, kioskMode, tags, and RPi5 deploy settings.
