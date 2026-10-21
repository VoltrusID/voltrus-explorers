# Voltrus Explorers

Open-source industrial protocol tools for macOS, built with Tauri v2 (Rust + React).

| App | What it does | Get it |
| --- | --- | --- |
| **BACnet Explorer** | Discover BACnet/IP devices, browse objects, read/write properties, COV, trend logs, schedules, device simulator | [Mac App Store](https://apps.apple.com/app/id6789600440) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **Modbus Poll** | Modbus TCP/RTU master and slave simulator, register templates, trends, CSV logs (macOS + iOS) | [App Store](https://apps.apple.com/app/id6767509445) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **DNP3 Explorer** | DNP3 master (connect to outstations, poll and read points, class scans, device controls) and outstation simulator with device templates | Mac App Store · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **S7 Explorer** | Connect to S7-compatible PLCs (S7-300/400/1200/1500) over Ethernet: read and write tags, diagnostics, block manager, trends, S7 simulator | Mac App Store (coming soon) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **BLE Inspector** | Scan Bluetooth Low Energy devices, connect, browse GATT services and characteristics, read values. Pro adds writing values, notification subscriptions and log export | Mac App Store (soon) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |

The App Store versions are the same code, signed and kept up to date automatically; Pro features there are a
one-time in-app purchase, which is the no-hassle way to get them and supports development. Builds from source and the
GitHub Releases (notarized DMGs, no auto-update) have every feature unlocked.

## Free and Pro (App Store builds)

The App Store builds of DNP3 Explorer are free to download; Pro is a one-time in-app purchase. People who bought the
app while it was paid (before 1.3.0) keep every feature. Builds from source and the GitHub Releases have everything.

| DNP3 Explorer | |
| --- | --- |
| Free | Connect to an outstation, poll points on an interval, read analog, binary and counter values, write point values, communication log with CSV export |
| Pro | Outstation simulator with an event buffer, device templates with live simulated values, class 1, 2 and 3 event scans, device controls (time sync, cold and warm restart, freeze counters), live trends |

### S7 Explorer

| | |
| --- | --- |
| Free | Connect to one PLC (S7-300/400/1200/1500, ISO-on-TCP port 102), read DB, I, Q, M, T and C addresses into a polled tag table, add, edit and remove tags, CPU info, PLC mode and diagnostic buffer, communication log with CSV export |
| Pro | Several PLC connections open at once, write values to the PLC, live trends, block manager (list, save to file, delete), built-in S7 simulator server |

Version 1.4.0 is the first free release.

S7 Explorer is an independent tool. Siemens, SIMATIC, S7-300, S7-400, S7-1200, S7-1500 and TIA Portal are trademarks of
Siemens AG; it is not affiliated with or endorsed by Siemens.

## Build from source

Requirements: macOS 13+, Xcode command line tools, [Rust](https://rustup.rs), [Bun](https://bun.sh).

```bash
bun install
bun run dev:bacnet     # or dev:modbus, dev:dnp3, dev:s7, dev:ble
```

Universal release build (unsigned):

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/build-macos.sh bacnet-explorer unsigned   # -> build/bacnet-explorer/unsigned/
```

Tests: `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-s7 -p mt-ble -p mt-store` and `bun run --cwd apps/modbus-poll test`.

## Layout

```
apps/bacnet-explorer   Tauri app (src/ React UI, src-tauri/ Rust commands), also builds for iOS (scripts/build-ios.sh)
apps/modbus-poll       Tauri app, also builds for iOS (scripts/build-ios.sh)
apps/dnp3-explorer     Tauri app (macOS)
apps/s7-explorer       Tauri app (macOS)
apps/ble-inspector     Tauri app (Bluetooth LE scanner and GATT inspector)
crates/mt-protocol     shared protocol client traits
crates/mt-bacnet       BACnet/IP client, discovery, COV, trends, schedules
crates/mt-modbus       Modbus TCP/RTU client, server, simulator
crates/mt-ble          Bluetooth LE scan, connect, GATT read/write/notify (btleplug)
crates/mt-dnp3         DNP3 master, outstation server, simulation
crates/mt-s7           S7 (ISO-on-TCP) client and simulator server
crates/mt-store        Pro entitlement for App Store builds (StoreKit purchase, receipt check)
packages/mt-ui         shared React components and design tokens
scripts/               macOS/iOS build, signing and App Store Connect helpers
```

Maintainers: signed releases run in GitHub Actions, see [docs/release.md](docs/release.md).

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). "Voltrus", "BACnet Explorer – Voltrus" and the app icons are trademarks of
JoyoDigitama and are not covered by the license: forks must use their own name and icon.
Contributions require agreeing to the [contributor terms](CONTRIBUTING.md).
