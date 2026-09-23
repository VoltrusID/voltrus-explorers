import type { LspRequest, LspRequestBody, LspResponse } from "./protocol";

// Creates the module Worker. Injectable so tests can supply a fake worker
// (jsdom/node provide no Worker implementation).
export type LspWorkerFactory = () => Worker;

export const defaultLspWorkerFactory: LspWorkerFactory = () =>
  new Worker(new URL("../wasm/lsp.worker.ts", import.meta.url), { type: "module" });

let workerFactory: LspWorkerFactory = defaultLspWorkerFactory;

export function setLspWorkerFactory(factory: LspWorkerFactory): void {
  workerFactory = factory;
}

type Pending = {
  resolve: (res: never) => void;
  reject: (err: Error) => void;
};

// Correlates requests to the wasm LSP worker by id; the worker is spawned
// lazily on the first request so cloud-mode editors only pay the wasm
// instantiation when language services are actually used.
export class LspHost {
  private worker: Worker | null = null;
  private pending = new Map<number, Pending>();
  private nextId = 1;

  private ensureWorker(): Worker {
    if (!this.worker) {
      const worker = workerFactory();
      worker.onmessage = (e: MessageEvent<LspResponse>) => {
        const msg = e.data;
        const waiting = this.pending.get(msg.id);
        if (!waiting) return;
        this.pending.delete(msg.id);
        if (msg.ok) waiting.resolve(msg as never);
        else waiting.reject(new Error(msg.error));
      };
      worker.onerror = () => {
        this.failAll(new Error("lsp worker crashed"));
      };
      this.worker = worker;
    }
    return this.worker;
  }

  private failAll(err: Error): void {
    const waiting = [...this.pending.values()];
    this.pending.clear();
    for (const p of waiting) p.reject(err);
  }

  request<Req extends LspRequestBody>(
    req: Req,
  ): Promise<Extract<LspResponse, { type: Req["type"] }>> {
    const id = this.nextId++;
    const worker = this.ensureWorker();
    const { promise, resolve, reject } =
      Promise.withResolvers<Extract<LspResponse, { type: Req["type"] }>>();
    this.pending.set(id, { resolve: resolve as (res: never) => void, reject });
    worker.postMessage({ ...req, id } as LspRequest);
    return promise;
  }

  dispose(): void {
    this.failAll(new Error("lsp host disposed"));
    this.worker?.terminate();
    this.worker = null;
  }
}

// Singleton; extensions in st-lsp route through it. Overridable in tests via
// setLspHost (a spy host), restored with resetLspHost.
const defaultLspHost = new LspHost();
let activeLspHost: LspHost = defaultLspHost;

export function getLspHost(): LspHost {
  return activeLspHost;
}

export function setLspHost(host: LspHost): void {
  activeLspHost = host;
}

export function resetLspHost(): void {
  activeLspHost = defaultLspHost;
}
