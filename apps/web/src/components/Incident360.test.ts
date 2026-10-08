import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Incident360 } from "./Incident360";
import { PopoverProvider } from "../lib/popover";
import type { UnifiedIncident } from "../lib/incidents";

/**
 * A bus event with no run id has no run to read, so "Where it was stopped"
 * and "What led to it" can never fill in. They used to wait on the run's
 * events forever and say "reading…", which reads as a console still working
 * on an answer it will never get. Server-rendered, like the other component
 * tests here: the first render is the state that matters, since no fetch can
 * ever change it.
 */
const runless = {
  id: "bus:7",
  source: "bus",
  severity: "high",
  title: "quality drift",
  detail: "",
  ackable: false,
  explainable: false,
  raw: {
    id: 7,
    env: "live",
    ts: "2026-10-08T07:00:00Z",
    source: "verdryx",
    type: "quality_drift",
    agent_id: "agent://meridian.io/data/data-quality-checker",
    run_id: null,
    severity: "high",
    schema: "taipanbox.dev/agent-event/v0.2",
    on_behalf_of: [],
    data: {},
    prev_hash: null,
    raw: "",
    file: null,
    off: null,
  },
} as unknown as UnifiedIncident;

const render = (row: UnifiedIncident) =>
  renderToStaticMarkup(
    createElement(PopoverProvider, null, createElement(Incident360, { row, onClose: () => {}, onOpenAgent: () => {} })),
  );

describe("Incident 360 on an event with no run id", () => {
  it("a_runless_incident_says_there_is_no_run_to_read_instead_of_reading_forever", () => {
    const html = render(runless);
    const sentence = "no run id on this event, so there is no run to read";
    expect(html.split(sentence).length - 1).toBe(2);
    // Both run sections answer; neither is left on the placeholder.
    const where = html.slice(html.indexOf("WHERE IT WAS STOPPED"), html.indexOf("WHAT LED TO IT"));
    expect(where).not.toContain("reading…");
    const led = html.slice(html.indexOf("WHAT LED TO IT"), html.indexOf("WHO ELSE THIS TOUCHES"));
    expect(led).not.toContain("reading…");
  });

  it("an_incident_with_a_run_id_still_reads_its_run", () => {
    const withRun = { ...runless, raw: { ...(runless.raw as object), run_id: "run-1" } } as UnifiedIncident;
    const html = render(withRun);
    expect(html).not.toContain("no run id on this event");
  });
});
