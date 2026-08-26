/** The top-level views the app shell switches between. */
export type ViewId =
  | "overview"
  | "anomalies"
  | "stats"
  | "money"
  | "policy"
  | "identity"
  | "onboard"
  | "quality"
  | "crypto"
  | "memory"
  | "drills"
  | "evidence"
  | "remote"
  | "graph"
  | "replay"
  | "posture"
  | "bus"
  | "egress"
  | "routines"
  | "copilot";

/** The left rail is grouped by how an operator actually works, most-used at
 * the top and rare setup at the bottom (Yurii, 2026-07-24): Operate is the
 * daily governance surface you land on; Investigate is where you go when
 * something looks off; Assure is the periodic compliance and drill work; Set
 * up is the rare one-time wiring (registering an agent, pointing at a box). */
export const NAV_SECTIONS: readonly {
  label: string;
  items: readonly { id: ViewId; label: string }[];
}[] = [
  {
    label: "Operate",
    items: [
      { id: "overview", label: "Overview" },
      // Beside Overview because it answers the same question at a different
      // depth. Overview's Incident Center is a ten-row summary and a capped
      // card is where an operator stops rather than where they start; this is
      // the whole stream, filterable, with every row opening the record behind
      // it. `@yurii` 2026-08-26 asked for it as its own tab for exactly that.
      { id: "anomalies", label: "Anomalies" },
      // Next to Overview on purpose. Overview answers "is anything on fire",
      // Statistics answers "who, and how much", and both are the daily
      // governance surface rather than an investigation.
      { id: "stats", label: "Statistics" },
      { id: "money", label: "Money" },
      { id: "policy", label: "Policy" },
      { id: "identity", label: "Identity" },
      { id: "copilot", label: "Copilot" },
    ],
  },
  {
    label: "Investigate",
    items: [
      { id: "graph", label: "Graph" },
      { id: "replay", label: "Replay" },
      { id: "bus", label: "Bus Explorer" },
      { id: "egress", label: "Web Egress" },
      { id: "quality", label: "Quality" },
      { id: "memory", label: "Memory" },
    ],
  },
  {
    label: "Assure",
    items: [
      { id: "crypto", label: "Crypto" },
      { id: "drills", label: "Drills" },
      { id: "evidence", label: "Evidence" },
      { id: "posture", label: "Posture" },
      { id: "routines", label: "Routines" },
    ],
  },
  {
    label: "Set up",
    items: [
      { id: "onboard", label: "Onboard" },
      { id: "remote", label: "Remote" },
    ],
  },
];

/** Flat view list in rail order, kept for anything that just needs to walk
 * every view once (labels, iteration). The grouping lives in [`NAV_SECTIONS`]. */
export const VIEWS: readonly { id: ViewId; label: string }[] = NAV_SECTIONS.flatMap(
  (s) => s.items,
);

/** Business-unit slug -> display name, for the ten units the post-reseed
 * console actually carries (Yurii, 2026-07-24). Every reader of a raw unit
 * id (`WatchDock.tsx`'s pinned units, `UnitCard.tsx`/`AgentDetailCard.tsx`'s
 * "business unit" field, the OverviewView/UserCard team labels, ...) routes
 * its DISPLAYED text through this - the raw slug stays the value/key
 * everywhere (pin lists, popover lookups, `<option>` values), only the
 * rendered text changes. */
const UNIT_LABELS: Readonly<Record<string, string>> = {
  finops: "FinOps",
  sre: "SRE",
  data: "Data",
  devops: "DevOps",
  platform: "Platform",
  "financial-crime": "Financial Crime",
  "credit-risk": "Credit Risk",
  "customer-support": "Customer Support",
  "corporate-banking": "Corporate Banking",
  compliance: "Compliance",
};

/** Pretty display name for a business-unit id. Known ids (the ten the
 * console carries post-reseed) use [`UNIT_LABELS`]'s exact copy; anything
 * else - a stale pin, a demo id, a unit this map has not caught up with yet -
 * falls back to a generic "split on hyphen, capitalize each word" render
 * rather than hiding or erroring, matching this app's "never a crash, never a
 * fabricated value" tolerance for data the current box does not know about. */
export function prettyUnit(id: string): string {
  const known = UNIT_LABELS[id];
  if (known) return known;
  return id
    .split("-")
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

/** Which business unit an agent's TEAM rolls up to. The console only ever
 * learns a team from the agent id path (`agent://org/<team>/<name>`); the unit
 * is a separate attribution the money plane resolves server-side, so any
 * frontend that wants to bucket or label by unit needs this map. It mirrors
 * the seeder's UNIT_OF exactly (5 business departments fed by the banking
 * teams, 5 IT platform teams whose team name IS their unit). A team this map
 * does not know is treated as its own unit, so an unmapped IT-style team still
 * renders sanely instead of collapsing into one bucket. */
const TEAM_TO_UNIT: Record<string, string> = {
  fraud: "financial-crime",
  "kyc-aml": "financial-crime",
  lending: "credit-risk",
  support: "customer-support",
  treasury: "corporate-banking",
  compliance: "compliance",
  finops: "finops",
  sre: "sre",
  data: "data",
  devops: "devops",
  platform: "platform",
};

export function unitForTeam(team: string): string {
  return TEAM_TO_UNIT[team] ?? team;
}
