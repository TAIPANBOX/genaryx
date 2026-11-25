// @vitest-environment jsdom
/**
 * Invariant 53: the Keys tab shows what Keyryx custody holds and what its
 * witness last signed, read-only, and never renders an unreachable custody as
 * an empty inventory.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { KeysPanel } from "../keysTypes";

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

import { KeysView } from "./KeysView";
import { NAV_SECTIONS } from "../lib/views";

const NAME = {
  name: "wardryx.approval",
  alg: "ES384",
  mode: "held",
  active_kid: "wardryx.approval-2026-09",
  active_since: 1758700000,
  next_kid: "wardryx.approval-2026-12",
  next_activates_at: 1769900000,
  last_rotation: 1763900000,
  states: { active: 1, pending: 1 },
  renew_due: false,
};

const ENTRY = {
  name: "wardryx.approval",
  alg: "ES384",
  kid: "wardryx.approval-2026-09",
  mode: "held",
  state: "active",
  created: 1758700000,
  activate_at: 1758700000,
};

function readCustody(over: Partial<Extract<KeysPanel["custody"], { state: "read" }>> = {}): KeysPanel["custody"] {
  return {
    state: "read",
    url: "https://keyryx.example.invalid",
    version: "1.4.0",
    names: [NAME],
    alarms: [],
    alarms_total: 0,
    entries: [ENTRY],
    entries_total: 1,
    inventory_error: null,
    ...over,
  };
}

function witnessHead(over: Partial<Extract<KeysPanel["witness"], { state: "read" }>> = {}): KeysPanel["witness"] {
  return {
    state: "read",
    url: "https://witness.example.invalid",
    size: 4242,
    root: "9f2c4e1a7b03d856",
    at: 1763990000,
    es384_kid: "witness.es384-2026-09",
    ml_dsa_65_kid: "witness.ml-dsa-65-2026-09",
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
    r.render(createElement(KeysView));
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

describe("the Keys tab", () => {
  it("the_keys_tab_is_in_the_assure_section", () => {
    const assure = NAV_SECTIONS.find((s) => s.label === "Assure");
    expect(assure?.items.map((i) => i.id)).toContain("keys");
  });

  it("custody_shows_each_key_name_with_its_active_and_next_key_and_last_rotation", async () => {
    box.answer = { custody: readCustody(), witness: witnessHead() } satisfies KeysPanel;
    await render();
    expect(box.calls).toEqual(["keys_custody"]);
    const names = byTestId("keys-names");
    expect(names?.textContent).toContain("wardryx.approval");
    expect(names?.textContent).toContain("wardryx.approval-2026-09");
    expect(names?.textContent).toContain("wardryx.approval-2026-12");
    expect(names?.textContent).toMatch(/activates/);
    expect(names?.textContent).toMatch(/active 1, pending 1/);
    expect(names?.textContent).not.toContain("renew due");
    expect(byTestId("keys-entries")?.textContent).toContain("wardryx.approval-2026-09");
    expect(byTestId("keys-custody-head")?.textContent).toContain("1.4.0");
  });

  it("an_alarm_custody_could_not_vouch_for_is_shown", async () => {
    box.answer = {
      custody: readCustody({
        alarms: [{ kind: "stale", name: "trailryx.seal", kid: "trailryx.seal-2026-10", reason: "not renewed in time" }],
        alarms_total: 1,
      }),
      witness: witnessHead(),
    } satisfies KeysPanel;
    await render();
    const alarms = byTestId("keys-alarms");
    expect(alarms?.textContent).toContain("stale trailryx.seal trailryx.seal-2026-10: not renewed in time");
  });

  it("a_custody_or_witness_not_named_says_so_and_names_the_setting", async () => {
    box.answer = {
      custody: { state: "not_configured", variable: "GENARYX_KEYRYX_URL" },
      witness: { state: "not_configured", variable: "GENARYX_KEYRYX_WITNESS_URL" },
    } satisfies KeysPanel;
    await render();
    expect(byTestId("keys-custody-not-configured")?.textContent).toContain("GENARYX_KEYRYX_URL");
    expect(byTestId("keys-witness-not-configured")?.textContent).toContain("GENARYX_KEYRYX_WITNESS_URL");
    expect(byTestId("keys-names")).toBeNull();
    expect(byTestId("keys-witness-head")).toBeNull();
  });

  it("an_unreachable_custody_is_not_shown_as_an_empty_inventory", async () => {
    box.answer = {
      custody: { state: "unreachable", url: "https://keyryx.example.invalid", reason: "connection refused" },
      witness: witnessHead(),
    } satisfies KeysPanel;
    await render();
    expect(byTestId("keys-custody-unreachable")?.textContent).toContain("connection refused");
    expect(byTestId("keys-names")).toBeNull();
    expect(byTestId("keys-entries")).toBeNull();
    expect(host?.textContent).not.toContain("holds no key names");
  });

  it("a_witness_with_no_head_yet_says_so", async () => {
    box.answer = { custody: readCustody(), witness: { state: "no_head_yet", url: "https://witness.example.invalid" } } satisfies KeysPanel;
    await render();
    expect(byTestId("keys-witness-no-head")?.textContent).toContain("no tree head yet");
    expect(byTestId("keys-witness-head")).toBeNull();
  });

  it("the_witness_head_shows_size_root_and_when_it_was_signed", async () => {
    box.answer = { custody: readCustody(), witness: witnessHead() } satisfies KeysPanel;
    await render();
    const head = byTestId("keys-witness-head")?.textContent ?? "";
    expect(head).toContain("tree size 4242");
    expect(head).toContain("root 9f2c4e1a7b03d856");
    expect(head).toMatch(/signed .*\d/);
    expect(head).toContain("witness.es384-2026-09");
    expect(head).toContain("witness.ml-dsa-65-2026-09");
  });

  it("the_keys_tab_has_no_control_that_changes_a_key", async () => {
    box.answer = { custody: readCustody(), witness: witnessHead() } satisfies KeysPanel;
    await render();
    expect(byTestId("keys-names")).not.toBeNull();
    expect(host?.querySelectorAll("button, form, input").length).toBe(0);
  });

  it("wire_text_renders_as_text_with_hidden_characters_written_out", async () => {
    box.answer = {
      custody: readCustody({
        names: [{ ...NAME, name: "<img src=x>‮" }],
        entries: [{ ...ENTRY, kid: "<img src=x>‮" }],
      }),
      witness: witnessHead(),
    } satisfies KeysPanel;
    await render();
    const names = byTestId("keys-names")!;
    expect(names.querySelector("img")).toBeNull();
    expect(names.textContent).toContain("<img src=x>");
    expect(names.textContent).toContain("\\u{202e}");
    expect(byTestId("keys-entries")?.querySelector("img")).toBeNull();
  });
});
