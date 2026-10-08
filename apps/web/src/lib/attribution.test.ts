/**
 * The TypeScript copy of `genaryx_core::attribution`, held to the same cases
 * the Rust tests use: an identity refusal is filed under the key that made
 * the call, never under the agent it claimed.
 */
import { describe, expect, it } from "vitest";
import { claimSentence, filedUnder, isKeySubject, refusedKey } from "./attribution";

const flint = "agent://taipanbox.dev/routers/flint";

describe("attribution", () => {
  it("an_identity_refusal_is_filed_under_its_key_and_every_other_event_under_its_agent", () => {
    const data = { key_id: "forge-imposter", agent_id: flint };
    expect(filedUnder({ type: "identity_mismatch", agent_id: flint, data })).toBe("key:forge-imposter");
    expect(refusedKey({ type: "identity_mismatch", data })).toBe("forge-imposter");
    expect(filedUnder({ type: "policy_deny", agent_id: flint, data })).toBe(flint);
    expect(refusedKey({ type: "policy_deny", data })).toBeNull();
    expect(claimSentence({ type: "identity_mismatch", agent_id: flint, data })).toBe(
      `claimed ${flint} with key forge-imposter`,
    );
    expect(claimSentence({ type: "policy_deny", agent_id: flint, data })).toBeNull();
  });

  it("an_identity_refusal_with_no_readable_key_names_no_agent", () => {
    for (const data of [null, {}, { key_id: null }, { key_id: "" }, { key_id: 42 }, { key_id: { k: "v" } }, "text", [1]]) {
      const got = filedUnder({ type: "identity_mismatch", agent_id: flint, data });
      expect(got).toBe("key:(none)");
      expect(got).not.toContain("flint");
    }
  });

  it("a_key_named_like_an_agent_is_still_a_key", () => {
    const got = filedUnder({ type: "identity_mismatch", agent_id: flint, data: { key_id: flint } });
    expect(got).toBe(`key:${flint}`);
    expect(isKeySubject(got)).toBe(true);
    expect(isKeySubject(flint)).toBe(false);
  });
});

/**
 * Agent 360's event list reads the bus by envelope `agent_id`, so an identity
 * refusal that CLAIMED this agent lands on its feed. It may stay there, since
 * someone trying to be this agent is worth seeing, but only as a claim by the
 * key, never as the agent's own activity.
 */
describe("an identity refusal on the claimed agent's own feed", () => {
  it("an_identity_refusal_on_the_claimed_agents_feed_reads_as_a_claim_by_the_key", async () => {
    const { agentFeedLine } = (await import("./attribution")) as unknown as {
      agentFeedLine: (e: { type: string; agent_id: string; data: unknown }, viewer: string) => { text: string; own: boolean };
    };
    const refusal = { type: "identity_mismatch", agent_id: flint, data: { key_id: "forge-imposter", agent_id: flint } };
    const line = agentFeedLine(refusal, flint);
    expect(line.own).toBe(false);
    expect(line.text).toBe("identity_mismatch: claimed by key forge-imposter, refused");
    const keyless = agentFeedLine({ ...refusal, data: { key_id: null } }, flint);
    expect(keyless.text).toBe("identity_mismatch: claimed with no key, refused");
    expect(keyless.own).toBe(false);
  });

  it("every_other_event_on_the_feed_is_the_agents_own_and_reads_as_its_type", async () => {
    const { agentFeedLine } = (await import("./attribution")) as unknown as {
      agentFeedLine: (e: { type: string; agent_id: string; data: unknown }, viewer: string) => { text: string; own: boolean };
    };
    const line = agentFeedLine({ type: "policy_deny", agent_id: flint, data: {} }, flint);
    expect(line).toEqual({ text: "policy_deny", own: true });
  });
});
