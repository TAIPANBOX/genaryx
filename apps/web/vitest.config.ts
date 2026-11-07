import { defineConfig } from "vitest/config";

// Standalone from `vite.config.ts` on purpose: that file's `server`/`proxy`
// block is tuned for `tauri dev`/`dev:web` and has no bearing on a plain
// unit-test run, so keeping this separate avoids any chance of the test
// runner picking up Tauri-dev-only settings (or vice versa). No plugins, and
// the "node" environment by default: most suites read pure modules or render
// with `react-dom/server`. A suite that needs a live document (state, clicks,
// unmounting) asks for one in its first line: `// @vitest-environment jsdom`.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
