import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const files = vi.hoisted(() => ({
  saveTextFile: vi.fn(),
  appendTextFile: vi.fn(),
  noteDeviceSuccess: vi.fn(),
  noteDeviceError: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@mactools/ui", () => files);

import { newSessionFields } from "../sessions";
import { useStore } from "../store";
import {
  closeConnection, connectTcp, disconnect, IDLE, pausePolling, runtimeOf, startCsvLog, startPolling, stopCsvLog, useConnRuntime, writeTag,
} from "../connections";
import type { ReadBlock } from "../types";

const block = (name: string, quantity: number, address = 0): ReadBlock => ({ name, address, quantity, type: "uint16", area: "holding" });
const config = { host: "h", port: 502, unit_id: 1, timeout_ms: 1000, poll_interval_ms: 1000 };

/** Let queued promise continuations run. */
const settle = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };

/** Answer `read_tags` with the value `base + index` for each tag, so connections are told apart. */
function answerReads(base: Record<string, number>) {
  invoke.mockImplementation(async (cmd: string, args: { connId: string; tags?: unknown[] }) => {
    if (cmd === "read_tags") return args.tags!.map((_, i) => ({ Float64: base[args.connId] + i }));
    return undefined;
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  invoke.mockReset();
  files.saveTextFile.mockReset();
  files.appendTextFile.mockReset();
  useStore.setState({ ...newSessionFields(), sessions: [{ id: "c1", fields: null }], activeSessionId: "c1", nextSessionSeq: 2 });
  useConnRuntime.setState({ byId: {} });
});

afterEach(async () => {
  for (const id of Object.keys(useConnRuntime.getState().byId)) await disconnect(id);
  vi.useRealTimers();
});

describe("connecting", () => {
  it("passes the connection id to the backend and tracks each connection separately", async () => {
    answerReads({ c1: 0, c2: 0 });
    expect(await connectTcp("c1", config)).toBe(true);
    expect(invoke).toHaveBeenCalledWith("connect", { connId: "c1", config });
    expect(runtimeOf("c1").connected).toBe(true);
    expect(runtimeOf("c2")).toBe(IDLE);
  });

  it("a failed connect records the error on that connection only", async () => {
    invoke.mockRejectedValueOnce("Connection refused");
    expect(await connectTcp("c2", config)).toBe(false);
    expect(runtimeOf("c2")).toMatchObject({ connected: false, loading: false, error: "Connection refused" });
    expect(runtimeOf("c1").error).toBeNull();
  });
});

describe("polling several connections", () => {
  it("each connection polls its own blocks and keeps its own values", async () => {
    answerReads({ c1: 100, c2: 200 });
    useStore.getState().addSession(); // c2 in front, c1 parked
    startPolling("c1", [block("a", 2)], 1000, true);
    startPolling("c2", [block("b", 3)], 1000, true);
    await settle();
    expect(runtimeOf("c1").values).toEqual([{ Float64: 100 }, { Float64: 101 }]);
    expect(runtimeOf("c2").values).toEqual([{ Float64: 200 }, { Float64: 201 }, { Float64: 202 }]);
    expect(invoke).toHaveBeenCalledWith("read_tags", expect.objectContaining({ connId: "c1" }));
    expect(invoke).toHaveBeenCalledWith("read_tags", expect.objectContaining({ connId: "c2" }));

    await vi.advanceTimersByTimeAsync(1000);
    expect(runtimeOf("c1").pollCount).toBe(2);
    expect(runtimeOf("c2").pollCount).toBe(2);
  });

  it("a background connection logs its polls into its own comm log, not the one in front", async () => {
    answerReads({ c1: 1, c2: 1 });
    useStore.getState().addSession(); // c2 in front
    startPolling("c1", [block("a", 2)], 1000, true);
    await settle();
    expect(useStore.getState().log).toEqual([]);
    const parked = useStore.getState().sessions.find((s) => s.id === "c1")!.fields!;
    expect(parked.log.map((l) => l.message)).toEqual([expect.stringMatching(/^Poll #1: a_000…a_001 \(2 values\)/)]);
  });

  it("a background connection keeps its trend series growing", async () => {
    answerReads({ c1: 5, c2: 0 });
    useStore.getState().setTrendSeries(new Map([["a_000", { name: "a_000", color: "#fff", points: [] }]]));
    useStore.getState().setBlocks([block("a", 1)]);
    useStore.getState().addSession();
    startPolling("c1", [block("a", 1)], 1000, true);
    await settle();
    await vi.advanceTimersByTimeAsync(1000);
    const parked = useStore.getState().sessions.find((s) => s.id === "c1")!.fields!;
    expect(parked.trendSeries.get("a_000")!.points.map((p) => p.v)).toEqual([5, 5]);
  });

  it("free polling stops at the first 20 values", async () => {
    answerReads({ c1: 0 });
    startPolling("c1", [block("a", 15), block("b", 15, 100)], 1000, false);
    await settle();
    const call = invoke.mock.calls.find(([cmd]) => cmd === "read_tags")!;
    expect((call[1] as { tags: unknown[] }).tags).toHaveLength(20);
    expect(runtimeOf("c1").values).toHaveLength(20);
  });

  it("restarting with a different plan changes the plan key and drops reads of the old one", async () => {
    answerReads({ c1: 0 });
    startPolling("c1", [block("a", 2)], 1000, true);
    const first = runtimeOf("c1").polledKey;
    startPolling("c1", [block("a", 3)], 1000, true);
    expect(runtimeOf("c1").polledKey).not.toBe(first);
  });

  it("pausing keeps the session open and stops the timer", async () => {
    answerReads({ c1: 0 });
    startPolling("c1", [block("a", 1)], 1000, true);
    await settle();
    pausePolling("c1");
    const before = runtimeOf("c1").pollCount;
    await vi.advanceTimersByTimeAsync(5000);
    expect(runtimeOf("c1").pollCount).toBe(before);
    expect(runtimeOf("c1").pollIntervalMs).toBe(1000);
  });

  it("a read failure marks values stale, is logged once and clears on the next good read", async () => {
    let fail = true;
    invoke.mockImplementation(async (cmd: string, args: { tags?: unknown[] }) => {
      if (cmd !== "read_tags") return undefined;
      if (fail) throw "timed out";
      return args.tags!.map(() => ({ Float64: 1 }));
    });
    startPolling("c1", [block("a", 1)], 1000, true);
    await settle();
    await vi.advanceTimersByTimeAsync(1000);
    expect(runtimeOf("c1")).toMatchObject({ stale: true, error: "timed out" });
    const errors = () => useStore.getState().log.filter((l) => l.direction === "error");
    expect(errors()).toHaveLength(1);
    fail = false;
    await vi.advanceTimersByTimeAsync(1000);
    expect(runtimeOf("c1")).toMatchObject({ stale: false, error: null });
  });
});

describe("disconnect and close", () => {
  it("disconnect ends only that connection's poll", async () => {
    answerReads({ c1: 0, c2: 0 });
    startPolling("c1", [block("a", 1)], 1000, true);
    startPolling("c2", [block("b", 1)], 1000, true);
    await settle();
    await disconnect("c1");
    expect(invoke).toHaveBeenCalledWith("disconnect", { connId: "c1" });
    expect(runtimeOf("c1")).toMatchObject({ connected: false, pollIntervalMs: null, values: [] });
    const c2Polls = runtimeOf("c2").pollCount;
    await vi.advanceTimersByTimeAsync(1000);
    expect(runtimeOf("c1").pollCount).toBe(0);
    expect(runtimeOf("c2").pollCount).toBe(c2Polls + 1);
  });

  it("closing a connection forgets its runtime", async () => {
    answerReads({ c1: 0 });
    await connectTcp("c1", config);
    await closeConnection("c1");
    expect(useConnRuntime.getState().byId.c1).toBeUndefined();
  });
});

describe("writes", () => {
  it("target the connection they were made on", async () => {
    invoke.mockResolvedValue(undefined);
    const tag = { name: "a", address: 1, quantity: 1, area: "holding", register_area: "holding", type: "uint16", scale: 1 };
    expect(await writeTag("c2", tag, 5)).toEqual({ ok: true });
    expect(invoke).toHaveBeenCalledWith("write_tag", { connId: "c2", tag, value: 5 });
  });

  it("report the device's refusal", async () => {
    invoke.mockRejectedValue("Illegal Data Address");
    const tag = { name: "a", address: 1, quantity: 1, area: "holding", register_area: "holding", type: "uint16", scale: 1 };
    expect(await writeTag("c1", tag, 5)).toEqual({ ok: false, error: "Illegal Data Address" });
    expect(runtimeOf("c1").error).toBe("Illegal Data Address");
  });
});

describe("CSV logging", () => {
  it("writes the header when the file is chosen, then one block of lines per poll", async () => {
    answerReads({ c1: 7 });
    files.saveTextFile.mockResolvedValue("/tmp/log.csv");
    files.appendTextFile.mockResolvedValue(undefined);
    expect(await startCsvLog("c1", "Plant A")).toBe("/tmp/log.csv");
    expect(files.saveTextFile).toHaveBeenCalledWith(expect.stringMatching(/^modbus-log-.*\.csv$/), "timestamp,connection,name,area,address,type,value,error\n");
    expect(runtimeOf("c1").csvLogPath).toBe("/tmp/log.csv");

    startPolling("c1", [block("a", 2)], 1000, true);
    await settle();
    await vi.advanceTimersByTimeAsync(1000);
    expect(files.appendTextFile).toHaveBeenCalledTimes(2);
    const [path, text] = files.appendTextFile.mock.calls[0];
    expect(path).toBe("/tmp/log.csv");
    expect((text as string).trim().split("\n").map((l: string) => l.split(",").slice(1))).toEqual([
      ["Plant A", "a_000", "holding", "0", "uint16", "7", ""],
      ["Plant A", "a_001", "holding", "1", "uint16", "8", ""],
    ]);
    expect(runtimeOf("c1").csvLogRows).toBe(4);
  });

  it("cancelling the file panel starts nothing", async () => {
    files.saveTextFile.mockResolvedValue(null);
    expect(await startCsvLog("c1", "x")).toBeNull();
    expect(runtimeOf("c1").csvLogPath).toBeNull();
  });

  it("each connection logs to its own file", async () => {
    answerReads({ c1: 1, c2: 2 });
    files.saveTextFile.mockResolvedValueOnce("/tmp/one.csv").mockResolvedValueOnce("/tmp/two.csv");
    files.appendTextFile.mockResolvedValue(undefined);
    await startCsvLog("c1", "one");
    await startCsvLog("c2", "two");
    startPolling("c1", [block("a", 1)], 1000, true);
    startPolling("c2", [block("b", 1)], 1000, true);
    await settle();
    const byPath = new Map(files.appendTextFile.mock.calls.map(([p, t]) => [p, t as string]));
    expect(byPath.get("/tmp/one.csv")).toContain(",one,a_000,");
    expect(byPath.get("/tmp/two.csv")).toContain(",two,b_000,");
  });

  it("stop reports the file and how many values were written, and stops writing", async () => {
    answerReads({ c1: 1 });
    files.saveTextFile.mockResolvedValue("/tmp/log.csv");
    files.appendTextFile.mockResolvedValue(undefined);
    await startCsvLog("c1", "x");
    startPolling("c1", [block("a", 3)], 1000, true);
    await settle();
    expect(await stopCsvLog("c1")).toEqual({ path: "/tmp/log.csv", rows: 3 });
    expect(runtimeOf("c1").csvLogPath).toBeNull();
    await vi.advanceTimersByTimeAsync(2000);
    expect(files.appendTextFile).toHaveBeenCalledTimes(1);
    expect(await stopCsvLog("c1")).toBeNull();
  });

  it("disconnecting stops the log", async () => {
    answerReads({ c1: 1 });
    files.saveTextFile.mockResolvedValue("/tmp/log.csv");
    files.appendTextFile.mockResolvedValue(undefined);
    await startCsvLog("c1", "x");
    await disconnect("c1");
    expect(runtimeOf("c1").csvLogPath).toBeNull();
  });

  it("a write failure stops logging once and says why in that connection's log", async () => {
    answerReads({ c1: 1 });
    files.saveTextFile.mockResolvedValue("/tmp/log.csv");
    files.appendTextFile.mockRejectedValue("disk full");
    await startCsvLog("c1", "x");
    startPolling("c1", [block("a", 1)], 1000, true);
    await settle();
    await vi.advanceTimersByTimeAsync(3000);
    expect(runtimeOf("c1").csvLogPath).toBeNull();
    const messages = useStore.getState().log.filter((l) => l.message.startsWith("CSV logging stopped"));
    expect(messages).toHaveLength(1);
    expect(messages[0].message).toContain("disk full");
  });

  it("values beyond the free cap are not logged", async () => {
    answerReads({ c1: 1 });
    files.saveTextFile.mockResolvedValue("/tmp/log.csv");
    files.appendTextFile.mockResolvedValue(undefined);
    await startCsvLog("c1", "x");
    startPolling("c1", [block("a", 25)], 1000, false);
    await settle();
    expect((files.appendTextFile.mock.calls[0][1] as string).trim().split("\n")).toHaveLength(20);
  });
});
