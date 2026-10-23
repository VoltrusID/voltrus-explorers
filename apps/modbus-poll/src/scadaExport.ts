import type { ModbusTag, ModbusTagList, WordOrder } from "@mactools/export";
import { buildReadPlan, isWritableArea } from "./helpers";
import type { ReadBlock } from "./types";

/** The connection settings an export needs, as the store holds them. */
export interface ExportConnection {
  transport: "tcp" | "rtu";
  tcpFormValues: Record<string, string | number>;
  rtuPort: string;
  rtuBaud: number;
  rtuDataBits: number;
  rtuStopBits: number;
  rtuParity: string;
  rtuUnitId: number;
  rtuPollInterval: number;
  endianness: string;
}

const wordOrder = (e: string): WordOrder => (e === "little" || e === "swapped" ? e : "big");

/**
 * What the app polls, as an export tag list: one tag per displayed value (`Voltage_3001`), with the 0-based wire address
 * the read plan uses. Coil and discrete tags are bits; a block's `scale` is not applied when polling, so tags carry 1.
 */
export function buildTagList(blocks: ReadBlock[], name: string, c: ExportConnection): ModbusTagList {
  const tags: ModbusTag[] = buildReadPlan(blocks).tags.map((t) => ({
    name: t.name,
    area: t.area as ModbusTag["area"],
    address: t.address,
    type: t.area === "coil" || t.area === "discrete" ? "bool" : (t.type as ModbusTag["type"]),
    scale: t.scale,
    writable: isWritableArea(t.area),
  }));
  const rtu = c.transport === "rtu";
  const parity = c.rtuParity === "even" || c.rtuParity === "odd" ? c.rtuParity : "none";
  return {
    protocol: "modbus",
    source: {
      name,
      transport: c.transport,
      host: rtu ? "" : String(c.tcpFormValues.host ?? ""),
      port: rtu ? 502 : Number(c.tcpFormValues.port ?? 502),
      unitId: rtu ? c.rtuUnitId : Number(c.tcpFormValues.unitId ?? 1),
      pollMs: rtu ? c.rtuPollInterval : Number(c.tcpFormValues.pollInterval ?? 1000),
      wordOrder: wordOrder(c.endianness),
      ...(rtu ? { serial: { port: c.rtuPort, baudRate: c.rtuBaud, dataBits: c.rtuDataBits, stopBits: c.rtuStopBits, parity } } : {}),
    },
    tags,
  };
}
