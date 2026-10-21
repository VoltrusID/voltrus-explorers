export interface ModbusConfig {
  host: string;
  port: number;
  unit_id: number;
  timeout_ms: number;
  poll_interval_ms: number;
  endianness?: string;
}

export interface ReadBlock {
  name: string;
  address: number;
  quantity: number;
  type: string;
  area: string;
  register_area?: string;
  scale?: number;
}

export interface SlaveLogEvent {
  timestamp: number;
  function_code: number;
  address: number;
  quantity: number;
  message: string;
}

export type DisplayFormat = "dec" | "hex" | "bin";

export interface ConnProfile {
  name: string;
  transport: "tcp" | "rtu";
  tcp: { host: string; port: number; unitId: number; pollInterval: number };
  rtu: { port: string; baudRate: number; dataBits: number; stopBits: number; parity: string; unitId: number };
  blocks: ReadBlock[];
}

export type ScanResult = { ip: string; port_open: boolean; hostname: string | null };
