// @vitest-environment jsdom
/**
 * Invariant 50: no control on the agent card does nothing and says nothing.
 *
 * Transfer owner, reassign unit and the two edits rest on commands only the
 * mock preview answers (`agent_record`, `org_directory`, `agent_set_*`). On a
 * real box the card used to hide its whole Manage section (no record came
 * back), and with it the controls that do work there; in the preview an edit
 * the backend refused closed its form as though it had landed.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const box = vi.hoisted(() => ({
  web: true,
  calls: [] as string[],
  answers: {} as Record<string, unknown>,
  refusals: {} as Record<string, unknown>,
}));

vi.mock("../lib/transport", () => ({
  hasBackend: () => true,
  isWebShell: () => box.web,
  webApiBase: () => "",
  subscribeBackend: () => () => {},
  requiredRoleFromCommandError: () => null,
  invokeBackend: async (command: string) => {
    box.calls.push(command);
    if (command in box.refusals) throw box.refusals[command];
    if (command in box.answers) return box.answers[command];
    throw { error: "unknown command" };
  },
}));

import { PopoverProvider } from "../lib/popover";
import { AgentDetailCard } from "./AgentDetailCard";

const AGENT = "agent://example.invalid/finance/forecaster";

const RECORD = {
  team: "finance",
  name: "forecaster",
  model: "claude-haiku-5-5",
  owner: "alice",
  budgetUsd: 2,
  allowed: ["read_ledger"],
  spentUsd: 0.5,
  calls: 3,
  history: [],
};

const PREVIEW_ONLY_EDITS = [
  "org_directory",
  "agent_set_budget",
  "agent_reassign_unit",
  "agent_transfer_owner",
  "agent_set_behaviour",
];

let host: HTMLDivElement;
let root: Root;

async function render() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root.render(createElement(PopoverProvider, null, createElement(AgentDetailCard, { agentId: AGENT })));
  });
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

function buttons(): HTMLButtonElement[] {
  return Array.from(host.querySelectorAll("button"));
}

function button(label: string): HTMLButtonElement | undefined {
  return buttons().find((b) => b.textContent?.trim() === label);
}

beforeEach(() => {
  box.calls = [];
  box.answers = { money_runs: [], identity_list_identities: [], lifecycle_blocks: { agents: [], units: [], users: [] } };
  box.refusals = {};
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe("the agent card on a real box", () => {
  it("a_real_box_does_not_offer_the_edits_it_cannot_make_and_says_why", async () => {
    box.web = true;
    await render();
    for (const label of ["Transfer owner", "Reassign unit", "Edit budget", "Edit behaviour"]) {
      expect(button(label), label).toBeUndefined();
    }
    expect(host.textContent).toMatch(/need an ownership store, which this box does not keep yet/);
  });

  it("a_real_box_still_offers_the_controls_that_act_on_the_planes", async () => {
    box.web = true;
    await render();
    expect(button("Stop agent")).toBeDefined();
    expect(button("Freeze")).toBeDefined();
  });

  it("a_real_box_card_sends_none_of_the_preview_only_edit_commands", async () => {
    box.web = true;
    await render();
    for (const c of PREVIEW_ONLY_EDITS) expect(box.calls, c).not.toContain(c);
  });
});

describe("the agent card in the preview", () => {
  it("a_refused_edit_in_the_preview_says_so_and_keeps_its_form", async () => {
    box.web = false;
    box.answers.agent_record = RECORD;
    box.answers.org_directory = { teams: [], users: [] };
    box.refusals.agent_set_budget = { error: "budget refused by the plane" };
    await render();
    await act(async () => button("Edit budget")!.click());
    await act(async () => {
      button("Apply")!.click();
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(box.calls).toContain("agent_set_budget");
    const alert = host.querySelector('[role="alert"]');
    expect(alert?.textContent).toMatch(/not changed: budget refused by the plane/);
    expect(button("Apply")).toBeDefined();
  });

  it("the_preview_offers_all_four_edits_where_the_record_is_kept", async () => {
    box.web = false;
    box.answers.agent_record = RECORD;
    box.answers.org_directory = { teams: [], users: [] };
    await render();
    for (const label of ["Transfer owner", "Reassign unit", "Edit budget", "Edit behaviour"]) {
      expect(button(label), label).toBeDefined();
    }
  });
});
