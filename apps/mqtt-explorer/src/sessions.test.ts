import { describe, expect, it } from "vitest";
import { HISTORY_LIMIT, applyMessage, markRetainedCleared, newSession, nextSessionId, retainedTopics, topicMatches } from "./sessions";

const msg = (topic: string, payload: string, retain = false, ts = 1) => ({ topic, payload, qos: 0, retain, ts });

describe("applyMessage", () => {
  it("keeps latest value, count and history per connection", () => {
    let s = newSession("c1");
    s = applyMessage(s, msg("a", "1"));
    s = applyMessage(s, msg("a", "2", false, 2));
    expect(s.topicValues.a.payload).toBe("2");
    expect(s.topicValues.a.count).toBe(2);
    expect(s.messages.map((m) => m.payload)).toEqual(["1", "2"]);
  });

  it("caps history", () => {
    let s = newSession("c1");
    for (let i = 0; i < HISTORY_LIMIT + 5; i++) s = applyMessage(s, msg("a", String(i)));
    expect(s.messages).toHaveLength(HISTORY_LIMIT);
    expect(s.messages[HISTORY_LIMIT - 1].payload).toBe(String(HISTORY_LIMIT + 4));
  });

  it("keeps decoded data and raw bytes on the topic value", () => {
    const s = applyMessage(newSession("c1"), {
      ...msg("spBv1.0/g/NDATA/e", "\uFFFD"),
      rawHex: "ff",
      decoded: { kind: "sparkplug", value: { seq: 1 } },
    });
    expect(s.topicValues["spBv1.0/g/NDATA/e"].rawHex).toBe("ff");
    expect(s.messages[0].decoded?.kind).toBe("sparkplug");
  });
});

describe("retained tracking", () => {
  it("remembers a retained delivery across live updates until cleared", () => {
    let s = newSession("c1");
    s = applyMessage(s, msg("a", "1", true));
    s = applyMessage(s, msg("a", "2", false));
    s = applyMessage(s, msg("b", "1", false));
    expect(retainedTopics(s, () => true)).toEqual(["a"]);
    s = markRetainedCleared(s, ["a"]);
    expect(retainedTopics(s, () => true)).toEqual([]);
  });

  it("filters by topic filter", () => {
    let s = newSession("c1");
    s = applyMessage(s, msg("x/a", "1", true));
    s = applyMessage(s, msg("y/a", "1", true));
    expect(retainedTopics(s, (t) => topicMatches("x/#", t))).toEqual(["x/a"]);
  });
});

describe("topicMatches", () => {
  it("handles wildcards like a broker", () => {
    expect(topicMatches("a/+/c", "a/b/c")).toBe(true);
    expect(topicMatches("a/+", "a/b/c")).toBe(false);
    expect(topicMatches("a/#", "a")).toBe(true);
    expect(topicMatches("#", "$SYS/x")).toBe(false);
    expect(topicMatches("a/b", "a")).toBe(false);
  });
});

describe("nextSessionId", () => {
  it("reuses freed ids", () => {
    const a = newSession("c1");
    const b = newSession("c3");
    expect(nextSessionId([a, b])).toBe("c2");
    expect(nextSessionId([])).toBe("c1");
  });
});
