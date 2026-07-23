import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  // Vite options tailored for Tauri development, applied in `tauri dev` /
  // `tauri build` (see tauri.conf.json: devUrl / frontendDist).
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
    // 4. `npm run dev:web` develops the browser shell against a real
    // genaryx-web. Proxying keeps it same-origin, so the session cookie and
    // the SSE stream behave exactly as they do in production.
    proxy: {
      "/api": {
        target: process.env.GENARYX_WEB_ORIGIN || "http://127.0.0.1:7420",
        changeOrigin: false,
      },
    },
  },
}));
