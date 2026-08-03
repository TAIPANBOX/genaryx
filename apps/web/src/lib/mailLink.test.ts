import { describe, expect, it } from "vitest";
import { mailLinkFrom, mailLinkNotice, parseMailLink } from "./mailLink";

describe("parseMailLink", () => {
  it("resolves the link the notifier actually builds", () => {
    const link = parseMailLink("/i/budget_threshold:run-42");
    expect(link).toEqual({
      kind: "incident",
      id: "budget_threshold:run-42",
      type: "budget_threshold",
      subject: "run-42",
      view: "money",
    });
  });

  it("is not a mail link for every other path in this app", () => {
    for (const path of ["/", "/index.html", "/auth/session", "/incidents/x", "/ii/x"]) {
      expect(parseMailLink(path)).toBeNull();
    }
  });

  it("splits on the FIRST colon, because an agent id contains one", () => {
    // An org-scoped event has no run, so the notifier falls back to the agent,
    // and `agent://acme.example/biller` would become three pieces under a
    // naive split.
    const link = parseMailLink("/i/spend_spike:agent:%2F%2Facme.example%2Fbiller");
    expect(link?.type).toBe("spend_spike");
    expect(link?.subject).toBe("agent://acme.example/biller");
  });

  it("decodes what the mail encoded", () => {
    const link = parseMailLink(`/i/${encodeURIComponent("run_killed:run/42 a")}`);
    expect(link?.subject).toBe("run/42 a");
  });

  it("survives a malformed escape rather than throwing on the boot path", () => {
    const link = parseMailLink("/i/run_killed:100%");
    expect(link?.type).toBe("run_killed");
    expect(link?.subject).toBe("100%");
  });

  it("tolerates a trailing slash, which link rewriters add", () => {
    expect(parseMailLink("/i/run_killed:run-1/")?.subject).toBe("run-1");
  });

  it("is null for an empty id rather than opening a panel about nothing", () => {
    expect(parseMailLink("/i/")).toBeNull();
    expect(parseMailLink("/i//")).toBeNull();
  });

  it("routes each producing plane's types to its own panel", () => {
    const cases: readonly [string, string][] = [
      ["budget_exhausted:run-1", "money"],
      ["run_killed:run-1", "money"],
      ["policy_deny:run-1", "policy"],
      ["approval_requested:run-1", "policy"],
      ["behavior_anomaly:agent-1", "identity"],
      ["quality_drift:agent-1", "quality"],
      ["sim_finding:drill-1", "drills"],
      ["crypto_drift:asset-1", "crypto"],
      ["contradiction_found:mem-1", "memory"],
    ];
    for (const [id, view] of cases) {
      expect(parseMailLink(`/i/${id}`)?.view).toBe(view);
    }
  });

  // The invariant: a console built before a plane started emitting a type must
  // not land the operator somewhere confidently wrong.
  it("does not guess a panel for a type it has never heard of", () => {
    const link = parseMailLink("/i/invented_by_a_future_plane:run-9");
    expect(link?.type).toBe("invented_by_a_future_plane");
    expect(link?.view).toBeNull();
  });

  it("handles an id with no subject at all", () => {
    const link = parseMailLink("/i/spend_spike");
    expect(link).toEqual({ kind: "incident", id: "spend_spike", type: "spend_spike", subject: "", view: "money" });
  });
});

// The other two coordinates an alert carries. An operator at two in the morning
// wants one of three things, and the mail offers all three rather than making
// them navigate: what happened, the agent itself (where freeze and kill are),
// and who is answerable for it.
describe("the agent and owner links", () => {
  it("opens an agent by id, and the id can contain slashes and colons", () => {
    const link = parseMailLink(`/a/${encodeURIComponent("agent://acme.example/biller")}`);
    expect(link?.kind).toBe("agent");
    expect(link?.subject).toBe("agent://acme.example/biller");
    expect(link?.view).toBe("overview");
  });

  it("opens an owner, which is the passport field and not the delegation chain", () => {
    const link = parseMailLink(`/o/${encodeURIComponent("team-finance@acme.example")}`);
    expect(link?.kind).toBe("owner");
    expect(link?.subject).toBe("team-finance@acme.example");
    expect(link?.view).toBe("identity");
  });

  it("says what each one landed on, in words an operator can act on", () => {
    const agent = parseMailLink("/a/billing-agent")!;
    expect(mailLinkNotice(agent)).toContain("freeze or kill it there, not from the mail");
    const owner = parseMailLink("/o/team-finance")!;
    expect(mailLinkNotice(owner)).toContain("everything they are answerable for");
  });

  it("is still null for an empty id on either", () => {
    expect(parseMailLink("/a/")).toBeNull();
    expect(parseMailLink("/o/")).toBeNull();
  });
});

describe("mailLinkNotice", () => {
  it("names what the mail was about", () => {
    const link = parseMailLink("/i/budget_threshold:run-42")!;
    expect(mailLinkNotice(link)).toBe("Opened from an alert about budget_threshold on run-42.");
  });

  it("says plainly that it could not place an unknown type, and shows the id", () => {
    const link = parseMailLink("/i/invented:run-9")!;
    const notice = mailLinkNotice(link);
    expect(notice).toContain("does not know which panel");
    expect(notice).toContain("invented:run-9");
  });

  it("omits the subject when there is none", () => {
    const link = parseMailLink("/i/spend_spike")!;
    expect(mailLinkNotice(link)).toBe("Opened from an alert about spend_spike.");
  });
});

describe("mailLinkFrom", () => {
  // A static host answers 404 for `/i/...` because there is no such file, so
  // the click dies before any of this runs. The fragment is what a static host
  // always serves as the page itself, and never sends upstream.
  it("reads the link out of the fragment when the path is not one", () => {
    const link = mailLinkFrom({ pathname: "/demo/", hash: "#/i/budget_exhausted:run-42" });
    expect(link?.kind).toBe("incident");
    expect(link?.type).toBe("budget_exhausted");
    expect(link?.subject).toBe("run-42");
    expect(link?.view).toBe("money");
  });

  it("takes the path over the fragment when both are links", () => {
    // A real console owns its routes, so its own path is the more specific
    // statement of intent than something a link rewriter may have appended.
    const link = mailLinkFrom({ pathname: "/a/billing-agent", hash: "#/i/policy_deny:run-9" });
    expect(link?.kind).toBe("agent");
    expect(link?.subject).toBe("billing-agent");
  });

  it("is not a link when neither the path nor the fragment is one", () => {
    expect(mailLinkFrom({ pathname: "/demo/", hash: "" })).toBeNull();
    expect(mailLinkFrom({ pathname: "/demo/", hash: "#" })).toBeNull();
    expect(mailLinkFrom({ pathname: "/", hash: "#section-two" })).toBeNull();
  });

  it("handles a fragment carrying an owner link, escaping and all", () => {
    const link = mailLinkFrom({ pathname: "/demo/", hash: "#/o/team-finance%40acme.example" });
    expect(link?.kind).toBe("owner");
    expect(link?.id).toBe("team-finance@acme.example");
    expect(link?.view).toBe("identity");
  });
});
