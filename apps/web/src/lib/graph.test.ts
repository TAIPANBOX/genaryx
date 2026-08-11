import { describe, expect, it } from "vitest";
import { isUserId, shortAgentLabel } from "./graph";

/**
 * The delegation chain carries PEOPLE as well as agents, and every surface that
 * lets you click a principal will be handed one.
 *
 * Until 2026-08-11 none of them checked. `AppShell`'s `onOpenAgent` was the
 * funnel for the delegation chips, the mini graph inside Agent 360 and the
 * standalone Graph view, and it opened an Agent 360 for whatever it was given.
 * Clicking `n.foster` produced an agent card about a person: "this agent has
 * never been seen on the delegation graph", "no idryx identity record for this
 * agent", "no runs for this agent yet". Every sentence true, all of them
 * nonsense.
 *
 * What made it survive is the pair asserted together below: `shortAgentLabel`
 * ALREADY understood `user://` and rendered "n.foster" correctly, so the chip
 * looked like a working chip. The label knew and the click did not, and a
 * screenshot of the chip showed nothing wrong.
 */
describe("principal ids", () => {
  it("tells a person from an agent", () => {
    expect(isUserId("user://meridian.io/n.foster")).toBe(true);
    expect(isUserId("agent://meridian.io/finops/unit-economics-analyst")).toBe(false);
    // Neither scheme. Falls to the agent side, which is where the old
    // behaviour sent everything, so an unrecognized shape is no worse off than
    // it was and is never mistaken for a person.
    expect(isUserId("mcp://meridian.io/sanctioned/observability-connector")).toBe(false);
    expect(isUserId("")).toBe(false);
    // Not a prefix match on the word: only the scheme counts.
    expect(isUserId("agent://meridian.io/user/bot")).toBe(false);
  });

  it("labels a person the same way it labels an agent, which is why this went unseen", () => {
    expect(shortAgentLabel("user://meridian.io/n.foster")).toBe("n.foster");
    expect(shortAgentLabel("agent://meridian.io/finops/unit-economics-analyst")).toBe(
      "finops/unit-economics-analyst",
    );
    // The two together: a chip whose LABEL is right and whose ROUTE was wrong
    // is a chip that looks correct in every screenshot.
    const person = "user://meridian.io/n.foster";
    expect(shortAgentLabel(person)).not.toBe(person);
    expect(isUserId(person)).toBe(true);
  });
});
