import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { EGRESS_EXPORT_COLUMNS, EgressRowView, EgressScope, egressExportMeta } from "./EgressView";
import { toCsv } from "../lib/download";
import type { EgressPanel, EgressRow } from "../egressTypes";

/**
 * Two fields the egress record carries and the table dropped: `run_id`, which
 * is how a refusal is traced back to the run that caused it, and
 * `url_sha384`, which is the only way two records can be compared without
 * either of them holding the address.
 *
 * Both are `| null` on the wire and the two nulls do not mean the same thing
 * (`egressTypes.ts` says so in its header), so neither may render as an empty
 * cell that reads as a value.
 */

function row(over: Partial<EgressRow> = {}): EgressRow {
  return {
    ts: "2026-08-25T09:14:02.113Z",
    agent_id: "agent://acme.example/fraud/checker",
    run_id: "run-42",
    outcome: "blocked",
    origin: "https://paste.example",
    url_sha384: "a".repeat(96),
    backend: null,
    enforcement: null,
    content_bytes: null,
    verdict: "deny_policy",
    reason: "host not on the allowlist",
    ...over,
  };
}

function panel(rows: EgressRow[], over: Partial<EgressPanel> = {}): EgressPanel {
  return {
    measured: true,
    note: `Read from the 4000 most recent events on the bus.`,
    totals: { fetched: 0, blocked: rows.length, by_verdict: { deny_policy: rows.length }, navigation_only: 0, subresources_unknown: 0 },
    rows,
    ...over,
  };
}

const render = (r: EgressRow) =>
  renderToStaticMarkup(
    createElement("table", null, createElement("tbody", null, createElement(EgressRowView, { r }))),
  );

describe("EgressRowView", () => {
  it("says which run reached for this", () => {
    expect(render(row())).toContain("run-42");
  });

  // Null run_id is a line the record carried no run for. An empty cell in a
  // column of run ids reads as "no run", which is a claim about the agent
  // rather than about the record.
  it("says the line carried no run rather than leaving the cell blank", () => {
    expect(render(row({ run_id: null }))).toContain("no run recorded");
  });

  // The digest is 96 hex characters and nobody reads one. It is on the row so
  // a person can compare two rows, and it costs no column.
  it("keeps the url digest reachable on the row", () => {
    expect(render(row())).toContain("a".repeat(96));
  });

  it("says the digest was not recorded rather than implying an empty url", () => {
    const out = render(row({ url_sha384: null }));
    expect(out).toMatch(/url hash: not recorded/);
    expect(out).not.toContain("null");
  });
});

describe("the egress export", () => {
  it("carries both dropped fields as columns of their own", () => {
    const keys = EGRESS_EXPORT_COLUMNS.map((c) => c.key);
    expect(keys).toContain("run_id");
    expect(keys).toContain("url_sha384");
  });

  it("writes an absent field as an empty cell, never as a zero or a word", () => {
    const csv = toCsv(EGRESS_EXPORT_COLUMNS, [row({ run_id: null, url_sha384: null, content_bytes: null })], egressExportMeta(panel([]), 200));
    const body = csv.split("\n").filter((l) => !l.startsWith("#"))[1];
    expect(body).not.toContain("null");
    expect(body).not.toContain("undefined");
    expect(body.split(",").filter((c) => c === "").length).toBeGreaterThanOrEqual(3);
  });

  // The totals on this panel are the aggregate of exactly the rows returned:
  // `egress_recent` breaks out of its loop the moment it has `limit` rows, so
  // nothing is counted that is not also listed. Saying so is what stops a
  // reader treating the header figures as the whole estate.
  it("says the totals are the same lines as the rows", () => {
    const meta = egressExportMeta(panel([row()]), 200);
    expect(meta.caveats?.join(" ")).toMatch(/aggregate of exactly these rows/i);
  });

  // And when the cap was actually reached, that stops being a note about how
  // the backend works and becomes a fact about this file.
  it("says so, with the number, only when the row cap was actually reached", () => {
    const under = egressExportMeta(panel([row(), row()]), 200);
    expect(under.caveats?.join(" ")).not.toMatch(/cap/i);

    const at = egressExportMeta(panel([row(), row()]), 2);
    expect(at.caveats?.join(" ")).toMatch(/cap/i);
    expect(at.caveats?.join(" ")).toContain("2");
  });

  it("carries what the backend said it read, and says when it said nothing", () => {
    expect(egressExportMeta(panel([row()]), 200).windows.join(" ")).toContain("4000 most recent events");
    expect(egressExportMeta(panel([row()], { note: null }), 200).windows.join(" ")).toMatch(/did not say/i);
  });

  it("says the origin is all there is, so nobody looks for the path in this file", () => {
    const meta = egressExportMeta(panel([row()]), 200);
    expect(meta.caveats?.join(" ")).toMatch(/path and query/i);
  });
});

/**
 * The numbers above the table are the aggregate of exactly the rows in it.
 *
 * `egress_recent` accumulates its totals inside the same loop that fills
 * `rows`, and `if out.len() >= limit { break; }` sits AFTER the push
 * (`crates/api/src/egress/mod.rs`), so fetched + blocked is always precisely
 * `rows.length` and can never exceed the cap. The hero renders that figure
 * under the words "what agents reached", which is a question about the box.
 *
 * `panel.note` is the backend saying which slice it read, and the panel put
 * it in a hover `title` on the freshness badge, where it qualifies nothing
 * anybody reads. That is CLAUDE.md invariant 8 on screen: every figure here
 * is accurate about itself and silent about what was asked.
 */
describe("EgressScope", () => {
  const render = (p: EgressPanel, limit: number) =>
    renderToStaticMarkup(createElement(EgressScope, { panel: p, limit }));

  it("says in the backend's own words which slice the numbers came from", () => {
    expect(render(panel([row()]), 200)).toContain("4000 most recent events");
  });

  it("ties the totals to the rows, so neither is read as the estate", () => {
    expect(render(panel([row(), row()]), 200)).toMatch(/these 2 line\(s\)/i);
  });

  // At the cap the sentence stops being about how the backend works and
  // becomes a fact about what is missing from this screen. Matched on
  // "never counted" rather than on the word "older": the backend's own note
  // ends "An older fetch than that is in the Bus Explorer, not here", so a
  // test looking for "older" would be reading the backend's sentence and
  // calling it mine.
  it("says older egress was never counted, only once the cap was actually hit", () => {
    expect(render(panel([row(), row()]), 200)).not.toMatch(/never counted/i);
    const at = render(panel([row(), row()]), 2);
    expect(at).toMatch(/never counted/i);
    expect(at).toMatch(/stopped at 2 line/i);
  });

  // A backend that said nothing about its window is a backend that said
  // nothing. Naming a window it did not name would be the invention.
  it("does not invent a window the backend did not describe", () => {
    const out = render(panel([row()], { note: null }), 200);
    expect(out).toMatch(/did not say/i);
    expect(out).not.toContain("null");
    expect(out).not.toContain("undefined");
  });
});
