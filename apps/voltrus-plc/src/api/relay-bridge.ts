// Cloud-mode command handlers for the device relay (PLC-23). Registered via
// the bridge extension registry, so cloud-mode invoke() routes the target/
// deploy command surface onto a RelayClient WebSocket while tauri mode keeps
// using the native commands untouched.
//
// Imports only the bridge-core leaf (never ./bridge, which side-effect-imports
// this module), so registration runs synchronously when the module loads.
import { BridgeError, emitLocalEvent, registerCloudCommands } from "./bridge-core";
import { RelayClient } from "./relay-client";
import type { DeployResult, IoMapping, ScadaRegistration, TargetStatus } from "../types";

// One relay session per browser tab; deploy CRC persists across reconnects
// like the desktop app's last_deployed_crc32 does across target sessions.
let client: RelayClient | null = null;
let lastDeployedCrc32: number | null = null;

function selectedDeviceId(cmd: string): string {
  const id = localStorage.getItem("vplc_device");
  if (!id) {
    throw new BridgeError("bad_request", cmd, "No relay device selected (vplc_device is not set)");
  }
  return id;
}

function requireClient(cmd: string): RelayClient {
  if (!client) {
    throw new BridgeError("bad_request", cmd, `Not connected to a relay device (${cmd})`);
  }
  return client;
}

/** Status payload the relay drivers report: flat for echo, nested for grpc. */
interface RelayStatusPayload {
  running?: boolean;
  runtime?: { status?: string | null; cycle_count?: number | null };
  deployed_crc32?: number | null;
}

function mapStatus(payload: unknown): TargetStatus {
  const p = (payload ?? {}) as RelayStatusPayload;
  return {
    connected: true,
    ip: localStorage.getItem("vplc_device"),
    runtime_state: p.runtime?.status ?? (p.running ? "running" : null),
    cycle_count: p.runtime?.cycle_count ?? null,
    deployed_crc32: lastDeployedCrc32,
    running_crc32: typeof p.deployed_crc32 === "number" ? p.deployed_crc32 : null,
  };
}

// The relay wire protocol has no start/stop/step ops yet — fail loudly rather
// than pretending the remote runtime changed state.
function unsupportedOverRelay(cmd: string): Promise<never> {
  return Promise.reject(
    new BridgeError("bad_request", cmd, `${cmd} is not supported over the device relay yet`),
  );
}

async function connectAndReadStatus(cmd: string): Promise<TargetStatus> {
  // Never accumulate sockets: close any previous client before dialing.
  if (client) {
    client.close();
    client = null;
  }
  const next = new RelayClient();
  next.onEvent((event) => emitLocalEvent(event.event, event.payload));
  next.onClose(() => {
    if (client === next) {
      client = null;
      // Unexpected drop only (deliberate close detaches onclose): mirror the
      // desktop `target-disconnected` event the store listens for.
      emitLocalEvent("target-disconnected", { ip: selectedDeviceId(cmd) });
    }
  });
  try {
    await next.connect(selectedDeviceId(cmd), localStorage.getItem("vplc_token") ?? "");
  } catch (e) {
    next.close();
    throw new BridgeError("network", cmd, e instanceof Error ? e.message : String(e));
  }
  client = next;
  try {
    return mapStatus(await next.request("status"));
  } catch (e) {
    next.close();
    if (client === next) client = null;
    throw new BridgeError("network", cmd, e instanceof Error ? e.message : String(e));
  }
}

async function deployOverRelay(cmd: string, args?: Record<string, unknown>): Promise<DeployResult> {
  const relay = requireClient(cmd);
  const module = args?.module;
  if (!module || typeof module !== "object") {
    throw new BridgeError("bad_request", cmd, `${cmd} requires a compiled module (args.module)`);
  }
  // Same proto IoMapping shape the desktop `deploy_via_grpc` puts in the
  // config JSON, so the Linux runtime wires its IoManager identically.
  const mappings = (Array.isArray(args?.ioMappings) ? args?.ioMappings : []) as IoMapping[];
  const configJson = JSON.stringify({
    io_mappings: mappings.map((m) => ({
      variable_name: m.variable,
      address: m.address,
      driver: m.driver_id,
      connection_json: m.protocol_address,
    })),
  });
  let payload: unknown;
  try {
    payload = await relay.request("deploy", { module_json: JSON.stringify(module), config_json: configJson });
  } catch (e) {
    // device_offline and friends propagate verbatim so logs show the cause.
    throw e instanceof BridgeError ? e : new BridgeError("network", cmd, e instanceof Error ? e.message : String(e));
  }
  const result = (payload ?? {}) as { bytes?: unknown; crc32?: unknown };
  const crc32 = typeof result.crc32 === "number" ? result.crc32 : null;
  if (crc32 !== null) lastDeployedCrc32 = crc32;
  return {
    success: true,
    message: "Deployed via relay",
    byte_count: typeof result.bytes === "number" ? result.bytes : null,
    crc32,
  };
}

// Browser mirror of commands::target::register_scada_device: same endpoint,
// same payload, so the SCADA registration works identically in cloud mode.
async function registerScada(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  const reg = args?.registration as ScadaRegistration | undefined;
  if (!reg || typeof reg.scada_url !== "string") {
    throw new BridgeError("bad_request", cmd, `${cmd} requires a registration object`);
  }
  const base = reg.scada_url.trim().replace(/\/+$/, "");
  if (!base.startsWith("http://") && !base.startsWith("https://")) {
    throw new BridgeError("bad_request", cmd, "SCADA URL must start with http:// or https://");
  }
  let res: Response;
  try {
    res = await fetch(`${base}/api/v1/voltrus-plc/devices`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: reg.device_name,
        url: reg.device_url,
        transport: reg.transport,
        interval_ms: reg.interval_ms,
      }),
    });
  } catch (e) {
    throw new BridgeError("network", cmd, `SCADA registration failed: ${e instanceof Error ? e.message : e}`);
  }
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new BridgeError("network", cmd, `SCADA registration failed (HTTP ${res.status}): ${body}`);
  }
  return res.json();
}

registerCloudCommands({
  target_connect: (cmd) => connectAndReadStatus(cmd),
  target_disconnect: async () => {
    client?.close();
    client = null;
  },
  target_status: async (cmd) => {
    const relay = requireClient(cmd);
    try {
      return mapStatus(await relay.request("status"));
    } catch (e) {
      throw new BridgeError("network", cmd, e instanceof Error ? e.message : String(e));
    }
  },
  // Both desktop deploy commands map onto the single relay deploy op; the
  // agent-side driver picks the transport encoding.
  deploy_to_target: (cmd, args) => deployOverRelay(cmd, args),
  deploy_via_grpc: (cmd, args) => deployOverRelay(cmd, args),
  target_start: (cmd) => unsupportedOverRelay(cmd),
  target_stop: (cmd) => unsupportedOverRelay(cmd),
  target_step: (cmd) => unsupportedOverRelay(cmd),
  grpc_start: (cmd) => unsupportedOverRelay(cmd),
  grpc_stop: (cmd) => unsupportedOverRelay(cmd),
  grpc_step: (cmd) => unsupportedOverRelay(cmd),
  // Device-side concern; the store fire-and-forgets this in cloud mode.
  set_target_kind: async () => undefined,
  // No LAN visibility from the cloud: an empty list, not an error.
  scan_mdns_targets: async () => [],
  register_scada_device: (cmd, args) => registerScada(cmd, args),
});
