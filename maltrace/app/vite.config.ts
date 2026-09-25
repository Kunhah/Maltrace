/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// @ts-expect-error process is a nodejs global
const apiTarget = process.env.MALTRACE_API ?? "http://127.0.0.1:1421";

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Vitest configuration. jsdom for DOM APIs (Notification, FileReader,
  // …) so the App-level paste / notify helpers can be unit-tested.
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
    // 4. forward web-mode API calls to a locally-running maltrace-server.
    //    Tauri builds don't go through here (they use IPC), so this is purely
    //    for `npm run dev:web`.
    proxy: {
      "/api": {
        target: apiTarget,
        changeOrigin: false,
      },
    },
  },
}));
