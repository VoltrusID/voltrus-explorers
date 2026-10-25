# Voltrus Tools

Industrial protocol tools in one native window, built with Rust and
[GPUI](https://gpui.rs) (Zed's GPU UI framework). Every frame is drawn on the
GPU, so live trends scroll at your display's refresh rate.

It shares the protocol crates in [`../../crates`](../../crates) with the
standalone Tauri apps in this repository.

## What it does

- **Modbus master**, TCP or RTU: register blocks of any area and type, word
  order, polling with automatic reconnect, writes to coils and holding
  registers, latency and error counters.
- **Modbus simulator**: serves a device template (SMA inverter, Schneider
  PM5560, Endress+Hauser flowmeter, ABB analyser, Emerson Daniel flow
  computer, Raritan PDU) or a plain register image. Values drift
  realistically; set any value to hold it.
- **Live points**: every value with its address, type, raw words and a
  sparkline of the last minute.
- **Trends**: multi-series charts of the points you pick, 30 s to 15 min.
- **Traffic and log**: every request and response, every connect and failure.
- **Tabs and splits**: watch a device and its simulator side by side, or two
  views of one session. Layout and connections are restored on launch.
- **Command palette** (⌘K) and 492 colour schemes with live preview.

Modbus is the first protocol. The others in this repository (MQTT, OPC UA,
S7, EtherNet/IP, BACnet, DNP3, NATS, serial, BLE) are being ported next; each
plugs into the same engine and views ([how](docs/architecture.md#adding-a-protocol)).

## Try it

Start the app and press **Start demo plant**: a simulated inverter opens next
to a client polling it. Or run a fixed scenario without touching your saved
workspace:

```bash
cargo run -p vt-app --release -- --demo points   # welcome, points, trend, traffic,
                                                 # simulator, editor, palette, light
```

## Shortcuts

| Keys | Action |
| --- | --- |
| ⌘K | Command palette |
| ⌘N / ⇧⌘N | New connection / simulator |
| ⌘Enter | Connect or disconnect the focused pane |
| ⌘1 … ⌘4 | Points, trend, traffic, log |
| ⌘D / ⇧⌘D | Split right / down |
| ⌘W | Close pane |
| ⌘B / ⌘I | Sidebar / inspector |
| ↑ ↓ | Move through points |

## Build

GitHub Actions builds, tests and bundles every push
([workflow](../../.github/workflows/voltrus-tools.yml)): download
`voltrus-tools-macos-arm64` and the screenshots from the run.

Locally (needs a few GB for GPUI's build):

```bash
cd apps/voltrus-tools
cargo test --workspace
cargo build --release -p vt-app
scripts/bundle-macos.sh target/release/voltrus-tools dist
```

Linux needs the usual GPUI libraries (`libxkbcommon`, `libwayland`, `libvulkan`,
`libfontconfig`) and `libudev` for serial ports.

Architecture: [docs/architecture.md](docs/architecture.md). Third-party
notices: [NOTICE.md](NOTICE.md).
