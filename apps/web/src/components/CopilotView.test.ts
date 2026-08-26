import { describe, expect, it, vi } from "vitest";

// `CopilotView` pulls its whole panel's imports in behind `transport`; mocking
// the module (hoisted above every import here) keeps this suite to the pure
// residency helper below, with no backend of either kind involved.
vi.mock("../lib/transport", () => ({
  hasBackend: vi.fn(() => false),
  isWebShell: vi.fn(() => false),
  invokeBackend: vi.fn(),
  subscribeBackend: vi.fn(),
  requiredRoleFromCommandError: vi.fn(() => null),
  webApiBase: vi.fn(() => ""),
}));

import type { CopilotStatus } from "../copilotTypes";
import { residencyFacts } from "./CopilotView";

/** A status as `crates/api/src/copilot/commands.rs`'s `CopilotStatusDto`
 * serialises it: `provider`, `model`, `endpoint` and `local` are `Some`
 * together exactly when a provider descriptor exists. */
function status(over: Partial<CopilotStatus> = {}): CopilotStatus {
  return {
    enabled: true,
    provider: "ollama",
    model: "qwen2.5:7b-instruct",
    endpoint: "http://127.0.0.1:11434/v1",
    local: true,
    disabled_reason: null,
    ...over,
  };
}

describe("residencyFacts", () => {
  // `endpoint` is on the wire already (`CopilotStatusDto.endpoint`,
  // crates/api/src/copilot/commands.rs) and the banner dropped it. Which
  // machine the prompts actually go to is the one fact this banner exists to
  // state, and it was the one field not rendered.
  it("shows the endpoint the box already serves", () => {
    expect(residencyFacts(status()).endpoint).toBe("http://127.0.0.1:11434/v1");
  });

  // The point of the whole track. A missing field is not a zero and not a
  // blank: the DTO can carry `endpoint: null`, and a blank cell in a
  // residency banner reads as "there is no endpoint", which is a claim
  // nobody measured.
  it("says an absent endpoint is not recorded, rather than showing nothing", () => {
    const facts = residencyFacts(status({ endpoint: null }));
    expect(facts.endpointRecorded).toBe(false);
    expect(facts.endpoint.trim()).not.toBe("");
    expect(facts.endpoint).toMatch(/not recorded/i);
  });

  // The mock preview's `copilotStatus()` omits `endpoint` entirely rather than
  // sending null, so the absent case arrives as `undefined` in the one build
  // most people will ever see. It must not render as the string "undefined".
  it("treats an omitted endpoint the same as a null one", () => {
    const facts = residencyFacts(status({ endpoint: undefined as unknown as null }));
    expect(facts.endpointRecorded).toBe(false);
    expect(facts.endpoint).not.toContain("undefined");
  });

  // Never a plausible-looking default. `FelyxConnectCard`'s provider table
  // carries exactly this string as Ollama's default base URL, and reaching for
  // it here would put a real-looking address under a field nobody measured.
  it("never fills an absent endpoint with a provider default", () => {
    const facts = residencyFacts(status({ endpoint: null }));
    expect(facts.endpoint).not.toContain("127.0.0.1");
  });

  // The remote branch read `Remote: ${provider} (BYO key)` and dropped the
  // model, so an operator on a cloud provider could not see WHICH model was
  // answering, though the box serves it in the same response.
  it("names the model on a remote provider too, not only a local one", () => {
    const facts = residencyFacts(
      status({ local: false, provider: "anthropic", model: "claude-haiku-4-5", endpoint: "https://api.anthropic.com" }),
    );
    expect(facts.headline).toContain("claude-haiku-4-5");
    expect(facts.headline).toContain("anthropic");
    expect(facts.local).toBe(false);
  });

  it("still says a local provider keeps the prompts on this machine", () => {
    const facts = residencyFacts(status());
    expect(facts.local).toBe(true);
    expect(facts.headline).toContain("qwen2.5:7b-instruct");
    expect(facts.headline).toMatch(/this machine/i);
  });

  // `provider`/`model` are nullable on the same DTO. The banner already had a
  // fallback for those; it must stay a stated unknown, never a blank.
  it("keeps an unknown provider or model stated rather than blank", () => {
    const facts = residencyFacts(status({ provider: null, model: null }));
    expect(facts.headline).toMatch(/unknown/i);
  });
});
