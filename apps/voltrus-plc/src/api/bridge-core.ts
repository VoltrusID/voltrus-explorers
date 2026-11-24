// Leaf module shared by bridge.ts and the feature bridges (relay, ai).
// It imports nothing from ./bridge, so feature modules can register their
// cloud-mode commands synchronously at load time without an import cycle.

export type BridgeErrorCode =
  | "cloud_unavailable"
  | "unauthorized"
  | "plan_required"
  | "not_found"
  | "bad_request"
  | "network";

export class BridgeError extends Error {
  constructor(
    public code: BridgeErrorCode,
    public cmd: string,
    message?: string,
  ) {
    super(message ?? `${code} (${cmd})`);
    this.name = "BridgeError";
  }
}

export type CloudCommandHandler = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

// Extension point for feature modules (relay, ai) that add cloud-mode command
// handlers without editing bridge.ts's switch. Register from your module:
//   registerCloudCommands({ my_cmd: (cmd, args) => ... })
// and side-effect-import the module from bridge.ts so it registers on load.
export const cloudCommandHandlers = new Map<string, CloudCommandHandler>();

export function registerCloudCommands(handlers: Record<string, CloudCommandHandler>): void {
  for (const [cmd, handler] of Object.entries(handlers)) cloudCommandHandlers.set(cmd, handler);
}

// Cloud-mode listen() subscribers, keyed by event name. Tauri events ride the
// native bus instead; this registry is fed by the wasm sim worker and the
// feature bridges via emitLocalEvent().
export const cloudListeners = new Map<string, Set<(e: { payload: unknown }) => void>>();

// Delivers a local event to cloud-mode listen() subscribers.
export function emitLocalEvent(event: string, payload: unknown): void {
  const set = cloudListeners.get(event);
  if (!set) return;
  for (const handler of set) handler({ payload });
}
