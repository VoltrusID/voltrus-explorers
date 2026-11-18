import { describe, expect, it } from "vitest";
import { REPLAY_INITIAL, applyReplayEvent } from "../replay";

describe("replay state", () => {
  const running = { ...REPLAY_INITIAL, running: true, info: { frames: 10, durationS: 1 } };

  it("follows progress and the end of a replay", () => {
    const s = applyReplayEvent(running, { done: false, pos: 4, total: 10, pass: 0 });
    expect(s.running).toBe(true);
    expect(s.progress).toEqual({ pos: 4, total: 10, pass: 0 });
    const end = applyReplayEvent(s, { done: true, pos: 10, total: 10, pass: 0 });
    expect(end.running).toBe(false);
    expect(end.stopped).toBeNull();
    expect(end.error).toBeNull();
  });

  it("says when a disconnect stopped it", () => {
    const s = applyReplayEvent(running, { done: true, stopped: "disconnected" });
    expect(s.running).toBe(false);
    expect(s.stopped).toMatch(/disconnected/);
    expect(s.progress).toBeNull();
  });

  it("keeps a send error", () => {
    const s = applyReplayEvent(running, { done: true, error: "the bus is closed", pos: 3, total: 10 });
    expect(s.error).toBe("the bus is closed");
    expect(s.progress?.pos).toBe(3);
  });
});
