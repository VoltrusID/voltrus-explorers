import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { LspRequest, LspResponse } from "./protocol";
import {
  LspHost,
  defaultLspWorkerFactory,
  setLspWorkerFactory,
} from "./lsp-host";

// jsdom/node ship no Worker, so tests drive a scripted fake through the same
// surface LspHost uses: postMessage in, onmessage out.
class FakeWorker {
  onmessage: ((e: { data: LspResponse }) => void) | null = null;
  onerror: ((e: ErrorEvent) => void) | null = null;
  sent: LspRequest[] = [];

  postMessage(data: LspRequest): void {
    this.sent.push(data);
  }

  receive(msg: LspResponse): void {
    this.onmessage?.({ data: msg } as MessageEvent<LspResponse>);
  }

  crash(): void {
    this.onerror?.(new ErrorEvent("error"));
  }

  terminate(): void {}
}

let worker: FakeWorker;

beforeEach(() => {
  worker = new FakeWorker();
  setLspWorkerFactory(() => worker as unknown as Worker);
});

afterEach(() => {
  setLspWorkerFactory(defaultLspWorkerFactory);
});

describe("LspHost", () => {
  it("spawns the worker lazily on the first request and correlates responses by id", async () => {
    const host = new LspHost();
    const pending = host.request({ type: "hover", uri: "file:///Main.st", line: 2, character: 4 });
    expect(worker.sent).toEqual([
      { id: 1, type: "hover", uri: "file:///Main.st", line: 2, character: 4 },
    ]);

    worker.receive({
      id: 1,
      ok: true,
      type: "hover",
      hover: {
        name: "m",
        type_name: "Motor",
        kind: "VAR",
        owner: "Main",
        decl_line: 3,
        uri: "file:///Main.st",
        range: { start: { line: 2, character: 4 }, end: { line: 2, character: 5 } },
      },
    });
    const res = await pending;
    expect(res.hover?.name).toBe("m");
  });

  it("rejects when the worker answers ok:false", async () => {
    const host = new LspHost();
    const pending = host.request({ type: "sync", uri: "file:///Main.st", text: "PROGRAM" });
    worker.receive({ id: 1, ok: false, error: "bad uri" });
    await expect(pending).rejects.toThrow("bad uri");
  });

  it("fails in-flight requests when the worker crashes", async () => {
    const host = new LspHost();
    const pending = host.request({ type: "sync", uri: "file:///Main.st", text: "PROGRAM" });
    worker.crash();
    await expect(pending).rejects.toThrow("lsp worker crashed");
  });

  it("fails in-flight requests and stops the worker on dispose", async () => {
    const host = new LspHost();
    const pending = host.request({ type: "sync", uri: "file:///Main.st", text: "PROGRAM" });
    host.dispose();
    await expect(pending).rejects.toThrow("lsp host disposed");
  });
});
