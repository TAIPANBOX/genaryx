// @vitest-environment jsdom
/**
 * A grant made from a Felyx proposal card mints the same one-time token as a
 * grant in the inbox. The operator must get it, because nothing can fetch it
 * later, and Felyx must not: the card shows it beside the transcript, and neither
 * the transcript, nor the audit link, nor the next question carries it.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CopilotAnswer } from "../copilotTypes";
import type { DecideOutcome } from "../policyTypes";

const RAW = "invented-claims-segment.invented-signature-segment";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OUTCOME: DecideOutcome = {
  summary: "approval ap_1 granted",
  http_status: 200,
  verify_result: "granted ceiling_usd:0.50 ttl_s:600",
  sig_alg: "webauthn-es256",
  sig_fpr: "cred",
  token: {
    agent_id: "agent://example.invalid/agents/held",
    run_id: "run-held",
    tools: ["fetch_url"],
    cost_ceiling_usd: 0.5,
    exp_unix: 4_102_444_800,
    tool_call_digest: "ab".repeat(32),
  },
  approval_token: RAW,
  bus_recorded: true,
  bus_error: null,
};

const ANSWER: CopilotAnswer = {
  text: "ap_1 is waiting for you.",
  tool_trace: [],
  usage: { input_tokens: 0, output_tokens: 0 },
  proposals: [
    {
      kind: "grant_deny",
      target: "ap_1",
      params: { verdict: "grant" },
      rationale: "The call is the one you expected.",
      confidence: 0.9,
      evidence_refs: [],
      policy_context: [],
    },
  ],
} as unknown as CopilotAnswer;

const calls = vi.hoisted(() => ({
  ask: [] as unknown[][],
  logged: [] as unknown[][],
  decide: [] as unknown[][],
}));

vi.mock("../lib/copilot", async (original) => ({
  ...(await original<typeof import("../lib/copilot")>()),
  fetchCopilotStatus: async () => ({ enabled: true, provider: "stub", model: "stub", local: true }),
  askCopilot: async (...args: unknown[]) => {
    calls.ask.push(args);
    return ANSWER;
  },
  logProposalApproved: async (...args: unknown[]) => {
    calls.logged.push(args);
    return { journaled: true, journal_error: null };
  },
}));

vi.mock("../lib/policy", async (original) => ({
  ...(await original<typeof import("../lib/policy")>()),
  decideApproval: async (...args: unknown[]) => {
    calls.decide.push(args);
    return OUTCOME;
  },
}));

let root: Root;
let host: HTMLDivElement;

async function settle(ms = 20): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function until<T>(what: string, read: () => T | null | undefined): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const found = read();
    if (found !== null && found !== undefined) return found;
    await settle();
  }
  throw new Error(`gave up waiting for ${what}`);
}

function buttonNamed(name: string): HTMLButtonElement | undefined {
  return Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.trim() === name);
}

async function press(name: string): Promise<void> {
  const button = await until(`a button named ${name}`, () => {
    const found = buttonNamed(name);
    return found && !found.disabled ? found : null;
  });
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
}

async function ask(question: string): Promise<void> {
  const input = await until("the question box", () => document.querySelector("input"));
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  await act(async () => {
    setValue?.call(input, question);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await press("Send");
}

async function grantFromTheCard(): Promise<void> {
  await ask("what is waiting for me?");
  await press("Approve");
  await press("Confirm approve");
  await until("the token panel", () => document.querySelector('[data-testid="approval-token"]'));
}

beforeEach(async () => {
  calls.ask.length = 0;
  calls.logged.length = 0;
  calls.decide.length = 0;
  const { CopilotView } = await import("./CopilotView");
  const { PopoverProvider } = await import("../lib/popover");
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root.render(
      createElement(
        PopoverProvider,
        null,
        createElement(CopilotView, { explainRequest: null, onExplainRequestHandled: () => {} }),
      ),
    );
  });
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  host.remove();
});

describe("a grant from a Felyx card", () => {
  it("a_grant_from_a_felyx_card_shows_the_raw_token_to_the_operator", async () => {
    await grantFromTheCard();
    expect(calls.decide).toEqual([["ap_1", "grant"]]);
    expect(document.querySelector('[data-testid="approval-token"]')?.textContent).toBe(RAW);
    expect(document.body.textContent).toContain("x-fuse-approval-token");
  });

  it("felyx_never_receives_the_token_from_a_grant_on_its_own_card", async () => {
    await grantFromTheCard();
    await until("the audit link", () => (calls.logged.length > 0 ? true : null));
    await settle(30);
    await ask("anything else?");
    await until("the second question", () => (calls.ask.length === 2 ? true : null));

    // The conversation on screen: everything but the panel the operator reads.
    const panel = document.querySelector('[data-testid="approval-token"]')?.closest(".panel");
    const whole = document.body.textContent ?? "";
    const transcript = whole.replace(panel?.textContent ?? "", "");
    expect(whole).toContain(RAW);
    expect(transcript).toContain("approval ap_1 granted");

    const toFelyx = JSON.stringify([calls.ask, calls.logged]);
    for (const part of [RAW, ...RAW.split(".")]) {
      expect(transcript).not.toContain(part);
      expect(toFelyx).not.toContain(part);
    }
  });

  it("a_grant_from_a_felyx_card_takes_the_focus_and_asks_before_the_page_goes", async () => {
    await grantFromTheCard();
    const token = document.querySelector('[data-testid="approval-token"]');
    const focused = document.activeElement;
    expect(focused !== null && focused !== document.body && focused.contains(token)).toBe(true);
    expect(document.querySelector('[data-testid="granted-token-notice"]')?.textContent).toContain("not copied yet");
    const leaving = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(leaving);
    expect(leaving.defaultPrevented).toBe(true);
  });

  it("leaving_felyx_drops_the_token_a_card_showed", async () => {
    await grantFromTheCard();
    await act(async () => {
      root.unmount();
    });
    expect(document.documentElement.innerHTML).not.toContain(RAW);
    root = createRoot(host);
  });
});
