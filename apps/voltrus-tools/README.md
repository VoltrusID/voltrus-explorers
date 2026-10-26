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
- **OPC UA client**: GetEndpoints lists the security policies, modes and
  sign-in a server offers; connect with None, Basic256Sha256 or the Aes
  policies (Sign or Sign & Encrypt), anonymously, with a user name or with an
  X.509 user certificate chosen in an open panel. An address-space browser
  beside the points reads the tree a level at a time and walks it from the
  keyboard (↑↓ move, →← open and close, Enter adds a variable, ⌘⇧B jumps in);
  the inspector shows a picked node's attributes and calls methods with typed
  arguments. Variables are monitored in one subscription with the publishing
  and sampling intervals you set, keep their units, and take typed writes
  (Boolean, integers, floats, text, times); OPC UA statuses and source times
  ride with each value.
- **OPC UA server simulator**: a process plant (batch reactor, steam boiler,
  utilities) with units, ranges and the StartBatch and ResetAlarms methods,
  on None, Basic256Sha256 and Aes256 RsaPss endpoints, for anonymous clients,
  a user and a generated user certificate. Writes from any client hold a
  measured value or change a setpoint.
- **DNP3 master** (on `mt-dnp3`): TCP to an outstation with its link
  addresses; an integrity poll on connect and every integrity period, class
  1-3 event polls between them, and class polls, time sync, counter freeze
  and warm or cold restart on demand (inspector or palette). Every point the
  outstation reports is listed (binary and double-bit inputs, binary outputs,
  counters, analog inputs and outputs), named ones first; its flags decide
  its quality (offline, restart, comm lost, over range read bad) and the
  inspector shows them with the object variation and the last event's time
  stamp. Binary events are logged as a sequence of events. Outputs take CROBs
  (latch, pulse, trip, close) or analog output blocks by select-before-operate
  or direct operate, and the outstation's echo decides success. The master
  acts on the internal indications: it writes the time when asked, clears a
  restart and polls everything again, and recovers from an event overflow.
- **DNP3 outstation simulator**: plays a 13.8 kV feeder, a transformer bay or
  a solar plant on any TCP port, for any master. Values move, every change is
  a time-stamped event, outputs answer select-before-operate and direct
  operate, and inputs follow them (trip the breaker and its status opens, the
  currents fall to zero, the energy counter stops). A point can be taken
  offline (COMM_LOST); inputs set from the app are held until released.
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

Modbus, MQTT, BACnet, DNP3 and OPC UA are ported. The others in this
repository (S7, EtherNet/IP, NATS, serial, BLE) are being ported next; each plugs
into the same engine and views ([how](docs/architecture.md#adding-a-protocol)).

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
                                                 # opcua-points, opcua-trend,
                                                 # opcua-method, opcua-editor,
                                                 # bacnet-command, dnp3-points,
                                                 # dnp3-trend, dnp3-traffic,
                                                 # dnp3-control, dnp3-station,
                                                 # dnp3-editor
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
