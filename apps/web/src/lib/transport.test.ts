// @vitest-environment jsdom
/**
 * `lib/transport.ts` is the one place the console reaches its box. What it
 * promises every plane module: a command is one POST to `/command/<name>` with
 * the session cookie; a non-2xx rejects with the box's own error body, never a
 * wrapper (so each plane's normaliser keeps working); a body that is not JSON
 * is handed on as text; a preview-only command never leaves the browser; and
 * the live stream is ONE EventSource shared by every subscriber, closed when
 * the last one leaves, with frames handed only to their own event name.
 *
 * `VITE_GENARYX_API` is read when the module loads, so each case loads a fresh
 * copy with the environment it needs.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./mockPreview", () => ({
  MOCK: false,
  mockInvoke: async () => {
    throw new Error("the mock transport must not be reached");
  },
  mockSubscribe: async () => {
    throw new Error("the mock transport must not be reached");
  },
}));

type Transport = typeof import("./transport");

async function load(apiBase: string | undefined): Promise<Transport> {
  vi.resetModules();
  if (apiBase === undefined) vi.stubEnv("VITE_GENARYX_API", "");
  else vi.stubEnv("VITE_GENARYX_API", apiBase);
  return await import("./transport");
}

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("with no box configured", () => {
  it("says there is no backend and refuses a command naming the setting", async () => {
    const t = await load(undefined);
    expect(t.hasBackend()).toBe(false);
    expect(t.isWebShell()).toBe(false);
    expect(t.webApiBase()).toBe("");
    await expect(t.invokeBackend("money_status")).rejects.toThrow(/VITE_GENARYX_API/);
  });

  it("hands a subscriber a no-op unsubscribe instead of leaving it waiting", async () => {
    const t = await load(undefined);
    const off = await t.subscribeBackend("bus:event", () => {});
    expect(() => off()).not.toThrow();
  });
});

describe("invokeBackend against a box", () => {
  let calls: { url: string; init: RequestInit }[];
  let reply: { status: number; body: string };

  beforeEach(() => {
    calls = [];
    reply = { status: 200, body: "{}" };
    vi.stubGlobal("fetch", async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(reply.body, { status: reply.status });
    });
  });

  it("POSTs the command with its args, the session cookie and any extra header", async () => {
    const t = await load("/api");
    expect(t.hasBackend()).toBe(true);
    expect(t.isWebShell()).toBe(true);
    expect(t.webApiBase()).toBe("/api");
    reply = { status: 200, body: '{"state":"ready"}' };

    const out = await t.invokeBackend("money_status", { env: "prod" }, { "x-genaryx-webauthn": "assert" });
    expect(out).toEqual({ state: "ready" });
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe("/api/command/money_status");
    expect(calls[0].init.method).toBe("POST");
    expect(calls[0].init.credentials).toBe("include");
    expect(calls[0].init.body).toBe('{"env":"prod"}');
    expect(calls[0].init.headers).toEqual({ "content-type": "application/json", "x-genaryx-webauthn": "assert" });
  });

  it("sends an empty object when a command takes no args, and reads an empty body as null", async () => {
    const t = await load("/api");
    reply = { status: 200, body: "" };
    expect(await t.invokeBackend("bus_status")).toBeNull();
    expect(calls[0].init.body).toBe("{}");
  });

  it("rejects a non-2xx with the box's structured error itself, not a wrapper", async () => {
    const t = await load("/api");
    reply = { status: 403, body: '{"error":"role admin required"}' };
    await expect(t.invokeBackend("money_set_budget")).rejects.toEqual({ error: "role admin required" });
  });

  it("hands a body that is not JSON on as its raw text", async () => {
    const t = await load("/api");
    reply = { status: 502, body: "<html>Bad Gateway</html>" };
    await expect(t.invokeBackend("money_status")).rejects.toBe("<html>Bad Gateway</html>");
    reply = { status: 200, body: "plain words" };
    expect(await t.invokeBackend("money_status")).toBe("plain words");
  });

  it("never sends a preview-only command to the box", async () => {
    const t = await load("/api");
    await expect(t.invokeBackend("agent_record", { agent_id: "agent://x/y" })).rejects.toEqual({
      error: "preview_only",
      command: "agent_record",
    });
    expect(calls).toHaveLength(0);
  });
});

describe("requiredRoleFromCommandError", () => {
  it("recognises only the role gate's own refusal", async () => {
    const t = await load("/api");
    expect(t.requiredRoleFromCommandError({ error: "role admin required" })).toBe("admin");
    expect(t.requiredRoleFromCommandError({ error: "role approver required" })).toBe("approver");
    expect(t.requiredRoleFromCommandError({ error: "role viewer required" })).toBe("viewer");
  });

  it("is not fooled by a plane error, another message, or something that is not an object", async () => {
    const t = await load("/api");
    for (const err of [
      null,
      undefined,
      "role admin required",
      42,
      { kind: "role_required", error: "role admin required" },
      { error: "role root required" },
      { error: "the role admin required" },
      { error: 403 },
      {},
    ]) {
      expect(t.requiredRoleFromCommandError(err)).toBeNull();
    }
  });
});

class FakeEventSource {
  static instances: FakeEventSource[] = [];
  listeners = new Map<string, ((e: MessageEvent<string>) => void)[]>();
  closed = false;
  constructor(
    public url: string,
    public init?: EventSourceInit,
  ) {
    FakeEventSource.instances.push(this);
  }
  addEventListener(name: string, fn: (e: MessageEvent<string>) => void) {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), fn]);
  }
  close() {
    this.closed = true;
  }
  emit(name: string, data: string) {
    for (const fn of this.listeners.get(name) ?? []) fn({ data } as MessageEvent<string>);
  }
}

describe("subscribeBackend against a box", () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
  });

  it("shares one EventSource across every name and every subscriber", async () => {
    const t = await load("/api");
    const bus: unknown[] = [];
    const tail: unknown[] = [];
    const bus2: unknown[] = [];
    const offBus = await t.subscribeBackend("bus:event", (p) => bus.push(p));
    const offBus2 = await t.subscribeBackend("bus:event", (p) => bus2.push(p));
    const offTail = await t.subscribeBackend("remote:tail-line", (p) => tail.push(p));

    expect(FakeEventSource.instances).toHaveLength(1);
    const es = FakeEventSource.instances[0];
    expect(es.url).toBe("/api/events");
    expect(es.init).toEqual({ withCredentials: true });
    // The bus rides the legacy "bus" frame name; one listener per name.
    expect(es.listeners.get("bus")).toHaveLength(1);
    expect(es.listeners.get("remote:tail-line")).toHaveLength(1);

    es.emit("bus", '{"id":1}');
    es.emit("remote:tail-line", '{"line":"x"}');
    expect(bus).toEqual([{ id: 1 }]);
    expect(bus2).toEqual([{ id: 1 }]);
    expect(tail).toEqual([{ line: "x" }]);

    offBus();
    offTail();
    expect(es.closed).toBe(false);
    offBus2();
    expect(es.closed).toBe(true);
  });

  it("drops a frame it cannot parse instead of handing it on", async () => {
    const t = await load("/api");
    const got: unknown[] = [];
    const off = await t.subscribeBackend("bus:event", (p) => got.push(p));
    FakeEventSource.instances[0].emit("bus", "{not json");
    expect(got).toEqual([]);
    off();
  });

  it("opens a fresh connection after the last subscriber left, without double-binding", async () => {
    const t = await load("/api");
    const off1 = await t.subscribeBackend("bus:event", () => {});
    const off2 = await t.subscribeBackend("remote:tail-ended", () => {});
    off1();
    // An empty set for one name stays; the connection is still live.
    const again: unknown[] = [];
    const off3 = await t.subscribeBackend("bus:event", (p) => again.push(p));
    const es = FakeEventSource.instances[0];
    expect(es.listeners.get("bus")).toHaveLength(1);
    es.emit("bus", "1");
    expect(again).toEqual([1]);
    off2();
    off3();
    expect(es.closed).toBe(true);

    const off4 = await t.subscribeBackend("bus:event", () => {});
    expect(FakeEventSource.instances).toHaveLength(2);
    off4();
  });
});
