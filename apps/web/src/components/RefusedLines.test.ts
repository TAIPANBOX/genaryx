import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ReasonItem } from "./RefusedLines";
import { formatTimestamp } from "../lib/format";
import type { QuarantineReason } from "../lib/quarantine";

/**
 * A refusal count with no time on it cannot answer the only question an
 * operator has about it: is that producer still broken, or was it fixed an
 * hour ago and this is the scar? `QuarantineReason.last_ts` is on the wire
 * (`crates/core/src/store.rs`, `Option<String>`) and was rendered nowhere.
 */

function reason(over: Partial<QuarantineReason> = {}): QuarantineReason {
  return {
    reason: "agent_id must start with agent://",
    count: 12,
    last_ts: "2026-08-25T09:14:02.113Z",
    example_file: "/var/lib/genaryx/aws-comparable.ndjson",
    example_offset: 4096,
    raw_excerpt: '{"agent_id":"aws-comparable-agent","run_id":"aws-176-blocked-001"',
    ...over,
  };
}

const render = (r: QuarantineReason) =>
  renderToStaticMarkup(createElement("ul", null, createElement(ReasonItem, { reason: r })));

describe("ReasonItem", () => {
  it("says when this producer last broke the envelope", () => {
    const r = reason();
    expect(render(r)).toContain(formatTimestamp(r.last_ts as string));
  });

  // Null is not a time and must not be rendered as one, nor left out: an
  // absent line reads as "recent" to anybody scanning the list.
  it("says the refusal carried no time rather than showing none", () => {
    const out = render(reason({ last_ts: null }));
    expect(out).toContain("not recorded");
    expect(out).not.toContain("Invalid Date");
    expect(out).not.toContain("null");
  });
});
