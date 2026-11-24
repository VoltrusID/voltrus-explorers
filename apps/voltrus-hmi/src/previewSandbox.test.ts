import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The Live Preview iframe loads the exported runtime from the hmipreview
// custom scheme, which Tauri treats as a local origin. It must stay sandboxed
// with scripts only: allow-same-origin would let the preview script reach the
// app window and its IPC.
describe("Live Preview iframe sandbox", () => {
  const app = readFileSync(fileURLToPath(new URL("./App.tsx", import.meta.url)), "utf8");
  it("uses sandbox=allow-scripts only", () => {
    const sandboxes = [...app.matchAll(/sandbox="([^"]*)"/g)].map((m) => m[1]);
    expect(sandboxes).toEqual(["allow-scripts"]);
  });
});
