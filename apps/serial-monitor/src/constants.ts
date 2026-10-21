import type { FrameTemplate } from "./types";

export const BAUD_PRESETS = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];
export const PARITY_OPTIONS = ["none", "even", "odd"];
export const DATA_BITS_OPTIONS = [5, 6, 7, 8];
export const STOP_BITS_OPTIONS = [1, 2];

export const TEMPLATES: Record<string, FrameTemplate> = {
  modbus_master: {
    name: "Modbus RTU Master",
    description: "Simulates a Modbus RTU master polling holding registers",
    frames: [
      { direction: "TX", label: "FC03 Read Request", hexData: "01 03 00 00 00 0A C5 CD", intervalMs: 3000 },
      { direction: "RX", label: "FC03 Read Response", hexData: "01 03 14 09 C4 00 00 05 DC 00 00 04 57 00 00 00 00 00 00 00 00 00 00 00 00 7A F1", intervalMs: 3200 },
      { direction: "TX", label: "FC03 Read Request", hexData: "01 03 00 0A 00 05 A4 0D", intervalMs: 6000 },
      { direction: "RX", label: "FC03 Read Response", hexData: "01 03 0A 00 00 00 64 00 00 00 C8 00 01 6D B3", intervalMs: 6200 },
    ],
  },
  modbus_slave: {
    name: "Modbus RTU Slave Response",
    description: "Simulates a Modbus RTU slave responding to master requests",
    frames: [
      { direction: "RX", label: "FC03 Request (recv)", hexData: "01 03 00 00 00 0A C5 CD", intervalMs: 3000 },
      { direction: "TX", label: "FC03 Response (send)", hexData: "01 03 14 09 C4 00 00 05 DC 00 00 04 57 00 00 00 00 00 00 00 00 00 00 00 00 7A F1", intervalMs: 3500 },
      { direction: "RX", label: "FC06 Write Request (recv)", hexData: "01 06 00 00 00 64 89 BD", intervalMs: 8000 },
      { direction: "TX", label: "FC06 Write Echo (send)", hexData: "01 06 00 00 00 64 89 BD", intervalMs: 8300 },
    ],
  },
};
