export interface PortInfo {
  port_name: string;
  port_type: string;
  manufacturer: string | null;
  product: string | null;
  serial_number: string | null;
}

export interface SerialConfig {
  port_name: string;
  baud_rate: number;
  data_bits: number;
  stop_bits: number;
  parity: string;
  timeout_ms: number;
}

export interface TrafficEntry {
  [key: string]: string;
  direction: string;
  data: string;
  timestamp: string;
}

export interface FrameTemplate {
  name: string;
  description: string;
  frames: Array<{
    direction: "TX" | "RX";
    label: string;
    hexData: string;
    intervalMs: number;
  }>;
}

export type SidebarMode = "monitor" | "virtual";
