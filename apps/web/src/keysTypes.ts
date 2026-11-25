/**
 * The Keys panel's wire shape, `genaryx_api::keys` (CLAUDE.md invariant 53):
 * what Keyryx custody holds and what its witness last signed, read from their
 * own read-only routes. Every time is UNIX SECONDS.
 */

export interface KeyName {
  name: string;
  alg: string;
  /** `held` (custody signs with it) or `delegated` (custody certifies a key its consumer holds). */
  mode: string;
  active_kid: string | null;
  active_since: number | null;
  next_kid: string | null;
  next_activates_at: number | null;
  last_rotation: number | null;
  /** Inventory entries of this name by state, e.g. { active: 1, retired: 2 }. */
  states: Record<string, number>;
  renew_due: boolean;
}

export interface Alarm {
  kind: string;
  name: string;
  kid: string;
  reason: string;
}

export interface Entry {
  name: string;
  alg: string;
  kid: string;
  mode: string;
  state: string;
  created: number;
  activate_at?: number;
  retired?: number;
  revoked?: number;
  owner?: string;
  not_after?: number;
}

export type CustodyPart =
  | { state: "not_configured"; variable: string }
  | { state: "refused"; variable: string; reason: string }
  | { state: "unreachable"; url: string; reason: string }
  | {
      state: "read";
      url: string;
      version: string;
      names: KeyName[];
      alarms: Alarm[];
      alarms_total: number;
      entries: Entry[];
      entries_total: number;
      inventory_error: string | null;
    };

export type WitnessPart =
  | { state: "not_configured"; variable: string }
  | { state: "refused"; variable: string; reason: string }
  | { state: "unreachable"; url: string; reason: string }
  | { state: "no_head_yet"; url: string }
  | {
      state: "read";
      url: string;
      size: number;
      root: string;
      at: number;
      es384_kid: string | null;
      ml_dsa_65_kid: string | null;
    };

export interface KeysPanel {
  custody: CustodyPart;
  witness: WitnessPart;
}
