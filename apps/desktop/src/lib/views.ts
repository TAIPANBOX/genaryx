/** The top-level views the app shell switches between. */
export type ViewId = "overview" | "money" | "policy" | "bus";

export const VIEWS: readonly { id: ViewId; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "money", label: "Money" },
  { id: "policy", label: "Policy" },
  { id: "bus", label: "Bus Explorer" },
];
