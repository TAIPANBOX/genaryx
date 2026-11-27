// @vitest-environment jsdom
/**
 * Invariant 4's other half: a card with no data says it has no data. Every
 * card and view below is mounted against a box that refuses every command with
 * its own error text, and then against no box at all. Each must render (no
 * crash), and a card fed a refusal must not show a row of the demo world: the
 * console shows the operator's real records or says it could not read them,
 * never a plausible stand-in.
 */

import { act, createElement, type ReactElement } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
vi.setConfig({ testTimeout: 30_000 });

const box = vi.hoisted(() => ({ present: true }));

vi.mock("./lib/transport", () => ({
  hasBackend: () => box.present,
  isWebShell: () => box.present,
  webApiBase: () => "",
  subscribeBackend: async () => () => {},
  requiredRoleFromCommandError: () => null,
  invokeBackend: async () => {
    throw { error: "the box is down for maintenance" };
  },
}));

vi.mock("./lib/webauthn", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./lib/webauthn")>()),
  invokeWithCeremony: async () => {
    throw { error: "the box is down for maintenance" };
  },
}));

class Inert {
  observe() {}
  unobserve() {}
  disconnect() {}
}

const crashes: string[] = [];

beforeAll(() => {
  vi.stubGlobal("ResizeObserver", Inert);
  vi.stubGlobal("IntersectionObserver", Inert);
  Element.prototype.scrollIntoView ??= function () {};
  Element.prototype.scrollTo ??= function () {} as never;
  vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
    const text = args.map(String).join(" ");
    if (/The above error occurred|Uncaught/.test(text)) crashes.push(text);
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterAll(() => vi.unstubAllGlobals());

async function render(el: ReactElement): Promise<string> {
  const { PopoverProvider } = await import("./lib/popover");
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => root.render(createElement(PopoverProvider, null, el)));
  for (let i = 0; i < 4; i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
  }
  const text = host.textContent ?? "";
  await act(async () => root.unmount());
  host.remove();
  return text;
}

const noop = () => {};
const AGENT = "agent://meridian.io/sre/rca-copilot";

/** Each entry: a name and how to build it. Built lazily so every module loads
 * under the mocked transport. */
const SURFACES: [string, () => Promise<ReactElement>][] = [
  ["UnitCard", async () => createElement((await import("./components/UnitCard")).UnitCard, { team: "finops", onOpenFullAgent: noop })],
  ["UserCard", async () => createElement((await import("./components/UserCard")).UserCard, { handle: "w.zhang", onOpenFullAgent: noop })],
  ["AgentDetailCard", async () => createElement((await import("./components/AgentDetailCard")).AgentDetailCard, { agentId: AGENT })],
  [
    "Agent360",
    async () =>
      createElement((await import("./components/Agent360")).Agent360, {
        agentId: AGENT,
        onClose: noop,
        onOpenAgent: noop,
        onNavigate: noop,
        onOpenReplay: noop,
      }),
  ],
  ["MemoryProvenance", async () => createElement((await import("./components/MemoryProvenance")).MemoryProvenance, { memoryId: "m-1", onForgotten: noop })],
  ["MemoryTimeline", async () => createElement((await import("./components/MemoryTimeline")).MemoryTimeline, { onOpenAgent: noop })],
  ["MemoryView", async () => createElement((await import("./components/MemoryView")).MemoryView, { onOpenAgent: noop })],
  ["PasskeySettings", async () => createElement((await import("./components/PasskeySettings")).PasskeySettings)],
  ["RetainedClosures", async () => createElement((await import("./components/RetainedClosures")).RetainedClosuresSection)],
  ["RemoteCloudInventory", async () => createElement((await import("./components/RemoteCloudInventory")).RemoteCloudInventory)],
  ["RemoteWgOperatorCard", async () => createElement((await import("./components/RemoteWgOperatorCard")).RemoteWgOperatorCard)],
  ["RemoteView", async () => createElement((await import("./components/RemoteView")).RemoteView)],
  ["CryptoEvidence", async () => createElement((await import("./components/CryptoEvidence")).CryptoEvidence, { defaultPath: "/root", environment: "prod" })],
  ["CryptoView", async () => createElement((await import("./components/CryptoView")).CryptoView)],
  ["EvidenceView", async () => createElement((await import("./components/EvidenceView")).EvidenceView)],
  ["BusExplorer", async () => createElement((await import("./components/BusExplorer")).BusExplorer)],
  ["IdentityView", async () => createElement((await import("./components/IdentityView")).IdentityView, { onOpenAgent: noop })],
  ["StatsView", async () => createElement((await import("./components/StatsView")).StatsView, { onOpenAgent: noop, onOpenUser: noop, onOpenUnit: noop })],
  ["CopilotView", async () => createElement((await import("./components/CopilotView")).CopilotView, { explainRequest: null, onExplainRequestHandled: noop })],
  ["OnboardView", async () => createElement((await import("./components/OnboardView")).OnboardView)],
  ["AdmissionVerify", async () => createElement((await import("./components/AdmissionVerify")).AdmissionVerify, { seed: null })],
  ["RunReplayView", async () => createElement((await import("./components/RunReplayView")).RunReplayView, { presetRunId: "run-1", onOpenAgent: noop })],
];

/** Rows only the demo world holds: if one of these shows up while every
 * command is refused, a card invented it. */
const DEMO_ONLY = ["meridian.io/finops/unit-economics-analyst", "prod-stack", "runtime-eu-1", "w.zhang@"];

describe("against a box that refuses every command", () => {
  it.each(SURFACES)("%s renders, and shows no row of the demo world", async (_name, build) => {
    box.present = true;
    const text = await render(await build());
    expect(crashes).toEqual([]);
    expect(text.length).toBeGreaterThan(0);
    for (const row of DEMO_ONLY) expect(text).not.toContain(row);
  });
});

describe("with no box at all", () => {
  it.each(SURFACES)("%s renders, and shows no row of the demo world", async (_name, build) => {
    box.present = false;
    const text = await render(await build());
    expect(crashes).toEqual([]);
    expect(text.length).toBeGreaterThan(0);
    for (const row of DEMO_ONLY) expect(text).not.toContain(row);
  });
});
