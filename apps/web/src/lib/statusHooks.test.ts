// @vitest-environment jsdom
/**
 * The eleven per-panel status hooks (`use*Status`) share one contract: fetch
 * the panel's status once on mount, re-poll every 3s while the backend says
 * it is still bootstrapping, give up after 20 polls so a stuck backend does
 * not poll forever (the last status stays on screen), settle the moment the
 * state is anything else, and stop polling when the panel unmounts. A hook
 * that re-polled a settled backend would hammer it every 3s for as long as
 * the console is open; one that stopped early would leave a panel saying
 * "connecting" for good.
 *
 * Each hook is driven through its real plane module (`lib/money.ts`, ...) with
 * only the transport faked, so the command name each one sends and its
 * error fallback are part of what is checked.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const backend = vi.hoisted(() => ({
  calls: [] as string[],
  answer: (_command: string): unknown => ({ state: "ready" }),
  present: true,
}));

vi.mock("./transport", () => ({
  hasBackend: () => backend.present,
  isWebShell: () => backend.present,
  webApiBase: () => "",
  subscribeBackend: () => () => {},
  requiredRoleFromCommandError: () => null,
  invokeBackend: async (command: string) => {
    backend.calls.push(command);
    const a = backend.answer(command);
    if (a instanceof Error) throw a;
    return a;
  },
}));

import { useAdmissionStatus } from "./useAdmissionStatus";
import { useCredentialsStatus } from "./useCredentialsStatus";
import { useCryptoStatus } from "./useCryptoStatus";
import { useDrillsStatus } from "./useDrillsStatus";
import { useEvidenceStatus } from "./useEvidenceStatus";
import { useIdentityStatus } from "./useIdentityStatus";
import { useMemoryStatus } from "./useMemoryStatus";
import { useMoneyStatus } from "./useMoneyStatus";
import { usePolicyStatus } from "./usePolicyStatus";
import { useQualityStatus } from "./useQualityStatus";
import { useRemoteStatus } from "./useRemoteStatus";

type Hook = () => unknown;

/** `wrap` turns a plain `{state}` into the hook's own status shape: the
 * admission hook polls on `gateway.state`, every other one on `state`. */
const HOOKS: { name: string; hook: Hook; command: string; wrap: (state: string) => unknown; stateOf: (s: unknown) => string }[] = [
  {
    name: "useAdmissionStatus",
    hook: useAdmissionStatus,
    command: "admission_status",
    wrap: (state) => ({ gateway: { state }, verdryx_bin: "", verdryx_bin_present: false, verdryx_db: null, drills_scenario_dir: null }),
    stateOf: (s) => (s as { gateway: { state: string } }).gateway.state,
  },
  ...(
    [
      ["useCredentialsStatus", useCredentialsStatus, "credentials_status"],
      ["useCryptoStatus", useCryptoStatus, "crypto_status"],
      ["useDrillsStatus", useDrillsStatus, "drills_status"],
      ["useEvidenceStatus", useEvidenceStatus, "evidence_status"],
      ["useIdentityStatus", useIdentityStatus, "identity_status"],
      ["useMemoryStatus", useMemoryStatus, "memory_status"],
      ["useMoneyStatus", useMoneyStatus, "money_status"],
      ["usePolicyStatus", usePolicyStatus, "policy_status"],
      ["useQualityStatus", useQualityStatus, "quality_status"],
      ["useRemoteStatus", useRemoteStatus, "remote_status"],
    ] as [string, Hook, string][]
  ).map(([name, hook, command]) => ({
    name,
    hook,
    command,
    wrap: (state: string) => ({ state }),
    stateOf: (s: unknown) => (s as { state: string }).state,
  })),
];

let host: HTMLDivElement | undefined;
let root: Root | undefined;
let seen: unknown[] = [];

async function flush() {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });
}

async function mount(hook: Hook) {
  function Probe() {
    seen.push(hook());
    return null;
  }
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root!.render(createElement(Probe));
  });
  await flush();
}

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
  await flush();
}

const latest = () => seen[seen.length - 1];

beforeEach(() => {
  vi.useFakeTimers();
  backend.calls = [];
  backend.present = true;
  seen = [];
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
  vi.useRealTimers();
});

describe.each(HOOKS)("$name", ({ hook, command, wrap, stateOf }) => {
  it("starts empty, fetches once, and settles on a non-bootstrapping state", async () => {
    backend.answer = () => wrap("ready");
    await mount(hook);
    expect(seen[0]).toBeNull();
    expect(backend.calls).toEqual([command]);
    expect(stateOf(latest())).toBe("ready");

    await advance(60_000);
    expect(backend.calls).toHaveLength(1);
  });

  it("re-polls every 3s while bootstrapping, then stops once it settles", async () => {
    let n = 0;
    backend.answer = () => wrap(++n < 3 ? "bootstrapping" : "ready");
    await mount(hook);
    expect(backend.calls).toHaveLength(1);
    expect(stateOf(latest())).toBe("bootstrapping");

    await advance(2_999);
    expect(backend.calls).toHaveLength(1);
    await advance(1);
    expect(backend.calls).toHaveLength(2);
    await advance(3_000);
    expect(backend.calls).toHaveLength(3);
    expect(stateOf(latest())).toBe("ready");

    await advance(30_000);
    expect(backend.calls).toHaveLength(3);
  });

  it("gives up after 20 polls and leaves the last status on screen", async () => {
    backend.answer = () => wrap("bootstrapping");
    await mount(hook);
    await advance(3_000 * 30);
    expect(backend.calls).toHaveLength(20);
    expect(stateOf(latest())).toBe("bootstrapping");
  });

  it("stops polling when the panel unmounts", async () => {
    backend.answer = () => wrap("bootstrapping");
    await mount(hook);
    act(() => root!.unmount());
    root = undefined;
    await advance(30_000);
    expect(backend.calls).toHaveLength(1);
  });

  it("drops an answer that lands after the panel unmounted", async () => {
    let release!: (v: unknown) => void;
    backend.answer = () => new Promise((r) => (release = r));
    await mount(hook);
    const rendersBefore = seen.length;
    act(() => root!.unmount());
    root = undefined;
    release(wrap("bootstrapping"));
    await flush();
    await advance(30_000);
    expect(seen.length).toBe(rendersBefore);
    expect(backend.calls).toHaveLength(1);
  });

  it("turns a failed fetch into a settled status instead of polling", async () => {
    backend.answer = () => new Error("connection refused");
    await mount(hook);
    const s = latest();
    expect(s).not.toBeNull();
    expect(stateOf(s)).not.toBe("bootstrapping");
    await advance(30_000);
    expect(backend.calls).toHaveLength(1);
  });

  it("asks nothing when there is no backend at all", async () => {
    backend.present = false;
    await mount(hook);
    expect(backend.calls).toEqual([]);
    expect(latest()).not.toBeNull();
    expect(stateOf(latest())).not.toBe("bootstrapping");
  });
});
