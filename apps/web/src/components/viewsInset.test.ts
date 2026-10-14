import { describe, expect, it } from "vitest";

/**
 * Every top-level view scrolls inside the same container: inset from the
 * rails (`px-5 py-4`) with a gap between its cards (`gap-*`). Web Egress and
 * Statistics once used a bare `flex-1 min-h-0 flex flex-col overflow-auto`,
 * so their cards ran against the side rails and touched each other.
 */
const SOURCES = import.meta.glob("./*View.tsx", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

describe("the views' scroll containers", () => {
  it("every_view_insets_its_cards_and_spaces_them", () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(15);
    const offenders: string[] = [];
    for (const [file, src] of Object.entries(SOURCES)) {
      for (const m of src.matchAll(/className="(flex-1 min-h-0[^"]*overflow[^"]*)"/g)) {
        const cls = m[1].split(/\s+/);
        if (!cls.includes("px-5") || !cls.some((c) => /^py-\d/.test(c)) || !cls.some((c) => /^gap-\d/.test(c))) {
          offenders.push(`${file}: ${m[1]}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
