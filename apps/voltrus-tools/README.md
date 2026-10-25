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
  computer, Raritan PDU) or a plain register image of up to 2000 registers.
  Template values drift realistically; a value set from the app or written by
  any master is held until released.
- **MQTT client** (on `mt-mqtt`): host and port, client id, username and
  password, TLS against the system's trust store, keep-alive, wildcard
  subscriptions with QoS. Every concrete topic becomes a point: a number,
  true/false, each numeric field of a JSON payload (or only the one at a JSON
  path such as `phases[0].v`), or each Sparkplug B metric with its unit (a
  missed birth is re-requested). Raw messages go to traffic, broker events to
  the log; the inspector shows the last message and publishes any payload with
  QoS and retain, and writing a point publishes to its topic.
- **MQTT broker simulator**: a broker on this computer that publishes a
  device template as retained numbers at `<prefix>/<value>`, a JSON nameplate
  at `<prefix>/status` and optionally a Sparkplug B edge node. A number
  published to a value's topic (or `<topic>/set`) by any client is held.
- **Live points**: every value with its Modicon address, name, unit, type, raw
  words and a sparkline of the last minute. Clients of a simulated template
  get its register names and units. Columns drop out as a pane narrows.
- **Trends**: multi-series charts of the points you pick, 30 s to 15 min.
  Series of very different size (watts beside hertz) get stacked lanes, each
  with its own scale.
- **Traffic and log**: a master's requests and responses with their payload
  words, the requests a simulator receives, and every connect, failure and
  external write. Both follow new entries until you scroll back or pause.
- **Tabs and splits**: watch a device and its simulator side by side, or two
  views of one session. Layout and connections are restored on launch.
- **Command palette** (⌘K) and 492 colour schemes with live preview.

Modbus and MQTT are ported. The others in this repository (OPC UA, S7,
EtherNet/IP, BACnet, DNP3, NATS, serial, BLE) are being ported next; each
plugs into the same engine and views ([how](docs/architecture.md#adding-a-protocol)).

## Try it

Start the app and press **Start demo plant**: a simulated inverter opens next
to a client polling it (each protocol on the welcome screen has its own
**Demo**). Or run a fixed scenario without touching your saved workspace:

```bash
cargo run -p vt-app --release -- --demo points   # welcome, points, trend, traffic,
                                                 # simulator, editor, palette, light,
                                                 # mqtt-points, mqtt-trend, mqtt-traffic,
                                                 # mqtt-inspector, mqtt-editor
```

Demo scenarios use the dark house theme (`light` the light one) whatever the
system appearance, so screenshots are comparable.

## Shortcuts

| Keys | Action |
| --- | --- |
| ⌘K | Command palette |
| ⌘N / ⇧⌘N | New connection / simulator (pick the protocol) |
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
