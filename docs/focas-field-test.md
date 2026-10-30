# FANUC FOCAS field test with CNC Explorer

The FANUC connection of CNC Explorer (and of Voltrus Tools) is **Beta**: its byte layouts come from pyfanuc's reverse
engineering and it has only ever talked to its own simulator. One visit to a machine with the FOCAS Ethernet option settles
most of it. This page makes that visit short: set a machine state, press **Save capture** in CNC Explorer, compare the
summary with the CNC screen, and keep the files.

What you leave with, per machine state: `NAME.capture.json` (the raw frames), `NAME.report.json` (what the decoder made of them),
`NAME.summary.txt` (readable, to compare with the screen), and photos of the screen. The captures then become regression tests
of the decoder (see "Turning a capture into a regression test").

This is the CNC Explorer part of opex-flow's `docs/focas-field-test.md`, which also covers the command-line probe
(`opex-flow focas-probe`) and the comparison with Fanuc's own library. Both write the same capture and report formats.
Everything the app does is **read-only**: it never calls a write function (macro writes are a separate action on the Macros tab
and are not part of a capture).

## Where the code is

CNC Explorer and Voltrus Tools use `crates/focas-proto`, a vendored copy of opex-flow's crate of the same name
(`crates/focas-proto/VENDORED` says which commit; `scripts/sync-focas-proto.sh` refreshes it, see the README). The capture and the
replay harness are in that crate (`src/capture.rs`, `src/probe.rs`, `tests/replay.rs`); the **Capture** tab is
`apps/cnc-explorer/src/FanucView.tsx` and `apps/cnc-explorer/src-tauri/src/capture.rs`.

## What to bring

| Item | Why |
| --- | --- |
| Laptop that can sit on the CNC's network (cable and a free address in the CNC's subnet) | the CNC speaks FOCAS on TCP 8193; the laptop must reach it |
| CNC Explorer | records the captures and shows the live values |
| Phone or camera | photograph the CNC screen next to the laptop at every state |
| A machine owner or operator who agrees to the states below | alarm and e-stop states need them |

## On site

1. **Network.** Put the laptop in the CNC's subnet. `ping <cnc-ip>`; then check the port: `nc -vz <cnc-ip> 8193`
   (Windows: `Test-NetConnection <cnc-ip> -Port 8193`). A refused or silent port means the FOCAS Ethernet function is off, a
   firewall is in the way, or the CNC listens elsewhere: ask the machine builder for the port set in the CNC's Ethernet/FOCAS
   settings (`[UNCONFIRMED]` menu name; it differs by series). The FOCAS Ethernet option is a precondition.
2. **One client at a time.** Close every other FOCAS client. A control may accept only a few simultaneous connections
   (`[UNCONFIRMED]` how many for your series). CNC Explorer itself holds one connection for the live view and **Save capture**
   opens a second, short one (it ends before the recording is offered for saving). If the controller refuses the second session,
   the Capture tab shows the error: close the other FOCAS clients and try again. A control that allows only one session cannot be
   recorded while the live view is connected; if you meet one, note the series (`[UNCONFIRMED]` whether any does).
3. **Write down the machine:** series and version (Overview tab, Identity), the model, the options fitted, metric or inch, the
   axes. Photograph the CNC's system information screen.
4. **Connect** CNC Explorer: connection type FANUC, host, port 8193. Add the macro variables the machine builder uses to
   **Watch macros #** on the Overview tab (they join the live values and are added to the recording).

## Recording one machine state

For each state in the table below: set the state, let it settle, photograph the screen, then

1. open the **Capture** tab, pick the **Machine state now** that matches, and press **Save capture**;
2. the app records (a few seconds), checks that the recording replays to the same decoded result ("Replays exactly"), and asks
   where to save `<host>-<state>.capture.json`, then `.report.json`, then `.summary.txt` (one panel each; keep the same
   name before the extension, and rename to `<machine>-<state>` if you like). Keep the capture and its report together:
   the report is what the capture is replayed against;
3. read the notes under the badge. "Labelled X, but the CNC reports ..." means the picked state and the controller's status
   line disagree: fix the label or the machine before you share the files;
4. photograph the screen again if the values moved.

| # | State | How | What should show |
| --- | --- | --- | --- |
| 1 | Idle | Power-on, mode MEM, reset, no program running | run `****`, motion `***`, no alarm, spindle 0 |
| 2 | Running | MEM mode, a known harmless program, cycle start (air cut, spindle on) | run `STRT`, motion `MTN`, feed and spindle non-zero, sequence number advancing. Record twice, a few seconds apart |
| 3 | Feed hold | During 2, press feed hold | run `HOLD`, positions frozen |
| 4 | Alarm | An alarm the owner is happy to see, for example the `PARAMETER WRITE ENABLE` alarm (P/S 100) while parameter write is enabled, then switch it off again | alarm status `ALM`, `alarm bits` non-zero, `alarm messages` with number and text exactly as on the alarm screen. If two alarms are active, both |
| 5 | E-stop | Press e-stop | emergency `EMG` (the codes `RST` and `WAIT` also exist), run `****` |
| 6 | MDI | Mode MDI, idle; then a harmless block (a dwell such as `G04 X3`) | mode `MDI`; run state of an MDI block as the screen shows it; executing block text |
| 7 | JOG or HANDLE (optional) | Mode JOG, jog an axis a little | mode `JOG` / `HND`, relative and machine position change, absolute follows. Pick the closest label (Idle) and say so in the note |

Also, once: **multi-path or multi-spindle** machines, a lathe, an inch machine: any of these differ from the 3-axis mill the
simulator models; note it.

The recording reads: system info, status, axis names, absolute / machine / relative / distance-to-go positions, actual feed,
spindle speed, servo load, program numbers, sequence number, program name, executing block, alarm bits, alarm messages, macros
`#100 #500 #501` and the ones you watch, parameters `1020 3401 3741 6711 6712`, diagnosis `308 309`, PMC `X0 Y0 R0` bytes and
`D2204` word, then the batched `snapshot` and `extended_snapshot` calls. Refused items are not errors of the app: a macro or PMC
address the builder never used, or an option the control lacks, is what you want to see.

## What to compare with the CNC screen

`NAME.summary.txt` starts with "Compare with the CNC screen". Check each line against the physical screen at the same moment
(photo timestamps help). The labels are the ones on the status line of the CNC.

| Summary item | On the CNC | Check |
| --- | --- | --- |
| `mode`, `run state`, `motion`, `emergency`, `alarm status`, `edit` | status line at the bottom (`MEM`, `MDI`, `JOG`, `STRT`, `HOLD`, `MTN`, `EMG`, `ALM`...) | same abbreviation. An unknown code appears as `?` in the summary and `Unknown(n)` in the report: send it |
| `axis X ... absolute / machine / relative / distance to go` (every axis) | the position screen: absolute, machine, relative and "distance to go" pages | same value, same decimals, correct sign, correct axis letter and order. Inch machines: check the unit |
| `actual feed` | actual feed on the position or program-check screen | same number; note the unit the screen shows |
| `spindle speed` | actual spindle speed | same rpm |
| `servo load` | servo load meter (diagnosis or the load meter screen) | order of magnitude and per axis. Least certain read: its request argument was inferred. The Axes tab shows the same values live |
| `program running` / `program main`, `program name`, `sequence number` | program screen: `O` number, program name (30i and later), `N` number | same; sequence number advances while running. The Program tab shows them live |
| `executing block` | the block the program screen highlights | same text |
| `alarm bits`, `alarm <type> <number>` | alarm screen | same numbers, types, **text word for word**, axis number for axis alarms |
| `macro #n` | macro variable page | value; an empty variable shows as `invalid` |
| `parameter n` | parameter page | value; for bit parameters the byte; per-axis values in axis order |
| `diagnosis n` | diagnosis page | value |
| `pmc ...` | PMC data monitor (PMC address display) | value of that address |

Screen names and key paths differ by series; ask the operator for the page rather than guessing (`[UNCONFIRMED]` per series).

## What to send back

Per machine state: the three files, photos of the CNC screen (position pages: absolute, machine, relative, distance to go; the
status line; the program page; the alarm page; the system information page), and a short note: machine make and model, control
series and version, options fitted, metric or inch, which state each file set is, anything odd (a read that took long, a
freeze, a state you could not set).

**Privacy.** A capture holds program names, the executing block text, macro, parameter and diagnosis values, the controller's
identity and the machine's address, as hex frames. Send captures only with the machine owner's agreement. Do not edit the hex
(a replay needs the frames intact); to share less, remove `controller` and the like from `meta` only. The app shows this note on
the Capture tab and the summary file repeats it.

## Reading the results

- `FAIL ... CNC returned EW_FUNC / EW_NOOPT / EW_NUMBER / ...`: the control answered but refused. `EW_FUNC` is "function not
  available" (an older series lacks it: `program name` is 30i and later), `EW_NOOPT` a missing option, `EW_NUMBER` a number the
  control does not have, `EW_LENGTH` a size the control rejects (the client's payload assumptions are then suspect),
  `EW_BUSY`/`EW_RESET` transient.
- `unexpected payload`: the control answered with bytes the decoder does not understand. The capture holds them; this is the most
  valuable result of the visit.
- `Timeout` or `connection lost`: the recording stops and the notes say which call ("The probe stopped early"); the capture holds
  the request that was sent last.
- A decoded value that disagrees with the screen while the call is `ok`: a decoder fault. Note the call and the raw payload (in the
  capture, the response of the call with that label).

## Turning a capture into a regression test

A capture whose summary has been checked against the CNC screen can join the decoder's tests. In opex-flow:

1. Copy `NAME.capture.json` and `NAME.report.json` to `crates/focas-proto/tests/fixtures/<machine>-<state>.capture.json` and
   `.report.json`. Do not edit the hex.
2. `cargo test -p focas-proto --test replay` must pass. The test takes the probe options (macros, parameters, PMC ranges) from the
   report, so a capture with watched macros replays as recorded.
3. Fix the decoder if the summary disagreed with the screen; do not bless a report that is wrong. After a deliberate decoder change,
   `UPDATE_FIXTURES=1 cargo test -p focas-proto --test replay` rewrites the reports; review the diff against the photos.
4. Run `scripts/sync-focas-proto.sh` in this repository so that CNC Explorer and Voltrus Tools run the same fixtures.

## Order of a visit, as a checklist

- [ ] CNC Explorer connects to the built-in CNC simulator and **Save capture** writes three files that say "Replays exactly" (dry run)
- [ ] Laptop on the CNC network, `ping` and port 8193 answer
- [ ] Machine details and system information screen photographed
- [ ] No other FOCAS client connected
- [ ] 1 Idle: capture, photos, `<machine>-idle`
- [ ] 2 Running: capture twice, photos, `<machine>-running`
- [ ] 3 Feed hold: capture, `<machine>-feed-hold`
- [ ] 4 Alarm: capture, alarm screen photo, `<machine>-alarm`
- [ ] 5 E-stop: capture, `<machine>-estop`
- [ ] 6 MDI: capture, `<machine>-mdi`
- [ ] Every summary line compared with the screen; mismatches noted
- [ ] Live panels (Overview, Axes, Program, Alarms, watched macros) compared with the screen, screenshot
- [ ] Files zipped with the photos and the note; shared with the owner's agreement
