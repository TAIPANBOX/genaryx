import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { EGRESS_EXPORT_COLUMNS, EgressRowView, egressExportMeta } from "./EgressView";
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
