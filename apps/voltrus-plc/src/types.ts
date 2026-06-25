export interface Diagnostic {
  severity: "error" | "warning" | "info";
  message: string;
  line: number;
  col: number;
  end_line: number;
  end_col: number;
  source: string;
}

export interface Instruction {
  opcode: string;
  operand: number | null;
}

export interface CompiledModule {
  name: string;
  instructions: Instruction[];
  variable_slots: [string, unknown][];
}

export interface CompileResult {
  success: boolean;
  diagnostics: Diagnostic[];
  pou_count: number;
  variable_count: number;
  module: CompiledModule | null;
}

export interface RuntimeStateInfo {
  state: "stopped" | "running" | "paused" | "error";
  cycle_count: number;
  last_cycle_ms: number;
  avg_cycle_ms: number;
  max_cycle_ms: number;
  error_message: string | null;
  cycle_history: number[];
}

export interface RuntimeConfig {
  scan_cycle_ms: number;
  max_cycle_ms: number;
}

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

export interface OpenFile {
  name: string;
  path: string;
  content: string;
  modified: boolean;
}

export interface IoDriverConfig {
  protocol: string;
  host: string;
  port: number;
  unit_id: number;
  opcua_endpoint_url?: string;
  s7_rack?: number;
  s7_slot?: number;
  mqtt_topic_prefix?: string;
  mqtt_client_id?: string;
  enip_tag_prefix?: string;
}

export interface IoStatus {
  connected: boolean;
  protocol: string;
  host: string;
  port: number;
  unit_id: number;
  error: string | null;
}

export interface IoReadResult {
  success: boolean;
  address: number;
  value: string;
  error: string | null;
}

export interface IoMapping {
  variable: string;
  direction: "input" | "output";
  address: string;
  driver_id: string;
  protocol_address: string;
}

export interface CompiledVariable {
  name: string;
  type_name: string;
  index: number;
}

export interface WatchedVar {
  name: string;
  value: unknown;
  display_value: string;
  type_name: string;
  quality: "good" | "bad" | "forced";
}

export interface LogEntry {
  direction: "in" | "out" | "system";
  message: string;
  timestamp: number;
}

export interface SampleTemplate {
  id: string;
  name: string;
  description: string;
  category: string;
  source: string;
}

export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

export interface ProjectSource {
  name: string;
  path: string;
  content: string;
  is_active?: boolean;
}

export interface ProjectAiSettings {
  provider: string;
  model: string;
  ollama_endpoint: string;
}

export interface ProjectFile {
  version: string;
  files: ProjectSource[];
  io_config: IoDriverConfig | null;
  io_mappings: IoMapping[];
  scan_cycle_ms: number;
  max_cycle_ms: number;
  ai_settings: ProjectAiSettings | null;
  watch_list: string[];
  breakpoints: [string, number[]][];
  target_ip?: string;
  scada_url?: string;
  execution_mode?: "local" | "remote";
  created_at: string;
}

export interface TargetStatus {
  connected: boolean;
  ip: string | null;
  runtime_state: string | null;
  cycle_count: number | null;
}

export interface DeployResult {
  success: boolean;
  message: string;
  byte_count: number | null;
}

export interface ScadaRegistration {
  scada_url: string;
  device_name: string;
  device_url: string;
  transport: string;
  interval_ms: number;
}

export interface DiscoveredTarget {
  name: string;
  ip: string;
  port: number;
}

export type TargetKind = "websocket" | "grpc";
