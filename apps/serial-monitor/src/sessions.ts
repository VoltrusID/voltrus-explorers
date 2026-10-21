import type { PortInfo, SerialConfig, Session, TrafficEntry } from "./types";

/** Ports the free tier may have open at once; more needs Pro. */
export const FREE_SESSION_LIMIT = 1;
/** Traffic rows kept per session. */
export const TRAFFIC_ROWS = 200;

export function defaultConfig(portName = ""): SerialConfig {
  return { port_name: portName, baud_rate: 115200, data_bits: 8, stop_bits: 1, parity: "none", timeout_ms: 1000 };
}

export function newSession(id: string, config: SerialConfig): Session {
  return {
    id,
    config,
    connected: false,
    connecting: false,
    lastDataTime: null,
    traffic: [],
    sendData: "",
    sendFormat: "ascii",
    repeating: false,
    repeatMs: 1000,
  };
}

/** `/dev/cu.usbserial-1410` -> `usbserial-1410`; `COM3` stays `COM3`. */
export function shortPortName(name: string): string {
  return name.replace(/^\/dev\/(cu|tty)\./, "").replace(/^\/dev\//, "") || "No port";
}

/** `115200 8N1` */
export function configSummary(c: SerialConfig): string {
  const parity = c.parity === "even" ? "E" : c.parity === "odd" ? "O" : "N";
  return `${c.baud_rate} ${c.data_bits}${parity}${c.stop_bits}`;
}

/** First listed port that no other session has picked, or "" when every port is taken. */
export function pickFreePort(ports: PortInfo[], sessions: Session[]): string {
  const taken = new Set(sessions.map((s) => s.config.port_name));
  return ports.find((p) => !taken.has(p.port_name))?.port_name ?? "";
}

export function appendTraffic(rows: TrafficEntry[], entry: TrafficEntry): TrafficEntry[] {
  return [...rows.slice(-TRAFFIC_ROWS), entry];
}
