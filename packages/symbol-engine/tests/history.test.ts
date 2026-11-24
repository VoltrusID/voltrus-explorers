import { describe, expect, it } from "vitest";
import { History } from "../src";

// Discipline (mirrors voltrus-hmi store.ts): the caller keeps `present`
// OUTSIDE the stack and calls push(current) BEFORE mutating. So evolving
// s1→s2→s3 means: push(s1), apply → s2; push(s2), apply → s3.

describe("History", () => {
  it("undo steps back through checkpoints, null when exhausted", () => {
    const h = new History<number[]>();
    h.push([1]); // checkpoint s1, change → s2
    h.push([2]); // checkpoint s2, change → s3
    expect(h.canUndo()).toBe(true);
    expect(h.undo([3])).toEqual([2]);
    expect(h.undo([2])).toEqual([1]);
    expect(h.undo([1])).toBeNull();
    expect(h.canUndo()).toBe(false);
  });

  it("redo steps forward, null when exhausted", () => {
    const h = new History<number[]>();
    h.push([1]);
    h.push([2]);
    let cur = h.undo([3])!; // [2]
    expect(h.canRedo()).toBe(true);
    cur = h.redo(cur)!;
    expect(cur).toEqual([3]);
    expect(h.canRedo()).toBe(false);
    expect(h.redo(cur)).toBeNull();
  });

  it("full round trip: walk back to the oldest, forward to the newest", () => {
    const h = new History<number[]>();
    h.push([1]);
    h.push([2]);
    h.push([3]); // present [4]
    let cur: number[] = [4];
    cur = h.undo(cur)!;
    expect(cur).toEqual([3]);
    cur = h.undo(cur)!;
    expect(cur).toEqual([2]);
    cur = h.undo(cur)!;
    expect(cur).toEqual([1]);
    expect(h.canUndo()).toBe(false);
    expect(h.canRedo()).toBe(true);
    cur = h.redo(cur)!;
    cur = h.redo(cur)!;
    cur = h.redo(cur)!;
    expect(cur).toEqual([4]);
    expect(h.canRedo()).toBe(false);
  });

  it("push truncates the redo tail on divergence", () => {
    const h = new History<number[]>();
    h.push([1]);
    h.push([2]); // stack [[1],[2]], present [3]
    const cur = h.undo([3])!; // [2], redo=[[3]]
    h.push([2]); // diverge from [2] → [4]
    expect(h.canRedo()).toBe(false);
    expect(h.redo([4])).toBeNull();
    expect(h.undo([4])).toEqual([2]);
  });

  it("push stores a clone — mutating the pushed object afterwards does not leak", () => {
    const h = new History<{ n: number }>();
    const s1 = { n: 1 };
    h.push(s1); // checkpoint s1
    s1.n = 999; // present mutated in place
    expect(h.undo({ n: 999 })).toEqual({ n: 1 });
  });

  it("undo parks a clone of current — mutating present after undo does not corrupt redo", () => {
    const h = new History<{ n: number }>();
    h.push({ n: 1 });
    h.push({ n: 2 }); // present p = {n:3}
    const p = { n: 3 };
    h.undo(p);
    p.n = 999; // caller keeps mutating their (now stale) present object
    expect(h.redo({ n: 1 })).toEqual({ n: 3 }); // parked copy was cloned at undo time
  });

  it("capacity defaults to 50 and drops the oldest checkpoints", () => {
    const h = new History<number[]>();
    for (let i = 0; i <= 60; i++) h.push([i]); // 61 checkpoints → keeps 11..60
    let cur: number[] = [61]; // present
    const seen: number[] = [];
    for (;;) {
      const prev = h.undo(cur);
      if (prev === null) break;
      cur = prev;
      seen.push(prev[0]);
    }
    expect(seen).toEqual(Array.from({ length: 50 }, (_, i) => 60 - i));
    expect(seen).not.toContain(10);
    expect(h.canUndo()).toBe(false);
  });

  it("honors a custom capacity", () => {
    const h = new History<number[]>(2);
    h.push([1]);
    h.push([2]);
    h.push([3]); // [1] evicted → stack [[2],[3]], present [4]
    expect(h.undo([4])).toEqual([3]);
    expect(h.undo([3])).toEqual([2]);
    expect(h.undo([2])).toBeNull();
  });

  it("clear wipes both stacks", () => {
    const h = new History<number[]>();
    h.push([1]);
    h.push([2]);
    h.undo([3]);
    h.clear();
    expect(h.canUndo()).toBe(false);
    expect(h.canRedo()).toBe(false);
    expect(h.undo([3])).toBeNull();
    expect(h.redo([3])).toBeNull();
  });
});
