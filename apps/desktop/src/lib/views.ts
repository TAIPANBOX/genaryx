/** The top-level views the app shell switches between. */
export type ViewId =
  | "overview"
  | "money"
  | "policy"
  | "identity"
  | "quality"
  | "crypto"
  | "graph"
  | "replay"
  | "posture"
  | "bus";

export const VIEWS: readonly { id: ViewId; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "money", label: "Money" },
  { id: "policy", label: "Policy" },
  { id: "identity", label: "Identity" },
  { id: "quality", label: "Quality" },
  { id: "crypto", label: "Crypto" },
  { id: "graph", label: "Graph" },
  { id: "replay", label: "Replay" },
  { id: "posture", label: "Posture" },
  { id: "bus", label: "Bus Explorer" },
];
