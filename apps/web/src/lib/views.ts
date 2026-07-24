/** The top-level views the app shell switches between. */
export type ViewId =
  | "overview"
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
  | "pocket"
  | "graph"
  | "replay"
  | "posture"
  | "bus"
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
      { id: "pocket", label: "Pocket" },
    ],
  },
];

/** Flat view list in rail order, kept for anything that just needs to walk
 * every view once (labels, iteration). The grouping lives in [`NAV_SECTIONS`]. */
export const VIEWS: readonly { id: ViewId; label: string }[] = NAV_SECTIONS.flatMap(
  (s) => s.items,
);
