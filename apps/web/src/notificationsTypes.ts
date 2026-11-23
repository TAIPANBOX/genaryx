/**
 * The Notifications panel's wire shape, `genaryx_api::notifications`
 * (CLAUDE.md invariant 51): what heraldyx recorded sending, read from its own
 * dispatch journal (`sent.ndjson`) where the box names it.
 */

export interface SentRecord {
  line: number;
  ts: string;
  kind: string;
  about: string;
  to: string[];
  to_total: number;
  transport: string;
  /** "accepted" (a server took it, not "delivered") or "refused". */
  outcome: string;
  error: string | null;
  agent_id: string;
  run_id: string | null;
}

export interface SkippedLine {
  line: number;
  reason: string;
}

export type ChainVerdict =
  | { state: "verifies"; chained: number; heads: number; unverifiable: number }
  | { state: "broken"; first_line: number; breaks: number }
  | { state: "nothing_bound" };

export type NotificationsPanel =
  | { state: "not_configured"; variable: string }
  | { state: "unreadable"; path: string; reason: string }
  | {
      state: "read";
      path: string;
      records: SentRecord[];
      total: number;
      accepted: number;
      refused: number;
      by_kind: Record<string, number>;
      chain: ChainVerdict;
      window_truncated: boolean;
      skipped: SkippedLine[];
      skipped_total: number;
    };
