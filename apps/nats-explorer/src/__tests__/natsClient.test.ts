import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { jsConsumerNext, jsCreateStream, kvCreate, schemaBind, sysRequest } from "../nats-client";

// Tauri maps command arguments from camelCase; a snake_case key makes the command fail with
// "missing required key", which is how Create stream failed on a running server.
describe("invoke argument names", () => {
  beforeEach(() => invoke.mockReset());

  it("js_create_stream uses camelCase keys", async () => {
    await jsCreateStream("EVENTS", ["events.>"], 10000, 0);
    expect(invoke).toHaveBeenCalledWith("js_create_stream", {
      name: "EVENTS",
      subjects: ["events.>"],
      maxMessages: 10000,
      maxAgeNs: 0,
    });
  });

  it("js_consumer_next uses camelCase keys", async () => {
    await jsConsumerNext("EVENTS", "c1", 10, 3000);
    expect(invoke).toHaveBeenCalledWith("js_consumer_next", {
      stream: "EVENTS",
      durable: "c1",
      batch: 10,
      waitMs: 3000,
    });
  });

  it("the other multi-word arguments are camelCase too", async () => {
    await kvCreate("b", 5, 0);
    await schemaBind("a.>", "pkg.Msg");
    await sysRequest("$SYS.REQ", 1000);
    const argKeys = invoke.mock.calls.map((c) => Object.keys(c[1] as object));
    for (const keys of argKeys) for (const k of keys) expect(k).not.toContain("_");
  });
});
