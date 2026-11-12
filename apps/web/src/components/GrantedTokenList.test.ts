// @vitest-environment jsdom
/**
 * The list of tokens a view shows, on its own and against the clock: what the
 * whole console cannot reach in a test, because its demo token lives ten real
 * minutes. A token that is past its expiry is no use to anybody, so it neither
 * holds the page nor raises the notice, and a token whose claims the console
 * could not read has no expiry it can see, so it holds the page until it is
 * copied or dismissed.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DecodedToken } from "../policyTypes";
import type { GrantedToken } from "../lib/approvalToken";
import { GrantedTokenList } from "./ApprovalsInbox";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const RAW = "invented-claims-segment.invented-signature-segment";
const NOW_UNIX = 2_000_000_000;

const CLAIMS: DecodedToken = {
  agent_id: "agent://example.invalid/agents/held",
  run_id: "run-held",
  tools: ["fetch_url"],
  cost_ceiling_usd: 0.5,
  exp_unix: NOW_UNIX + 600,
  tool_call_digest: "ab".repeat(32),
};

let root: Root;
let host: HTMLDivElement;

function render(tokens: GrantedToken[], onDismiss: (approvalId: string) => void = () => {}): void {
  act(() => {
    root.render(createElement(GrantedTokenList, { tokens, onDismiss }));
  });
}

function leavingIsQuestioned(): boolean {
  const event = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(event);
  return event.defaultPrevented;
}

function notice(): Element | null {
  return document.querySelector('[data-testid="granted-token-notice"]');
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
  vi.setSystemTime(NOW_UNIX * 1000);
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => {
    root.unmount();
  });
  host.remove();
  vi.useRealTimers();
});

describe("the tokens a view shows", () => {
  it("an_expired_token_neither_asks_nor_raises_the_notice", () => {
    render([{ approvalId: "ap_1", raw: RAW, claims: { ...CLAIMS, exp_unix: NOW_UNIX - 1 } }]);
    expect(leavingIsQuestioned()).toBe(false);
    expect(notice()).toBeNull();
  });

  it("a_token_that_expires_on_screen_stops_asking", () => {
    render([{ approvalId: "ap_1", raw: RAW, claims: { ...CLAIMS, exp_unix: NOW_UNIX + 2 } }]);
    expect(leavingIsQuestioned()).toBe(true);
    expect(notice()).not.toBeNull();
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(leavingIsQuestioned()).toBe(false);
    expect(notice()).toBeNull();
  });

  it("a_token_whose_claims_could_not_be_read_asks_until_it_is_dismissed", () => {
    render([{ approvalId: "ap_1", raw: RAW, claims: null }]);
    expect(leavingIsQuestioned()).toBe(true);
    act(() => {
      vi.advanceTimersByTime(60 * 60 * 1000);
    });
    expect(leavingIsQuestioned()).toBe(true);
    render([]);
    expect(leavingIsQuestioned()).toBe(false);
    expect(notice()).toBeNull();
  });

  it("the_notice_counts_the_tokens_not_copied_yet", () => {
    render([
      { approvalId: "ap_2", raw: "second.token", claims: CLAIMS },
      { approvalId: "ap_1", raw: RAW, claims: CLAIMS },
    ]);
    expect(notice()?.textContent).toContain("2 approval tokens are on screen and not copied yet");
    for (const part of [RAW, ...RAW.split("."), "second.token"]) {
      expect(notice()?.textContent).not.toContain(part);
    }
  });

  it("the_page_is_let_go_once_the_list_is_gone", () => {
    render([{ approvalId: "ap_1", raw: RAW, claims: CLAIMS }]);
    expect(leavingIsQuestioned()).toBe(true);
    act(() => {
      root.unmount();
    });
    expect(leavingIsQuestioned()).toBe(false);
    root = createRoot(host);
  });
});
