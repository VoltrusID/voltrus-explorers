import { describe, expect, it } from "vitest";
import { formatPayload, payloadBytes, payloadInfoFromEvent, summarizeDecoded, toBase64, toHex, type Decoded } from "./payload";

describe("payload bytes", () => {
  it("uses the exact bytes of binary payloads", () => {
    const p = payloadInfoFromEvent({ payload: "\uFFFD\uFFFDA", raw_hex: "00ff41" });
    expect(Array.from(payloadBytes(p))).toEqual([0, 255, 65]);
    expect(toHex(p)).toBe("00 ff 41");
    expect(toBase64(p)).toBe("AP9B");
  });

  it("encodes text payloads as UTF-8", () => {
    const p = payloadInfoFromEvent({ payload: "é" });
    expect(toHex(p)).toBe("c3 a9");
    expect(toBase64(p)).toBe("w6k=");
  });

  it("reads broker topic rows", () => {
    const p = payloadInfoFromEvent({ last_payload: "x", decoded: { kind: "protobuf", value: {} } });
    expect(p.payload).toBe("x");
    expect(p.decoded?.kind).toBe("protobuf");
  });
});

describe("summarizeDecoded", () => {
  const sp: Decoded = {
    kind: "sparkplug",
    value: {
      topic: { message_type: "NDATA" },
      seq: 3,
      metrics: [
        { name: "Temp", value: 21.5 },
        { alias: 4, value: true },
        { name: "Tbl", value: { columns: ["a", "b"], rows: [[1, 2]] } },
        { name: "Arr", value: [1, 2, 3] },
        { name: "Null", value: null },
        { name: "Long", value: "a very long string that will be cut off" },
      ],
    },
  };

  it("lists Sparkplug metrics on one line", () => {
    expect(summarizeDecoded(sp)).toBe(
      'NDATA #3 · Temp=21.5 · alias 4=true · Tbl=dataset 1x2 · Arr=[3 items] · Null=null · Long="a very long string that …"',
    );
  });

  it("truncates after eight metrics", () => {
    const many: Decoded = {
      kind: "sparkplug",
      value: { topic: { message_type: "NBIRTH" }, metrics: Array.from({ length: 10 }, (_, i) => ({ name: `m${i}`, value: i })) },
    };
    const s = summarizeDecoded(many);
    expect(s.startsWith("NBIRTH · m0=0")).toBe(true);
    expect(s.endsWith("+2 more")).toBe(true);
  });

  it("shows errors and protobuf JSON", () => {
    expect(summarizeDecoded({ kind: "sparkplug", error: "bad" })).toBe("Sparkplug B decode failed: bad");
    expect(summarizeDecoded({ kind: "protobuf", value: { a: 1 } })).toBe('{"a":1}');
  });
});

describe("formatPayload", () => {
  it("falls back to raw text when a message has no decoded form", () => {
    expect(formatPayload({ payload: "hi" }, "decoded")).toBe("hi");
  });

  it("pretty prints JSON and leaves other text alone", () => {
    expect(formatPayload({ payload: '{"a":1}' }, "json")).toBe('{\n  "a": 1\n}');
    expect(formatPayload({ payload: "nope" }, "json")).toBe("nope");
  });
});
