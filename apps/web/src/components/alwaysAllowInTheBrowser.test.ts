// @vitest-environment jsdom
/**
 * Always allow in a real browser document: the whole console in its demo build,
 * driven by clicks, so everything between the button and the policy list is the
 * code an operator runs. The first row of the demo's queue is the no-grant floor.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;

async function settle(ms = 20): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function until<T>(what: string, read: () => T | null | undefined): Promise<T> {
  for (let i = 0; i < 300; i++) {
    const found = read();
    if (found !== null && found !== undefined) return found;
    await settle();
  }
  throw new Error(`gave up waiting for ${what}`);
}

async function click(element: Element): Promise<void> {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
}

function buttonNamed(name: string): HTMLButtonElement | undefined {
  return Array.from(document.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === name || b.getAttribute("aria-label") === name,
  );
}

async function press(name: string): Promise<void> {
  await click(await until(`a button named ${name}`, () => buttonNamed(name)));
}

/** The confirm step of the row: its label names the tool. */
async function confirm(): Promise<void> {
  const step = await until("the confirm step", () =>
    Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.startsWith("Confirm: always allow")),
  );
  await click(step);
}

beforeEach(async () => {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", "1");
  vi.stubEnv("VITE_GENARYX_API", "");
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  Element.prototype.scrollIntoView = () => {};
  const { AppShell } = await import("./AppShell");
  const { PopoverProvider } = await import("../lib/popover");
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root.render(createElement(PopoverProvider, null, createElement(AppShell)));
  });
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  host.remove();
  localStorage.clear();
  sessionStorage.clear();
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("always allow, clicked", () => {
  it("always_allow_in_the_console_writes_a_policy_and_the_row_says_this_call_still_waits", async () => {
    await press("Policy");
    await press("Always allow");
    await confirm();
    await until("the notice", () => (document.body.textContent?.includes("in force now") ? true : null));
    await until("the row's line", () =>
      document.body.textContent?.includes("This call still waits for an answer") ? true : null,
    );
    expect(buttonNamed("Always allow")).toBeUndefined();
    expect(buttonNamed("Grant")).toBeDefined();
  }, 30_000);
});
