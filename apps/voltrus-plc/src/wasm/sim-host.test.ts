import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SimRequest, SimWorkerOutbound } from "./protocol";
import {
  SimHost,
  defaultSimWorkerFactory,
  getSimHost,
  resetSimHost,
  setSimEventHandler,
  setSimHost,
  setSimWorkerFactory,
} from "./sim-host";

// jsdom/node ship no Worker, so tests drive a scripted fake through the same
// surface SimHost uses: postMessage in, onmessage out.
class FakeWorker {
  onmessage: ((e: { data: SimWorkerOutbound }) => void) | null = null;
  onerror: ((e: ErrorEvent) => void) | null = null;
  sent: SimRequest[] = [];
  terminated = false;

  postMessage(data: SimRequest): void {
    this.sent.push(data);
  }

  terminate(): void {
    this.terminated = true;
  }

  receive(msg: SimWorkerOutbound): void {
    this.onmessage?.({ data: msg } as MessageEvent<SimWorkerOutbound>);
  }
}

let worker: FakeWorker;
let factoryCalls = 0;

beforeEach(() => {
  worker = new FakeWorker();
  factoryCalls = 0;
  setSimWorkerFactory(() => {
    factoryCalls++;
    return worker as unknown as Worker;
  });
});

afterEach(() => {
  setSimEventHandler(null);
  setSimWorkerFactory(defaultSimWorkerFactory);
  resetSimHost();
});

describe("SimHost", () => {
  it("creates the worker lazily on the first command only", async () => {
    const host = new SimHost();
    expect(factoryCalls).toBe(0);
    const pending = host.readWatch();
    expect(factoryCalls).toBe(1);
    worker.receive({ id: 1, type: "readWatch", ok: true, result: [] });
    await expect(pending).resolves.toEqual([]);
    const next = host.readWatch();
    expect(factoryCalls).toBe(1);
    worker.receive({ id: 2, type: "readWatch", ok: true, result: [] });
    await next;
  });

  it("correlates responses to requests by id when they arrive out of order", async () => {
    const host = new SimHost();
    const watchPromise = host.readWatch();
    const stepPromise = host.step();

    const [watchReq, stepReq] = worker.sent;
    expect(watchReq.type).toBe("readWatch");
    expect(stepReq.type).toBe("step");
    expect(watchReq.id).not.toBe(stepReq.id);

    worker.receive({ id: stepReq.id, type: "step", ok: true, result: null });
    worker.receive({
      id: watchReq.id,
      type: "readWatch",
      ok: true,
      result: [{ name: "X0", value: true, display_value: "TRUE", type_name: "BOOL", quality: "forced" }],
    });

    await expect(stepPromise).resolves.toBeNull();
    await expect(watchPromise).resolves.toEqual([
      { name: "X0", value: true, display_value: "TRUE", type_name: "BOOL", quality: "forced" },
    ]);
  });

  it("routes run/stop/step/force payloads verbatim and rejects on error responses", async () => {
    const host = new SimHost();
    const config = { scan_cycle_ms: 25, max_cycle_ms: 500 };

    const run = host.run(config);
    const stop = host.stop();
    const step = host.step();
    const force = host.force("X0", true);
    const unforce = host.unforce("X0");
    const cont = host.continueExecution();
    const stepInstr = host.stepInstruction();

    for (const [index, expected] of [
      { type: "run", config },
      { type: "stop" },
      { type: "step" },
      { type: "force", name: "X0", value: true },
      { type: "unforce", name: "X0" },
      { type: "continue" },
      { type: "stepInstruction" },
    ].entries()) {
      expect(worker.sent[index]).toMatchObject(expected as object);
    }

    const ids = Object.fromEntries(worker.sent.map((r) => [r.type, r.id]));
    worker.receive({ id: ids.run, type: "run", ok: true, result: null });
    worker.receive({ id: ids.stop, type: "stop", ok: true, result: null });
    worker.receive({ id: ids.step, type: "step", ok: true, result: null });
    worker.receive({ id: ids.force, type: "force", ok: true, result: null });
    worker.receive({ id: ids.unforce, type: "unforce", ok: true, result: null });
    worker.receive({
      id: ids.continue,
      type: "continue",
      ok: true,
      result: { state: "paused", current_line: 7 },
    });
    worker.receive({ id: ids.stepInstruction, type: "stepInstruction", ok: false, error: "No runtime. Compile first." });

    await expect(run).resolves.toBeNull();
    await expect(stop).resolves.toBeNull();
    await expect(step).resolves.toBeNull();
    await expect(force).resolves.toBeNull();
    await expect(unforce).resolves.toBeNull();
    await expect(cont).resolves.toEqual({ state: "paused", current_line: 7 });
    await expect(stepInstr).rejects.toThrow("No runtime. Compile first.");
  });

  it("forwards worker events to the registered handler without touching pending requests", async () => {
    const host = new SimHost();
    const events: SimWorkerOutbound[] = [];
    setSimEventHandler((msg) => events.push(msg));

    const pending = host.readWatch();
    worker.receive({
      type: "event",
      event: "watch-update",
      payload: [{ name: "Y0", value: false, display_value: "FALSE", type_name: "BOOL", quality: "good" }],
    });
    worker.receive({
      type: "event",
      event: "runtime-state",
      payload: {
        state: "running",
        cycle_count: 3,
        last_cycle_ms: 0.1,
        avg_cycle_ms: 0.1,
        max_cycle_ms: 0.2,
        error_message: null,
        cycle_history: [0.1, 0.1],
      },
    });
    worker.receive({ id: worker.sent[0].id, type: "readWatch", ok: true, result: [] });

    await pending;
    expect(events.map((e) => ("event" in e ? e.event : ""))).toEqual(["watch-update", "runtime-state"]);
  });

  it("dispose terminates the worker and flushes pending requests", async () => {
    const host = new SimHost();
    const failing = host.readWatch();
    const disposed = host.dispose();
    worker.receive({ id: worker.sent[0].id, type: "readWatch", ok: false, error: "worker went away" });
    const disposeReq = worker.sent.find((r) => r.type === "dispose")!;
    worker.receive({ id: disposeReq.id, type: "dispose", ok: true, result: null });

    await expect(failing).rejects.toThrow("worker went away");
    await disposed;
    expect(worker.terminated).toBe(true);
  });
});

describe("sim host singleton", () => {
  it("exposes an overridable active host for tests", () => {
    expect(getSimHost()).toBeInstanceOf(SimHost);
    const spy = { readWatch: () => [] };
    setSimHost(spy as unknown as SimHost);
    expect(getSimHost()).toBe(spy);
    resetSimHost();
    expect(getSimHost()).not.toBe(spy);
  });
});
