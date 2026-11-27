import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { EventRow } from "./EventRow";
import type { UiEvent } from "../types";

const ev = (o: Partial<UiEvent> = {}): UiEvent => ({
  id: 1,
  env: "prod",
  ts: "2026-11-27T10:11:12.345",
  source: "tokenfuse",
  type: "run_killed",
  agent_id: "agent://org/a",
  run_id: "run-7",
  severity: "high",
  schema: "v1",
  on_behalf_of: [],
  data: { k: 1 },
  prev_hash: null,
  raw: '{"k":1}',
  file: null,
  off: null,
  ...o,
});

const render = (event: UiEvent, extra: object = {}) =>
  renderToStaticMarkup(createElement(EventRow, { event, expanded: false, onToggle: () => {}, ...extra }));

describe("an event row", () => {
  it("a_collapsed_row_shows_the_clock_with_milliseconds_and_no_detail", () => {
    const out = render(ev());
    expect(out).toContain("10:11:12.345");
    expect(out).toContain('aria-expanded="false"');
    expect(out).not.toContain("raw NDJSON line");
  });

  it("a_timestamp_that_is_not_a_date_is_shown_as_it_came", () => {
    expect(render(ev({ ts: "not-a-time" }))).toContain(">not-a-time<");
  });

  it("a_pinned_row_is_dimmed_in_place", () => {
    expect(render(ev(), { selected: true })).toContain("opacity:0.35");
    expect(render(ev())).not.toContain("opacity:0.35");
  });

  it("an_expanded_row_lists_provenance_and_a_missing_value_reads_as_a_dash_or_none", () => {
    const out = render(ev(), { expanded: true });
    expect(out).toContain("raw NDJSON line");
    expect(out).toContain('aria-expanded="true"');
    expect(out).toContain("run-7");
    expect(out).toContain(">none<");
    expect(out).not.toContain("on_behalf_of");
    const bare = render(ev({ run_id: null }), { expanded: true });
    expect(bare).toContain(">-<");
  });

  it("an_expanded_row_with_a_delegation_chain_and_an_offset_shows_both", () => {
    const out = render(ev({ on_behalf_of: ["user://a", "user://b"], off: 0, file: "f.ndjson" }), { expanded: true });
    expect(out).toContain("user://a -&gt; user://b");
    expect(out).toContain(">0<");
    expect(out).toContain("f.ndjson");
  });
});
