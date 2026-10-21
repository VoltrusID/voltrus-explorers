import { describe, expect, it } from "vitest";
import { messagesToCsv, type MessageRow } from "./csv";

const row = (over: Partial<MessageRow>): MessageRow => ({
  timestamp: Date.UTC(2026, 0, 2, 3, 4, 5),
  topic: "a/b",
  payload: "hello",
  encoding: "utf8",
  qos: 1,
  retain: false,
  decoded: "",
  ...over,
});

describe("messagesToCsv", () => {
  it("writes a header and one line per message", () => {
    expect(messagesToCsv([row({})])).toBe(
      "timestamp,topic,payload,qos,retain,encoding,decoded\r\n2026-01-02T03:04:05.000Z,a/b,hello,1,false,utf8,\r\n",
    );
  });

  it("marks binary payloads as hex and carries the decoded tree", () => {
    const out = messagesToCsv([row({ payload: "00ff", encoding: "hex", decoded: '{"seq":1,"a":"x,y"}', qos: null, retain: true })]);
    expect(out).toContain(',,true,hex,"{""seq"":1,""a"":""x,y""}"');
  });

  it("quotes commas and quotes and neutralises formulas", () => {
    const out = messagesToCsv([row({ payload: '=1+1, "x"' })]);
    expect(out).toContain(`"'=1+1, ""x"""`);
  });
});
