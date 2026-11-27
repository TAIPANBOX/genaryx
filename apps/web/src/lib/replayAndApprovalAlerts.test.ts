// @vitest-environment jsdom
/**
 * Two small pieces of the console that run on their own clock or the
 * browser's: the Replay view's playback clock (`useReplayClock`), and the
 * desktop notification an approval request raises (`lib/notifications.ts`).
 *
 * The clock replays a run at the pace it happened, bounded so a gap of an
 * hour does not stall the screen and a burst does not blur past: a step waits
 * the real gap divided by the speed, never under 150 ms or over 2 s. It stops
 * itself at the end, and a new event list starts it from nothing.
 *
 * An approval alert is raised only for Wardryx's own approval_requested
 * event, carries what the operator needs to find it, honours mutes by agent,
 * run and environment, and a click on it reaches whoever subscribed.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { REPLAY_SPEEDS, useReplayClock, type ReplayClock } from "./useReplayClock";
import {
  agentIdFromMuteKey,
  ensureNotificationPermission,
  extractApprovalAlert,
  isMuted,
  muteKey,
  notificationCopy,
  raiseApprovalNotification,
  registerApprovalActions,
  subscribeApprovalActionClicks,
  type ApprovalAlert,
} from "./notifications";
import type { UiEvent } from "../types";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const ev = (ts: string, over: Partial<UiEvent> = {}): UiEvent =>
  ({
    ts,
    source: "tokenfuse",
    type: "call",
    agent_id: "agent://acme/bot",
    run_id: "run-1",
    env: "prod",
    data: {},
    ...over,
  }) as UiEvent;

describe("useReplayClock", () => {
  let host: HTMLDivElement;
  let root: Root;
  let clock: ReplayClock;

  function Probe({ events }: { events: readonly UiEvent[] }) {
    clock = useReplayClock(events);
    return null;
  }

  async function mount(events: readonly UiEvent[]) {
    host = document.createElement("div");
    root = createRoot(host);
    await act(async () => root.render(createElement(Probe, { events })));
  }
  async function rerender(events: readonly UiEvent[]) {
    await act(async () => root.render(createElement(Probe, { events })));
  }
  async function wait(ms: number) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  }
  async function call(f: () => void) {
    await act(async () => f());
  }

  beforeEach(() => vi.useFakeTimers());
  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
  });

  // Gaps between consecutive events: 1 s, 10 ms, 1 h, then a timestamp that is not one.
  const RUN = [
    ev("2026-11-26T10:00:00.000Z"),
    ev("2026-11-26T10:00:01.000Z"),
    ev("2026-11-26T10:00:01.010Z"),
    ev("2026-11-26T11:00:01.010Z"),
    ev("not a time"),
  ];

  it("starts paused with nothing revealed, and offers the speeds the view shows", async () => {
    await mount(RUN);
    expect(clock.revealedCount).toBe(0);
    expect(clock.playing).toBe(false);
    expect(clock.speed).toBe(1);
    expect(clock.atEnd).toBe(false);
    expect(REPLAY_SPEEDS).toEqual([0.5, 1, 2, 4, 8]);
  });

  it("steps at the run's own pace, bounded to 150 ms and 2 s", async () => {
    await mount(RUN);
    await call(() => clock.play());
    expect(clock.playing).toBe(true);

    // The first event has no predecessor: the minimum step.
    await wait(149);
    expect(clock.revealedCount).toBe(0);
    await wait(1);
    expect(clock.revealedCount).toBe(1);

    // 1 s gap at speed 1.
    await wait(999);
    expect(clock.revealedCount).toBe(1);
    await wait(1);
    expect(clock.revealedCount).toBe(2);

    // 10 ms gap, raised to the 150 ms floor.
    await wait(149);
    expect(clock.revealedCount).toBe(2);
    await wait(1);
    expect(clock.revealedCount).toBe(3);

    // A one-hour gap is capped at 2 s.
    await wait(1_999);
    expect(clock.revealedCount).toBe(3);
    await wait(1);
    expect(clock.revealedCount).toBe(4);

    // A gap to a timestamp that is not one waits the middle of the range.
    await wait(1_074);
    expect(clock.revealedCount).toBe(4);
    await wait(1);
    expect(clock.revealedCount).toBe(5);
    expect(clock.atEnd).toBe(true);

    // At the end it stops by itself.
    await wait(10);
    expect(clock.playing).toBe(false);
  });

  it("divides the real gap by the speed", async () => {
    await mount(RUN);
    await call(() => clock.seek(1));
    await call(() => clock.setSpeed(4));
    await call(() => clock.play());
    await wait(249);
    expect(clock.revealedCount).toBe(1);
    await wait(1);
    expect(clock.revealedCount).toBe(2);
  });

  it("pauses, toggles, seeks within bounds and resets", async () => {
    await mount(RUN);
    await call(() => clock.toggle());
    expect(clock.playing).toBe(true);
    await call(() => clock.toggle());
    expect(clock.playing).toBe(false);

    await call(() => clock.play());
    await call(() => clock.pause());
    expect(clock.playing).toBe(false);
    await wait(5_000);
    expect(clock.revealedCount).toBe(0);

    await call(() => clock.seek(2.6));
    expect(clock.revealedCount).toBe(3);
    await call(() => clock.seek(99));
    expect(clock.revealedCount).toBe(5);
    expect(clock.atEnd).toBe(true);
    await call(() => clock.seek(-4));
    expect(clock.revealedCount).toBe(0);

    await call(() => clock.seek(5));
    await call(() => clock.play());
    expect(clock.playing).toBe(false);
    await call(() => clock.toggle());
    expect(clock.playing).toBe(false);

    await call(() => clock.reset());
    expect(clock.revealedCount).toBe(0);
    expect(clock.playing).toBe(false);
  });

  it("restart plays again from nothing", async () => {
    await mount(RUN);
    await call(() => clock.seek(5));
    await call(() => clock.restart());
    expect(clock.revealedCount).toBe(0);
    expect(clock.playing).toBe(true);
    await wait(150);
    expect(clock.revealedCount).toBe(1);
  });

  it("will not play an empty run, and starts over when the events change", async () => {
    await mount([]);
    await call(() => clock.play());
    expect(clock.playing).toBe(false);
    await call(() => clock.toggle());
    expect(clock.playing).toBe(false);
    expect(clock.atEnd).toBe(false);

    await rerender(RUN);
    await call(() => clock.seek(3));
    await call(() => clock.play());
    await rerender(RUN.slice(0, 2));
    expect(clock.revealedCount).toBe(0);
    expect(clock.playing).toBe(false);
  });
});

describe("approval alerts", () => {
  const request = ev("2026-11-26T10:00:00Z", {
    source: "wardryx",
    type: "approval_requested",
    data: { approval_id: "ap-7", reason: "spend above $5" },
  });

  it("are raised only for Wardryx's own approval request carrying an id", () => {
    expect(extractApprovalAlert(request)).toEqual({
      approvalId: "ap-7",
      agentId: "agent://acme/bot",
      runId: "run-1",
      env: "prod",
      reason: "spend above $5",
    });
    expect(extractApprovalAlert({ ...request, source: "tokenfuse" })).toBeNull();
    expect(extractApprovalAlert({ ...request, type: "approval_decided" })).toBeNull();
    expect(extractApprovalAlert({ ...request, data: { reason: "x" } })).toBeNull();
    expect(extractApprovalAlert({ ...request, data: { approval_id: 7 } })).toBeNull();
    expect(extractApprovalAlert({ ...request, data: null } as never)).toBeNull();
    expect(extractApprovalAlert({ ...request, data: { approval_id: "ap-8", reason: 3 } })?.reason).toBeNull();
  });

  it("are muted by agent, run or environment, and by nothing else", () => {
    const alert = extractApprovalAlert(request)!;
    expect(isMuted(alert, new Set())).toBe(false);
    expect(isMuted(alert, new Set([muteKey("agent", "agent://acme/bot")]))).toBe(true);
    expect(isMuted(alert, new Set([muteKey("run", "run-1")]))).toBe(true);
    expect(isMuted(alert, new Set([muteKey("env", "prod")]))).toBe(true);
    expect(isMuted(alert, new Set([muteKey("env", "staging"), muteKey("run", "run-2")]))).toBe(false);
    expect(isMuted({ ...alert, runId: null }, new Set([muteKey("run", "")]))).toBe(false);
  });

  it("read an agent back out of its own mute key only", () => {
    expect(muteKey("agent", "agent://a/b")).toBe("agent:agent://a/b");
    expect(agentIdFromMuteKey("agent:agent://a/b")).toBe("agent://a/b");
    expect(agentIdFromMuteKey("run:run-1")).toBeNull();
  });

  it("say what needs approving and why, or that no reason was given", () => {
    const alert = extractApprovalAlert(request)!;
    expect(notificationCopy(alert)).toEqual({ title: "Approval needed", body: "agent://acme/bot (spend above $5)" });
    expect(notificationCopy({ ...alert, reason: null }).body).toBe("agent://acme/bot (no reason given)");
  });
});

describe("the browser notification", () => {
  const alert: ApprovalAlert = { approvalId: "ap-9", agentId: "agent://a/b", runId: null, env: "prod", reason: null };
  let made: { title: string; opts: NotificationOptions; onclick: (() => void) | null }[];

  beforeEach(() => {
    made = [];
    class FakeNotification {
      static permission = "default";
      static requestPermission = vi.fn(async () => "granted");
      onclick: (() => void) | null = null;
      constructor(title: string, opts: NotificationOptions) {
        made.push(this as never);
        Object.assign(this, { title, opts });
      }
    }
    vi.stubGlobal("Notification", FakeNotification);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("raises one notification whose click reaches the current subscriber, and not after it left", async () => {
    const focus = vi.spyOn(window, "focus").mockImplementation(() => {});
    const clicked: string[] = [];
    const off = await subscribeApprovalActionClicks((id) => clicked.push(id));
    await registerApprovalActions();
    await raiseApprovalNotification(alert);
    expect(made).toHaveLength(1);
    expect((made[0] as unknown as { title: string }).title).toBe("Approval needed");
    made[0].onclick!();
    expect(focus).toHaveBeenCalled();
    expect(clicked).toEqual(["ap-9"]);

    const replaced: string[] = [];
    const offNewer = await subscribeApprovalActionClicks((id) => replaced.push(id));
    off();
    made[0].onclick!();
    expect(replaced).toEqual(["ap-9"]);
    offNewer();
    made[0].onclick!();
    expect(replaced).toEqual(["ap-9"]);
  });

  it("never throws a UI action over a notification the browser refused", async () => {
    vi.stubGlobal(
      "Notification",
      class {
        constructor() {
          throw new Error("blocked");
        }
      },
    );
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    await expect(raiseApprovalNotification(alert)).resolves.toBeUndefined();
    expect(logged).toHaveBeenCalled();
  });

  it("asks for permission once, and only while the browser has not been asked", async () => {
    await ensureNotificationPermission();
    await ensureNotificationPermission();
    const N = (globalThis as unknown as { Notification: { requestPermission: ReturnType<typeof vi.fn> } }).Notification;
    expect(N.requestPermission).toHaveBeenCalledTimes(1);
  });
});
