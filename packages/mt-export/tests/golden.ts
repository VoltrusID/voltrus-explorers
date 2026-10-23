import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect } from "vitest";

const DIR = join(dirname(fileURLToPath(import.meta.url)), "golden");

/** Compare with tests/golden/<name>. `UPDATE_GOLDEN=1` rewrites the file; a missing file is an error otherwise. */
export function expectGolden(name: string, content: string): void {
  const path = join(DIR, name);
  if (process.env.UPDATE_GOLDEN === "1") {
    mkdirSync(DIR, { recursive: true });
    writeFileSync(path, content);
    return;
  }
  if (!existsSync(path)) throw new Error(`Missing golden file ${name}; run UPDATE_GOLDEN=1 bun run test and review it`);
  expect(content).toBe(readFileSync(path, "utf8"));
}
