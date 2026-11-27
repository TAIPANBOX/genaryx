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

  it("the delegation graph draws every demo agent's label, and a click on a node opens that agent", async () => {
    const { DelegationGraphView } = await import("./components/DelegationGraphView");
    const { shortAgentLabel } = await import("./lib/graph");
    const graph = (await mock.mockInvoke("agent_graph")) as { nodes: { id: string; x: number; y: number }[] };
    expect(graph.nodes.length).toBeGreaterThan(0);

    // A canvas that records what is drawn, and a layout box of a real size.
    const drawn: { op: string; args: unknown[] }[] = [];
    const ctx = new Proxy({} as Record<string, unknown>, {
      get: (target, prop: string) =>
        prop in target ? target[prop] : (...args: unknown[]) => void drawn.push({ op: prop, args }),
      set: (target, prop: string, value) => ((target[prop] = value), true),
    });
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx as never);
    Element.prototype.setPointerCapture ??= function () {};
    vi.stubGlobal(
      "ResizeObserver",
      class {
        cb: (entries: unknown[]) => void;
        constructor(cb: (entries: unknown[]) => void) {
          this.cb = cb;
        }
        observe(el: Element) {
          this.cb([{ target: el, contentRect: { width: 800, height: 500 } }]);
        }
        unobserve() {}
        disconnect() {}
      },
    );

    const onOpenAgent = vi.fn();
    const host = await show(createElement(DelegationGraphView, { onOpenAgent, height: 500 }));
    const scaleNow = () => ([...drawn].reverse().find((d) => d.op === "scale")!.args as number[])[0];
    // Labels are drawn only once the graph is legible (scale over 0.35).
    for (let i = 0; i < 10 && scaleNow() <= 0.35; i++) {
      await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Zoom in"]')!.click());
    }
    const labels = drawn.filter((d) => d.op === "fillText").map((d) => d.args[0]);
    for (const n of graph.nodes) expect(labels).toContain(shortAgentLabel(n.id));

    // Where the last frame put the graph: translate, then scale.
    const lastTranslate = [...drawn].reverse().find((d) => d.op === "translate")!.args as number[];
    const lastScale = [...drawn].reverse().find((d) => d.op === "scale")!.args as number[];
    const target = graph.nodes[0];
    const at = { x: lastTranslate[0] + target.x * lastScale[0], y: lastTranslate[1] + target.y * lastScale[0] };
    const canvas = host.querySelector("canvas")!;
    const fire = (type: string, x: number, y: number) =>
      canvas.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX: x, clientY: y }));

    await act(async () => fire("pointermove", at.x, at.y));
    await act(async () => fire("pointerdown", at.x, at.y));
    await act(async () => fire("pointerup", at.x, at.y));
    expect(onOpenAgent).toHaveBeenCalledWith(target.id);

    // A drag pans and opens nothing; zoom and fit redraw.
    onOpenAgent.mockClear();
    await act(async () => fire("pointerdown", at.x, at.y));
    await act(async () => fire("pointermove", at.x + 40, at.y + 40));
    await act(async () => fire("pointerup", at.x + 40, at.y + 40));
    expect(onOpenAgent).not.toHaveBeenCalled();
    await act(async () => fire("pointerleave", 0, 0));
    const before = drawn.length;
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Zoom in"]')!.click());
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Zoom out"]')!.click());
    await act(async () => canvas.dispatchEvent(new WheelEvent("wheel", { deltaY: -200, clientX: 100, clientY: 100, cancelable: true })));
    for (const b of Array.from(host.querySelectorAll("button")).filter((x) => /^(Refresh|Reset view)$/.test(x.textContent ?? ""))) {
      await act(async () => b.click());
    }
    await settle();
    expect(drawn.length).toBeGreaterThan(before);

    getContext.mockRestore();
    vi.stubGlobal("ResizeObserver", Inert);
  });
});

function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

function byPlaceholder(host: HTMLElement, placeholder: string): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>(`[placeholder="${placeholder}"]`);
  if (!el) throw new Error(`no field with placeholder ${placeholder}`);
  return el;
}

function button(host: HTMLElement, text: RegExp): HTMLButtonElement {
  const b = Array.from(host.querySelectorAll("button")).find((x) => text.test(x.textContent ?? ""));
  if (!b) throw new Error(`no button matching ${text}`);
  return b as HTMLButtonElement;
}

describe("Onboard in the demo world", () => {
  it("proposes a bundle for exactly the agent typed in, and only once it is complete", async () => {
    const { OnboardView } = await import("./components/OnboardView");
    const host = await show(createElement(OnboardView));

    const generate = () => button(host, /^Generat/);
    await act(async () => {
      typeInto(byPlaceholder(host, "bank.example"), "");
      typeInto(byPlaceholder(host, "treasury/recon-batch"), "");
    });
    expect(generate().disabled).toBe(true);

    await act(async () => {
      typeInto(byPlaceholder(host, "bank.example"), "acme.example");
      typeInto(byPlaceholder(host, "treasury/recon-batch"), "finops/forecaster");
      typeInto(byPlaceholder(host, "user://bank.example/olena, or free text"), "user://acme.example/ops");
    });
    const unitField = host.querySelector<HTMLInputElement>('[placeholder="unit id (no identity map loaded - free text)"]');
    if (unitField) await act(async () => typeInto(unitField, "finops"));
    const select = host.querySelector<HTMLSelectElement>("select");
    if (select && !unitField) {
      await act(async () => {
        select.value = select.options[select.options.length > 1 ? 1 : 0].value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
      });
    }

    await act(async () => button(host, /\+ Add folder/).click());
    await act(async () => button(host, /\+ Add model/).click());
    const removeFolder = host.querySelector<HTMLButtonElement>('[aria-label="Remove this folder"]');
    const removeModel = host.querySelector<HTMLButtonElement>('[aria-label="Remove this model"]');
    expect(removeFolder).not.toBeNull();
    expect(removeModel).not.toBeNull();
    await act(async () => removeFolder!.click());
    await act(async () => removeModel!.click());

    expect(generate().disabled).toBe(false);
    await act(async () => generate().click());
    await settle();

    const text = host.textContent ?? "";
    expect(text).toContain("Generated bundle");
    expect(text).toContain("agent://acme.example/finops/forecaster");

    // The demo keeps no disk: writing answers nothing, and the card must not
    // claim a file was written.
    await act(async () => button(host, /^Write to /).click());
    await settle();
    expect(host.textContent).not.toContain("written ·");
  });
});

describe("Felyx in the demo world", () => {
  it("answers a typed question with the mock's own answer, and shows the question", async () => {
    const { CopilotView } = await import("./components/CopilotView");
    const question = "how can I reduce cost?";
    const expected = (await mock.mockInvoke("copilot_ask", { question })) as { text: string };
    const host = await show(createElement(CopilotView, { explainRequest: null, onExplainRequestHandled: () => {} }));
    const input = byPlaceholder(host, "Ask Felyx...");
    const send = () => button(host, /^(Send|Asking\.\.\.)$/);
    expect(send().disabled).toBe(true);
    await act(async () => typeInto(input, question));
    expect(send().disabled).toBe(false);
    await act(async () => send().click());
    await settle();
    const text = host.textContent ?? "";
    expect(text).toContain(question);
    expect(text).toContain(expected.text.slice(0, 60));
  });

  it("explains an incident handed to it from elsewhere, once", async () => {
    const { CopilotView } = await import("./components/CopilotView");
    const handled = vi.fn();
    const host = await show(
      createElement(CopilotView, { explainRequest: { nonce: 1, incidentId: "bus:1" }, onExplainRequestHandled: handled }),
    );
    const expected = (await mock.mockInvoke("copilot_explain", { incident_id: "bus:1" })) as { text: string };
    expect(handled).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain(expected.text.slice(0, 60));
  });
});

describe("the watch dock in the demo world", () => {
  it("seeds the demo's pins once, pins and unpins from a toggle, collapses, and opens what is pinned", async () => {
    const { WatchDock, WatchToggleButton } = await import("./components/WatchDock");
    const onOpenAgent = vi.fn();
    const onOpenUnit = vi.fn();
    const seeded = "agent://meridian.io/sre/rca-copilot";
    const host = await show(
      createElement(
        "div",
        null,
        createElement(WatchDock, { onOpenAgent, onOpenUnit }),
        createElement(WatchToggleButton, { kind: "agent", id: "agent://meridian.io/data/new-bot", label: "new-bot" }),
        createElement(WatchToggleButton, { kind: "unit", id: "treasury", label: "treasury" }),
        createElement(WatchToggleButton, { kind: "user", id: "a.new", label: "a.new" }),
      ),
    );
    expect(JSON.parse(localStorage.getItem("genaryx.watch.agents") ?? "[]")).toContain(seeded);
    const dock = () => host.querySelector<HTMLElement>('[aria-label="Watch dock"]');
    expect(dock()).not.toBeNull();

    // Pin three new things from their toggles; each shows in the dock and is persisted.
    for (const label of ["new-bot", "treasury", "a.new"]) {
      const pin = host.querySelector<HTMLButtonElement>(`[aria-label="Pin ${label} to the watch dock"]`)!;
      expect(pin.getAttribute("aria-pressed")).toBe("false");
      await act(async () => pin.click());
      const now = host.querySelector<HTMLButtonElement>(`[aria-label="Unpin ${label} from the watch dock"]`)!;
      expect(now.getAttribute("aria-pressed")).toBe("true");
    }
    await settle();
    expect(localStorage.getItem("genaryx.watch.agents")).toContain("agent://meridian.io/data/new-bot");

    // Unpin one from the dock itself.
    const unpin = host.querySelector<HTMLButtonElement>('[aria-label^="Unpin "]:not([aria-pressed])');
    expect(unpin).not.toBeNull();
    const before = (localStorage.getItem("genaryx.watch.agents") ?? "") + (localStorage.getItem("genaryx.watch.units") ?? "");
    await act(async () => unpin!.click());
    const after = (localStorage.getItem("genaryx.watch.agents") ?? "") + (localStorage.getItem("genaryx.watch.units") ?? "");
    expect(after.length).toBeLessThan(before.length);

    // Collapse and expand, remembered across a reload of the dock.
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Collapse watch dock"]')!.click());
    expect(host.querySelector('[aria-label="Watch dock (collapsed)"]')).not.toBeNull();
    expect(localStorage.getItem("genaryx.watchDock")).toBe("true");
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Expand watch dock"]')!.click());
    expect(dock()).not.toBeNull();

    // Opening a pinned agent hands its id to the shell.
    const openers = Array.from(dock()!.querySelectorAll<HTMLElement>("button, [role='button']")).filter(
      (b) => !(b.getAttribute("aria-label") ?? "").match(/^(Unpin|Collapse|Expand|Resize)/),
    );
    for (const b of openers) await act(async () => b.click());
    await settle();
    expect(onOpenAgent.mock.calls.length + onOpenUnit.mock.calls.length).toBeGreaterThan(0);
  });
});
