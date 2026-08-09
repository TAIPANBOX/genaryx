/** What agents did on the web, as the egress plane recorded it.
 *
 * Field-for-field mirror of `crates/api/src/egress/mod.rs`. Every optional here
 * is optional THERE for a reason, and the reasons are not interchangeable:
 * `backend` is absent on a refusal because no backend was reached, while
 * `urlSha384` absent means the record did not carry one. A component that
 * rendered both as an empty cell would be telling the reader they are the same
 * fact. */

/** One line: a fetch that happened, or one that did not. */
export type EgressRow = {
  ts: string;
  agent_id: string;
  run_id: string | null;
  outcome: "fetched" | "blocked";

  /** Scheme and host, which is ALL the record carries.
   *
   * The path and the query string were never written: a URL is personal data,
   * and the plane that recorded this deliberately never assembled them into the
   * event. There is nothing here to reveal, which is the point, and no UI
   * affordance should suggest a full URL is one click away. */
  origin: string;
  url_sha384: string | null;

  /** On a fetch. */
  backend: string | null;
  enforcement: string | null;
  content_bytes: number | null;

  /** On a refusal. */
  verdict: string | null;
  reason: string | null;
};

export type EgressTotals = {
  fetched: number;
  blocked: number;
  /** Refusals by verdict. Open-ended on purpose: the vocabulary belongs to the
   * egress plane, and a fixed set of keys here would be a copy of somebody
   * else's list going stale the first time they add one. */
  by_verdict: Record<string, number>;
  /** Fetches served by a backend that enforces only at the navigation. */
  navigation_only: number;
  /** Fetches whose backend could not say what the page asked for.
   *
   * Different from "asked for nothing", and the difference is why this exists
   * as a count of its own. */
  subresources_unknown: number;
};

export type EgressPanel = {
  /** False when nothing could be READ.
   *
   * When this is false the rows are empty and mean nothing, and the view is
   * required to render `note` instead of an empty table. An empty table says
   * "your agents fetched nothing", which is the one wrong answer that reads as
   * good news. */
  measured: boolean;
  note: string | null;
  totals: EgressTotals;
  rows: EgressRow[];
};

export type EgressError =
  | { kind: "no_environment" }
  | { kind: "backend"; message: string };
