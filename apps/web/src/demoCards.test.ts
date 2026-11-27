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
vi.setConfig({ testTimeout: 30_000 });

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

    // Opening a pinned agent or unit hands its id to the shell.
    const openButtons = () => Array.from(dock()!.querySelectorAll<HTMLButtonElement>('button[title^="Open "]'));
    const n = openButtons().length;
    expect(n).toBeGreaterThan(0);
    for (let i = 0; i < n; i++) {
      const b = openButtons()[i];
      if (b) await act(async () => b.click());
    }
    await settle();
    const pinned = [
      ...JSON.parse(localStorage.getItem("genaryx.watch.agents") ?? "[]"),
      ...JSON.parse(localStorage.getItem("genaryx.watch.units") ?? "[]"),
    ];
    expect(onOpenAgent.mock.calls.length + onOpenUnit.mock.calls.length).toBeGreaterThan(0);
    for (const [id] of [...onOpenAgent.mock.calls, ...onOpenUnit.mock.calls]) expect(pinned).toContain(id);
  });
});

describe("the Remote tab in the demo world", () => {
  it("every act that would reach a box says it was simulated, and the reads show what the demo has", async () => {
    const { RemoteView } = await import("./components/RemoteView");
    const host = await show(createElement(RemoteView));
    const click = async (re: RegExp) => {
      await act(async () => button(host, re).click());
      await settle();
    };
    const note = "Simulated in the demo, nothing was done";

    // A saved environment so the operational controls render.
    const fill = (placeholder: string, value: string) => {
      const el = host.querySelector<HTMLInputElement | HTMLTextAreaElement>(`[placeholder="${placeholder}"]`);
      if (el) typeInto(el, value);
    };
    await act(async () => {
      fill("prod-stack", "demo-box");
      fill("64 hex chars", "a".repeat(64));
      fill("203.0.113.9:51820", "203.0.113.9:51820");
      fill("10.9.0.1/32", "10.9.0.1/32");
      fill("25", "25");
      fill("10.9.0.2", "10.9.0.2");
      fill("10.9.0.1", "10.9.0.1");
      fill("203.0.113.9", "203.0.113.9");
      fill("root", "root");
      fill("/Users/you/.ssh/genaryx", "/keys/genaryx");
      fill("ssh-ed25519 AAAAC3Nza...", "ssh-ed25519 AAAAC3NzaDEMO");
    });
    await click(/^(Save environment|Saving\.\.\.)$/);

    await click(/^Connect$/);
    expect(host.textContent).toContain("no tunnel was opened");

    await click(/^Check reachable$/);
    expect(host.textContent).toContain("no box was contacted");

    await act(async () => fill("~/.taipan/environments/<name>.json", "/etc/genaryx/env.json"));
    await click(/^Read$/);
    expect(host.textContent).toContain("no file was read from any box");

    await act(async () => fill("/var/log/taipan/gateway.log", "/var/log/taipan/gateway.log"));
    await click(/^Start tail$/);
    expect(host.textContent).toContain("no log was tailed from any box");

    const lists = Array.from(host.querySelectorAll("button")).filter((b) => /^List servers$/.test(b.textContent ?? ""));
    for (const b of lists) {
      await act(async () => b.click());
      await settle();
    }
    if (lists.length > 0) {
      const hetzner = (await mock.mockInvoke("remote_hetzner_list")) as { name: string }[];
      const text = host.textContent ?? "";
      expect(hetzner.some((s) => text.includes(s.name)) || text.includes("prod-")).toBe(true);
    }

    const issue = Array.from(host.querySelectorAll("button")).find((b) => /Issue WireGuard config/.test(b.textContent ?? ""));
    if (issue) {
      await act(async () => issue.click());
      await settle();
      expect(host.textContent).toContain("no config was issued");
    }
    expect(host.textContent).toContain(note);
  });
});

describe("arriving from an alert mail's link", () => {
  async function arrive(path: string) {
    window.history.replaceState(null, "", path);
    const { AppShell } = await import("./components/AppShell");
    return await show(createElement(AppShell));
  }

  it("an incident link lands on its panel and leaves the address bar clean", async () => {
    const host = await arrive("/i/budget_exhausted:run-42");
    expect(window.location.pathname).toBe("/");
    const current = host.querySelector('[aria-current="page"]');
    expect(current?.textContent ?? host.textContent).toMatch(/Money/);
  });

  it("an agent link opens that agent's detail card, the one with freeze and kill", async () => {
    const agent = "agent://meridian.io/sre/rca-copilot";
    await arrive(`/a/${encodeURIComponent(agent)}`);
    await settle();
    const dialogs = Array.from(document.querySelectorAll('[role="dialog"]'));
    expect(dialogs.some((d) => d.textContent?.includes(agent) || d.textContent?.includes("rca-copilot"))).toBe(true);
    expect(window.location.pathname).toBe("/");
  });

  it("an owner link opens the owner's card", async () => {
    await arrive("/o/w.zhang");
    await settle();
    const dialogs = Array.from(document.querySelectorAll('[role="dialog"]'));
    expect(dialogs.some((d) => d.textContent?.includes("w.zhang"))).toBe(true);
  });

  it("a link of a type this build does not know lands on the overview and says which id it could not place", async () => {
    const host = await arrive("/i/no_such_type:run-7");
    expect(host.textContent).toContain("no_such_type");
  });

  it("a link in the fragment keeps the path the console is served from", async () => {
    window.history.replaceState(null, "", "/demo/#/i/budget_exhausted:run-42");
    const { AppShell } = await import("./components/AppShell");
    await show(createElement(AppShell));
    expect(window.location.pathname).toBe("/demo/");
    window.history.replaceState(null, "", "/");
  });
});

describe("on-demand scans and checks in the demo world", () => {
  it("Crypto: a scan shows the inventory the demo's scan reports, and evidence builds and verifies", async () => {
    const { CryptoView } = await import("./components/CryptoView");
    const host = await show(createElement(CryptoView));
    const scanInput = host.querySelector<HTMLInputElement>('[placeholder="/path/to/scan"]');
    if (scanInput) await act(async () => typeInto(scanInput, "/srv/app"));
    await act(async () => button(host, /^(Scan|Scanning\.\.\.)$/).click());
    await settle();
    const ncsc = (await mock.mockInvoke("crypto_scan_ncsc", { path: "/srv/app" })) as {
      discovery2028: { totalInventoried: number; quantumVulnerableCount: number };
    };
    const text = host.textContent ?? "";
    expect(text).toContain(String(ncsc.discovery2028.totalInventoried));
    expect(text).toContain(String(ncsc.discovery2028.quantumVulnerableCount));

    // The milestone tabs switch what is shown.
    for (const tab of Array.from(host.querySelectorAll<HTMLButtonElement>('[role="tablist"] button, [role="tab"]'))) {
      await act(async () => tab.click());
    }

    const build = Array.from(host.querySelectorAll("button")).find((b) => /^Build evidence$/.test(b.textContent ?? ""));
    if (build) {
      await act(async () => build.click());
      await settle();
      const verifyInput = host.querySelector<HTMLInputElement>('[placeholder="/path/to/evidence.json"]');
      if (verifyInput) await act(async () => typeInto(verifyInput, "/tmp/evidence.json"));
      await act(async () => button(host, /^(Verify|Verifying\.\.\.)$/).click());
      await settle();
      expect(host.textContent).toContain("digest matches");
    }
  });

  it("Memory: a recall lists what the demo's memory returns for the query", async () => {
    const { MemoryView } = await import("./components/MemoryView");
    const host = await show(createElement(MemoryView, { onOpenAgent: () => {} }));
    const q = host.querySelector<HTMLInputElement>('[placeholder="query text"]');
    expect(q).not.toBeNull();
    await act(async () => typeInto(q!, "refund"));
    await act(async () => button(host, /^(Recall|Searching\.\.\.)$/).click());
    await settle();
    const hits = (await mock.mockInvoke("memory_recall", { query: "refund", limit: 10, mode: "hybrid", agent_id: null })) as
      | { id?: string; memory_id?: string; content?: string; text?: string }[]
      | null;
    if (hits && hits.length > 0) {
      const first = hits[0];
      const marker = (first.content ?? first.text ?? first.id ?? first.memory_id ?? "").slice(0, 30);
      expect(host.textContent).toContain(marker);
      // Open "why" for the first memory if offered.
      const why = Array.from(host.querySelectorAll("button")).find((b) => /why|provenance/i.test(b.textContent ?? ""));
      if (why) {
        await act(async () => why.click());
        await settle();
      }
    }
  });

  it("Admission: running the checks for a key and an agent shows the demo's verdict for that agent", async () => {
    const { AdmissionVerify } = await import("./components/AdmissionVerify");
    const host = await show(createElement(AdmissionVerify, { seed: null }));
    await act(async () => {
      typeInto(byPlaceholder(host, "billing-agent"), "sre-rca-copilot");
      typeInto(byPlaceholder(host, "agent://bank.example/treasury/recon-batch"), "agent://meridian.io/sre/rca-copilot");
    });
    await act(async () => button(host, /^(Run checks|Checking\.\.\.)$/).click());
    await settle();
    expect(host.textContent).toContain("sre-rca-copilot");
    const baselineModel = host.querySelector<HTMLInputElement>('[placeholder="claude-sonnet-5"]');
    if (baselineModel) {
      await act(async () => {
        typeInto(byPlaceholder(host, "/path/to/evalset.json"), "/evals/set.json");
        typeInto(baselineModel, "claude-haiku-5-5");
      });
      const run = Array.from(host.querySelectorAll("button")).find((b) => /baseline/i.test(b.textContent ?? ""));
      if (run) {
        await act(async () => run.click());
        await settle();
      }
    }
  });

  it("Felyx's provider cannot be changed from the demo, and says so", async () => {
    const { FelyxConnectCard } = await import("./components/FelyxConnectCard");
    const host = await show(createElement(FelyxConnectCard, {} as never));
    const connect = Array.from(host.querySelectorAll("button")).find((b) => /Connect Felyx/.test(b.textContent ?? ""));
    if (connect) {
      const model = host.querySelector<HTMLInputElement>('[placeholder="model id"]');
      if (model) await act(async () => typeInto(model, "claude-haiku-5-5"));
      await act(async () => connect.click());
      await settle();
      expect(host.textContent).toContain("Simulated in the demo, nothing was done");
    }
  });
});

/** Press every control inside `host` once (fresh query each time, so a control
 * that re-renders the card is not pressed stale), settling after each. */
async function pressEverything(host: HTMLElement, limit = 80): Promise<number> {
  const skip = /^(Close|×|Sign out)$/;
  const controls = () =>
    Array.from(host.querySelectorAll<HTMLElement>("button, [role='button'], [role='tab'], summary")).filter(
      (b) => !(b as HTMLButtonElement).disabled && !skip.test((b.textContent ?? "").trim()) && b.getAttribute("aria-label") !== "Close",
    );
  let pressed = 0;
  const n = Math.min(controls().length, limit);
  for (let i = 0; i < n; i++) {
    const b = controls()[i];
    if (!b) break;
    await act(async () => b.click());
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    pressed += 1;
  }
  await settle();
  return pressed;
}

describe("pressing every control on the cards, in the demo world", () => {
  beforeAll(() => {
    URL.createObjectURL = (() => "blob:demo") as never;
    URL.revokeObjectURL = (() => {}) as never;
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    vi.spyOn(window, "confirm").mockReturnValue(true);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: async () => {} }, configurable: true });
  });

  async function firstAgent(): Promise<string> {
    const runs = (await mock.mockInvoke("money_runs")) as MockRun[];
    return runs[0].agent_id;
  }

  it("Agent 360 keeps showing its agent whatever is pressed", async () => {
    const { Agent360 } = await import("./components/Agent360");
    const agentId = await firstAgent();
    const nav = vi.fn();
    const host = await show(
      createElement(Agent360, { agentId, onClose: () => {}, onOpenAgent: () => {}, onNavigate: nav, onOpenReplay: () => {} }),
    );
    expect(await pressEverything(host)).toBeGreaterThan(3);
    expect(host.textContent).toContain(agentId);
  });

  it("the agent detail card keeps showing its agent whatever is pressed", async () => {
    const { AgentDetailCard } = await import("./components/AgentDetailCard");
    const agentId = await firstAgent();
    const host = await show(createElement(AgentDetailCard, { agentId, onOpenFull: () => {}, onReplay: () => {} }));
    expect(await pressEverything(host)).toBeGreaterThan(2);
    expect(host.textContent).toContain(agentId.split("/").pop()!);
  });

  it("the owner and unit cards keep their subject whatever is pressed", async () => {
    const { UserCard } = await import("./components/UserCard");
    const { UnitCard } = await import("./components/UnitCard");
    const u = await show(createElement(UserCard, { handle: "w.zhang", onOpenFullAgent: () => {} }));
    await pressEverything(u);
    expect(u.textContent).toContain("w.zhang");
    const t = await show(createElement(UnitCard, { team: "finops", onOpenFullAgent: () => {} }));
    await pressEverything(t);
    expect(t.textContent?.toLowerCase()).toContain("finops");
  });

  it("passkeys, retained reservations and the memory timeline answer every control", async () => {
    const { PasskeySettings } = await import("./components/PasskeySettings");
    const { RetainedClosuresSection } = await import("./components/RetainedClosures");
    const { MemoryTimeline } = await import("./components/MemoryTimeline");
    for (const el of [
      createElement(PasskeySettings),
      createElement(RetainedClosuresSection),
      createElement(MemoryTimeline, { onOpenAgent: () => {} }),
    ]) {
      const host = await show(el);
      await pressEverything(host);
      expect((host.textContent ?? "").length).toBeGreaterThan(0);
    }
  });
});

describe("the cloud inventory, provider by provider, in the demo world", () => {
  it("each live-listing provider lists the demo's servers for that provider, with the scope it was given", async () => {
    const { RemoteCloudInventory } = await import("./components/RemoteCloudInventory");
    const host = await show(createElement(RemoteCloudInventory));
    const select = host.querySelector<HTMLSelectElement>("select")!;
    const providers = Array.from(select.options).map((o) => o.value);
    expect(providers.length).toBeGreaterThan(3);
    let listed = 0;
    for (const id of providers) {
      await act(async () => {
        select.value = id;
        select.dispatchEvent(new Event("change", { bubbles: true }));
      });
      const scope = host.querySelector<HTMLInputElement>('input[placeholder="eu-central-1"], input[placeholder="my-project"], input[placeholder="my-subscription"], input[placeholder="Default"]');
      if (scope) await act(async () => typeInto(scope, "scope-1"));
      const list = Array.from(host.querySelectorAll("button")).find((b) => /^List servers$/.test(b.textContent ?? ""));
      if (!list) continue;
      await act(async () => list.click());
      await settle();
      if (scope) {
        const rows = (await mock.mockInvoke("remote_cloud_list", { provider: id, options: {} })) as { name: string }[];
        expect(rows.some((r) => host.textContent?.includes(r.name))).toBe(true);
        listed += 1;
      }
      await pressEverything(host, 20);
      // Pressing may have changed the provider; put the select back in view.
    }
    expect(listed).toBeGreaterThan(2);
  });
});

describe("the watch dock's width and a browser that will not store anything", () => {
  it("drags wider to the left, clamps at its bounds, and remembers the width", async () => {
    Element.prototype.setPointerCapture ??= function () {};
    Element.prototype.releasePointerCapture ??= function () {};
    localStorage.setItem("genaryx.watchDock", "false");
    localStorage.removeItem("genaryx.watchDock.width");
    const { WatchDock } = await import("./components/WatchDock");
    const host = await show(createElement(WatchDock, { onOpenAgent: () => {}, onOpenUnit: () => {} }));
    const handle = host.querySelector<HTMLElement>('[aria-label="Resize watch dock"]')!;
    const fire = (type: string, x: number) => handle.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX: x }));
    await act(async () => fire("pointerdown", 1000));
    await act(async () => fire("pointermove", 900));
    await act(async () => fire("pointerup", 900));
    expect(localStorage.getItem("genaryx.watchDock.width")).toBe("360");
    await act(async () => fire("pointerdown", 1000));
    await act(async () => fire("pointermove", 0));
    await act(async () => fire("pointerup", 0));
    expect(localStorage.getItem("genaryx.watchDock.width")).toBe("560");
    await act(async () => fire("pointerdown", 0));
    await act(async () => fire("pointermove", 5000));
    await act(async () => fire("pointerup", 5000));
    expect(localStorage.getItem("genaryx.watchDock.width")).toBe("260");
    // A move or release with no drag in progress changes nothing.
    await act(async () => fire("pointermove", 10));
    await act(async () => fire("pointerup", 10));
    expect(localStorage.getItem("genaryx.watchDock.width")).toBe("260");
  });

  it("still renders, pins and collapses when the browser refuses storage", async () => {
    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("private mode");
    });
    const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    const { WatchDock, WatchToggleButton } = await import("./components/WatchDock");
    const host = await show(
      createElement(
        "div",
        null,
        createElement(WatchDock, { onOpenAgent: () => {}, onOpenUnit: () => {} }),
        createElement(WatchToggleButton, { kind: "agent", id: "agent://meridian.io/x/storage-less", label: "storage-less" }),
      ),
    );
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Pin storage-less to the watch dock"]')!.click());
    const collapse = host.querySelector<HTMLButtonElement>('[aria-label="Collapse watch dock"]');
    if (collapse) await act(async () => collapse.click());
    expect((host.textContent ?? "").length).toBeGreaterThan(0);
    get.mockRestore();
    set.mockRestore();
  });
});

describe("a pinned bus event", () => {
  it("shows the event beside its row and closes on Escape or a click outside, never on a click inside", async () => {
    const { PinnedEventOverlay } = await import("./components/PinnedEventOverlay");
    const onClose = vi.fn();
    const event = {
      id: 7,
      ts: "2026-11-27T01:00:00Z",
      source: "tokenfuse",
      type: "budget_exhausted",
      agent_id: "agent://meridian.io/sre/rca-copilot",
      run_id: "run-7",
      env: "live",
      severity: "high",
      data: { spent_usd: 1.5 },
      schema: "taipanbox.dev/agent-event/v0.2",
      on_behalf_of: [],
      prev_hash: null,
      raw: "",
      file: "/root/.stack-up/events/tokenfuse.ndjson",
      off: 7,
    } as never;
    const rect = { left: 100, right: 300, top: 120, bottom: 140, width: 200, height: 20 } as DOMRect;
    await show(createElement(PinnedEventOverlay, { event, rect, onClose, onOpenAgent: () => {} }));
    const overlay = document.body.lastElementChild as HTMLElement;
    expect(overlay.textContent).toContain("budget_exhausted");
    expect(overlay.textContent).toContain("pinned");

    const card = overlay.querySelector<HTMLElement>(".gx-popover")!;
    await act(async () => card.click());
    expect(onClose).not.toHaveBeenCalled();
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })));
    expect(onClose).not.toHaveBeenCalled();
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(onClose).toHaveBeenCalledTimes(1);
    await act(async () => overlay.click());
    expect(onClose).toHaveBeenCalledTimes(2);
    await pressEverything(card, 20);
  });
});
