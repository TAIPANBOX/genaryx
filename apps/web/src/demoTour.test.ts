// @vitest-environment jsdom
/**
 * A walk through the whole console as the published demo serves it: the real
 * `AppShell` over the mock transport, every view in the rail opened in turn.
 *
 * What it holds, per view: opening it asks its own plane (the command the view
 * exists to read reaches the transport), the view's own data reaches the page
 * where the demo world has a known row for it, and no component crashes on the
 * way (React reports a crash through `console.error` with "The above error
 * occurred"). A view that stopped reading its plane, or that threw on the
 * demo's own data, is exactly what an operator would meet first on
 * it-rat.com, and no other suite opens every view.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const seen = vi.hoisted(() => ({ commands: [] as string[] }));

vi.mock("./lib/mockPreview", async (importOriginal) => {
  const real = await importOriginal<typeof import("./lib/mockPreview")>();
  return {
    ...real,
    mockInvoke: async (command: string, args?: Record<string, unknown>) => {
      seen.commands.push(command);
      return real.mockInvoke(command, args);
    },
  };
});

class Inert {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}

let root: Root;
let host: HTMLDivElement;
const crashes: string[] = [];
let VIEWS: readonly { id: string; label: string }[];
let realInvoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;

async function settle() {
  for (let i = 0; i < 4; i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
  }
}

function navButton(label: string): HTMLButtonElement {
  const nav = Array.from(document.querySelectorAll("button")).filter(
    (b) => b.textContent?.trim() === label,
  );
  if (nav.length === 0) throw new Error(`no rail button labelled ${label}`);
  return nav[0] as HTMLButtonElement;
}

beforeAll(async () => {
  vi.stubEnv("VITE_GENARYX_MOCK", "1");
  vi.stubEnv("VITE_GENARYX_API", "");
  vi.stubGlobal("ResizeObserver", Inert);
  vi.stubGlobal("IntersectionObserver", Inert);
  window.matchMedia ??= ((q: string) => ({
    matches: false,
    media: q,
    onchange: null,
    addListener() {},
    removeListener() {},
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  Element.prototype.scrollIntoView ??= function () {};
  vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
    const text = args.map(String).join(" ");
    if (/The above error occurred|Uncaught|Error: Minified React error/.test(text)) crashes.push(text);
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});

  const { AppShell } = await import("./components/AppShell");
  const { PopoverProvider } = await import("./lib/popover");
  const mock = await vi.importActual<typeof import("./lib/mockPreview")>("./lib/mockPreview");
  realInvoke = mock.mockInvoke as typeof realInvoke;
  VIEWS = (await import("./lib/views")).VIEWS;

  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root.render(createElement(PopoverProvider, null, createElement(AppShell)));
  });
  await settle();
});

afterAll(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

/** The command each view exists to read. A view may ask more; it must ask this. */
const READS: Record<string, RegExp> = {
  overview: /^money_overview$/,
  anomalies: /^(money_incidents|recent_events|identity_list_alerts)$/,
  stats: /^stats_counts$/,
  money: /^money_runs$/,
  policy: /^policy_list_approvals$/,
  identity: /^identity_list_identities$/,
  copilot: /^copilot_status$/,
  graph: /^agent_graph$/,
  replay: /^(run_events|money_runs|recent_events)$/,
  bus: /^(recent_events|bus_status|bus_quarantine)$/,
  egress: /^egress_recent$/,
  quality: /^quality_list_run_summaries$/,
  memory: /^memory_(status|stats)$/,
  crypto: /^crypto_status$/,
  drills: /^drills_status$/,
  evidence: /^evidence_status$/,
  posture: /^credentials_/,
  routines: /^routines_status$/,
  notifications: /^notifications_sent$/,
  keys: /^keys_custody$/,
  record: /^record_status$/,
  onboard: /^onboard_status$/,
  remote: /^remote_status$/,
};

describe("the demo console, view by view", () => {
  it("opens on the overview, which asks the money plane", () => {
    expect(seen.commands).toContain("money_overview");
    expect(crashes).toEqual([]);
  });

  it("names a read for every view in the rail, so a new view is walked too", () => {
    expect(VIEWS.map((v) => v.id).sort()).toEqual(Object.keys(READS).sort());
  });

  it("every view asks its own plane when opened, and none of them crashes", async () => {
    const missing: string[] = [];
    for (const v of VIEWS) {
      seen.commands = [];
      await act(async () => {
        navButton(v.label).click();
      });
      await settle();
      if (!seen.commands.some((c) => READS[v.id].test(c))) {
        missing.push(`${v.id}: asked ${JSON.stringify([...new Set(seen.commands)])}`);
      }
    }
    expect(missing).toEqual([]);
    expect(crashes).toEqual([]);
  });

  it("the money view lists the demo's runs by id", async () => {
    await act(async () => navButton("Money").click());
    await settle();
    const runs = (await realInvoke("money_runs")) as { run_id: string }[];
    expect(runs.length).toBeGreaterThan(0);
    const text = document.body.textContent ?? "";
    const shown = runs.filter((r) => text.includes(r.run_id));
    expect(shown.length).toBeGreaterThan(0);
  });

  it("the identity view lists the demo's identities", async () => {
    await act(async () => navButton("Identity").click());
    await settle();
    const ids = (await realInvoke("identity_list_identities")) as Record<string, unknown>[];
    expect(ids.length).toBeGreaterThan(0);
    const text = document.body.textContent ?? "";
    const values = ids.flatMap((i) => Object.values(i).filter((x): x is string => typeof x === "string" && x.length > 6));
    expect(values.some((v) => text.includes(v))).toBe(true);
  });
});
