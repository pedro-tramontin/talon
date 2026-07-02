/// <reference types="vitest" />
// Use vitest/config (not "vite") so the `test:` block is typed as vitest
// config rather than as a vite-specific unknown field. Required for
// vitest 3.x to suppress the "test field is not a vite option" warning.
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri expects the dev server on a fixed port and forwards HMR over IPC.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
    hmr: { protocol: "ws", host: "127.0.0.1", port: 5174 },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
  },
});
