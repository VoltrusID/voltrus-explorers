import { describe, it, expect } from "vitest";
import { serverMessagesFromRecent, type ServerRecentLike } from "../serverMessages";

const msg = (subject: string, timestamp: number): ServerRecentLike => ({
  subject,
  payload: "hello",
  timestamp,
  headers: [],
});

describe("serverMessagesFromRecent", () => {
  it("keeps the server order and does not grow when the same buffer is polled again", () => {
    const recent = [msg("a", 10), msg("b", 20)];
    const first = serverMessagesFromRecent(recent, 0);
    const second = serverMessagesFromRecent(recent, 0);
    expect(first.map((m) => m.subject)).toEqual(["a", "b"]);
    expect(second).toHaveLength(2);
  });

  it("hides messages at or before the clear time", () => {
    const recent = [msg("a", 10), msg("b", 20), msg("c", 30)];
    expect(serverMessagesFromRecent(recent, 20).map((m) => m.subject)).toEqual(["c"]);
  });

  it("keeps only the newest messages when over the cap", () => {
    const recent = Array.from({ length: 5 }, (_, i) => msg(`s${i}`, i + 1));
    expect(serverMessagesFromRecent(recent, 0, 3).map((m) => m.subject)).toEqual(["s2", "s3", "s4"]);
  });

  it("stamps each row with the server time", () => {
    expect(serverMessagesFromRecent([msg("a", 42)], 0)[0]._ts).toBe(42);
  });
});
