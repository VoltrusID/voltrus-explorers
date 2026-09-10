import { deepClone } from "./util.js";

/**
 * Undo/redo history of immutable model states.
 *
 * Mirrors voltrus-hmi's store semantics: the caller keeps `present` outside
 * the stack; `push` stores a deep clone (truncate the redo tail); `undo`/
 * `redo` take the current state, park a clone of it on the opposite stack,
 * and return a clone of the neighbouring state. Overflow drops the oldest
 * entry past `capacity`.
 */
export class History<T> {
  private undoStack: T[] = [];
  private redoStack: T[] = [];

  constructor(private capacity: number = 50) {}

  /** Checkpoint `state`; clears the redo tail. */
  push(state: T): void {
    this.undoStack.push(deepClone(state));
    if (this.undoStack.length > this.capacity) this.undoStack.shift();
    this.redoStack.length = 0;
  }

  /** Step back: returns the previous state, or null when at the oldest. */
  undo(current: T): T | null {
    const prev = this.undoStack.pop();
    if (prev === undefined) return null;
    this.redoStack.push(deepClone(current));
    return prev;
  }

  /** Step forward: returns the next state, or null when at the newest. */
  redo(current: T): T | null {
    const next = this.redoStack.pop();
    if (next === undefined) return null;
    this.undoStack.push(deepClone(current));
    return next;
  }

  canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  clear(): void {
    this.undoStack.length = 0;
    this.redoStack.length = 0;
  }
}
