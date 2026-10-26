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
- **EtherNet/IP client** (on `mt-enip`): a device picker that sends
  ListIdentity (broadcast, and directed at an address and at the simulators
  on this computer) and lists each device once with its product, vendor,
  device type, revision and serial number. Logix tags are read by name
  (`Program:Main.Speed`, `Recipe[3]`, `Status.5`) in Multiple_Service_Packet
  batches, with their type as the controller reports it; a tag the controller
  refuses costs only its own point. Any CIP attribute is a point too, e.g.
  `@0x04/100/3+4:REAL` for a REAL at byte 4 of input assembly 100. With no
  points the client browses every controller and program tag (arrays by their
  first elements, structures by their members). Writes go to tags by name and
  to attributes with Set_Attribute_Single, range-checked against the type. An
  optional route path (`1,0`) reaches a ControlLogix through its Ethernet
  module. The points table shows each tag's program (or `Ctrl`) and the
  inspector its whole address. The inspector sends any explicit message
  (service, class, instance, attribute, data) and shows the reply as hex and
  as the values it may hold.
- **EtherNet/IP adapter simulator**: plays a motor drive, a tank farm or a
  packaging line on any port, as a Logix-style target: tag browse, Read/Write
  Tag, the Identity object, input assembly 100 (every tag's value) and output
  assembly 150, Forward_Open and class 1 I/O, and ListIdentity on the same UDP
  port. A value set from the app or written by any client is held until
  released.
- **Serial monitor** (on `mt-serial`): a port picker that lists USB, Bluetooth
  and built-in ports and updates as devices come and go (the same picker fills
  a Modbus RTU master's port), baud with presets, 5 to 8 data bits, parity,
  stop bits and RTS/CTS or XON/XOFF flow control. A missing or busy port is
  retried every second, so a device plugged in later is picked up. The
  terminal shows what crosses the port as text lines (CR, LF or CR LF;
  control bytes as ␂-style pictures) or a hex dump, with timestamps, pause and
  clear. The send bar sends text (with `\r`, `\n`, `\xNN` escapes and the
  connection's line ending) or hex, keeps a history (↑ ↓), and runs saved
  macros (⌃1 … ⌃9, the inspector or ⌘K). **Log to file…** writes every byte in
  and out to a CSV file you pick. Received lines become points that trend:
  `name=value` pairs with their units, CSV columns (named by a header line or
  the form), or a regular expression's capture groups. Modbus RTU stays a
  Modbus connection: **Poll as Modbus RTU** carries the port settings over.
- **Serial device simulator**: plays any device template on a virtual serial
  port (a pseudo-terminal) with a stable link in the temporary folder,
  printing its drifting values as `key=value` or CSV lines. It answers
  `HELP`, `READ`, `SET <name> <value>` (held until `RELEASE`) and
  `RATE <ms>`; a value set from the app is held too.
- **Siemens S7 client** (on `mt-s7`, ISO-on-TCP): host and port, rack and
  slot (with S7-1200/1500, S7-300 and S7-400 presets) and the PDU size to
  ask for. Tags are addresses as engineers write them (`DB1.DBD0`,
  `DB1.DBX4.0`, `MW20`, `I0.3`, `QB2`, `T5`, `C10`, `%`-prefixed or German
  `E`/`A`/`Z` too), validated as you type, read as Bool, Byte, Word, Int,
  DWord, DInt, Real, String, Timer (ms) or Counter, packed into as few
  requests as the negotiated PDU allows. A tag the CPU refuses (a missing DB)
  goes bad on its own and is logged; the rest keep reading. Writes are checked
  against the type before they go out. The **PLC** view (⌘5) shows the CPU's
  identity (module, order number, firmware, serial, station), its operating
  mode, the diagnostic buffer newest first and every block by type; ⌘R reads
  it again.
- **S7 PLC simulator** (`mt-s7`'s server): plays an S7-1500 motor drive, an
  S7-300 reactor or an S7-1200 packaging line on any TCP port, with data
  blocks, inputs, outputs, markers, timers, counters and strings, a CPU
  identity, diagnostic buffer and block list. A value set from the app or
  written by any client is held until released.
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
  words, a BACnet client's requests, answers and COV notifications, an
  EtherNet/IP client's frames by service and tag, the requests a simulator
  receives, and every connect, failure and external
  words, a BACnet client's requests, answers and COV notifications, an S7
  client's and simulator's PDUs with their addresses and item results, the
  requests a simulator receives, and every connect, failure and external
  write. Both follow new entries until you scroll back or pause.
- **Tabs and splits**: watch a device and its simulator side by side, or two
  views of one session. Layout and connections are restored on launch.
- **Command palette** (⌘K) and 492 colour schemes with live preview.

Modbus, MQTT, BACnet, DNP3, EtherNet/IP, serial, S7 and Bluetooth LE are
ported. The others in this repository (OPC UA, NATS) are being ported next;
each plugs into the same engine and views ([how](docs/architecture.md#adding-a-protocol)).

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
                                                 # bacnet-command, dnp3-points,
                                                 # dnp3-trend, dnp3-traffic,
                                                 # dnp3-control, dnp3-station,
                                                 # dnp3-editor, enip-points,
                                                 # enip-trend, enip-traffic,
                                                 # enip-discover, enip-message,
                                                 # serial-terminal,
                                                 # serial-hex, serial-trend,
                                                 # serial-points, serial-editor,
                                                 # serial-simulator, s7-points, s7-trend,
                                                 # s7-traffic, s7-plc, s7-write, s7-editor,
                                                 # ble-points, ble-trend,
                                                 # ble-scan, ble-inspector
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
| ⌘5 / ⌘R | Device view (an S7 client's PLC) / read it again |
| ⌘D / ⇧⌘D | Split right / down |
| ⌘W | Close pane |
| ⌘B / ⌘I | Sidebar / inspector |
| ↑ ↓ | Move through points |
| ⌘L | Type in a serial pane's send bar |
| ↑ ↓ (send bar) | Walk the sends history |
| ⌃1 … ⌃9 (send bar) | Send a macro |

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
