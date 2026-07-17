/** The three top-level views the app shell switches between. */
export type ViewId = "overview" | "money" | "bus";

export const VIEWS: readonly { id: ViewId; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "money", label: "Money" },
  { id: "bus", label: "Bus Explorer" },
];
