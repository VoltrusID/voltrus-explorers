# mt-secsgem

A pure-Rust **SECS/GEM** stack: an async GEM **host** (client) and a GEM **equipment simulator**,
both on top of the same HSMS and SECS-II implementation. No C library is involved, so it builds
and runs on macOS (and anywhere else Rust does).

- `hsms`: HSMS (SEMI E37) over TCP: framing, Select / Deselect / Linktest / Separate / Reject,
  timers T3 T5 T6 T7 T8, active and passive connections, a message log stream.
- `item` and SML: the SECS-II (SEMI E5) item codec and the SML text notation
  (`S1F13 W <L [2] <A "MDLN"> <A "SOFTREV">>.`), rendering and parsing.
- `GemHost`: typed async helpers for the GEM (SEMI E30) messages a host uses, plus raw sends.
- `EquipmentSimulator`: a wafer processing chamber behind a GEM interface, driven by a mutable,
  serde-serializable `EquipmentModel` a UI can change while a host is connected.

```rust
let host = GemHost::connect(GemHostConfig::new("192.168.1.60:5000")).await?;
let (ack, tool) = host.establish_communications().await?;     // S1F13/S1F14
host.define_reports(&[ReportDef { rptid: 1, vids: vec![12, 4] }]).await?;          // S2F33
host.link_event_reports(&[EventLink { ceid: 100, rptids: vec![1] }]).await?;       // S2F35
host.enable_events(true, &[100]).await?;                                           // S2F37
let mut events = host.subscribe();                                 // S6F11 arrive here, already acked
host.remote_command("START", &[CommandParam::new("PPID", Item::ascii("ETCH_SIO2"))]).await?;
host.disconnect();
```

```text
cargo run -p mt-secsgem --example gem_sim -- 5000 127.0.0.1 --run
```

## Not verified against real equipment

**This crate has never talked to real semiconductor equipment or a real factory host.** It was
tested against itself and against one third-party implementation (below). The SEMI standards
(E37, E5, E30) are paid documents and were **not consulted**; message layouts and codes come from
the sources listed under "Sources", so a misreading shared by those sources and this crate would not
have been caught. Real tools differ in id types, optional items, state-model details and which GEM
capabilities they implement; expect to adjust `IdFormat`, timers and the raw message API.

## What is implemented

### HSMS (E37)

- Frame: 4-byte length, 10-byte header (session id, byte 2, byte 3, PType, SType, system bytes).
  Frames split across reads or coalesced in one read are handled; lengths below 10 or above
  `max_message_len` (16 MiB) close the connection.
- Control messages: Select.req/rsp (status 0 ok, 1 already active, 2 not ready, 3 exhausted),
  Deselect.req/rsp, Linktest.req/rsp (session id `0xFFFF`), Separate.req, Reject.req (SType not
  supported, PType not supported, transaction not open, entity not selected).
- Active mode (`HsmsConnection::connect`, `GemHost::connect`) and passive mode (`HsmsListener`,
  `GemHost::accept`, the simulator's default). Both ends use the same `HsmsConnection`.
- Timers, with E37 defaults: **T3** reply 45 s (fails the one call, `Timeout(T3)`), **T5**
  connect separation 10 s (`Connector` waits between attempts; the simulator's active mode uses
  it), **T6** control transaction 5 s (closes the connection), **T7** not-selected 10 s (runs
  while TCP is up and the session is not selected, also after a Deselect), **T8** inter-character
  5 s (applied between reads inside a frame; an idle line is not a T8). Optional periodic Linktest.
  All are `HsmsConfig` fields in milliseconds.
- System bytes, device id, a log stream (`subscribe_log`) of every message in both directions,
  and `start_io` to run HSMS over any byte stream.

### SECS-II (E5) items

List, Binary, Boolean, ASCII, JIS8 (raw JIS-X-0201 bytes, no transcoding), I1 I2 I4 I8, U1 U2 U4
U8, F4 F8. One to three length bytes: the encoder uses the fewest, the decoder accepts all three.
Truncation, unknown format codes, trailing bytes, bad array lengths and nesting deeper than 64 are
errors. ASCII keeps one char per byte (bytes above 0x7f survive a round trip as U+0080..U+00FF).

SML: `Item::to_sml` / `to_sml_pretty` / `from_sml`, `SecsMessage::to_sml` / `from_sml`. Text
that is not printable ASCII is written as `0x..` bytes (`<A "line" 0x0A "next">`) instead of an
escape syntax; the parser also takes `[n]` sizes (checked), hex or decimal numbers and
`T`/`F`/`TRUE`/`FALSE`.

### GEM host (`GemHost`)

| Helper | Messages |
| --- | --- |
| `are_you_there` | S1F1 / S1F2 |
| `establish_communications` | S1F13 / S1F14 |
| `request_offline`, `request_online` | S1F15 / S1F16, S1F17 / S1F18 |
| `status_variables`, `status_variable_names` | S1F3 / S1F4, S1F11 / S1F12 |
| `equipment_constants`, `set_equipment_constants`, `equipment_constant_names` | S2F13 / S2F14, S2F15 / S2F16, S2F29 / S2F30 |
| `define_reports`, `link_event_reports`, `enable_events` | S2F33 / S2F34, S2F35 / S2F36, S2F37 / S2F38 |
| `remote_command` | S2F41 / S2F42 |
| `enable_alarms`, `list_alarms` | S5F3 / S5F4, S5F5 / S5F6 |
| `process_program_list` | S7F19 / S7F20 |
| `terminal_display` | S10F3 / S10F4 |
| `send_raw`, `send_sml` | anything else |

Received and answered automatically, and published on `subscribe()` as `HostEvent`: S6F11 event
report (S6F12 `<B 0>`), S5F1 alarm report (S5F2), S10F1 terminal request (S10F2), S1F1 (S1F2
`<L>`), S1F13 (S1F14 with `COMMACK` 0); S9Fx messages from the equipment; unknown primaries get
S9F5 (known stream) or S9F3 (unknown stream) and appear as `HostEvent::Unhandled`.

Acks are typed enums (`Hcack`, `Eac`, `Drack`, `Lrack`, `Erack`, `OnlAck`, `Ackc5`, ...); an
equipment refusing is a normal answer. Errors (`SecsError`) are: `Timeout(T3|T5|T6|T7|T8)`,
`S9(S9Error)` (an S9Fx the equipment sent about this very transaction, matched by the system bytes
in its `MHEAD`, so it fails at once instead of after T3), `Rejected { reason }` (Reject.req),
`Aborted` (`S{n}F0`), `SelectRefused`, `UnexpectedReply` (the reply is not the documented shape),
codec and I/O errors. The id item type the host sends is `IdFormat` (default U4); every integer
width is accepted on receipt.

### Equipment simulator (`EquipmentSimulator`)

Passive on `127.0.0.1:5000`, device id 0, by default (`SimulatorConfig`); `SimMode::Active` connects
to a host instead and reconnects after T5. One selected session at a time; a second connection is
closed. Simulated time advances on a tick (`tick_ms`) or by hand (`advance`).

- **Control state** (E30): equipment offline, attempt online, host offline, online local, online
  remote (`ControlState`, SV 2). S1F15 takes an online tool to host offline, S1F17 takes host offline
  online (`ONLACK` 2 when already online, 1 when the operator holds it offline); the operator's
  side is `operator_offline`, `operator_online`, `operator_set_remote`. Attempt online succeeds when
  the host is communicating and otherwise falls back to equipment offline. While equipment offline,
  S1F1 is answered with S1F0.
- **Communication state**: disabled, wait CRA, wait delay, communicating. After Select the
  simulator sends S1F13 until the host answers `COMMACK` 0, waiting the establish-communications
  timeout (EC 1005) between attempts (`initiate_communication`, on by default); a host S1F13 is
  answered at once. Events and alarms are only sent while communicating and online.
- **Tool**: a chamber with process state (idle, executing, paused), pressure and temperature that
  follow the process setpoints, wafer count, lot id and active PPID. Remote commands `START`
  (`PPID`, `LOTID`), `STOP`, `PAUSE`, `RESUME` (only online remote; `HCACK` 2 otherwise).
- **SVIDs** 1 Clock, 2 ControlState, 3 PreviousControlState, 4 ProcessState, 5
  PreviousProcessState, 6 CommunicationState, 10 ChamberPressure, 11 ChamberTemperature, 12
  WaferCount, 13 ActivePPID, 14 ProcessElapsed, 15 LotID, 16 AlarmsSet.
- **ECIDs** with ranges (1001 ProcessTemperature 20-400, 1002 ProcessTime 1-3600, 1003
  ProcessPressure 1-1000, 1004 MaxWafersPerLot 1-25, 1005 EstablishCommTimeout 1-120). S2F15 checks
  existence (`EAC` 1), busy (2; the three process constants are locked while a process runs) and
  range/type (3) for the whole message before applying any of it; a number of another type is
  converted when it fits.
- **CEIDs** 100 ProcessStarted, 101 ProcessCompleted, 102 ProcessStopped, 103 ProcessPaused, 104
  ProcessResumed, 201-204 control state entered (offline, local, remote, host offline), 300 AlarmSet,
  301 AlarmCleared. All start disabled; S2F37 enables them.
- **Reports**: S2F33 and S2F35 are stored as the spec describes (`DRACK` 1-4, `LRACK` 1-5, empty
  lists delete, a report with no VIDs deletes it). VIDs are status variables and equipment
  constants. An enabled event is sent as S6F11 with the values of that moment, linked or not.
- **Alarms** (ALID with ALCD and ALTX): 1 chamber over temperature (category 2), 2 door open (1), 3
  pump warning (6). `raise_alarm` / `clear_alarm` send S5F1 (set bit in `ALCD`) when the alarm is
  enabled (S5F3), then fire the AlarmSet / AlarmCleared event. S5F5 lists them.
- S7F19 process program list (`ppids`), S10F3 terminal display (kept in `terminal_messages`).
- Unknown streams and functions get S9F3 / S9F5, malformed text gets S9F7, a wrong device id gets
  S9F1.
- The model is `Serialize`/`Deserialize`; `with_model` changes it under the lock the handlers use
  and sends what the change queued. `subscribe_log` streams every message.

### Not implemented

Spooling (S6F23 and the spool state model: events during a lost connection are dropped), trace
data (S2F23, S6F1), limits monitoring (S2F45), data variables as a separate id space, multi-block
messages (S6F5), S6F15/S6F19 event report requests, S1F21-S1F23, S2F17/S2F31 clock, S2F21 remote
command without parameters, S2F43 spooling streams, process program transfer (S7F1-S7F6, S7F17),
S5F7, S9F9/S9F11/S9F13 generation, the E37 selected-state edge cases not listed above, HSMS-SS
generic sessions, TLS, SECS-I (serial), and every other stream (3, 4, 14, 16, 17...). Use
`send_raw` for the other messages; the simulator answers them with S9F5.

## What was verified

- `cargo test -p mt-secsgem`: codec round trips for every item type, byte-exact layouts, the
  three length-byte forms, malformed input; SML render/parse round trip (single line and
  indented); byte-exact HSMS control handshakes, reject cases, framing (split, coalesced,
  byte by byte), typed select/abort/S9/reject errors; **T3, T6, T7, T8 and periodic linktest with a
  paused tokio clock** (exact durations); T5 spacing; host and simulator over loopback TCP for
  every helper (establish communications, online/offline, SV read and names, EC read/names/set with
  range, busy and type checks, define/link/enable then S6F11 on process start and completion with
  the right values, alarms set/clear/enable/list, remote commands, process program list, terminal
  display, S9 errors, equipment-initiated S1F13 with retry, active-mode simulator with T5
  reconnect, model changes while running, log stream).
- **Third-party cross-check**, run once with
  [bparzella/secsgem](https://github.com/bparzella/secsgem) 0.3.0 (installed into a temporary
  virtualenv, deleted afterwards; nothing of it is in this repository):
  - *secsgem as host, this simulator as equipment* (simulator in `--run` mode): S1F13/S1F14 in both
    directions, S1F1, S1F3, S1F11, S2F13, S2F29, S2F15 (accepted, out of range = `EAC` 3, unknown =
    `EAC` 1), S7F19, S1F15, S1F17 (`ONLACK` 0 then 2), `subscribe_collection_event` (S2F33, S2F35,
    S2F37) followed by decoded S6F11 reports with the right values, S2F41 STOP/START/unknown
    (`HCACK` 0/0/1), S5F5, S5F3 then a received S5F1, S10F3. All 21 checks passed.
  - *this host against secsgem as equipment* (`GemEquipmentHandler` with two SVs, one EC with a range,
    two events, one alarm and a remote command): S1F13 both ways, S1F1, S1F3, S1F11, S2F13, S2F29, S2F15
    (accepted, out of range, unknown), S2F33, S2F35, S2F37 then a received S6F11 with both SV values, S5F3
    (known and unknown alarm), S5F5, a received S5F1 (which secsgem sends without the W-bit), S2F41
    (`HCACK` 4 for the registered command, 1 for an unknown one, 3 for a bad parameter). All 21
    checks passed; S1F17, S1F15 and S1F17 were also sent and answered (printed, not asserted). S7F19 and S10F3, which secsgem's equipment does not implement,
    came back as `S9(UnrecognizedFunction)`, as intended.
  - What it showed: secsgem leaves the W-bit clear on S5F3, S5F1 and some others and still waits
    for the reply, so both sides here answer recognized primaries without the W-bit too (a reply nobody
    expects is harmless; a missing one is a hang). When both sides send S1F13 at once secsgem logs
    "unexpected function s01f14" and carries on. secsgem encodes EC min/max as I8 and equipment
    constant values as the type declared in Python; this crate returns whatever type arrives.
- **Not verified:** anything against real equipment or a commercial host, the exact wording of
  any SEMI document, Windows/macOS runs (the tests ran on Linux).

### Inferred, not from a source I could check

- E37 details taken from memory and secsgem rather than the standard: Select.rsp echoes the
  request's session id; Reject.req uses the rejected message's session id and system bytes, with
  byte 2 the rejected PType/SType (0 for a data message) and byte 3 the reason; the
  Reject for a reply with no open transaction; T7 also applies after Deselect.
- GEM behaviours that real equipment may do differently: unknown SVID/ECID values come back as a
  zero-length list, unknown names as empty strings; S5F3 with a zero-length `ALID` means all alarms;
  S1F1 in equipment offline is aborted; control-state CEIDs are the only events sent while host
  offline; the `HCACK` choices for STOP/PAUSE/RESUME ("already in condition" is 5); the ids,
  names, ranges, alarm texts and process dynamics are made up to look plausible.
- Ack code names (`Hcack`, `Eac`, ...) and meanings follow the secsgem data-item documentation.

## Naming

SECS/GEM, SECS-II, HSMS and the SEMI standards E5, E30 and E37 belong to **SEMI**. `mt-secsgem` is an
independent implementation; it is not affiliated with, endorsed by or certified by SEMI, and it
contains no text from the SEMI standards. Passing its tests says nothing about SEMI compliance.

## Sources

- [bparzella/secsgem](https://github.com/bparzella/secsgem) (Python, LGPL-2.1-or-later): stream and
  function layouts, data item meanings, ack codes, the state models and the HSMS header layout
  were read from its source and ported to Rust; used as the cross-check peer. See `NOTICE`.
- The SML notation as printed by secsgem and the usual SECS tools (`S1F13 W <L ...>.`).
- Public descriptions of HSMS (E37), SECS-II (E5) and GEM (E30), as reflected in the source above.
  The standards themselves are available from [semi.org](https://www.semi.org) and were not used.

This crate is licensed GPL-3.0-or-later like the rest of the workspace.
