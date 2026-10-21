import type { LogEntry } from "@mactools/ui";

export interface BacnetConfig {
  id: string;
  ip: string;
  port: number;
  device_id: number;
}

export interface BacnetObject {
  name: string;
  object_type: string;
  object_instance: number;
  property: string;
}

export interface BacnetDiscoveredDevice {
  device_instance: number;
  ip: string;
  port: number;
  vendor_name: string | null;
  model_name: string | null;
  firmware_version: string | null;
  object_list_count: number | null;
}

export interface BacnetObjectInfo {
  object_type: string;
  object_instance: number;
  object_name: string | null;
}

export interface BacnetPropertyEntry {
  property_name: string;
  property_identifier: number;
  value: BacnetValue;
  raw_value: string;
}

export type BacnetValue =
  | { Real: number }
  | { Double: number }
  | { Unsigned: number }
  | { Boolean: boolean }
  | { Enum: number }
  | { Float64: number }
  | { String: string }
  | { Null: null };

export interface BacnetObjectDetail {
  object_type: string;
  object_instance: number;
  object_name: string | null;
  description: string | null;
  present_value: string | null;
  units: string | null;
  units_symbol: string | null;
  status_flags: string | null;
  reliability: string | null;
  out_of_service: boolean | null;
  properties: BacnetPropertyEntry[];
}

export interface DeviceInspection {
  device_instance: number;
  object_name: string | null;
  vendor_name: string | null;
  vendor_id: number | null;
  model_name: string | null;
  firmware_revision: string | null;
  application_software_version: string | null;
  description: string | null;
  location: string | null;
  protocol_version: number | null;
  protocol_revision: number | null;
  max_apdu_length: number | null;
  segmentation_supported: string | null;
  apdu_timeout: number | null;
  number_of_apdu_retries: number | null;
  device_address_binding: string | null;
  object_count: number | null;
  system_status: string | null;
  local_time: string | null;
  local_date: string | null;
  database_revision: number | null;
  services_supported: string | null;
  object_types_supported: string | null;
}

export interface SimPoint {
  key: string;
  label: string;
  object_type: string;
  object_instance: number;
  property: string;
  base: number;
  variance: number;
  noise: number;
  period: number;
  min?: number;
  max?: number;
  decimals?: number;
  unit?: string;
  isBool?: boolean;
}

export interface DeviceTemplate {
  name: string;
  description: string;
  points: SimPoint[];
}

/** A simulated object definition passed to the Rust device server. Mirrors the
 * `SimPointDef` / `SimObj` shape in `crates/mt-bacnet/src/device.rs`. */
export interface SimPointDef {
  key: string;
  label: string;
  object_type: string;
  object_instance: number;
  base: number;
  variance: number;
  noise: number;
  period: number;
  min?: number;
  max?: number;
  decimals: number;
  unit?: string;
  is_bool: boolean;
  writable: boolean;
}

export interface SimInfo {
  device_id: number;
  port: number;
}

export interface CovSubscription {
  object_type: string;
  object_instance: number;
  lifetime: number;
  confirmed: boolean;
}

export interface CovNotificationValue {
  property_name: string;
  value: string;
  raw_value: string;
}

export interface CovNotificationEvent {
  subscriber_process_identifier: number;
  initiating_device_identifier: number;
  monitored_object_type: string;
  monitored_object_instance: number;
  time_remaining: number;
  values: CovNotificationValue[];
  timestamp: number;
}

export interface AlarmSummaryEntry {
  object_identifier: string;
  object_type: string;
  object_instance: number;
  alarm_state: string;
  acked_transitions: string;
}

export interface EventInfoEntry {
  object_identifier: string;
  object_type: string;
  object_instance: number;
  event_state: string;
  event_type: string;
  message_text: string;
  notify_type: string;
  time_stamp: string;
}

export function bacnetValueToNumber(v: BacnetValue): number {
  if ("Float64" in v) return v.Float64;
  if ("Real" in v) return v.Real;
  if ("Double" in v) return v.Double;
  if ("Unsigned" in v) return v.Unsigned;
  if ("Enum" in v) return v.Enum;
  if ("Boolean" in v) return v.Boolean ? 1 : 0;
  return NaN;
}

export function bacnetValueToString(v: BacnetValue): string {
  if ("String" in v) return v.String;
  if ("Null" in v) return "—";
  const num = bacnetValueToNumber(v);
  return isNaN(num) ? "—" : String(num);
}

export function formatObjectType(ot: string): string {
  const labels: Record<string, string> = {
    analogInput: "Analog Input",
    analogOutput: "Analog Output",
    analogValue: "Analog Value",
    binaryInput: "Binary Input",
    binaryOutput: "Binary Output",
    binaryValue: "Binary Value",
    multiStateInput: "Multi-State Input",
    multiStateOutput: "Multi-State Output",
    multiStateValue: "Multi-State Value",
    device: "Device",
    trendLog: "Trend Log",
    trendLogMultiple: "Trend Log Multiple",
    schedule: "Schedule",
    calendar: "Calendar",
    notificationClass: "Notification Class",
  };
  return labels[ot] || ot;
}
