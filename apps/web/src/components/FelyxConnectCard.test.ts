import { beforeEach, describe, expect, it, vi } from "vitest";

// The card reaches the box through `transport`. Mocking the module (hoisted
// above every import here) is what lets a REAL box, the mock preview and a
// bare no-backend build all be driven from one node-environment suite, which
// is the whole subject of this file.
vi.mock("../lib/transport", () => ({
  hasBackend: vi.fn(),
  isWebShell: vi.fn(),
  invokeBackend: vi.fn(),
}));

import { hasBackend, isWebShell } from "../lib/transport";
import { PROVIDERS, felyxConnectSupport, planConnect, refusalText } from "./FelyxConnectCard";

const mockHasBackend = vi.mocked(hasBackend);
const mockIsWebShell = vi.mocked(isWebShell);

/** A genaryx-web box: a configured web API, no mock transport. */
function realBox() {
  mockHasBackend.mockReturnValue(true);
  mockIsWebShell.mockReturnValue(true);
}

/** The mock preview (`--mode mock`): a backend, but not a web shell. */
function mockPreview() {
  mockHasBackend.mockReturnValue(true);
  mockIsWebShell.mockReturnValue(false);
}

/** A bare `vite preview`: no backend of either kind. */
function noBackend() {
  mockHasBackend.mockReturnValue(false);
  mockIsWebShell.mockReturnValue(false);
}

const ollama = PROVIDERS.find((p) => p.id === "ollama")!;
const anthropic = PROVIDERS.find((p) => p.id === "anthropic")!;

beforeEach(() => {
  vi.resetAllMocks();
});

describe("felyxConnectSupport", () => {
  // The gap this suite exists for. `copilot_connect` is named in exactly two
  // places in this repository: this card, and `lib/mockPreview.ts`'s own
  // switch. There is no handler in `crates/web/src/dispatch.rs`, so a real box
  // answers 404 "unknown command" from the dispatch fallback, and a non-admin
  // never gets that far: `crates/web/src/roles.rs`'s `required_role` fails
  // closed to Admin for an unclassified name, so the chokepoint answers 403
  // first. Either way the console offered a form for something that cannot
  // happen here.
  it("says a real box cannot connect Felyx", () => {
    realBox();
    const support = felyxConnectSupport();
    expect(support.supported).toBe(false);
  });

  // "not supported here" and "not built yet" are different statements, and the
  // card said the second one. The provider is configured, it is just not
  // configured from a browser: `crates/api/src/copilot/state.rs`'s
  // `config_from_env` reads the whole surface off the process environment.
  // The refusal has to point there, or the operator is told to wait for
  // something that already exists.
  it("points at where the provider actually is configured, not at a missing feature", () => {
    realBox();
    const support = felyxConnectSupport();
    if (support.supported) throw new Error("expected a refusal");
    const said = `${support.reason} ${support.detail}`;
    expect(said).toContain("GENARYX_COPILOT_PROVIDER");
    expect(said).not.toMatch(/\byet\b/);
  });

  // `crates/copilot/src/config.rs`: "A pointer to a secret, resolved at use,
  // never stored in the config value." The box holds `api_key_ref`, an `env:`
  // or `file:` pointer it resolves at the moment of use. The card asked for
  // the key itself, which that design has no place to put.
  it("says the box holds a reference to the key, never the key", () => {
    realBox();
    const support = felyxConnectSupport();
    if (support.supported) throw new Error("expected a refusal");
    expect(support.detail).toContain("GENARYX_COPILOT_API_KEY_REF");
  });

  it("offers the form in the mock preview, which is the one backend that answers it", () => {
    mockPreview();
    expect(felyxConnectSupport().supported).toBe(true);
  });

  it("refuses with no backend at all, where there is nothing to configure", () => {
    noBackend();
    expect(felyxConnectSupport().supported).toBe(false);
  });
});

describe("planConnect", () => {
  // The sharpest form of the gap. Pressing Connect on a real box did not fail
  // before the request: `invokeBackend` POSTs the whole body, API key included,
  // to `/api/command/copilot_connect`, and the refusal (403 from the role gate,
  // or 404 from dispatch) happens on the box AFTER the body has crossed the
  // wire. A form that cannot succeed must not send the secret first.
  it("never sends the key to a box that cannot accept it", () => {
    realBox();
    const plan = planConnect(
      { provider: anthropic, model: "claude-haiku-4-5", baseUrl: "", apiKey: "sk-live-do-not-send", maxUsd: "5" },
      felyxConnectSupport(),
    );
    expect(plan.send).toBe(false);
    expect(JSON.stringify(plan)).not.toContain("sk-live-do-not-send");
  });

  it("sends the same wire shape the preview backend reads", () => {
    mockPreview();
    const plan = planConnect(
      { provider: ollama, model: "qwen2.5:7b-instruct", baseUrl: "http://127.0.0.1:11434/v1", apiKey: "", maxUsd: "5" },
      felyxConnectSupport(),
    );
    if (!plan.send) throw new Error(`expected a send, got: ${plan.error}`);
    expect(plan.args).toEqual({
      provider: "ollama",
      model: "qwen2.5:7b-instruct",
      base_url: "http://127.0.0.1:11434/v1",
      api_key: null,
      allow_non_local_endpoints: false,
      max_usd_per_day: 5,
      local: true,
    });
  });

  it("keeps the two form checks the card already made", () => {
    mockPreview();
    const support = felyxConnectSupport();
    const blankModel = planConnect(
      { provider: ollama, model: "   ", baseUrl: "", apiKey: "", maxUsd: "5" },
      support,
    );
    expect(blankModel.send).toBe(false);
    const noKey = planConnect(
      { provider: anthropic, model: "claude-haiku-4-5", baseUrl: "", apiKey: " ", maxUsd: "5" },
      support,
    );
    expect(noKey.send).toBe(false);
  });

  // A cap the operator cannot read back is worse than no cap, so an
  // unparseable one falls back to the card's own stated default rather than
  // riding to the backend as NaN.
  it("falls back to the stated default when the cap is not a number", () => {
    mockPreview();
    const plan = planConnect(
      { provider: ollama, model: "qwen2.5:7b-instruct", baseUrl: "", apiKey: "", maxUsd: "not a number" },
      felyxConnectSupport(),
    );
    if (!plan.send) throw new Error(`expected a send, got: ${plan.error}`);
    expect(plan.args.max_usd_per_day).toBe(5);
  });
});

describe("refusalText", () => {
  // `invokeBackend` rejects with the box's OWN structured error body, not an
  // `Error`, for every non-2xx. The card's catch tested `instanceof Error`
  // first, so a real 403 or 404 fell through to a hardcoded guess and the
  // operator never saw the two words the box actually said.
  it("renders the reason the box gave, not a guess about it", () => {
    expect(refusalText({ error: "unknown command", command: "copilot_connect" })).toContain("unknown command");
    expect(refusalText({ error: "role admin required" })).toContain("role admin required");
  });

  // "may not support connecting Felyx yet" is a guess, and it is the wrong
  // one: a box's provider is configurable, from its process environment. A
  // refusal must not tell the operator to wait for something that exists.
  it("never guesses that the box may support this later", () => {
    for (const refusal of [{ error: "unknown command" }, new Error("network down"), "unparseable"]) {
      expect(refusalText(refusal)).not.toMatch(/\byet\b/);
    }
  });

  // The whole point of the catch: the operator must know the key did not land
  // somewhere. "Could not save" left that open.
  it("says nothing was saved", () => {
    expect(refusalText({ error: "unknown command" }).toLowerCase()).toContain("nothing was saved");
  });
});
