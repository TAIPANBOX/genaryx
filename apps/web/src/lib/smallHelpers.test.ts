// @vitest-environment jsdom
/**
 * The small shared helpers every panel leans on: money and time formatting,
 * the leave guard that protects an uncopied approval token, and the
 * console-state signal that makes a lifecycle action show on every open panel.
 */

import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { formatBytes, formatHm, formatTimestamp, formatUsd } from "./format";
import { LEAVE_QUESTION, holdLeave, mayLeave, useLeaveGuard } from "./leaveGuard";
import { CONSOLE_STATE_CHANGED_EVENT, notifyConsoleStateChanged, useConsoleStateVersion } from "./consoleState";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("formatUsd", () => {
  it("shows sub-dollar spend to four places so a sub-cent amount is never $0.00", () => {
    expect(formatUsd(0.0012)).toBe("$0.0012");
    expect(formatUsd(-0.5)).toBe("$-0.5000");
  });
  it("shows a dollar or more, and exactly zero, to two places", () => {
    expect(formatUsd(0)).toBe("$0.00");
    expect(formatUsd(1)).toBe("$1.00");
    expect(formatUsd(1234.567)).toBe("$1234.57");
  });
});

describe("formatHm and formatTimestamp", () => {
  it("formats local hours and minutes with zero padding", () => {
    const ms = new Date(2026, 10, 26, 7, 5, 9).getTime();
    expect(formatHm(ms)).toBe("07:05");
  });
  it("says a dash for a time that is not a time", () => {
    expect(formatHm(Number.NaN)).toBe("-");
  });
  it("formats a table-row timestamp with seconds", () => {
    const iso = new Date(2026, 10, 26, 7, 5, 9).toISOString();
    expect(formatTimestamp(iso)).toMatch(/ 07:05:09$/);
  });
  it("hands back a string that is not a date unchanged rather than 'Invalid Date'", () => {
    expect(formatTimestamp("not a date")).toBe("not a date");
  });
});

describe("formatBytes", () => {
  it("never shows a fabricated 0 for a store with no file", () => {
    expect(formatBytes(null)).toBe("in-memory / n/a");
  });
  it("walks the units and keeps two places under ten, one above", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1024)).toBe("1.00 KB");
    expect(formatBytes(15 * 1024)).toBe("15.0 KB");
    expect(formatBytes(3 * 1024 ** 2)).toBe("3.00 MB");
    expect(formatBytes(2 * 1024 ** 3)).toBe("2.00 GB");
    expect(formatBytes(5 * 1024 ** 4)).toBe("5.00 TB");
  });
  it("stops at TB rather than running off the end of the unit list", () => {
    expect(formatBytes(2048 * 1024 ** 4)).toBe("2048.0 TB");
  });
});

describe("the leave guard", () => {
  afterEach(() => vi.restoreAllMocks());

  it("lets a view change go at once while nothing holds the page", () => {
    const confirm = vi.spyOn(window, "confirm");
    expect(mayLeave()).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("asks before leaving while held, and stops asking once released", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const release = holdLeave();
    expect(mayLeave()).toBe(false);
    expect(confirm).toHaveBeenCalledWith(LEAVE_QUESTION);

    confirm.mockReturnValue(true);
    expect(mayLeave()).toBe(true);

    release();
    confirm.mockClear();
    expect(mayLeave()).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("raises the browser's own question on unload only while held", () => {
    const release = holdLeave();
    const held = new Event("beforeunload", { cancelable: true }) as BeforeUnloadEvent;
    window.dispatchEvent(held);
    expect(held.defaultPrevented).toBe(true);
    release();

    const free = new Event("beforeunload", { cancelable: true }) as BeforeUnloadEvent;
    window.dispatchEvent(free);
    expect(free.defaultPrevented).toBe(false);
  });

  it("stacks holds: the page is free only when every one is released, and a double release is harmless", () => {
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const a = holdLeave();
    const b = holdLeave();
    a();
    a();
    expect(mayLeave()).toBe(false);
    b();
    expect(mayLeave()).toBe(true);
  });

  it("holds through the hook only while held is true and the component is mounted", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(false);
    function Probe({ held }: { held: boolean }) {
      useLeaveGuard(held);
      return null;
    }
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(createElement(Probe, { held: false })));
    expect(mayLeave()).toBe(true);
    await act(async () => root.render(createElement(Probe, { held: true })));
    expect(mayLeave()).toBe(false);
    await act(async () => root.unmount());
    expect(mayLeave()).toBe(true);
  });
});

describe("the console-state signal", () => {
  it("bumps every mounted view's version once per notification, and stops after unmount", async () => {
    const versions: number[] = [];
    function Probe() {
      versions.push(useConsoleStateVersion());
      return null;
    }
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(createElement(Probe)));
    expect(versions[versions.length - 1]).toBe(0);

    await act(async () => notifyConsoleStateChanged());
    await act(async () => notifyConsoleStateChanged());
    expect(versions[versions.length - 1]).toBe(2);

    await act(async () => root.unmount());
    const listener = vi.fn();
    window.addEventListener(CONSOLE_STATE_CHANGED_EVENT, listener);
    notifyConsoleStateChanged();
    expect(listener).toHaveBeenCalledTimes(1);
    window.removeEventListener(CONSOLE_STATE_CHANGED_EVENT, listener);
  });

  it("never throws a UI action over a window that cannot dispatch", () => {
    const spy = vi.spyOn(window, "dispatchEvent").mockImplementation(() => {
      throw new Error("locked-down embed");
    });
    expect(() => notifyConsoleStateChanged()).not.toThrow();
    spy.mockRestore();
  });
});
