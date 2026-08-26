import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { FeedEmptyState, FeedReadFailure } from "./BusExplorer";

/**
 * The one thing this panel must not do: look the same when it has nothing to
 * show and when it could not look.
 *
 * `fetchRecentEvents` has carried an `error` string since the fixture
 * fallback was removed, documented in `lib/recentEvents.ts` as being there
 * "so a panel can say what happened instead of rendering an unexplained empty
 * list". The panel destructured `events` and `source` and dropped it, so the
 * list said "no events yet" - a claim about the bus - while the status bar
 * said the box had not answered. One of those two sentences was false.
 *
 * Server-rendered rather than mounted: there is no DOM in this repo's test
 * environment (`vitest.config.ts` is `environment: "node"`, no jsdom in the
 * tree). Both components take their whole world as props, so static markup is
 * the real output.
 */

const html = (node: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(node);

describe("FeedReadFailure", () => {
  it("says what the box said, rather than leaving the list unexplained", () => {
    const out = html(
      createElement(FeedReadFailure, { source: "error", error: "connection refused (os error 61)", count: 0 }),
    );
    expect(out).toContain("connection refused (os error 61)");
  });

  it("says the box gave no reason rather than printing an empty one", () => {
    const out = html(createElement(FeedReadFailure, { source: "error", error: undefined, count: 0 }));
    expect(out).toContain("gave no reason");
    expect(out).not.toContain("undefined");
  });

  // Rows CAN arrive after a failed first read: the live subscription is a
  // separate path, and it prepends. The list then looks like the bus and is
  // only what happened since, which is the smaller claim it has to make.
  it("does not let live rows stand in for the history that was never read", () => {
    const out = html(createElement(FeedReadFailure, { source: "error", error: "no answer", count: 3 }));
    expect(out).toContain("3");
    expect(out).toMatch(/arrived|since/i);
  });

  it("makes no claim at all when the read succeeded", () => {
    expect(html(createElement(FeedReadFailure, { source: "backend", error: undefined, count: 0 }))).toBe("");
    expect(html(createElement(FeedReadFailure, { source: "mock", error: undefined, count: 0 }))).toBe("");
  });
});

describe("FeedEmptyState", () => {
  // An empty bus is information. Saying it about a bus nobody could read is
  // the invariant-4 failure one field down.
  it("calls an empty bus empty", () => {
    expect(html(createElement(FeedEmptyState, { source: "backend" }))).toContain("no events yet");
  });

  it("refuses to call an unread bus empty", () => {
    const out = html(createElement(FeedEmptyState, { source: "error" }));
    expect(out).not.toContain("no events yet");
    expect(out).toContain("not a report that the bus is empty");
  });
});
