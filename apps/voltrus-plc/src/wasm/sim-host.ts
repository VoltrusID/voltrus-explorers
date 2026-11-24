import type { RuntimeConfig, WatchedVar } from "../types";
import type {
  DebugStepInfo,
  SimCompileResult,
  SimEventMessage,
  SimRequestBody,
  SimRequestType,
  SimResultMap,
  SimWorkerOutbound,
} from "./protocol";

// Creates the module Worker. Injectable so tests can supply a fake worker
// (jsdom/node provide no Worker implementation).
export type SimWorkerFactory = () => Worker;

export const defaultSimWorkerFactory: SimWorkerFactory = () =>
  new Worker(new URL("./sim.worker.ts", import.meta.url), { type: "module" });

let workerFactory: SimWorkerFactory = defaultSimWorkerFactory;

export function setSimWorkerFactory(factory: SimWorkerFactory): void {
  workerFactory = factory;
}

// The host forwards worker events (watch-update / runtime-state /
// breakpoint-hit) to this hook; bridge.ts wires it to the local listener
// registry so existing `listen()` subscribers see sim events like Tauri ones.
export type SimEventHandler = (msg: SimEventMessage) => void;

let eventHandler: SimEventHandler | null = null;

export function setSimEventHandler(handler: SimEventHandler | null): void {
  eventHandler = handler;
}

type Pending = {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
};

export class SimHost {
  private worker: Worker | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();

  private ensureWorker(): Worker {
    if (!this.worker) {
      const worker = workerFactory();
      worker.onmessage = (e: MessageEvent<SimWorkerOutbound>) => {
        const msg = e.data;
        if (msg.type === "event") {
          eventHandler?.(msg);
          return;
        }
        const entry = this.pending.get(msg.id);
        if (!entry) return;
        this.pending.delete(msg.id);
        if (msg.ok) entry.resolve(msg.result);
        else entry.reject(new Error(msg.error));
      };
      worker.onerror = (e: ErrorEvent) => {
        this.failAllPending(new Error(`sim worker error: ${e.message}`));
        // Drop the dead worker so the next request() builds a fresh one instead
        // of posting into a broken worker and hanging forever.
        this.worker = null;
      };
      this.worker = worker;
    }
    return this.worker;
  }

  private failAllPending(error: Error): void {
    for (const entry of this.pending.values()) entry.reject(error);
    this.pending.clear();
  }

  request<M extends SimRequestType>(req: Extract<SimRequestBody, { type: M }>): Promise<SimResultMap[M]> {
    const worker = this.ensureWorker();
    const id = this.nextId++;
    return new Promise<SimResultMap[M]>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
      worker.postMessage({ ...req, id });
    });
  }

  compile(sources: Record<string, string>): Promise<SimCompileResult> {
    return this.request({ type: "compile", sources });
  }

  run(config: RuntimeConfig): Promise<null> {
    return this.request({ type: "run", config });
  }

  stop(): Promise<null> {
    return this.request({ type: "stop" });
  }

  step(): Promise<null> {
    return this.request({ type: "step" });
  }

  continueExecution(): Promise<DebugStepInfo> {
    return this.request({ type: "continue" });
  }

  stepInstruction(): Promise<DebugStepInfo> {
    return this.request({ type: "stepInstruction" });
  }

  setBreakpoint(line: number): Promise<null> {
    return this.request({ type: "setBreakpoint", line });
  }

  clearBreakpoint(line: number): Promise<null> {
    return this.request({ type: "clearBreakpoint", line });
  }

  readWatch(): Promise<WatchedVar[]> {
    return this.request({ type: "readWatch" });
  }

  write(name: string, value: unknown): Promise<null> {
    return this.request({ type: "write", name, value });
  }

  force(name: string, value: unknown): Promise<null> {
    return this.request({ type: "force", name, value });
  }

  unforce(name: string): Promise<null> {
    return this.request({ type: "unforce", name });
  }

  dispose(): Promise<null> {
    return this.request({ type: "dispose" }).finally(() => {
      this.failAllPending(new Error("sim host disposed"));
      this.worker?.terminate();
      this.worker = null;
    });
  }
}

// Singleton; the bridge routes sim commands here. Overridable in tests via
// setSimHost (a spy host), restored with resetSimHost.
const defaultSimHost = new SimHost();
let activeSimHost: SimHost = defaultSimHost;

export function getSimHost(): SimHost {
  return activeSimHost;
}

export function setSimHost(host: SimHost): void {
  activeSimHost = host;
}

export function resetSimHost(): void {
  activeSimHost = defaultSimHost;
}
