# Voltrus Explorers

Open-source industrial protocol tools for macOS, built with Tauri v2 (Rust + React).

| App | What it does | Get it |
| --- | --- | --- |
| **BACnet Explorer** | Discover BACnet/IP devices, browse objects, read/write properties, COV, trend logs, schedules, device simulator | [Mac App Store](https://apps.apple.com/app/id6789600440) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **Modbus Poll** | Modbus TCP/RTU master and slave simulator, register templates, trends, CSV logs (macOS, iOS and Windows) | [App Store](https://apps.apple.com/app/id6767509445) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) · [Windows](https://voltrus.id/modbus-poll/windows/) |
| **DNP3 Explorer** | DNP3 master (connect to outstations, poll and read points, class scans, device controls) and outstation simulator with device templates | Mac App Store · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **S7 Explorer** | Connect to S7-compatible PLCs (S7-300/400/1200/1500) over Ethernet: read and write tags, diagnostics, block manager, trends, S7 simulator | Mac App Store (coming soon) · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **BLE Inspector** | Scan Bluetooth Low Energy devices, connect, browse GATT services and characteristics, read and write values, notifications. Pro adds notify plots, value decoding, RSSI chart, write presets and macros, several notifications and devices at once, and log export | Free download ([DMG](https://dl-ble-inspector.voltrus.id/latest) · [voltrus.id](https://voltrus.id/ble-inspector/)); Pro by licence key ([buy in the console](https://console.voltrus.id/checkout?product=ble-inspector)) |
| **EtherNet/IP Explorer** | Discover EtherNet/IP devices, browse CIP tags and objects, read tags and attributes, EDS import, implicit I/O, packet log, PLC simulator | Mac App Store · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **Serial Monitor** | RS232/RS485 serial terminal: open a port, send and receive in HEX or ASCII, line view with live traffic table, frame simulation. Pro adds log export, send macros, repeat-send and saved profiles | Free download ([DMG](https://dl-serial-monitor.voltrus.id/latest) · [voltrus.id](https://voltrus.id/serial-terminal/)); Pro by licence key ([buy in the console](https://console.voltrus.id/checkout?product=serial-terminal)) |
| **NATS Explorer** | NATS client (subscribe, publish, request/reply) with an embedded server, JetStream, KV and object stores, protobuf decoding | [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **MQTT Explorer** | MQTT client with topic table, tree and history, plus an embedded test broker | [Build from source](#build-from-source) |
| **OPC UA Explorer** | Run a local OPC UA server with device templates, or connect to any OPC UA server to browse the address space, read values, subscribe, trend and write | Mac App Store: not released yet · [GitHub Releases](https://github.com/VoltrusID/voltrus-explorers/releases) |
| **CNC Explorer** | Monitor CNC machines: MTConnect agents (device tree, live data, conditions, trends, assets) and FANUC controllers over FOCAS/Ethernet (status, axes, program, alarms, macro variables; Beta), with built-in MTConnect agent and CNC simulators | Free download ([DMG](https://dl-cnc-explorer.voltrus.id/latest) · [voltrus.id](https://voltrus.id/cnc-explorer/)); Pro by licence key, USD 39.99 one-time, one Mac ([buy in the console](https://console.voltrus.id/checkout/p/6f42df6e-6dd2-46f1-af23-72d2e074534c)) |
| **SECS/GEM Explorer** | GEM host for semiconductor and electronics equipment over HSMS (establish communications, status variables, equipment constants, reports and events, alarms, remote commands, process programs, terminal, SML message log; Beta), with a built-in equipment simulator | Free download ([DMG](https://dl-secsgem-explorer.voltrus.id/latest)); Pro by licence key, USD 39.99 one-time, one Mac ([buy in the console](https://console.voltrus.id/checkout/p/a7bfae63-8d11-4e19-bae7-f7b1b41cade4)) |

The App Store versions are the same code, signed and kept up to date automatically; Pro features there are a
one-time in-app purchase, which is the no-hassle way to get them and supports development. Builds from source and the
GitHub Releases (notarized DMGs, no auto-update) have every feature unlocked.

**Modbus Poll for Windows** (10 and 11, x64 and ARM64) is a direct download, not a Microsoft Store app: an installer
and a portable zip on the `modbus-poll-windows-v<version>` GitHub Release, with `SHA256SUMS.txt`. The files are not
code-signed, so Windows SmartScreen asks you to confirm the first run. It has the same free tier as the Mac app, and
Pro unlocks with a licence key (Info, then Enter key) bought at [voltrus.id](https://voltrus.id/modbus-poll/windows/);
the key is checked online once. Building it yourself (`cargo tauri build --features keys` on Windows) gives the same
app; a build without `--features keys` has everything unlocked, like every other source build. See
[docs/release.md](docs/release.md#windows-modbus-poll).

Serial Monitor, BLE Inspector, CNC Explorer and SECS/GEM Explorer are not on the App Store or GitHub Releases. They are free to download from
[voltrus.id](https://voltrus.id) and the [Voltrus Console](https://console.voltrus.id) (notarized DMG, no sign-in);
Pro is a licence key bought there, see [Serial Monitor](#serial-monitor), [BLE Inspector](#ble-inspector), [CNC Explorer](#cnc-explorer) and [SECS/GEM Explorer](#secsgem-explorer).

## Free and Pro (App Store and licence-key builds)

The App Store builds of DNP3 Explorer and OPC UA Explorer are free to download; Pro is a one-time in-app purchase.
People who bought DNP3 Explorer while it was paid (before 1.3.0) keep every feature. Builds from source and the GitHub
Releases have everything.

Modbus Poll and BACnet Explorer are still paid on the App Store. Once they go free, the split below applies (it only
takes effect when `free_since` is set in their `store.json`).

| Modbus Poll | |
| --- | --- |
| Free | Modbus TCP master with one connection, read up to 20 values across any number of blocks, all data types, byte orders and addressing, communication log with CSV export, traffic view (last 500 frames), one saved connection profile. A saved profile with more than 20 values loads, but only its first 20 values are polled |
| Pro | Unlimited values, writes (coils and registers), RTU over serial, bus scan and auto-detect, network scan, slave simulator with templates, live trends, traffic CSV export and copy-all, block CSV import and export, export of the tag list to Ignition, Kepware, Node-RED, RustRED, Home Assistant, Voltrus SCADA and Voltrus Gateway files (or generic CSV and JSON), unlimited saved profiles, several connections open at once (one tab each), logging of polled values to a CSV file |

| BACnet Explorer | |
| --- | --- |
| Free | Who-Is discovery, connect to one device at a time, browse objects, read any property, watch list with manual refresh, communication log with CSV export |
| Pro | Write properties (all priorities, relinquish), several devices open at once, object list CSV export, export of the object list to Ignition, Voltrus SCADA and Voltrus Gateway files (or generic CSV and JSON), auto-refresh watch list, live trends and COV, trend log and schedule viewers, alarms and events with acknowledge, device simulator |

| DNP3 Explorer | |
| --- | --- |
| Free | Connect to one outstation, integrity poll of an editable point list (up to 10 points), read analog, binary and counter values, 1 saved connection profile, communication log with CSV export |
| Pro | Unlimited points in the poll list, point list CSV import and export, controls (CROB and analog output, as direct operate or select before operate), class 1, 2 and 3 event scans, device controls (time sync, cold and warm restart, freeze counters), outstation simulator with an event buffer, device templates with live simulated values, live trends, unlimited saved profiles |

### S7 Explorer

| | |
| --- | --- |
| Free | Connect to one PLC (S7-300/400/1200/1500, ISO-on-TCP port 102), read DB, I, Q, M, T and C addresses into a polled tag table (up to 10 live tags; timers show milliseconds and counters 0 to 999, both read-only), add, edit and remove tags, CPU info, PLC mode and diagnostic buffer, communication log with CSV export |
| Pro | Unlimited live tags, several PLC connections open at once, write values to the PLC, live trends, tag list CSV import (TIA Portal PLC tag table export saved as CSV, or a simple Name,Address,Type file) and export, value logging to a CSV file, block manager (list, save to file, delete), built-in S7 simulator server |

Version 1.4.0 is the first free release.

S7 Explorer is an independent tool. Siemens, SIMATIC, S7-300, S7-400, S7-1200, S7-1500 and TIA Portal are trademarks of
Siemens AG; it is not affiliated with or endorsed by Siemens.
| EtherNet/IP Explorer | |
| --- | --- |
| Free | Discover devices and read their identity, connect, browse the full tag list, live polling of 10 tags (any other tag can be read once on demand), hex view, CIP object tree and browser (Get Attribute Single and All, list instances), EDS import, packet log viewer, communication log with CSV export, 1 saved tag watch list with CSV import |
| Pro | Unlimited live tags, more than one saved tag watch list, tag watch list CSV export, write tags, trends for tag values, Set Attribute and any other CIP service, implicit I/O (ForwardOpen, cyclic data), custom response decoders, packet log CSV export, adapter mode (built-in EtherNet/IP server and PLC simulator with templates) |

EtherNet/IP Explorer buyers from before 1.5.0 keep Pro without purchasing anything.

### CNC Explorer

Free to download as a notarized DMG from [voltrus.id/cnc-explorer](https://voltrus.id/cnc-explorer/) or the
[Voltrus Console](https://console.voltrus.id), no sign-in needed (direct link: `https://dl-cnc-explorer.voltrus.id/latest`).
Pro is a one-time licence key (USD 39.99, no subscription, one Mac) bought through Polar in the console: open Info (or any Pro
prompt) in the app, choose Buy Pro, then paste the key and press Activate. The key is kept in the macOS Keychain and
checked against the licence server at launch; without a connection Pro stays on, and it only locks when the server
says the key is no longer valid. Remove key (Info) forgets it on this device. Builds from source have everything.

| CNC Explorer | |
| --- | --- |
| Free | Connect to one MTConnect agent or one FANUC CNC, device and component tree, live values for 10 MTConnect items (any other item can be read once with Refresh), conditions (Normal, Warning, Fault), assets with raw XML, FANUC identity, status, axis positions, feed, spindle, servo load, program, alarms, macro variable, parameter, diagnosis and PMC reads, communication log with CSV export, 1 saved connection profile |
| Pro | Unlimited live MTConnect items, live trends (numeric MTConnect samples, FANUC feed, spindle, servo load and positions), MTConnect agent simulator (3-axis VMC and 2-axis lathe, SHDR adapter) and FANUC CNC simulator (31i-B and 0i-F), macro variable writes (with a confirm step), data table CSV export, unlimited saved profiles |

The FANUC connection is a **Beta**: it is tested against the built-in simulator only, not against a real controller.
Please report your controller model to support@voltrus.id.

CNC Explorer is an independent tool. FANUC and FOCAS are trademarks of FANUC CORPORATION, and MTConnect is a
registered trademark of AMT - The Association For Manufacturing Technology; it is not affiliated with or endorsed by
either.

### SECS/GEM Explorer

Free to download as a notarized DMG (direct link: `https://dl-secsgem-explorer.voltrus.id/latest`), no sign-in needed.
Pro is a one-time licence key (USD 39.99, no subscription, one Mac) bought through Polar in the console: open Info (or any Pro
prompt) in the app, choose Buy Pro, then paste the key and press Activate. The key is kept in the macOS Keychain and
checked against the licence server at launch; without a connection Pro stays on, and it only locks when the server
says the key is no longer valid. Remove key (Info) forgets it on this device. Builds from source have everything.

| SECS/GEM Explorer | |
| --- | --- |
| Free | GEM host: connect over HSMS (active or passive, device id, T3 T5 T6 T7 T8), establish communications, request online and offline, control and communication state, status variable names and values (live poll of up to 10 selected variables), equipment constant reads, alarm list, enable and live alarm reports, process program list, terminal messages (S10F3 and S10F1), HSMS message log with every message rendered as SML, communication log export, 1 saved connection profile |
| Pro | Equipment simulator (passive or active: control state switches, process state, raise and clear alarms, edit status variables and constants), live trends, equipment constant writes (with a confirm step and range errors), report wizard (define, link, enable) with a live S6F11 table, remote commands (S2F41) with typed parameters, SML composer for any message, CSV export of the message log and of events, unlimited saved profiles |

The app is a **Beta**. It uses [`mt-secsgem`](crates/mt-secsgem), an independent implementation that was cross-checked
against the open-source [secsgem](https://github.com/bparzella/secsgem) library only: it has never been tested on real
equipment or against a commercial factory host. Please report what you connect it to at support@voltrus.id.

SECS/GEM, SECS-II and HSMS are the SEMI standards E5, E30 and E37. SECS/GEM Explorer is not affiliated with, endorsed by
or certified by SEMI.

### Serial Monitor

Free to download as a notarized DMG from [voltrus.id/serial-terminal](https://voltrus.id/serial-terminal/) or the
[Voltrus Console](https://console.voltrus.id), no sign-in needed (direct link: `https://dl-serial-monitor.voltrus.id/latest`).
Pro is a one-time licence key (USD 4.99, no subscription) bought through Polar in the console: open Info (or any Pro
prompt) in the app, choose Buy Pro, then paste the key and press Activate. The key is kept in the macOS Keychain and
checked against the licence server at launch; without a connection Pro stays on, and it only locks when the server
says the key is no longer valid. Remove key (Info) forgets it on this device. Builds from source have everything.

| Serial Monitor | |
| --- | --- |
| Free | Open one port (baud rate, data and stop bits, parity, read timeout), send and receive in HEX or ASCII, traffic table, the last 200 communication log lines with single-line copy, one saved port profile, Modbus RTU frame simulation templates |
| Pro | Several ports open side by side, capture all traffic to a CSV file (timestamp, port, direction, hex, text), copy-all and CSV export of the communication log, unlimited log scrollback, send macros, repeat-send at a fixed interval, unlimited saved port profiles |

The virtual serial port (PTY, a Pro feature) is available in source builds and the direct download only; the App Sandbox of an App Store build would not allow creating pseudo-terminals.

### NATS Explorer

Free to download on the App Store; Pro is a one-time in-app purchase. People who bought the app while it was paid
(before 1.2.0) keep every feature. NATS Explorer was never released as paid, so this applies to nobody yet.

| | |
| --- | --- |
| Free | Embedded NATS server, connect with user/password, token, NKey, JWT, credentials file or TLS, subscribe with wildcards and queue groups, publish with headers, request/reply, payload viewer (JSON, MessagePack, hex), read-only JetStream stream list with message and consumer counts, one saved connection profile, communication log export |
| Pro | JetStream stream and consumer create, delete and purge and stored message fetch, KV store browser with live watch, object store browser with upload and download, `$SYS` server monitor, protobuf schema decoding, message CSV export, unlimited saved connection profiles |

### BLE Inspector

Free to download as a notarized DMG from [voltrus.id/ble-inspector](https://voltrus.id/ble-inspector/) or the
[Voltrus Console](https://console.voltrus.id), no sign-in needed (direct link: `https://dl-ble-inspector.voltrus.id/latest`).
Pro is a one-time licence key (USD 4.99, no subscription) bought through Polar in the console: open Info (or any Pro
prompt) in the app, choose Buy Pro, then paste the key and press Activate. The key is kept in the macOS Keychain and
checked against the licence server at launch; without a connection Pro stays on, and it only locks when the server
says the key is no longer valid. Remove key (Info) forgets it on this device. Builds from source have everything.

| | |
| --- | --- |
| Free | Scan for nearby devices, connect to one device, browse services and characteristics, read values, write values, one notification subscription at a time, communication log with single-line copy |
| Pro | Plots of notify streams (uint8, int8, int16, uint32, float32 and more, little or big endian, with byte offset and scale) with CSV export, value decoding in the GATT tree (numbers, UTF-8 and ASCII text, hex), standard GATT service and characteristic names, RSSI chart with CSV export, saved write presets and macros (several writes in order with delays and repeat), several notification subscriptions at once, several connected devices at once, communication log CSV export and copy all |

### MQTT Explorer

| | |
| --- | --- |
| Free | Connect with host, port, client ID, username and password (optionally over TLS), subscribe with QoS 0/1/2 and wildcards, publish (including retained), topic table, tree and message history, Raw, JSON, Hex and Base64 payload views, one saved connection profile, communication log with CSV export |
| Pro | Embedded local broker (TCP, WebSocket, TLS, username/password), more than one saved connection profile, custom CA, client certificates (mTLS) and Last Will on client connections, message history export to CSV, Sparkplug B decoding (metric names resolved from BIRTH aliases) and protobuf decoding from your own .proto schema, charts of numeric topic values (plain numbers, JSON fields, Sparkplug metrics), several simultaneous client connections, bulk-clear retained messages |

Version 1.2.0 is the first release, and it is free.
| OPC UA Explorer | |
| --- | --- |
| Free | Connect (anonymous or username/password, security policies and modes), endpoint and server discovery, browse the address space, node attributes and references, namespaces, read values on demand, live updates for up to 10 monitored items, one server session, communication log with CSV export, start and stop the built-in server |
| Pro | Unlimited live monitored items and the subscriptions panel (sampling, deadband, queue), several server sessions at once, CSV export of monitored values (current values and recorded changes), live trend charts, writing values (client and server), certificate management (trust, reject, regenerate), device templates for the built-in server |

## Build from source

Requirements: macOS 13+, Xcode command line tools, [Rust](https://rustup.rs), [Bun](https://bun.sh).

```bash
bun install
bun run dev:bacnet     # or dev:modbus, dev:dnp3, dev:s7, dev:ble, dev:enip, dev:serial, dev:nats, dev:mqtt, dev:opcua, dev:cnc, dev:secsgem
```

Universal release build (unsigned):

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/build-macos.sh bacnet-explorer unsigned   # -> build/bacnet-explorer/unsigned/
```

Tests: `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-s7 -p mt-ble -p mt-enip -p mt-mtconnect -p mt-focas -p mt-secsgem -p mt-store -p mt-connect` and `bun run --cwd apps/cnc-explorer test`, `bun run --cwd apps/secsgem-explorer test`, `bun run --cwd apps/modbus-poll test`.
The licence-key path of `mt-store` has its own tests: `cargo test -p mt-store --features license`.
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
apps/cnc-explorer      Tauri app (macOS), MTConnect and FANUC FOCAS client and simulators
apps/secsgem-explorer  Tauri app (macOS), SECS/GEM host and equipment simulator over HSMS
apps/s7-explorer       Tauri app, also builds for iOS (scripts/build-ios.sh)
apps/ble-inspector     Tauri app (Bluetooth LE scanner and GATT inspector)
apps/mqtt-explorer     Tauri app (MQTT client + embedded broker)
apps/opcua-explorer    Tauri app, also builds for iOS (scripts/build-ios.sh)
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
crates/mt-mtconnect    MTConnect agent client (probe, current, sample, stream, assets), agent simulator and SHDR adapter
crates/mt-focas        FANUC FOCAS/Ethernet client and CNC simulator (pure Rust, Beta)
crates/mt-secsgem      SECS/GEM host and equipment simulator: HSMS, SECS-II, GEM (pure Rust, Beta)
crates/mt-s7           S7 (ISO-on-TCP) client and simulator server
crates/mt-mqtt         MQTT client (rumqttc), embedded broker (rumqttd), Sparkplug B and protobuf payload decoding
crates/mt-opcua        OPC UA client (browse, read, write, subscriptions), server and simulator, certificate store
crates/mt-store        Pro entitlement: App Store builds (StoreKit purchase, receipt check) and direct-download builds (licence key)
crates/mt-connect      Voltrus Connect client (sign-in with PKCE, Keychain tokens, profile sync, App Store subscription linking)
packages/mt-ui         shared React components and design tokens
scripts/               macOS/iOS build, signing and App Store Connect helpers
```

Maintainers: signed releases run in GitHub Actions, see [docs/release.md](docs/release.md).

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). "Voltrus", "BACnet Explorer – Voltrus" and the app icons are trademarks of
JoyoDigitama and are not covered by the license: forks must use their own name and icon.
Contributions require agreeing to the [contributor terms](CONTRIBUTING.md).
