// @vitest-environment jsdom
/**
 * The raw token in a real browser document: the whole console, in its demo
 * build, driven by clicks. The demo's transport answers `policy_decide_approval`
 * with a token, so everything between the Grant button and the panel is the code
 * an operator runs.
 *
 * What this holds is what no static render can: that the token is on the screen
 * after a grant, that nothing the page writes to holds it, and that leaving the
 * view takes it away.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const DEMO_TOKEN = "demo-approval-token.not-a-real-credential";

// React's act() needs to be told it is in a test.
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

function tokenOnScreen(): string | null {
  return document.querySelector('[data-testid="approval-token"]')?.textContent ?? null;
}

/** The token inside whatever holds the focus, or `null` when the focus is elsewhere. */
function focusedToken(): string | null {
  const focused = document.activeElement;
  if (!focused || focused === document.body) return null;
  return focused.querySelector('[data-testid="approval-token"]')?.textContent ?? null;
}

function notice(): Element | null {
  return document.querySelector('[data-testid="granted-token-notice"]');
}

/** Whether the page asks before it goes: what a reload, a closed tab or a typed
 * address raises, and what the browser turns into its own question. */
function leavingIsQuestioned(): boolean {
  const event = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(event);
  return event.defaultPrevented;
}

function stubClipboard(): void {
  const writeText = async (_text: string) => {};
  vi.stubGlobal("navigator", Object.assign(Object.create(navigator), { clipboard: { writeText } }));
}

async function openPolicy(): Promise<void> {
  await press("Policy");
  await until("the approvals queue", () => buttonNamed("Grant"));
}

/** Grant the first hold in the queue, through its own confirm step. */
async function grantTheFirstHold(): Promise<void> {
  await press("Grant");
  await press("Confirm grant");
}

beforeEach(async () => {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", "1");
  vi.stubEnv("VITE_GENARYX_API", "");
  // jsdom draws nothing, so the two browser services the console asks after are
  // answered here. Neither touches a token.
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

describe("a person's grant in the console", () => {
  it("a_grant_in_the_console_shows_the_raw_token_once_with_its_header", async () => {
    await openPolicy();
    expect(tokenOnScreen()).toBeNull();
    await grantTheFirstHold();
    expect(await until("the token", tokenOnScreen)).toBe(DEMO_TOKEN);
    const text = document.body.textContent ?? "";
    expect(text).toContain("x-fuse-approval-token");
    expect(text).toContain("cannot be shown again");
    expect(text).toMatch(/expires in \d\d:\d\d/);
  });

  it("the_copy_button_puts_the_raw_token_on_the_clipboard_and_says_so", async () => {
    const writeText = vi.fn(async (_text: string) => {});
    vi.stubGlobal("navigator", Object.assign(Object.create(navigator), { clipboard: { writeText } }));
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    await press("Copy the approval token");
    await until("the copy to finish", () => (document.body.textContent?.includes("Copied") ? true : null));
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith(DEMO_TOKEN);
  });

  it("a_copy_the_browser_refuses_says_so_and_the_token_stays", async () => {
    vi.stubGlobal(
      "navigator",
      Object.assign(Object.create(navigator), {
        clipboard: {
          writeText: async () => {
            throw new Error("not allowed");
          },
        },
      }),
    );
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    await press("Copy the approval token");
    await until("the refusal", () => document.querySelector('[role="alert"]'));
    expect(document.body.textContent).toContain("copy it by hand");
    expect(tokenOnScreen()).toBe(DEMO_TOKEN);
  });

  it("leaving_the_view_drops_the_token_and_coming_back_does_not_restore_it", async () => {
    const confirm = vi.fn(() => true);
    vi.stubGlobal("confirm", confirm);
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);

    await press("Overview");
    expect(confirm).toHaveBeenCalledTimes(1);
    await until("the policy view to go", () => (buttonNamed("Copy the approval token") ? null : true));
    expect(document.documentElement.innerHTML).not.toContain(DEMO_TOKEN);

    await openPolicy();
    await settle(50);
    expect(tokenOnScreen()).toBeNull();
    expect(document.documentElement.innerHTML).not.toContain(DEMO_TOKEN);
  });

  it("dismissing_drops_the_token_and_it_cannot_be_read_back", async () => {
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    await press("Dismiss");
    await settle(50);
    expect(tokenOnScreen()).toBeNull();
    expect(document.documentElement.innerHTML).not.toContain(DEMO_TOKEN);
  });

  it("two_grants_in_a_row_keep_both_tokens_on_screen", async () => {
    await openPolicy();
    await grantTheFirstHold();
    await until("the first token", tokenOnScreen);
    await settle(50);
    await grantTheFirstHold();
    await until("both tokens", () =>
      document.querySelectorAll('[data-testid="approval-token"]').length === 2 ? true : null,
    );
  });

  it("a_grant_brings_the_token_into_view_and_puts_the_focus_on_it", async () => {
    const scrolled: Element[] = [];
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this);
    };
    await openPolicy();
    await grantTheFirstHold();
    expect(await until("the token", tokenOnScreen)).toBe(DEMO_TOKEN);
    expect(scrolled.some((element) => element.querySelector('[data-testid="approval-token"]') !== null)).toBe(true);
    expect(focusedToken()).toBe(DEMO_TOKEN);
  });

  it("a_notice_says_a_token_is_on_screen_and_shows_it_again", async () => {
    stubClipboard();
    await openPolicy();
    expect(notice()).toBeNull();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    expect(notice()?.textContent).toContain("not copied yet");
    expect(notice()?.textContent).toContain("cannot be shown again");
    expect(notice()?.textContent).not.toContain(DEMO_TOKEN);

    (document.activeElement as HTMLElement | null)?.blur();
    expect(focusedToken()).toBeNull();
    await press("Show the token");
    expect(focusedToken()).toBe(DEMO_TOKEN);

    await press("Copy the approval token");
    await until("the copy to finish", () => (notice()?.textContent?.includes("Approval token copied") ? true : null));
    await press("Dismiss");
    expect(notice()).toBeNull();
  });

  it("reloading_the_page_with_an_uncopied_token_asks_first", async () => {
    await openPolicy();
    expect(leavingIsQuestioned()).toBe(false);
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    expect(leavingIsQuestioned()).toBe(true);
  });

  it("a_copied_or_dismissed_token_lets_the_page_go_without_asking", async () => {
    stubClipboard();
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    await press("Copy the approval token");
    await until("the copy to finish", () => (buttonNamed("Copied") ? true : null));
    expect(leavingIsQuestioned()).toBe(false);

    await settle(50);
    await grantTheFirstHold();
    await until("the second token", () =>
      document.querySelectorAll('[data-testid="approval-token"]').length === 2 ? true : null,
    );
    expect(leavingIsQuestioned()).toBe(true);
    await press("Dismiss");
    expect(leavingIsQuestioned()).toBe(false);
  });

  it("a_copy_the_browser_refused_still_asks_before_the_page_goes", async () => {
    vi.stubGlobal(
      "navigator",
      Object.assign(Object.create(navigator), {
        clipboard: {
          writeText: async () => {
            throw new Error("not allowed");
          },
        },
      }),
    );
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    await press("Copy the approval token");
    await until("the refusal", () => document.querySelector('[role="alert"]'));
    expect(leavingIsQuestioned()).toBe(true);
  });

  it("a_token_copied_by_hand_counts_as_copied", async () => {
    await openPolicy();
    await grantTheFirstHold();
    const token = await until("the token", () => document.querySelector('[data-testid="approval-token"]'));
    expect(leavingIsQuestioned()).toBe(true);
    await act(async () => {
      token.dispatchEvent(new Event("copy", { bubbles: true, cancelable: true }));
    });
    expect(leavingIsQuestioned()).toBe(false);
  });

  it("leaving_the_view_with_an_uncopied_token_asks_first_and_staying_keeps_it", async () => {
    const confirm = vi.fn((_question?: string) => false);
    vi.stubGlobal("confirm", confirm);
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);

    await press("Overview");
    await settle(50);
    expect(confirm).toHaveBeenCalledTimes(1);
    const question = confirm.mock.calls[0]?.[0] ?? "";
    expect(question).toContain("cannot be shown again");
    expect(question).not.toContain(DEMO_TOKEN);
    expect(tokenOnScreen()).toBe(DEMO_TOKEN);
  });

  it("leaving_the_view_after_the_copy_does_not_ask", async () => {
    stubClipboard();
    const confirm = vi.fn(() => false);
    vi.stubGlobal("confirm", confirm);
    await openPolicy();
    await grantTheFirstHold();
    await until("the token", tokenOnScreen);
    expect(leavingIsQuestioned()).toBe(true);
    await press("Copy the approval token");
    await until("the copy to finish", () => (buttonNamed("Copied") ? true : null));

    await press("Overview");
    await until("the policy view to go", () => (buttonNamed("Copied") ? null : true));
    expect(confirm).not.toHaveBeenCalled();
    expect(document.documentElement.innerHTML).not.toContain(DEMO_TOKEN);
  });

  it("the_token_reaches_no_browser_storage_no_url_no_cookie_and_no_console_line", async () => {
    const written: unknown[][] = [];
    const keep = (...args: unknown[]) => {
      written.push(args);
    };
    for (const method of ["log", "info", "warn", "error", "debug", "trace"] as const) {
      vi.spyOn(console, method).mockImplementation(keep);
    }
    // jsdom has no clipboard, so the copy fails and leaving asks first: say yes.
    vi.stubGlobal("confirm", () => true);
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    const pushState = vi.spyOn(history, "pushState");
    const replaceState = vi.spyOn(history, "replaceState");

    await openPolicy();
    await grantTheFirstHold();
    expect(await until("the token", tokenOnScreen)).toBe(DEMO_TOKEN);
    await press("Copy the approval token");
    await settle(50);
    await press("Overview");
    await settle(50);

    const seen = [
      JSON.stringify(written),
      JSON.stringify(setItem.mock.calls),
      JSON.stringify(pushState.mock.calls),
      JSON.stringify(replaceState.mock.calls),
      JSON.stringify(history.state),
      JSON.stringify({ ...localStorage }),
      JSON.stringify({ ...sessionStorage }),
      document.cookie,
      window.location.href,
    ].join("\n");
    for (const part of [DEMO_TOKEN, ...DEMO_TOKEN.split(".")]) {
      expect(seen).not.toContain(part);
    }
  });
});
