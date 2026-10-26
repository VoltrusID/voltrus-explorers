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
- **BACnet/IP client**: a device picker that sends Who-Is (broadcast on the
  BACnet port, and directed at an address and at the simulators on this
  computer) and lists the devices that answer with their instance, vendor and
  address. The client reads the device's object list and shows every analog,
  binary and multi-state input, output and value with its object name and
  units, subscribes to change-of-value notifications where the device takes
  them and polls the rest with ReadPropertyMultiple (ReadProperty for devices
  without it). A fault status flag marks the value bad; alarm and override
  flags are logged. Writes are WriteProperty commands at the connection's
  priority, and **Relinquish** writes NULL to give the command up. The local
  address and UDP port are configurable (0.0.0.0:47808 to receive
  broadcasts).
- **BACnet device simulator**: plays an air handler, a chiller or a VAV box
  on any UDP port. It answers Who-Is, ReadProperty(Multiple), WriteProperty
  with a 16-level priority array, and SubscribeCOV. A value set from the app
  or commanded by any client is held until released or relinquished.
- **Bluetooth LE client** (on `mt-ble`): a scan that lists advertising
  peripherals with their signal (dBm and bars), advertised services,
  manufacturer and service data and transmit power. Connecting browses the
  GATT services and makes every characteristic a point, named by its
  Bluetooth SIG number: readable ones are read and re-read, notifying and
  indicating ones subscribed. Standard characteristics decode themselves
  (heart rate, battery, temperature and IEEE 11073 thermometer readings,
  humidity, pressure, elevation, device information text); any other can be
  decoded as an integer or float of any width and byte order at an offset
  with a scale, as UTF-8 or as hex, and the choice is kept with the
  connection. The inspector shows the service, UUID and properties, reads
  on demand, turns notifications on and off, and writes a number in the
  characteristic's layout or raw bytes or text. A lost link reconnects.
  A **Simulated** adapter (a heart rate monitor, an environment sensor with a
  writable setpoint and a UART that echoes, and a beacon) runs the same
  session without hardware. The app asks for Bluetooth access the first time
  it scans.
- **Live points**: every value with its Modicon address, name, unit, type, raw
  words and a sparkline of the last minute. Clients of a simulated template
  get its register names and units. Columns drop out as a pane narrows.
- **Trends**: multi-series charts of the points you pick, 30 s to 15 min.
  Series of very different size (watts beside hertz) get stacked lanes, each
  with its own scale.
- **Traffic and log**: a master's requests and responses with their payload
  words, a BACnet client's requests, answers and COV notifications, the
  requests a simulator receives, and every connect, failure and external
  write. Both follow new entries until you scroll back or pause.
- **Tabs and splits**: watch a device and its simulator side by side, or two
  views of one session. Layout and connections are restored on launch.
- **Command palette** (⌘K) and 492 colour schemes with live preview.

Modbus, MQTT, BACnet and Bluetooth LE are ported. The others in this
repository (OPC UA, S7, EtherNet/IP, DNP3, NATS, serial) are being ported
next; each plugs into the same engine and views
([how](docs/architecture.md#adding-a-protocol)).

## Try it

Start the app and press **Start demo plant**: a simulated inverter opens next
to a client polling it (each protocol on the welcome screen has its own
**Demo**). Or run a fixed scenario without touching your saved workspace:

```bash
cargo run -p vt-app --release -- --demo points   # welcome, points, trend, traffic,
                                                 # simulator, editor, palette, light,
                                                 # mqtt-points, mqtt-trend, mqtt-traffic,
                                                 # mqtt-inspector, mqtt-editor
                                                 # bacnet-points, bacnet-trend,
                                                 # bacnet-traffic, bacnet-discover,
                                                 # bacnet-command, ble-points,
                                                 # ble-trend, ble-scan,
                                                 # ble-inspector
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
`libfontconfig`), `libudev` for serial ports and `libdbus-1` for Bluetooth.

macOS only lets an app with a Bluetooth usage description use the radio:
try Bluetooth LE on real peripherals from the bundle (`dist/Voltrus Tools.app`),
not `cargo run`. The sandboxed build needs the `com.apple.security.device.bluetooth`
entitlement.

Architecture: [docs/architecture.md](docs/architecture.md). Third-party
notices: [NOTICE.md](NOTICE.md).
