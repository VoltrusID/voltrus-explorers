# CAN Explorer changelog

## 1.2.0 (2026-11-18)

Adapters that tell the truth, recordings that keep every frame, DBC files that load, and updates inside the app.

- **Updates inside the app.** CAN Explorer checks for a new version when it starts and shows what is new; "Install and
  restart" updates it in place. With a bus connected, a recording running or the simulator on, it installs when you quit
  instead, so nothing is cut. About has "Check for updates". The direct download is no longer sandboxed (a sandboxed app
  cannot replace itself); your saved profiles, transmit lists and licence carry over.
- **SLCAN adapters are identified before use.** The app asks the adapter for its version and refuses to connect when
  nothing answers (a wrong port, or a CANable running candleLight firmware: use gs_usb), instead of showing a silent bus.
  CANable and Lawicel/USBtin firmware number their bit rates differently (S7 is 750 kbit/s on CANable, 800 kbit/s on
  Lawicel); each gets its own table, and settings an adapter does not have are refused. CAN FD data rates on SLCAN are
  2 and 5 Mbit/s, the ones CANable 2 has. Listen only is refused on CANable slcan firmware, which would still acknowledge
  frames; it works on Lawicel and USBtin, and on candleLight through gs_usb.
- **No repeated frames on reconnect.** The adapter is closed when the app quits, and a connection starts by closing the
  channel instead of sending empty lines, which made a CANable send its last frame again.
- **Bus state and error counters.** Error warning, error passive and bus off are shown with the transmit and receive error
  counters (gs_usb and SocketCAN). Error storms, such as sending with no other node on the bus, are summarised instead of
  flooding the log, and frames nobody acknowledges fail after 2 s with a hint to check wiring, termination and bit rate.
  CAN FD frames are refused on a classic connection. gs_usb: 8 Mbit/s data rate on 40 MHz boards, FD boards read their
  data-phase limits correctly, and unplugging the adapter ends the connection cleanly.
- **Recording keeps every frame.** Frames are written to the file as they arrive, not from what the screen keeps up with,
  and a recording replaces the file you chose instead of appending to it.
- **Replay.** A log whose timestamps go backwards no longer hangs; Stop still works after switching tabs; disconnecting
  stops a replay; a one-frame log no longer loops flat out.
- **Opening trace files.** Plain `candump` output, logs without timestamps, Vector ASC, PCAN TRC and the app's own CSV open
  as well as candump logs; bad lines are skipped and counted instead of rejecting the file. Large files open in pages of
  10,000 frames, so the whole file can be read.
- **Exact cyclic rates.** A 1 ms frame goes out 1,000 times a second and a 10 ms frame 100 times (they ran slow on a busy
  machine). An empty or zero period is refused, and a cyclic frame can stop after a set count. Remote frames keep their
  DLC. The simulator keeps its message rates too, and its DBC now covers every frame it sends.
- **DBC files.** Extended multiplexing (`SG_MUL_VAL_`, nested switches), files that used to load with no messages (unusual
  `NS_` blocks, old Mac line endings), files with a bad line (now a warning with its line number), Windows-1252 text from
  CANdb++ (°C, µ), long names, cycle-time defaults, duplicate IDs, and signals wider than 53 bits (exact values, also
  when sending). J1939 DBCs match by PGN, so a message decodes from any source address and priority.
- **Protocol labels.** OBD-II multi-frame answers are reassembled (VIN, trouble codes), with negative responses and 29-bit
  addressing; J1939 transport protocol (BAM, RTS/CTS) is reassembled and DM1 shows its trouble codes; CANopen SDO, EMCY and
  node guarding are spelled out.
- **A faster Monitor.** Only the visible rows are drawn, so thousands of identifiers stay responsive; the table keeps the
  5,000 most recently seen IDs. Periods are measured over a second, so frames read in one batch no longer shrink them.
  The ID filter matches identifiers only; names and data bytes (`xx 12 xx`) have their own search.
- **Pro at launch.** A licensed Mac shows Pro straight away; the licence is checked in the background.
- **Linux SocketCAN.** A larger receive buffer, frames the kernel had to drop are counted, and frames carry the kernel's
  receive time.

## 1.1.1 (2026-11-16)

- **Simulator on SocketCAN (Linux).** The simulator can run on a SocketCAN interface such as `vcan0` instead of the in-app
  virtual bus (Simulator, Run on), so candump or your own code on the same machine sees its frames too.
- **Steady simulator rates.** Message times follow the clock. They used to count timer ticks, so a busy machine slowed every
  message down (0x4A0 every 200 ms instead of 100 ms).
- **Signals tab and multiplexed messages.** The table keeps the last value of every page of a multiplexed message, so all 64
  cell voltages of the battery pack show at once instead of the 16 of the latest frame.
- **CAN FD payloads in the Signals header** are shown 16 bytes a line under the message details instead of in one long badge,
  and the send editor starts from rounded values (3.917, not 3.9170000000000).

## 1.1.0 (2026-11-11)

CAN FD, all the way through.

- **CAN FD battery pack in the simulator.** A BMS on the virtual bus sends `BatteryPackFD` (0x4A0) every 100 ms: a 64-byte
  CAN FD frame with bit rate switch carrying pack voltage, current, state of charge, state of health, temperatures, the
  lowest and highest cell with their numbers, pack state, contactor, balancing and insulation resistance, plus 16 of the
  64 cell voltages (multiplexed, four pages). It is in the simulator's DBC, so the Signals tab decodes it, and it can be
  switched off on the simulator dashboard (on by default). The J1939, CANopen and OBD-II traffic is unchanged.
- **Error state indicator (ESI).** Read from SocketCAN and gs_usb adapters, shown as a badge in the monitor and the trace,
  and kept in candump logs (the flags digit after `##`: 1 = BRS, 2 = ESI).
- **DBC frame formats.** `VFrameFormat` (StandardCAN_FD, ExtendedCAN_FD) and `CANFD_BRS`, with their defaults, mark a message
  as CAN FD with or without bit rate switch. Sending a message by signal values uses them (an 8-byte FD message used to go
  out as classic CAN, and BRS was always off).
- **64-byte payloads in the UI.** Long payloads wrap in lines of 16 bytes in the monitor; the trace detail shows ESI and the
  length; the Signals tab shows a CAN FD badge and the cansend form of the frame it will send.

## 1.0.0 (2026-11-10)

First release: CAN and CAN FD monitor through SLCAN and gs_usb (candleLight) adapters and Linux SocketCAN, DBC decoding,
J1939, CANopen and OBD-II labels, and a built-in bus simulator.
