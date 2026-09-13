// Standalone vitest config for the PLC-10 golden generator. The main
// vitest.config.ts excludes scripts/** so the generator never runs in the
// default `pnpm test` suite; run it explicitly with:
//   cd apps/voltrus-plc && pnpm vitest run --config scripts/gen-goldens.vitest.config.ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  root: __dirname,
  test: {
    environment: "node",
    globals: true,
    include: ["gen-goldens.test.ts"],
  },
});
