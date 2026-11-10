# Voltrus Logic Analyzer

A logic analyzer for the cheap Cypress FX2 USB boards (the "24 MHz 8-channel" boards sold as
Saleae Logic compatibles, around USD 10), made for plant wiring. It treats Modbus RTU on RS-485
as a first-class protocol, decodes CAN at bit level, and handles SPI (with the MCP2515 CAN
controller), I2C and UART. A built-in simulator generates industrial signals, so the app works
without a board.

- Protocol code: [`crates/mt-logic`](../../crates/mt-logic)
- App: Tauri v2 + React, the same layout and `@mactools/ui` as the other explorers
- Firmware licence and how the app gets it: [FIRMWARE.md](FIRMWARE.md)

## Status

| Part | State |
| --- | --- |
| Decoders (UART, Modbus RTU, SPI + MCP2515, I2C, CAN) | Tested against synthesized waveforms, including glitches, wrong baud and bit rates, CRC errors, missing ACKs and error frames |
| Capture engine, triggers, files (.vla, sigrok .sr, VCD) | Tested |
| Simulator (5 scenarios) | Tested; used for the screenshots |
| FX2 driver logic (detection, firmware upload, re-enumeration, start command, streaming) | Tested against an emulated FX2 (`mt_logic::fx2::mock`) |
| **FX2 on real hardware (the nusb USB backend)** | **Untested.** No board has been attached yet. It compiles and lists USB devices; firmware upload and streaming have never run on a real chip. The app says so next to the board picker. |

## Using it

1. Pick **Simulator** or **USB board**, a sample rate, a length and a trigger, then press **Run
   capture**.
2. Waveform: scroll to zoom around the pointer, drag to pan, use Shift or a trackpad to pan
   sideways. Click to place cursor A, Shift-click for cursor B. Cursors snap to the nearest
   edge of the channel under the pointer. The bar above shows A, B, Δt and 1/Δt. Measure gives
   the frequency and duty cycle of a channel between the cursors, or over the visible window.
   Keys: `+` / `-`, arrow keys, `Home` / `End`, `F` to fit.
3. Decoders: **Add** one per bus. Each adds annotation rows under the waveform and, where it
   makes sense, a protocol table at the bottom. Clicking a table row zooms to it; clicking a frame
   annotation selects its row.
4. Save as `.vla` (samples, channel names and colours, the decoder stack, cursors and view), or
   as `.sr` / `.vcd`. Open `.vla` and sigrok `.sr` files.

### Modbus RTU on RS-485

Probe the logic side of the transceivers, never the A/B pair directly:

- **One channel:** the `RO` pin of a listening transceiver sees both directions. Set it as the
  request channel and leave the response channel empty. The decoder tells requests from responses
  by pairing: after a request, the next frame from the same slave with the same function code (or
  its exception) is the response.
- **Two channels:** the master's `DI` (TX) as the request channel, the slave's `RO` or TX as the
  response channel. Add the `DE` lines on spare channels to check driver-enable timing against
  the frames.

Frames are split on 3.5 character times of silence (a fixed 1.75 ms above 19200 baud). Gaps
longer than 1.5 characters inside a frame are flagged. CRC errors, exceptions, missing responses
(after the response timeout or the next request) and broadcasts are reported. The table shows
response times. When a slave answers in less than 3.5 characters, the decoder splits the frames
where the CRC checks out. Defaults are 19200 baud 8E1, the specification's default; **Detect**
guesses the baud rate from the shortest pulses.

### CAN

Probe `CAN_RX` (or `CAN_TX`) on the logic side of the transceiver; dominant is low. Use at least
8 samples per bit (4 MHz for 500 kbit/s; 8 MHz or more is better). The decoder hard-syncs on SOF,
resynchronises on every recessive-to-dominant edge, removes stuff bits and checks CRC-15, the
ACK slot, delimiters and EOF. It reports stuff errors and error flags. CAN FD frames are recognised
by their FDF bit and marked "not decoded".

## FX2 boards

Detected by their USB id before the firmware is loaded:

| VID:PID | Board | Firmware file |
| --- | --- | --- |
| 0925:3881 | Saleae Logic and the 24 MHz 8-channel clones | `fx2lafw-saleae-logic.fw` |
| 04b4:8613 | Bare CY7C68013A boards (blank EEPROM) | `fx2lafw-cypress-fx2.fw` |
| 1d50:608c | sigrok FX2 8 ch (WeAct and similar) | `fx2lafw-sigrok-fx2-8ch.fw` |
| 1d50:608d | sigrok FX2 16 ch (first 8 channels) | `fx2lafw-sigrok-fx2-16ch.fw` |
| 08a9:0014 / 0015 / 0009 / 0005 | USBee AX / DX / SX / ZX compatibles | `fx2lafw-cwav-usbee*.fw` |
| 16d0:0498 | Braintechnology USB-LPS | `fx2lafw-braintechnology-usb-lps.fw` |

Sample rates are 20 kHz to 24 MHz: 48 MHz or 30 MHz divided by `delay + 1`, with the delay at most
1535. 24 MHz needs an otherwise idle USB 2.0 port. If the board stops sending, the app says so and
suggests a lower rate. Memory stays bounded: the driver keeps about 300 ms of transfers queued,
and the capture is capped at its length (at most 512 Mi samples).

The direct-download app **bundles** the official fx2lafw 0.1.7 images with their GPL/LGPL licence
texts and a source offer (the source is mirrored on `dl-logic-analyzer.voltrus.id`). Device →
Firmware… shows which image each board will get, and can also download a release from sigrok.org
or use a `.fw` file you choose. See [FIRMWARE.md](FIRMWARE.md).

### Linux: USB permissions (udev)

Without a rule, opening the board fails with "Permission denied" and the app shows how to fix
it. Install this as `/etc/udev/rules.d/60-voltrus-logic.rules`:

```
# Voltrus Logic Analyzer: FX2 logic analyzers, before and after the fx2lafw firmware is loaded
SUBSYSTEM=="usb", ATTRS{idVendor}=="0925", ATTRS{idProduct}=="3881", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="usb", ATTRS{idVendor}=="04b4", ATTRS{idProduct}=="8613", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="usb", ATTRS{idVendor}=="1d50", ATTRS{idProduct}=="608c", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="usb", ATTRS{idVendor}=="1d50", ATTRS{idProduct}=="608d", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="usb", ATTRS{idVendor}=="08a9", ATTRS{idProduct}=="0005|0009|0014|0015", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="usb", ATTRS{idVendor}=="16d0", ATTRS{idProduct}=="0498", MODE="0660", TAG+="uaccess"
```

Then run `sudo udevadm control --reload && sudo udevadm trigger` and replug the board. If
sigrok's own `60-libsigrok.rules` is installed, it already covers these boards.

### macOS and Windows

macOS needs no driver; the build has `com.apple.security.device.usb`. It is sold as a direct
download only (no Mac App Store build, because of the bundled GPL firmware). On Windows the
board needs the WinUSB driver (Zadig), as for PulseView. Windows builds are not part of this
first pass.

## Free and Pro (licence-key builds)

USD 39.99 one-time, one Mac, movable (Polar product `3233bac5-95dd-4f7d-9186-06d45c6bfa2a`,
benefit `VOLTRUS-LOGIC`); agreed by the owner on 2026-11-11.

| | |
| --- | --- |
| Free | Simulator, FX2 capture up to 2 M samples at any rate, edge triggers, waveform with cursors and measurements, UART, Modbus RTU (with the protocol table), SPI and I2C decoders, open .vla and .sr, save .vla |
| Pro | CAN decoder, MCP2515 decoding, pattern triggers, captures up to 512 Mi samples, protocol table CSV export, .sr and VCD export |

## Later: link with CAN Explorer (design note, not built)

The goal is that a CAN frame in Logic Analyzer and the same frame in CAN Explorer point at each
other. That gives two views of one bus: the electrical one (bit timing, stuffing, error flags) and
the message one (DBC signals, J1939/CANopen names).

- **Shared types.** `mt-logic`'s `CanBitFrame` (id, extended, rtr, dlc, data) maps 1:1 onto
  `mt_can::CanFrame`. Put a `From` impl in a small bridge module behind a feature, so neither
  crate depends on the other by default.
- **DBC in the logic analyzer.** Load a DBC (reusing `mt_can::dbc`) into the CAN decoder's
  settings. The Frames row then shows message and signal names (`EEC1 EngineSpeed=1500 rpm`) and
  the table gains a signal column.
- **Export and import.** Export the decoded frames as a candump log with the capture's timestamps
  (`mt_can::log` already writes it), which CAN Explorer opens in Replay/Trace. The reverse is a
  candump from CAN Explorer opened here as an overlay: frames matched by ID and data within a
  time window, with mismatches flagged (a frame the adapter saw but the wire did not, or the other
  way round).
- **Live link (later still).** When both apps run, CAN Explorer could post its frames to a local
  socket and Logic Analyzer could arm its trigger on "next frame with ID X". That needs a
  frame-level trigger in the capture engine, which would decode CAN while streaming (the bit
  decoder is incremental-friendly: SOF hard-sync, then a fixed field walk).
- **Simulator.** The CAN scenario already sends the same J1939 EEC1 engine frames as CAN
  Explorer's genset simulator, so the demos line up.

## Development

```bash
bun install
bun run dev:logic              # Tauri dev app
cargo test -p mt-logic -p logic_analyzer
bun run --cwd apps/logic-analyzer test
```
