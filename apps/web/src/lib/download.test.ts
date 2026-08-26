import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC = dirname(dirname(fileURLToPath(import.meta.url)));

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...sourceFiles(full));
    else if (/\.tsx?$/.test(entry.name)) out.push(full);
  }
  return out;
}

/** Every module in `apps/web/src` that actually hands the browser a file. */
function downloaders(): string[] {
  return sourceFiles(SRC)
    .filter((f) => readFileSync(f, "utf8").includes("URL.createObjectURL("))
    .map((f) => relative(SRC, f))
    .sort();
}

/** `download.ts`'s leading block comment, which is where the claim lives. */
function header(): string {
  const text = readFileSync(join(SRC, "lib", "download.ts"), "utf8");
  const end = text.indexOf("*/");
  return text.slice(0, end + 2);
}

describe("download.ts's account of itself", () => {
  // CLAUDE.md invariant 7's shape, one level down from a gate: the claim was
  // written when it was true of the intent, and nothing ever ran it against
  // the tree. This module's header said it was "the console's first download
  // of any kind (nothing else in `apps/web/src` calls `createObjectURL`)".
  // Three other modules did, two of them older than this file. A comment is
  // the only documentation a reader of a helper gets, and a false one sends
  // the next author to build a second helper next to the three that exist.
  it("does not claim to be the only module that downloads anything", () => {
    const found = downloaders();
    expect(found.length).toBeGreaterThan(1);

    const claim = header();
    for (const exclusivity of [
      "nothing else in",
      "first download of any kind",
      "the only download",
    ]) {
      expect(claim.toLowerCase()).not.toContain(exclusivity);
    }
  });

  // Deleting the false half is not the whole correction. A reader arriving at
  // this helper needs to know the other two exist and why they are NOT built
  // on it, or the next author writes a fourth. So the header has to name them,
  // and it has to keep the distinction that earns this one a provenance block:
  // the siblings save one self-describing artefact each (a signed evidence
  // zip, a WireGuard `.conf`), this one saves a table of numbers.
  it("names the siblings it is not, and the distinction that earns it a provenance block", () => {
    const claim = header().toLowerCase();
    expect(claim).toContain("evidence.ts");
    expect(claim).toContain("remote.ts");
    expect(claim).toContain("table");
    expect(claim).toContain("provenance");
  });
});
