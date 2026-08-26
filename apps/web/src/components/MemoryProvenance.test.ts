/**
 * What the Memory provenance card SHOWS, rendered rather than described.
 *
 * A `.ts` test for a `.tsx` component on purpose: `createElement` +
 * `renderToStaticMarkup` need no JSX in the test file and no DOM at all, so
 * this runs in the repo's existing node-environment vitest config
 * (`src/**\/*.test.ts`) with no new dependency and no config change. It reads
 * the markup, which is the only place a claim like "the view drops this
 * field" can actually be settled.
 *
 * Every fixture here is a JSON STRING parsed at runtime, not a TypeScript
 * object literal. That is the point: the bug this file was written for was a
 * console-side type that disagreed with the wire, and an object literal can
 * only ever be as right as the type it is checked against. Parsing the bytes
 * `genaryx_connectors::EngramProvenance` actually serializes puts the wire,
 * not the mirror, in charge.
 */
import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ProvenanceFields } from "./MemoryProvenance";
import type { EngramProvenance } from "../memoryTypes";

/** The markup of one labelled field's VALUE side: everything between the
 * label and the end of that row. Throws when the label is not on the card at
 * all, so a field that stopped rendering reads as a failure rather than as an
 * empty pass (CLAUDE.md invariant 7: "did not fail" must be tellable from
 * "did not run"). */
function rowBlock(html: string, label: string): string {
  const marker = `>${label}</span>`;
  const at = html.indexOf(marker);
  if (at < 0) throw new Error(`no field labelled "${label}" in the rendered card`);
  const rest = html.slice(at + marker.length);
  const end = rest.indexOf("</div>");
  return end < 0 ? rest : rest.slice(0, end);
}

/** The visible text of one field's value side, tags stripped. */
function fieldText(html: string, label: string): string {
  return rowBlock(html, label).replace(/<[^>]*>/g, "");
}

/** One entry per rendered element carrying a `title`, in document order. A
 * list rendered as a list yields one per item; a list flattened into a single
 * string yields exactly one, which is how this file tells them apart. */
function fieldTitles(html: string, label: string): string[] {
  return [...rowBlock(html, label).matchAll(/title="([^"]*)"/g)].map((m) => m[1]);
}

function render(wireJson: string): string {
  const provenance = JSON.parse(wireJson) as EngramProvenance;
  return renderToStaticMarkup(createElement(ProvenanceFields, { provenance }));
}

/** Byte-for-byte the payload `crates/connectors/src/engram.rs`'s own
 * `provenance_parses_both_shapes` test feeds its parser, so this fixture and
 * the backend's agree by construction. */
const EPISODIC_WIRE = JSON.stringify({
  kind: "episodic",
  id: "e1",
  content: "observed a login",
  timestamp: "2026-07-17T10:00:00+00:00",
  actors: ["bot"],
  tags: ["auth"],
  salience: 0.4,
  emotional_valence: 0.0,
  importance_score: 0.2,
  summary_of: ["ep-1", "ep-2"],
  agent_id: "agent://acme/x",
  access_count: 3,
  last_accessed: "2026-07-17T11:00:00+00:00",
  note: "raw observation",
});

const SEMANTIC_WIRE = JSON.stringify({
  kind: "semantic",
  id: "f1",
  subject: "acme",
  predicate: "owes",
  object: "12000",
  confidence: 0.9,
  valid_from: "2026-01-01T00:00:00+00:00",
  valid_to: null,
  recorded_at: "2026-07-17T10:00:00+00:00",
  extracted_from: "ep-3",
  extracted_by_reflection_run: "run-7",
  extraction_model: "claude-sonnet-5",
});

describe("the semantic branch", () => {
  it("shows the triple and the whole extraction chain", () => {
    const html = render(SEMANTIC_WIRE);
    expect(fieldText(html, "subject")).toBe("acme");
    expect(fieldText(html, "predicate")).toBe("owes");
    expect(fieldText(html, "object")).toBe("12000");
    expect(fieldText(html, "confidence")).toBe("0.900");
    expect(fieldText(html, "extracted from")).toBe("ep-3");
    expect(fieldText(html, "reflection run")).toBe("run-7");
    expect(fieldText(html, "extraction model")).toBe("claude-sonnet-5");
  });

  it("says a fact with no end date is still valid, never a blank cell", () => {
    expect(fieldText(render(SEMANTIC_WIRE), "valid to")).toBe("still valid");
  });
});

describe("the episodic branch", () => {
  it("shows the encoding and access metadata", () => {
    const html = render(EPISODIC_WIRE);
    expect(fieldText(html, "content")).toBe("observed a login");
    expect(fieldText(html, "actors")).toBe("bot");
    expect(fieldText(html, "tags")).toBe("auth");
    expect(fieldText(html, "salience")).toBe("0.400");
    expect(fieldText(html, "importance score")).toBe("0.200");
    expect(fieldText(html, "access count")).toBe("3");
  });

  it("distinguishes a score of zero from a score nobody recorded", () => {
    // `emotional_valence: 0.0` is a measurement and prints as one; `null` is
    // not, and must never print as 0.000.
    expect(fieldText(render(EPISODIC_WIRE), "emotional valence")).toBe("0.000");
    const unscored = JSON.parse(EPISODIC_WIRE) as Record<string, unknown>;
    unscored.emotional_valence = null;
    unscored.salience = null;
    const html = render(JSON.stringify(unscored));
    expect(fieldText(html, "emotional valence")).toBe("n/a");
    expect(fieldText(html, "salience")).toBe("n/a");
  });

  it("renders every summarized episode id as its own value", () => {
    // `summary_of` is a LIST on the wire (`Episode.summary_of: list[str]`),
    // and typing it as a scalar is the exact mistake that once made every
    // episodic `why` fail to deserialize in the backend
    // (`crates/connectors/src/engram.rs:141-146`). The console mirrored it as
    // `string | null` anyway: `[]` is not nullish, so the `?? "-"` fallback
    // never fired and two ids rendered as one run-together token.
    const html = render(EPISODIC_WIRE);
    expect(fieldTitles(html, "summary of")).toEqual(["ep-1", "ep-2"]);
  });

  it("says an empty summary list is empty rather than rendering nothing", () => {
    const solo = JSON.parse(EPISODIC_WIRE) as Record<string, unknown>;
    solo.summary_of = [];
    const html = render(JSON.stringify(solo));
    // Not a blank cell, and not a "-" that could be read as a value: an
    // episode that summarizes nothing is a fact about the episode.
    expect(fieldText(html, "summary of")).toBe("summarizes nothing");
    expect(fieldTitles(html, "summary of")).toEqual([]);
  });
});
