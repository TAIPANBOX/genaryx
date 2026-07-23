import { defineConfig } from "vitest/config";

// Standalone from `vite.config.ts` on purpose: that file's `server`/`proxy`
// block is tuned for `tauri dev`/`dev:web` and has no bearing on a plain
// unit-test run, so keeping this separate avoids any chance of the test
// runner picking up Tauri-dev-only settings (or vice versa). No plugins:
// today's only suite (`src/lib/access.ts`) is a pure TS module with no JSX/
// CSS import, so the default "node" environment is enough - add `jsdom` (and
// the `react` plugin) here if a future suite needs to render components.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
