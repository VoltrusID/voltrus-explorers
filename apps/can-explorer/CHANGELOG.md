# CAN Explorer changelog

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
