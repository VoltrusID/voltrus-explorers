# mt-focas

A pure-Rust FANUC **FOCAS/Ethernet** client (TCP 8193) and a **CNC simulator** that speaks the same
wire protocol. No FANUC library is involved, so it builds and runs on macOS (and anywhere else
Rust does). FANUC ships FOCAS only as a C library for Windows and Linux.

- `FocasClient`: async (tokio) client with typed, serde-serializable results and timeouts.
- `FocasSimulator`: TCP server answering the same calls from a `CncModel` the host can change
  while it runs (`with_model`), with `start` / `stop`.
- `codec` and `functions`: the framing and payload layouts both sides share.

```rust
let client = FocasClient::connect(FocasConfig::new("192.168.1.50")).await?;
let info = client.sysinfo().await?;          // series, version, max axes
let snap = client.snapshot().await?;         // status, positions, feed, spindle, program, alarms
client.disconnect().await?;
```

```text
cargo run -p mt-focas --example focas_sim -- 8193 127.0.0.1 31i --run
```

## Not verified against a real controller

**This crate has never talked to a real FANUC controller.** Every byte layout comes from the
reference below, and the client and simulator were only tested against each other and against
pyfanuc (the reference client). Where the reference has no capture, this README says so.

The reference reports testing on: an EDM machine with a **Series 160** control (the diagnosis
sample is labelled 160i-W), a **Robodrill with a 30i**, and a **31i** (its second diagnosis sample).
Its multi-packet samples were captured from machines it does not name. Nothing else is claimed:
other families (0i-D/F, 30i-B, 32i, Power Motion i, lathes) are untested here and in the
reference. The 0i-F profile of the simulator is only an identity string.

## Protocol

All values are big endian. Frame: `a0 a0 a0 a0 | version u16 | type u16 | payload length u16 | payload`.

| Type | Meaning |
| --- | --- |
| `0101` / `0102` | open request / response; request payload `00 02` opens the control stream (`00 01` the program-transfer stream) |
| `0201` / `0202` | close request / response, no payload |
| `2101` / `2102` | data request / response: `count u16` then `count` sub-packets, each `length u16` (counting itself) + body |

Request sub-packet body: `device u16` (1 CNC, 2 PMC), `class u16` (1), `function u16`, five `i32`
arguments, optional extra bytes. Response body: the same six command bytes, `error u16` (an `EW_*`
code, 0 = ok), 4 fill bytes, `payload length u16`, payload; an error carries no payload. Several
sub-packets travel in one request (the samples show 4 and 9; the client splits at 9).
Scaled numbers are 8 bytes: `i32` mantissa, `00`, base (10 or 2), flag, exponent; the value is
`mantissa / base^exponent`, and `ff ff` in the last two bytes marks the slot invalid.

Details and the byte-exact samples are in `src/codec.rs` and `src/functions.rs` (unit tests).

## Calls

| Client call | Function | Source |
| --- | --- | --- |
| `sysinfo` | `01 18` | pyfanuc `getsysinfo` |
| `statinfo` (aut, run, motion, mstb, emergency, alarm, edit) | `01 19` | pyfanuc `statinfo` (14-byte payload only) |
| `axis_names` | `01 89` | pyfanuc `protocol.txt` + `sample_multipacket.md` |
| `read_position` / `read_positions` (absolute 4, machine 1, relative 6, distance-to-go 7, skip 8) | `01 26` | pyfanuc `readaxes`, `sample_with_error.md` |
| `actual_feed`, `spindle_speed` | `01 24`, `01 25` | pyfanuc `readactfeed`, `readactspindlespeed` |
| `servo_load` | `01 56` | `sample_multipacket.md` (request and response shown, no pyfanuc function) |
| `program_numbers` (running, main) | `01 1c` | pyfanuc `readprognum` |
| `sequence_number` | `01 1d` | `sample_with_error.md` |
| `main_program_name` | `01 b9` | pyfanuc `readprogname` (30i and later) |
| `executing_block` | `01 20` | pyfanuc `readexecprog` |
| `list_programs` | `01 06` | pyfanuc `listprog` |
| `alarm_bits` | `01 1a` | pyfanuc `readalarm` |
| `alarm_messages` (number, type, axis, text) | `01 23` | pyfanuc `readalarmcode` |
| `read_macro` | `01 15` | pyfanuc `readmacro`, pyfocas `read_macro` |
| `write_macro` | `01 a8` | pyfocas `write_macro_double` only (see below) |
| `read_params` | `01 0e` | pyfanuc `readparam` |
| `read_diag` | `01 30` | pyfanuc `readdiag` |
| `read_pmc` (byte/word/dword, areas G F Y X A R T K C D) | `02 8001` | pyfanuc `readpmc` |
| `snapshot` | several in one request | `sample_with_error.md` shows the multi-request |
| `request` | any | raw sub-packets with the same framing, splitting and echo checks |

Field names and the meaning of the `statinfo`, `sysinfo`, alarm and axis-position values follow
FANUC's FOCAS documentation as published at inventcom.net (`cnc_statinfo`, `cnc_sysinfo`,
`cnc_alarm`, `cnc_rdalmmsg`, `cnc_rdaxisdata`, return status codes).

### Not implemented

Everything else in the reference:

- program upload/download stream (`0x1501`, `0x1604`, `0x1701`, `0x1702` frames, pyfanuc `getprog`)
- directory calls of the 30i family (`b0`, `b3`, `b4`), `readdir*`
- date and time (`45`, `46`, pyfanuc `getdate`/`gettime`/`settime`): the reference is inconsistent
  about the date byte order (README capture shows little-endian words, the code reads big-endian)
- number formats (`1b`), MDI (`20` is read as the executing block only), G-code reads (`21`, `96`),
  spindle names/load (`8a`, `40`), signal names (`61`), the 31i diagnosis form (`93`, `a2`),
  parameter info (`a0`), `cnc_rdparam3` (`8d`), the controlled-axis call (`a4`), macro read as
  double (`a7`), set macro by function `16`, PMC writes
- anything else in `protocol.txt` marked `??`

## Simulator

`FocasSimulator::start(SimulatorConfig { port: 8193, .. })` listens and answers every call above
from a `CncModel`: a 3-axis mill (X, Y, Z) with identity strings of a 31i-B (CNC type `31`, which
pyfanuc's `statinfo` requires) or a 0i-F (` 0`), a spindle, programs O3000/O3001 that
`cycle_start()` plays (axes move, distance-to-go shrinks, feed and spindle follow, sequence
numbers advance), a part counter in macro #500 (and parameters 6711/6712) that counts finished
cycles, alarms (`raise_alarm` / `clear_alarm`, which stop a running cycle and show in the alarm bit
field, `statinfo` and `alarm_messages`), emergency stop, macro variables, parameters, diagnosis
values and PMC memory. `tick: Some(100ms)` advances the model in real time; with `None` the
host calls `advance`. The model is plain data (`Serialize`/`Deserialize`).

The identity strings, parameter, diagnosis and PMC contents are made up to look plausible; none of
it was recorded from hardware.

## What was verified

- **Codec:** byte-exact encode and decode of the samples in pyfanuc's `sample_multipacket.md`
  (4-sub-packet request), `sample_with_error.md` (9-sub-packet request and response, including the
  error sub-packets 1 and 6), its README (PMC D2204 request and response, diagnosis 980/981
  payload) and open/close packets; the scaled-number decoder against the sample values and the
  pyfocas doctest value. `cargo test -p mt-focas`.
- **Round trip:** client against simulator over loopback TCP for every implemented call, plus
  timeouts, dropped connections, a silent server, garbage input, several clients, multi-request
  splitting, and a raw-socket replay of the 9-sub-packet pyfanuc request.
- **Third-party client:** pyfanuc (unmodified, Python 3) was run once against the simulator:
  connect, `sysinfo`, `statinfo`, `readaxes` (all five kinds), `readactfeed`,
  `readactspindlespeed`, `readprognum`, `readprogname`, `readexecprog`, `readalarm`,
  `readalarmcode`, `readmacro`, `readparam` (byte, bit, int), `readdiag`, `readpmc` (word,
  dword), `listprog` and `disconnect` all returned the simulated values, in a running-cycle and an
  alarm state. That script was a throwaway and is not part of the repository.
- **Not verified:** anything against a real controller, a real FOCAS library, or a network
  capture of this crate's own traffic.

### Inferred, not from a capture

- The simulator sends an **empty open response** (the reference says real controllers send about
  360 bytes of unexplained content; no client checks it) and uses frame version `0002` for
  responses (samples show `0002` and `0004`).
- `write_macro`: the request layout is pyfocas's; the reference has no capture of the answer, so
  the simulator and client assume the usual sub-packet layout with error word 0.
- Axis names: four bytes per axis with the name first; a suffix character is taken from the first
  non-zero later byte, a layout the samples (all suffix-less) do not show.
- Servo load argument `1`, the `block` word of `executing_block` (the simulator sends the block
  index), single-axis position requests (the simulator only serves "all axes"), and the
  decimal places of feed and spindle values (the simulator sends 0).
- Parameter type 2 (short): pyfanuc's own decoder is broken for it, so the 16-bit value is read
  from the low two bytes of the slot and was never checked against a capture. Parameter and
  diagnosis payloads use `max_axis * 4 + 8` bytes per number as pyfanuc's `readparam` does, with
  `max_axis` from `sysinfo`; the 31i diagnosis form uses another layout (function `93`).
- `statinfo` accepts only the 14-byte (7-word) payload the reference decodes, although the FOCAS
  `ODBST` of 16/18/21/0i/30i has 9 words; pyfanuc itself only decodes it for CNC types `16` and `31`.
- The `AlarmType` names are the 30i / 0i-D/F table; older series number alarm bits differently.
- `snapshot` sends at most 9 sub-packets per request (the largest seen in the samples).

## Naming

FANUC and FOCAS are trademarks of FANUC CORPORATION. `mt-focas` is an independent project, not
affiliated with, endorsed by or supported by FANUC, and contains no FANUC code or library.

## NOTICE

The wire protocol was reverse-engineered by **diohpix** in
[pyfanuc](https://github.com/diohpix/pyfanuc), released into the public domain under the
Unlicense. The framing, function numbers, request arguments, payload layouts, scaled-number format
and sample packets used here come from its `pyfanuc.py`, `protocol.txt`, `README.md`,
`sample_multipacket.md` and `sample_with_error.md`. Thank you.

The macro write function (`01 a8`) and the byte-level description of scaled numbers were checked
against [pyfocas](https://pypi.org/project/pyfocas/) 0.1.1 (MIT, Copyright (c) 2026 Moritz
Breurather, Rene Schwaiger), which in turn credits pyfanuc. No pyfocas code is copied; only the
function number and request layout of `write_macro` are taken from it.

Field meanings follow FANUC's FOCAS function documentation as published by inventcom.net; no text
or code from it is included.

This crate is licensed GPL-3.0-or-later like the rest of the workspace.
