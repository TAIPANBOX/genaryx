import { describe, expect, it } from "vitest";

/**
 * Every module in `apps/web/src`, as its own source text, with the suites
 * themselves left out (a test that names a pattern must not be counted as an
 * instance of it).
 *
 * `import.meta.glob` rather than `node:fs` on purpose, twice over: this suite
 * then sees exactly the tree Vite resolves rather than a path guessed from
 * `import.meta.url`, and this app has no `@types/node`, so a `node:fs` import
 * runs green under vitest and fails `tsc --noEmit`. A check that passes one
 * gate and breaks another is not a check.
 */
const SOURCES = import.meta.glob(["../**/*.{ts,tsx}", "!../**/*.test.{ts,tsx}"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/**
 * One `src`-relative path per module.
 *
 * Vite keys a glob against the IMPORTER's own directory, so this suite sitting
 * in `src/lib` sees its neighbours as `./download.ts` and the rest of the tree
 * as `../demo/wgDemoConfig.ts`. Two depths for one tree makes a failure
 * message read as a puzzle, so both are rewritten to the form the header
 * itself uses.
 */
function srcRelative(key: string): string {
  if (key.startsWith("./")) return `lib/${key.slice(2)}`;
  if (key.startsWith("../")) return key.slice(3);
  return key;
}

/** Every module that actually hands the browser a file, `src`-relative. */
function downloaders(): string[] {
  return Object.entries(SOURCES)
    .filter(([, source]) => source.includes("URL.createObjectURL("))
    .map(([path]) => srcRelative(path))
    .sort();
}

/** `download.ts`'s leading block comment, which is where the claim lives. */
function header(): string {
  const text = SOURCES["./download.ts"];
  if (typeof text !== "string") throw new Error("download.ts did not come back from the glob");
  return text.slice(0, text.indexOf("*/") + 2);
}

describe("download.ts's account of itself", () => {
  // CLAUDE.md invariant 7's shape, one level below a gate. This module's
  // header said it was "the console's first download of any kind", and gave
  // as its reason that nothing else under `apps/web/src` reached for
  // `createObjectURL`. Three other modules did, and `git log -S` puts all
  // three of those calls in the tree BEFORE this file existed. A header
  // comment is the whole of the documentation a reader of a helper gets, and
  // a false one sends the next author to build a fourth private copy beside
  // the three that already exist. The claim was true of the intent and
  // nothing ever ran it against the tree; this runs it.
  it("does not claim to be the only module that downloads anything", () => {
    expect(downloaders().length).toBeGreaterThan(1);

    const claim = header().toLowerCase();
    for (const exclusivity of ["nothing else in", "first download of any kind", "the only download"]) {
      expect(claim).not.toContain(exclusivity);
    }
  });

  // Deleting the false half is not the whole correction, and this is the half
  // that keeps working after today. A reader arriving here needs to know the
  // other modules exist and why they are NOT built on this helper, or the next
  // author writes a fifth. So the header must name each one, and the check is
  // driven off the TREE rather than off a list typed here: a fifth downloader
  // added tomorrow fails this test until the header accounts for it, which is
  // the same fault the old sentence was, caught the day it appears.
  it("names every other module that hands the browser a file", () => {
    const claim = header().toLowerCase();
    const siblings = downloaders().filter((path) => path !== "lib/download.ts");
    expect(siblings.length).toBeGreaterThan(0);
    for (const sibling of siblings) {
      const name = sibling.slice(sibling.lastIndexOf("/") + 1).toLowerCase();
      expect(claim, `download.ts's header does not name ${sibling}`).toContain(name);
    }
  });

  // And it has to keep the distinction that earns this one a provenance block:
  // the siblings each save one self-describing artefact (a signed evidence
  // zip, a WireGuard `.conf`), this one saves a table of numbers. Without that
  // sentence the header names three modules and gives no reason they are
  // separate, which is an invitation to fold them in.
  //
  // Honestly labelled: this one is a REGRESSION GUARD, not a defect catcher.
  // Run against the unfixed header it passes, because both words were already
  // there. It is here so the rewrite above cannot drop them, and it proves
  // nothing about the fault the other two caught.
  it("keeps the distinction that earns it a provenance block", () => {
    const claim = header().toLowerCase();
    expect(claim).toContain("table");
    expect(claim).toContain("provenance");
  });

  // The checks above are only worth their run if the glob is actually reaching
  // the tree. An empty or one-entry result would let every assertion here pass
  // by measuring nothing, which is the exact failure CLAUDE.md's invariant 7
  // names: a check must be able to tell "did not fail" from "did not run".
  it("measured the real tree, not an empty glob", () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(50);
    expect(downloaders()).toContain("lib/download.ts");
    expect(downloaders()).toContain("lib/evidence.ts");
  });
});
