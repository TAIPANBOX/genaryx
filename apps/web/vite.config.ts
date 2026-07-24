import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  server: {
    port: 1420,
    strictPort: true,
    // `pnpm dev` develops this UI against a real genaryx-web. Proxying keeps
    // it same-origin, so the session cookie and the SSE stream behave exactly
    // as they do in production.
    proxy: {
      "/api": {
        target: process.env.GENARYX_WEB_ORIGIN || "http://127.0.0.1:7420",
        changeOrigin: false,
      },
    },
  },
}));
