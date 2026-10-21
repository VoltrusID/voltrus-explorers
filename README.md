# Voltrus Explorers

Open-source industrial protocol tools for macOS, built with Tauri v2 (Rust + React).

| App | What it does | Get it |
| --- | --- | --- |
| **BACnet Explorer** | Discover BACnet/IP devices, browse objects, read/write properties, COV, trend logs, schedules, device simulator | [Mac App Store](https://apps.apple.com/app/id6789600440) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **Modbus Poll** | Modbus TCP/RTU master and slave simulator, register templates, trends, CSV logs (macOS + iOS) | [App Store](https://apps.apple.com/app/id6767509445) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **DNP3 Explorer** | DNP3 master (connect to outstations, poll and read points, class scans, device controls) and outstation simulator with device templates | Mac App Store · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **S7 Explorer** | Connect to S7-compatible PLCs (S7-300/400/1200/1500) over Ethernet: read and write tags, diagnostics, block manager, trends, S7 simulator | Mac App Store (coming soon) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **BLE Inspector** | Scan Bluetooth Low Energy devices, connect, browse GATT services and characteristics, read and write values, notifications. Pro adds several notifications and devices at once and log export | Mac App Store (soon) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **EtherNet/IP Explorer** | Discover EtherNet/IP devices, browse CIP tags and objects, read tags and attributes, EDS import, implicit I/O, packet log, PLC simulator | Mac App Store · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **Serial Monitor** | RS232/RS485 serial terminal: open a port, send and receive in HEX or ASCII, line view with live traffic table, frame simulation. Pro adds log export, send macros, repeat-send and saved profiles | [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **NATS Explorer** | NATS client (subscribe, publish, request/reply) with an embedded server, JetStream, KV and object stores, protobuf decoding | [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **MQTT Explorer** | MQTT client with topic table, tree and history, plus an embedded test broker | [Build from source](#build-from-source) |
| **OPC UA Explorer** | Run a local OPC UA server with device templates, or connect to any OPC UA server to browse the address space, read values, subscribe, trend and write | Mac App Store: not released yet · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |

The App Store versions are the same code, signed and kept up to date automatically; Pro features there are a
one-time in-app purchase, which is the no-hassle way to get them and supports development. Builds from source and the
GitHub Releases (notarized DMGs, no auto-update) have every feature unlocked.

## Free and Pro (App Store builds)

The App Store builds of DNP3 Explorer and OPC UA Explorer are free to download; Pro is a one-time in-app purchase.
People who bought DNP3 Explorer while it was paid (before 1.3.0) keep every feature. Builds from source and the GitHub
Releases have everything.

Modbus Poll and BACnet Explorer are still paid on the App Store. Once they go free, the split below applies (it only
takes effect when `free_since` is set in their `store.json`).

| Modbus Poll | |
| --- | --- |
| Free | Modbus TCP master with one connection, read up to 20 values across any number of blocks, all data types, byte orders and addressing, communication log with CSV export, traffic view (last 500 frames), one saved connection profile |
| Pro | Unlimited values, writes (coils and registers), RTU over serial, bus scan and auto-detect, network scan, slave simulator with templates, live trends, traffic CSV export and copy-all, block CSV import and export, unlimited saved profiles |

| BACnet Explorer | |
| --- | --- |
| Free | Who-Is discovery, connect to a device, browse objects, read any property, communication log with CSV export |
| Pro | Write properties (all priorities, relinquish), live trends and COV, trend log and schedule viewers, alarms and events with acknowledge, device simulator |

| DNP3 Explorer | |
| --- | --- |
| Free | Connect to one outstation, integrity poll of an editable point list (up to 10 points), read analog, binary and counter values, 1 saved connection profile, communication log with CSV export |
| Pro | Unlimited points in the poll list, controls (CROB and analog output), class 1, 2 and 3 event scans, device controls (time sync, cold and warm restart, freeze counters), outstation simulator with an event buffer, device templates with live simulated values, live trends, unlimited saved profiles |

### S7 Explorer

| | |
| --- | --- |
| Free | Connect to one PLC (S7-300/400/1200/1500, ISO-on-TCP port 102), read DB, I, Q, M, T and C addresses into a polled tag table (up to 10 live tags), add, edit and remove tags, CPU info, PLC mode and diagnostic buffer, communication log with CSV export |
| Pro | Unlimited live tags, several PLC connections open at once, write values to the PLC, live trends, block manager (list, save to file, delete), built-in S7 simulator server |

Version 1.4.0 is the first free release.

S7 Explorer is an independent tool. Siemens, SIMATIC, S7-300, S7-400, S7-1200, S7-1500 and TIA Portal are trademarks of
Siemens AG; it is not affiliated with or endorsed by Siemens.
| EtherNet/IP Explorer | |
| --- | --- |
| Free | Discover devices and read their identity, connect, browse the full tag list, live polling of 10 tags (any other tag can be read once on demand), hex view, CIP object tree and browser (Get Attribute Single and All, list instances), EDS import, packet log viewer, communication log with CSV export |
| Pro | Unlimited live tags, write tags, trends for tag values, Set Attribute and any other CIP service, implicit I/O (ForwardOpen, cyclic data), custom response decoders, packet log CSV export, adapter mode (built-in EtherNet/IP server and PLC simulator with templates) |

EtherNet/IP Explorer buyers from before 1.5.0 keep Pro without purchasing anything.
The App Store build of Serial Monitor is free to download too; Pro is a one-time in-app purchase. Builds from source
and the GitHub Releases have everything.

| Serial Monitor | |
| --- | --- |
| Free | Open one port (baud rate, data and stop bits, parity, read timeout), send and receive in HEX or ASCII, traffic table, the last 200 communication log lines with single-line copy, one saved port profile, Modbus RTU frame simulation templates |
| Pro | Copy-all and CSV export of the communication log, unlimited log scrollback, send macros, repeat-send at a fixed interval, unlimited saved port profiles |

The virtual serial port (PTY) is available in source builds and GitHub Releases only; the App Sandbox of the App Store build does not allow creating pseudo-terminals.

### NATS Explorer

Free to download on the App Store; Pro is a one-time in-app purchase. People who bought the app while it was paid
(before 1.2.0) keep every feature. NATS Explorer was never released as paid, so this applies to nobody yet.

| | |
| --- | --- |
| Free | Embedded NATS server, connect with user/password, token, NKey, JWT, credentials file or TLS, subscribe with wildcards and queue groups, publish with headers, request/reply, payload viewer (JSON, MessagePack, hex), read-only JetStream stream list with message and consumer counts, communication log export |
| Pro | JetStream stream and consumer create, delete and purge and stored message fetch, KV store browser with live watch, object store browser with upload and download, `$SYS` server monitor, protobuf schema decoding, message CSV export |

### BLE Inspector

Free to download on the App Store; Pro is a one-time in-app purchase. Version 1.2.0 is the first release, and it is free.

| | |
| --- | --- |
| Free | Scan for nearby devices, connect to one device, browse services and characteristics, read values, write values, one notification subscription at a time, communication log with single-line copy |
| Pro | Several notification subscriptions at once, several connected devices at once, communication log CSV export and copy all |

### MQTT Explorer

| | |
| --- | --- |
| Free | Connect with host, port, client ID, username and password (optionally over TLS), subscribe with QoS 0/1/2 and wildcards, publish (including retained), topic table, tree and message history, Raw, JSON, Hex and Base64 payload views, one saved connection profile, communication log with CSV export |
| Pro | Embedded local broker (TCP, WebSocket, TLS, username/password), more than one saved connection profile, custom CA, client certificates (mTLS) and Last Will on client connections, message history export to CSV |

Version 1.2.0 is the first release, and it is free.
| OPC UA Explorer | |
| --- | --- |
| Free | Connect (anonymous or username/password, security policies and modes), endpoint and server discovery, browse the address space, node attributes and references, namespaces, read values on demand, live updates for up to 10 monitored items, communication log with CSV export, start and stop the built-in server |
| Pro | Unlimited live monitored items and the subscriptions panel (sampling, deadband, queue), live trend charts, writing values (client and server), certificate management (trust, reject, regenerate), device templates for the built-in server |

## Build from source

Requirements: macOS 13+, Xcode command line tools, [Rust](https://rustup.rs), [Bun](https://bun.sh).

```bash
bun install
bun run dev:bacnet     # or dev:modbus, dev:dnp3, dev:s7, dev:ble, dev:enip, dev:serial, dev:nats, dev:mqtt, dev:opcua
```

Universal release build (unsigned):

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/build-macos.sh bacnet-explorer unsigned   # -> build/bacnet-explorer/unsigned/
```

Tests: `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-s7 -p mt-ble -p mt-enip -p mt-store` and `bun run --cwd apps/modbus-poll test`.
Tests: `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-store -p mt-serial` and
`bun run --cwd apps/modbus-poll test` / `bun run --cwd apps/serial-monitor test`.
Tests: `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-s7 -p mt-nats -p mt-store` and `bun run --cwd apps/modbus-poll test`.
Tests: `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-s7 -p mt-mqtt -p mt-store` and `bun run --cwd apps/modbus-poll test`.
Tests: `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-opcua -p mt-store` and `bun run --cwd apps/modbus-poll test`.

## Layout

```
apps/bacnet-explorer   Tauri app (src/ React UI, src-tauri/ Rust commands), also builds for iOS (scripts/build-ios.sh)
apps/bacnet-explorer   Tauri app (src/ React UI, src-tauri/ Rust commands)
apps/enip-explorer      Tauri app (EtherNet/IP scanner and adapter)
apps/modbus-poll       Tauri app, also builds for iOS (scripts/build-ios.sh)
apps/dnp3-explorer     Tauri app (macOS)
apps/s7-explorer       Tauri app (macOS)
apps/ble-inspector     Tauri app (Bluetooth LE scanner and GATT inspector)
apps/mqtt-explorer     Tauri app (MQTT client + embedded broker)
apps/opcua-explorer    Tauri app (macOS)
crates/mt-protocol     shared protocol client traits
crates/mt-bacnet       BACnet/IP client, discovery, COV, trends, schedules
crates/mt-enip         EtherNet/IP and CIP client, discovery, server, simulator
apps/serial-monitor    Tauri app (macOS only), serial terminal
crates/mt-protocol     shared protocol client traits
crates/mt-bacnet       BACnet/IP client, discovery, COV, trends, schedules
crates/mt-serial       RS232/RS485 port configuration, enumeration and client
apps/nats-explorer     Tauri app (macOS)
crates/mt-protocol     shared protocol client traits
crates/mt-bacnet       BACnet/IP client, discovery, COV, trends, schedules
crates/mt-nats         NATS client, embedded server, JetStream, KV/object stores, payload decoding
crates/mt-modbus       Modbus TCP/RTU client, server, simulator
crates/mt-ble          Bluetooth LE scan, connect, GATT read/write/notify (btleplug)
crates/mt-dnp3         DNP3 master, outstation server, simulation
crates/mt-s7           S7 (ISO-on-TCP) client and simulator server
crates/mt-mqtt         MQTT client (rumqttc) and embedded broker (rumqttd)
crates/mt-opcua        OPC UA client (browse, read, write, subscriptions), server and simulator, certificate store
crates/mt-store        Pro entitlement for App Store builds (StoreKit purchase, receipt check)
packages/mt-ui         shared React components and design tokens
scripts/               macOS/iOS build, signing and App Store Connect helpers
```

Maintainers: signed releases run in GitHub Actions, see [docs/release.md](docs/release.md).

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). "Voltrus", "BACnet Explorer – Voltrus" and the app icons are trademarks of
JoyoDigitama and are not covered by the license: forks must use their own name and icon.
Contributions require agreeing to the [contributor terms](CONTRIBUTING.md).
