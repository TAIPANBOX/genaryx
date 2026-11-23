// @vitest-environment jsdom
/**
 * Invariant 51: the Notifications tab shows what heraldyx recorded sending,
 * and to whom, from its own journal, and never renders "nothing was sent" for
 * a box that could not look.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NotificationsPanel } from "../notificationsTypes";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const box = vi.hoisted(() => ({ answer: null as unknown, refusal: null as unknown, calls: [] as string[] }));

vi.mock("../lib/transport", () => ({
  hasBackend: () => true,
  isWebShell: () => true,
  webApiBase: () => "",
  subscribeBackend: () => () => {},
  requiredRoleFromCommandError: () => null,
  invokeBackend: async (command: string) => {
    box.calls.push(command);
    if (box.refusal) throw box.refusal;
    return box.answer;
  },
}));

import { NotificationsView } from "./NotificationsView";
import { NAV_SECTIONS } from "../lib/views";

const RECORD = {
  line: 2,
  ts: "2026-11-23T09:16:00Z",
  kind: "alert",
  about: "kill_switch:run-0002",
  to: ["ops@example.invalid", "oncall@example.invalid"],
  to_total: 2,
  transport: "smtp",
  outcome: "refused",
  error: "550 5.7.1 relay denied",
  agent_id: "agent://example.invalid/sre/rca-copilot",
  run_id: "run-0002",
};

function readPanel(over: Partial<Extract<NotificationsPanel, { state: "read" }>> = {}): NotificationsPanel {
  return {
    state: "read",
    path: "/var/lib/stack/heraldyx/sent.ndjson",
    records: [RECORD],
    total: 1,
    accepted: 0,
    refused: 1,
    by_kind: { alert: 1 },
    chain: { state: "nothing_bound" },
    window_truncated: false,
    skipped: [],
    skipped_total: 0,
    ...over,
  };
}

let host: HTMLDivElement | undefined;
let root: Root | undefined;

async function render() {
  const h = document.createElement("div");
  document.body.appendChild(h);
  host = h;
  const r = createRoot(h);
  root = r;
  await act(async () => {
    r.render(createElement(NotificationsView));
  });
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

function byTestId(id: string): HTMLElement | null {
  return host?.querySelector(`[data-testid="${id}"]`) ?? null;
}

beforeEach(() => {
  box.answer = null;
  box.refusal = null;
  box.calls = [];
});

afterEach(() => {
  const r = root;
  if (r) act(() => r.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
});

describe("the Notifications tab", () => {
  it("the_notifications_tab_is_in_the_assure_section", () => {
    const assure = NAV_SECTIONS.find((s) => s.label === "Assure");
    expect(assure?.items.map((i) => i.id)).toContain("notifications");
  });

  it("a_sent_message_shows_who_was_written_to_and_how_the_transport_answered", async () => {
    box.answer = readPanel();
    await render();
    expect(box.calls).toEqual(["notifications_sent"]);
    const table = byTestId("notifications-table");
    expect(table?.textContent).toContain("ops@example.invalid, oncall@example.invalid");
    expect(table?.textContent).toContain("kill_switch:run-0002");
    expect(table?.textContent).toContain("refused");
    expect(table?.textContent).toContain("550 5.7.1 relay denied");
    expect(byTestId("notifications-summary")?.textContent).toMatch(/1 message\(s\) recorded \(alert 1\): 0 accepted, 1 refused/);
  });

  it("a_box_that_names_no_journal_says_so_and_names_the_setting", async () => {
    box.answer = { state: "not_configured", variable: "GENARYX_HERALDYX_SENT" } satisfies NotificationsPanel;
    await render();
    expect(byTestId("notifications-not-configured")?.textContent).toContain("GENARYX_HERALDYX_SENT");
    expect(byTestId("notifications-empty")).toBeNull();
    expect(byTestId("notifications-table")).toBeNull();
  });

  it("a_journal_that_could_not_be_read_is_not_shown_as_an_empty_one", async () => {
    box.answer = { state: "unreadable", path: "/x/sent.ndjson", reason: "no file there" } satisfies NotificationsPanel;
    await render();
    expect(byTestId("notifications-unreadable")?.textContent).toMatch(/\/x\/sent\.ndjson.*no file there/);
    expect(byTestId("notifications-empty")).toBeNull();
  });

  it("a_box_that_does_not_answer_is_not_shown_as_nothing_sent", async () => {
    box.refusal = { error: "role viewer required" };
    await render();
    expect(byTestId("notifications-error")?.textContent).toContain("role viewer required");
    expect(byTestId("notifications-empty")).toBeNull();
  });

  it("an_empty_journal_says_nothing_was_recorded", async () => {
    box.answer = readPanel({ records: [], total: 0, refused: 0, by_kind: {} });
    await render();
    expect(byTestId("notifications-empty")?.textContent).toContain("no messages");
  });

  it("a_broken_chain_is_said_plainly_with_its_line", async () => {
    box.answer = readPanel({ chain: { state: "broken", first_line: 4, breaks: 1 } });
    await render();
    expect(byTestId("notifications-chain")?.textContent).toMatch(/chain BROKEN at line 4/);
  });

  it("a_journal_of_one_record_is_not_called_verified", async () => {
    box.answer = readPanel();
    await render();
    const chain = byTestId("notifications-chain")?.textContent ?? "";
    expect(chain).toContain("binds nothing");
    expect(chain).not.toContain("verifies");
  });

  it("journal_text_renders_as_text_with_hidden_characters_written_out", async () => {
    box.answer = readPanel({
      records: [{ ...RECORD, about: "<img src=x onerror=alert(1)>‮gnp.exe", to: ["a@example.invalid\u0000"] }],
    });
    await render();
    const table = byTestId("notifications-table")!;
    expect(table.querySelector("img")).toBeNull();
    expect(table.textContent).toContain("<img src=x onerror=alert(1)>");
    expect(table.textContent).toContain("\\u{202e}");
    expect(table.textContent).toContain("\\u{0}");
  });
});
