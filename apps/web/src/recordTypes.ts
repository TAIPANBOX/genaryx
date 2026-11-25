/**
 * The Record panel's wire shape, `genaryx_api::record` (CLAUDE.md invariant 54):
 * what Trailryx sealed, read from the node's own files, whether the current
 * root carries a Keyryx signature, and the last verification this process ran.
 * Every time is unix seconds.
 */

export interface LastSegment {
  segment: number;
  records: number;
  first_recorded_at: number;
  last_recorded_at: number;
}

export type Signature =
  | { state: "signed"; key: string; algorithm: string; at: number }
  | { state: "queued"; since: number }
  | { state: "none" }
  | { state: "unreadable"; reason: string };

export type SealState =
  | { state: "not_configured"; variable: string }
  | { state: "refused"; variable: string; reason: string }
  | { state: "unreadable"; path: string; reason: string }
  | { state: "nothing_sealed"; path: string }
  | {
      state: "sealed";
      path: string;
      segments: number;
      records: number;
      last: LastSegment;
      /** The store root, 96 hex. */
      root: string;
      signature: Signature;
      other_shards: number;
    };

export type VerifyState =
  | { state: "not_run" }
  | { state: "verified"; root: string; at: number; millis: number; findings: string[] }
  | { state: "not_verified"; root: string; at: number; millis: number; findings: string[] }
  | { state: "refused"; root: string; at: number; reason: string }
  | { state: "not_checked"; at: number; reason: string };

export interface RecordPanel {
  seal: SealState;
  verify: VerifyState;
  verify_stale: boolean;
}
