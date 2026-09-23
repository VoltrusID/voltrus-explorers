import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

const host = process.env.TAURI_DEV_HOST;

// wasm-pack output dirs for the in-browser compile + simulate pipeline
// (consumed by src/wasm/sim.worker.ts). Pointed at the pkg directories so the
// packages' `main`/`types` resolve; plain ESM glue, hence excluded from dep
// pre-bundling (its `new URL(..._bg.wasm, import.meta.url)` must survive).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  resolve: {
    alias: {
      "@voltrus/stc-wasm": fileURLToPath(new URL("../../crates/mt-stc-wasm/pkg", import.meta.url)),
      "@voltrus/runtime-wasm": fileURLToPath(
        new URL("../../crates/mt-plc-runtime-wasm/pkg", import.meta.url),
      ),
    },
  },
  optimizeDeps: {
    exclude: ["@voltrus/stc-wasm", "@voltrus/runtime-wasm"],
  },
  worker: {
    format: "es",
  },
  server: {
    port: 1440,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1441 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
