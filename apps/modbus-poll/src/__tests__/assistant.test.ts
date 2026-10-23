import { describe, expect, it } from "vitest";
import {
  blockSummary,
  commonWordOrder,
  failureInfo,
  isFailure,
  parseErrorCode,
  parseRegisters,
  quotaLine,
  type AssistantBlock,
  type AssistantQuota,
} from "../../../../packages/mt-ui/src/assistantLogic";
import { addedMessage, assistantError, sessionConfig, toReadBlocks } from "../assistantAdapter";
import type { ReadBlock } from "../types";

const block = (over: Partial<AssistantBlock> = {}): AssistantBlock => ({
  name: "Voltage L1-N",
  area: "holding",
  address: 3027,
  quantity: 2,
  type: "float32",
  word_order: "big",
  confidence: "high",
  ...over,
});

const quota: AssistantQuota = {
  period: "2026-10",
  resets_at: Date.UTC(2026, 10, 1, 12),
  text: { used: 4, limit: 300, remaining: 296 },
  image: { used: 1, limit: 30, remaining: 29 },
};

describe("parseRegisters", () => {
  it("reads decimal and hex values separated by spaces, commas or semicolons", () => {
    expect(parseRegisters("16840 0")).toEqual([16840, 0]);
    expect(parseRegisters("0x41C8, 0x0000; 65535")).toEqual([0x41c8, 0, 65535]);
  });

  it("says what is wrong instead of sending bad input", () => {
    expect(parseRegisters("   ")).toContain("Enter the register values");
    expect(parseRegisters("70000")).toContain('"70000"');
    expect(parseRegisters("12 abc")).toContain('"abc"');
    expect(parseRegisters(Array(17).fill("1").join(" "))).toBe("At most 16 registers.");
  });
});

describe("parseErrorCode", () => {
  it("accepts how Modbus exception codes are written", () => {
    expect(parseErrorCode("2")).toBe(2);
    expect(parseErrorCode("02")).toBe(2);
    expect(parseErrorCode("0x0B")).toBe(11);
    expect(parseErrorCode("0B")).toBe(11);
    expect(parseErrorCode("11")).toBe(11);
  });

  it("refuses anything else", () => {
    expect(parseErrorCode("")).toBeUndefined();
    expect(parseErrorCode("two")).toBeUndefined();
    expect(parseErrorCode("999")).toBeUndefined();
  });
});

describe("quota and failures", () => {
  it("shows what is left of both allowances", () => {
    expect(quotaLine(quota)).toMatch(/^Text 296 of 300 left, photos 29 of 30 left\. Resets /);
    expect(quotaLine(null)).toBeNull();
  });

  it("words every refusal the console can send, and never mentions a price", () => {
    const f = (code: string, status = 400, extra = {}) => failureInfo({ status, code, message: "server text", ...extra });
    expect(f("not_signed_in", 401).kind).toBe("signin");
    expect(f("invalid_token", 401).kind).toBe("signin");
    expect(f("entitlement_required", 402)).toEqual({ kind: "upgrade", text: "The Assistant is included with Voltrus Connect." });
    expect(f("quota_exceeded", 429, { quota }).kind).toBe("quota");
    expect(f("quota_exceeded", 429, { quota }).text).toContain("It resets");
    expect(f("ai_unavailable", 403).text).toContain("not available for this account");
    expect(f("ai_unavailable", 503).text).toContain("not counted");
    expect(f("invalid_model_output", 502).text).toContain("not counted");
    expect(f("search_unavailable", 503).text).toContain("Paste the register table");
    expect(f("network", 0).text).toContain("Could not reach Voltrus");
    expect(f("invalid_request").text).toBe("server text");
    for (const code of ["not_signed_in", "entitlement_required", "quota_exceeded", "ai_unavailable", "invalid_model_output", "network"])
      expect(f(code).text).not.toMatch(/\$|USD|price|Rp\b|subscribe at|http/i);
  });

  it("recognises a failure object thrown by the Tauri command", () => {
    expect(isFailure({ status: 429, code: "quota_exceeded", message: "x" })).toBe(true);
    expect(isFailure("plain string")).toBe(false);
    expect(isFailure({ code: "x" })).toBe(false);
  });
});

describe("blocks", () => {
  it("summarises a block with the number the manual printed", () => {
    expect(blockSummary(block({ documented_address: "3028", scale: 0.1, unit: "V" }))).toBe(
      "holding 3027, 2 registers, float32 x0.1 V (manual says 3028)",
    );
    expect(blockSummary(block({ quantity: 1, type: "uint16", documented_address: "3027" }))).toBe("holding 3027, 1 register, uint16");
  });

  it("proposes a word order only when the blocks that span registers agree on a non-default one", () => {
    expect(commonWordOrder([block({ word_order: "little" }), block({ word_order: "little" })])).toBe("little");
    expect(commonWordOrder([block({ word_order: "little" }), block({ word_order: "big" })])).toBeNull();
    expect(commonWordOrder([block()])).toBeNull();
    expect(commonWordOrder([block({ word_order: "swapped", quantity: 1, type: "uint16" })])).toBeNull();
  });
});

describe("assistantError", () => {
  it("turns the app's read failure into an exception with its code", () => {
    expect(assistantError("Read failed: Illegal Data Address (0x02) reading holding 2999..3034")).toMatchObject({ kind: "exception", code: 2 });
  });

  it("recognises timeouts and leaves other text as other", () => {
    expect(assistantError("Connection timed out")?.kind).toBe("timeout");
    expect(assistantError("request timeout after 1000 ms")?.kind).toBe("timeout");
    expect(assistantError("Port busy")?.kind).toBe("other");
    expect(assistantError(null)).toBeUndefined();
  });
});

describe("toReadBlocks", () => {
  const existing: ReadBlock[] = [{ name: "Voltage L1-N", address: 0, quantity: 1, type: "uint16", area: "holding" }];

  it("counts values, not registers, and keeps names unique", () => {
    const out = toReadBlocks([block({ quantity: 6 }), block({ name: "Alarms", area: "coil", type: "uint16", quantity: 8, address: 10 })], existing);
    expect(out.skipped).toEqual([]);
    expect(out.blocks).toEqual([
      { name: "Voltage L1-N 2", address: 3027, quantity: 3, type: "float32", area: "holding" },
      { name: "Alarms", address: 10, quantity: 8, type: "uint16", area: "coil" },
    ]);
  });

  it("gives two proposals with the same name different names", () => {
    const out = toReadBlocks([block({ name: "A" }), block({ name: "A", address: 10 })], []);
    expect(out.blocks.map((b) => b.name)).toEqual(["A", "A 2"]);
  });

  it("leaves out what can not be polled and says why", () => {
    const out = toReadBlocks(
      [block({ quantity: 3 }), block({ name: "High", address: 65535, quantity: 2, type: "uint16" }), block({ name: "Fine", address: 5 })],
      [],
    );
    expect(out.blocks.map((b) => b.name)).toEqual(["Fine"]);
    expect(out.skipped).toHaveLength(2);
    expect(out.skipped[0]).toContain("3 registers do not fit float32");
    expect(out.skipped[1]).toContain("past address 65535");
  });

  it("keeps a scale factor on the block", () => {
    expect(toReadBlocks([block({ type: "uint16", quantity: 1, scale: 0.1 })], []).blocks[0]?.scale).toBe(0.1);
  });
});

describe("addedMessage", () => {
  const added: ReadBlock[] = [{ name: "A", address: 0, quantity: 1, type: "uint16", area: "holding" }];

  it("reports what was added and what is left to the user", () => {
    expect(addedMessage(added, [], null, "big")).toBe("Added 1 block.");
    const msg = addedMessage([...added, { ...added[0]!, name: "B", scale: 0.1 }], ["X: no"], "little", "big");
    expect(msg).toContain("Added 2 blocks.");
    expect(msg).toContain("Left out: X: no.");
    expect(msg).toContain("apply the manual's scale factors yourself");
    expect(msg).toContain("Set the word order to CD AB");
  });

  it("does not ask for a word order change the connection already has", () => {
    expect(addedMessage(added, [], "little", "little")).toBe("Added 1 block.");
  });
});

describe("sessionConfig", () => {
  const blocks: ReadBlock[] = Array.from({ length: 30 }, (_, i) => ({ name: `B${i}`, address: i, quantity: 1, type: "uint16", area: "holding" }));

  it("never includes the host and caps the block list", () => {
    const cfg = sessionConfig({
      transport: "tcp",
      tcp: { host: "10.1.2.3", port: 502, unit_id: 1 },
      rtu: { baudRate: 9600, dataBits: 8, stopBits: 1, parity: "none", unitId: 1, timeoutMs: 1000, pollIntervalMs: 1000 },
      endianness: "big",
      oneBasedDisplay: true,
      blocks,
    });
    expect(JSON.stringify(cfg)).not.toContain("10.1.2.3");
    expect(cfg).toMatchObject({ transport: "tcp", tcp: { port: 502, unit_id: 1 }, wordOrder: "big", addressesShownFrom: 1 });
    expect((cfg.blocks as unknown[]).length).toBe(20);
  });

  it("describes serial settings for RTU", () => {
    const cfg = sessionConfig({
      transport: "rtu",
      tcp: {},
      rtu: { baudRate: 19200, dataBits: 8, stopBits: 1, parity: "even", unitId: 3, timeoutMs: 500, pollIntervalMs: 1000 },
      endianness: "little",
      oneBasedDisplay: false,
      blocks: [],
    });
    expect(cfg).toMatchObject({ transport: "rtu", rtu: { baudRate: 19200, parity: "even", unitId: 3 }, wordOrder: "little" });
  });
});
