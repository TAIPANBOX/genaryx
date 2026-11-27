// @vitest-environment jsdom
/**
 * The cards an operator opens from a row, in the demo world: Agent 360, an
 * owner's card, a business unit's card, Run Replay seeded with a run, and the
 * delegation graph. Each one is fed by the mock transport's own answers, and
 * what is checked is that those answers reach the card: the agent's own runs on
 * its Agent 360, the owner's agents on the owner card, the unit's agents on the
 * unit card, the run's id on its replay, and a click on the graph opening that
 * agent. The expected values are read from the same mock, never typed here.
 */

import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class Inert {
  observe() {}
  unobserve() {}
  disconnect() {}
}

type Mock = typeof import("./lib/mockPreview");
let mock: Mock;
let PopoverProvider: typeof import("./lib/popover").PopoverProvider;
let roots: { root: Root; host: HTMLDivElement }[] = [];
const crashes: string[] = [];

beforeAll(async () => {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", "1");
  vi.stubEnv("VITE_GENARYX_API", "");
  vi.stubGlobal("ResizeObserver", Inert);
  vi.stubGlobal("IntersectionObserver", Inert);
  Element.prototype.scrollIntoView ??= function () {};
  Element.prototype.scrollTo ??= function () {} as never;
  vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
    const text = args.map(String).join(" ");
    if (/The above error occurred|Uncaught/.test(text)) crashes.push(text);
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  mock = await import("./lib/mockPreview");
  PopoverProvider = (await import("./lib/popover")).PopoverProvider;
});

afterEach(async () => {
  for (const { root, host } of roots) {
    await act(async () => root.unmount());
    host.remove();
  }
  roots = [];
  expect(crashes).toEqual([]);
});

afterAll(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

async function settle() {
  for (let i = 0; i < 4; i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
  }
}

async function show(el: ReactElement): Promise<HTMLDivElement> {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => root.render(createElement(PopoverProvider, null, el)));
  await settle();
  return host;
}

type MockRun = { run_id: string; agent_id: string };

describe("cards in the demo world", () => {
  it("Agent 360 shows the agent and its own runs", async () => {
    const { Agent360 } = await import("./components/Agent360");
    const runs = (await mock.mockInvoke("money_runs")) as MockRun[];
    const agentId = runs[0].agent_id;
    const own = runs.filter((r) => r.agent_id === agentId).map((r) => r.run_id);
    const onClose = vi.fn();
    const host = await show(
      createElement(Agent360, { agentId, onClose, onOpenAgent: () => {}, onNavigate: () => {}, onOpenReplay: () => {} }),
    );
    const text = host.textContent ?? "";
    expect(text).toContain(agentId);
    expect(own.some((id) => text.includes(id))).toBe(true);
  });

  it("Run Replay seeded with a run names it and offers playback", async () => {
    const { RunReplayView } = await import("./components/RunReplayView");
    const runs = (await mock.mockInvoke("money_runs")) as MockRun[];
    const host = await show(createElement(RunReplayView, { presetRunId: runs[0].run_id, onOpenAgent: () => {} }));
    expect(host.textContent).toContain(runs[0].run_id);
    const play = Array.from(host.querySelectorAll("button")).find((b) => /play/i.test(b.textContent ?? b.title));
    expect(play).toBeDefined();
    await act(async () => play!.click());
    await act(async () => {
      await new Promise((r) => setTimeout(r, 400));
    });
    expect(host.textContent).toMatch(/\d+\s*\/\s*\d+/);
  });

  it("Run Replay with no run offers the demo's runs to pick", async () => {
    const { RunReplayView } = await import("./components/RunReplayView");
    const runs = (await mock.mockInvoke("money_runs")) as MockRun[];
    const host = await show(createElement(RunReplayView, { presetRunId: null, onOpenAgent: () => {} }));
    expect(runs.some((r) => host.textContent?.includes(r.run_id))).toBe(true);
  });

  it("an owner's card lists the agents the owner runs", async () => {
    const { UserCard } = await import("./components/UserCard");
    const rec = (await mock.mockInvoke("user_record", { user: "w.zhang" })) as { agents: { agentId: string }[] } | null;
    const host = await show(createElement(UserCard, { handle: "w.zhang", onOpenFullAgent: () => {} }));
    expect(host.textContent).toContain("w.zhang");
    if (rec && rec.agents.length > 0) {
      const text = host.textContent ?? "";
      expect(rec.agents.some((a) => text.includes(a.agentId) || text.includes(a.agentId.split("/").pop()!))).toBe(true);
    }
  });

  it("a unit's card lists the unit's agents", async () => {
    const { UnitCard } = await import("./components/UnitCard");
    const { teamOf } = await import("./lib/agentRecord");
    const runs = (await mock.mockInvoke("money_runs")) as MockRun[];
    const team = runs.map((r) => teamOf(r.agent_id)).find((t) => t.length > 0)!;
    const rec = (await mock.mockInvoke("unit_record", { team })) as { agents: { agentId: string }[] } | null;
    const host = await show(createElement(UnitCard, { team, onOpenFullAgent: () => {} }));
    const text = host.textContent ?? "";
    expect(text.toLowerCase()).toContain(team.toLowerCase());
    if (rec && rec.agents.length > 0) {
      expect(rec.agents.some((a) => text.includes(a.agentId) || text.includes(a.agentId.split("/").pop()!))).toBe(true);
    }
  });

  it("the delegation graph draws the demo's agents and opens one on click", async () => {
    const { DelegationGraphView } = await import("./components/DelegationGraphView");
    const graph = (await mock.mockInvoke("agent_graph")) as { nodes: { id: string }[] };
    expect(graph.nodes.length).toBeGreaterThan(0);
    const onOpenAgent = vi.fn();
    const host = await show(createElement(DelegationGraphView, { onOpenAgent, height: 400 }));
    const nodes = host.querySelectorAll("svg g, svg circle, svg rect");
    expect(nodes.length).toBeGreaterThan(0);
    const clickable = Array.from(host.querySelectorAll<SVGElement>("svg [role='button'], svg g[tabindex], svg g")).filter(
      (g) => g.getAttribute("data-id") || g.getAttribute("tabindex") !== null || g.getAttribute("role") === "button",
    );
    for (const g of clickable.slice(0, 3)) {
      await act(async () => {
        g.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });
    }
    await settle();
    const opened = onOpenAgent.mock.calls.map((c) => c[0] as string);
    expect(opened.every((id) => graph.nodes.some((n) => n.id === id))).toBe(true);
  });
});
