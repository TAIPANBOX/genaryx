import { defineConfig } from "vitest/config";

// Prints, after a coverage run, every source file below the 90% target with
// its uncovered lines, most uncovered first. CI's own coverage reporters are
// fixed on the command line (json-summary and text-summary: totals only), so
// without this the run says how much is missing but never where. A test
// reporter, not a coverage reporter, because the command line replaces the
// coverage reporter list and leaves this one alone.
type FileCov = {
  toSummary(): { lines: { total: number; covered: number; pct: number } };
  getUncoveredLines(): string[];
};
type CovMap = { files(): string[]; fileCoverageFor(f: string): FileCov };

function ranges(lines: string[]): string {
  const ns = lines.map(Number).sort((a, b) => a - b);
  const out: string[] = [];
  for (let i = 0; i < ns.length; ) {
    let j = i;
    while (j + 1 < ns.length && ns[j + 1] === ns[j] + 1) j++;
    out.push(i === j ? `${ns[i]}` : `${ns[i]}-${ns[j]}`);
    i = j + 1;
  }
  return out.join(",");
}

const coverageByFile = {
  onCoverage(coverage: unknown) {
    const map = coverage as CovMap;
    if (!map || typeof map.files !== "function") return;
    const root = process.cwd() + "/";
    const rows = map
      .files()
      .map((f) => {
        const fc = map.fileCoverageFor(f);
        const l = fc.toSummary().lines;
        return { f: f.replace(root, ""), pct: l.pct, miss: l.total - l.covered, lines: fc.getUncoveredLines() };
      })
      .filter((r) => r.pct < 90)
      .sort((a, b) => b.miss - a.miss);
    console.log(`\ncoverage by file: ${rows.length} below 90% of lines, most uncovered first`);
    for (const r of rows) {
      console.log(`  ${r.f}  ${r.pct.toFixed(1)}%  missing ${r.miss}  [${ranges(r.lines)}]`);
    }
  },
};

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
    reporters: ["default", coverageByFile],
    // `pnpm exec vitest run --coverage` (CI's web job). Every source file
    // counts, not only the ones some test imports, so a module no test loads
    // shows as uncovered instead of vanishing from the total.
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.ts", "src/**/*.d.ts"],
    },
  },
});
