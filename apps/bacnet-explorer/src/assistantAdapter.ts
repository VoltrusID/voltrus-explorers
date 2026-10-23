// BACnet Explorer's side of the Assistant (Ask and Diagnose only): what it tells the Assistant about the
// session. Pure, so it can be unit tested.
import type { AssistantContext } from "@mactools/ui";

/** A failure message as the app shows it, as the Assistant's error: a timeout (no BACnet answer) or something else. */
export function bacnetError(message: string | null | undefined): AssistantContext["error"] {
  if (!message) return undefined;
  const kind = /time(d)? ?out|no (bacnet )?response|did not answer/i.test(message) ? "timeout" : "other";
  return { kind, message: message.slice(0, 300) };
}

export interface BacnetSessionFacts {
  /** The open device, or null. Its address is never sent: the Assistant only needs the instance and port. */
  device: { port: number; device_id: number } | null;
  openDevices: number;
}

export function bacnetConfig(f: BacnetSessionFacts): Record<string, unknown> {
  return {
    ...(f.device && { device_instance: f.device.device_id, udp_port: f.device.port }),
    open_devices: f.openDevices,
  };
}
