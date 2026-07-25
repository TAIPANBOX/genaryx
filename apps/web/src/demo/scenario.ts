// Shared scenario state for the Live Demo sandbox (it-rat.com "Live demo").
//
// The demo runs the REAL console under the mock transport (VITE_GENARYX_MOCK):
// every command routes to src/lib/mockPreview.ts and the bus to its synthetic
// stream, so nothing leaves the browser. This module is the one piece of
// shared state the demo adds on top: which storyline the simulated world is
// telling right now. Two readers use it - the mockPreview world simulator
// (what events/state it emits) and the demo funnel's scenario switcher (the
// calm <-> incident toggle the operator flips).
//
// In-memory ONLY, on purpose: a hard sandbox guarantee is "refresh = clean
// slate", so this never touches localStorage. Refreshing the page reloads the
// module and resets to the default below.

export type DemoScenario = "calm" | "incident";

/** The default storyline. "incident" leads with the runaway-agent-caught demo
 * (budget near cap, kill-switch save) because that is what sells governance;
 * the switcher flips to "calm" (all green, operator just walking the tabs). */
let current: DemoScenario = "incident";

const listeners = new Set<() => void>();

export function getScenario(): DemoScenario {
  return current;
}

export function setScenario(next: DemoScenario): void {
  if (next === current) return;
  current = next;
  for (const l of listeners) l();
}

/** Subscribe to scenario flips; returns an unsubscribe. The world simulator
 * uses this to re-arm its tick-loop when the operator switches storyline. */
export function onScenarioChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
