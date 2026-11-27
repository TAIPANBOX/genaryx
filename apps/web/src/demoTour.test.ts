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
vi.setConfig({ testTimeout: 30_000 });

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
  Object.defineProperty(navigator, "clipboard", { value: { writeText: async () => {} }, configurable: true });
  URL.createObjectURL ??= (() => "blob:demo") as never;
  URL.revokeObjectURL ??= (() => {}) as never;
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  vi.spyOn(window, "confirm").mockReturnValue(true);
  Element.prototype.scrollTo ??= function () {} as never;
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
    // Leave the overview first, so opening it is a real visit too.
    await act(async () => navButton(VIEWS[VIEWS.length - 1].label).click());
    await settle();
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
    // The runs board's id cell, not the page at large: a run id also turns
    // up in incident titles and replay links, which would hide a board that
    // stopped printing it.
    const cells = Array.from(document.querySelectorAll(".rid")).map((c) => c.textContent?.trim());
    expect(cells.length).toBeGreaterThan(0);
    for (const cell of cells) expect(runs.map((r) => r.run_id)).toContain(cell);
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

  it("each incident on the Incidents tab opens its Incident 360 over the console, which closes again", async () => {
    await act(async () => navButton("Incidents").click());
    await settle();
    const rows = () => Array.from(document.querySelectorAll<HTMLElement>(".d-arow.clk[role='button']"));
    const count = rows().length;
    expect(count).toBeGreaterThan(0);
    for (let i = 0; i < Math.min(count, 6); i++) {
      const row = rows()[i];
      const m = row.querySelector(".m")!.cloneNode(true) as HTMLElement;
      m.querySelectorAll("button").forEach((b) => b.remove());
      const title = (m.textContent ?? "").trim();
      await act(async () => row.click());
      await settle();
      const close = document.querySelector<HTMLButtonElement>('[aria-label="Close Incident 360"]');
      expect(close).not.toBeNull();
      const overlay = close!.parentElement!;
      const words = title.split(/\s+/).filter((w) => w.length > 4).slice(0, 2);
      for (const w of words) expect(overlay.textContent).toContain(w);
      await act(async () => close!.click());
      expect(document.querySelector('[aria-label="Close Incident 360"]')).toBeNull();
    }
    expect(crashes).toEqual([]);
  });

  it("pressing every control in every view, the demo answers and nothing crashes or leaves the view", async () => {
    const skip = /^(Sign out|Log out|dismiss)$/i;
    const pressed: Record<string, number> = {};
    for (const v of VIEWS) {
      await act(async () => navButton(v.label).click());
      await settle();
      const main = () => document.querySelector(".main-col")!;
      const controls = () =>
        Array.from(main().querySelectorAll<HTMLElement>("button, [role='button'], [role='tab']")).filter(
          (b) => !(b as HTMLButtonElement).disabled && !skip.test((b.textContent ?? "").trim()),
        );
      const n = Math.min(controls().length, 60);
      pressed[v.id] = 0;
      for (let i = 0; i < n; i++) {
        const b = controls()[i];
        if (!b) break;
        await act(async () => {
          b.click();
        });
        await act(async () => {
          await new Promise((r) => setTimeout(r, 0));
        });
        pressed[v.id] += 1;
        // A control that navigated away is followed back, so the rest of this
        // view's controls are still pressed.
        if (document.querySelector('[aria-current="page"]')?.textContent?.trim() !== v.label) {
          await act(async () => navButton(v.label).click());
          await settle();
        }
        // Close whatever it opened over the console.
        for (const c of Array.from(
          document.querySelectorAll<HTMLButtonElement>('[aria-label="Close Incident 360"], [role="dialog"] [aria-label="Close"]'),
        )) {
          await act(async () => c.click());
        }
      }
      expect(crashes).toEqual([]);
      expect(main().textContent?.length ?? 0).toBeGreaterThan(0);
    }
    expect(Object.values(pressed).reduce((a, b) => a + b, 0)).toBeGreaterThan(100);
  }, 120_000);
});
