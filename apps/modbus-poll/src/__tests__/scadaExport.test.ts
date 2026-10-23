import { describe, expect, it } from "vitest";
import { exportTags } from "@mactools/export";
import { buildTagList, type ExportConnection } from "../scadaExport";
import type { ReadBlock } from "../types";

const TCP: ExportConnection = {
  transport: "tcp",
  tcpFormValues: { host: "10.0.0.5", port: 1502, unitId: 4, pollInterval: 2000 },
  rtuPort: "/dev/ttyUSB0", rtuBaud: 9600, rtuDataBits: 8, rtuStopBits: 1, rtuParity: "none", rtuUnitId: 1, rtuPollInterval: 1000,
  endianness: "little",
};
const BLOCKS: ReadBlock[] = [
  { name: "Voltage", address: 3000, quantity: 2, type: "float32", area: "holding" },
  { name: "Run", address: 0, quantity: 2, type: "uint16", area: "coil" },
];

describe("buildTagList", () => {
  it("expands blocks into one tag per value with wire addresses and connection settings", () => {
    const list = buildTagList(BLOCKS, "Meter", TCP);
    expect(list.source).toEqual({ name: "Meter", transport: "tcp", host: "10.0.0.5", port: 1502, unitId: 4, pollMs: 2000, wordOrder: "little" });
    expect(list.tags.map((t) => [t.name, t.address, t.type, t.area, t.writable])).toEqual([
      ["Voltage_3000", 3000, "float32", "holding", true],
      ["Voltage_3002", 3002, "float32", "holding", true],
      ["Run_000", 0, "bool", "coil", true],
      ["Run_001", 1, "bool", "coil", true],
    ]);
  });

  it("carries serial settings for RTU", () => {
    const list = buildTagList(BLOCKS, "Pump", { ...TCP, transport: "rtu", rtuParity: "even", rtuBaud: 19200, rtuUnitId: 9, rtuPollInterval: 500 });
    expect(list.source).toMatchObject({ transport: "rtu", unitId: 9, pollMs: 500, serial: { port: "/dev/ttyUSB0", baudRate: 19200, parity: "even" } });
  });

  it("feeds every exporter without throwing", () => {
    const list = buildTagList(BLOCKS, "Meter", TCP);
    for (const id of ["csv", "json", "ignition", "kepware", "rustred", "nodered", "homeassistant", "voltrus-scada", "voltrus-gateway"] as const) {
      expect(exportTags(id, list).content.length).toBeGreaterThan(0);
    }
  });
});
