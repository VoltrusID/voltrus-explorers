import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

// Same aliases as vite.config.ts so test files resolve the wasm-pack pkgs too.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@voltrus/stc-wasm": fileURLToPath(new URL("../../crates/mt-stc-wasm/pkg", import.meta.url)),
      "@voltrus/runtime-wasm": fileURLToPath(
        new URL("../../crates/mt-plc-runtime-wasm/pkg", import.meta.url),
      ),
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    globals: true,
    exclude: ["e2e/**", "**/node_modules/**", "cloudflare-worker/**", "scripts/**"],
  },
});
