/**
 * UI-facing event shape for the Bus Explorer. Mirrors the Rust `UiEvent` in
 * `crates/api/src/events.rs`, which is itself a field-for-field serde mirror
 * of `genaryx_core::store::StoredEvent` (see that file's `From` impl for the
 * exact mock -> real-bus wiring point).
 */
export interface UiEvent {
  id: number;
  env: string;
  ts: string;
  source: string;
  type: string;
  agent_id: string;
  run_id: string | null;
  /**
   * Kept as a raw string, not narrowed to `Severity`: the core keeps
   * severity tolerant on purpose (an unrecognized value must never make a
   * whole event unrenderable, see `genaryx_core::event` doc comments).
   * `SeverityBadge` falls back gracefully for anything outside `SEVERITIES`.
   */
  severity: string | null;
  schema: string;
  on_behalf_of: string[];
  data: unknown | null;
  prev_hash: string | null;
  raw: string;
  file: string | null;
  off: number | null;
}

export type Severity = "info" | "low" | "medium" | "high" | "critical";

export const SEVERITIES: readonly Severity[] = [
  "info",
  "low",
  "medium",
  "high",
  "critical",
];

/** The six stack services that emit onto the bus (idryx never does; see
 * `genaryx_core::demo` module docs). */
export type SourceId =
  | "tokenfuse"
  | "wardryx"
  | "engram"
  | "verdryx"
  | "mockryx"
  | "qryx";

export const SOURCES: readonly SourceId[] = [
  "tokenfuse",
  "wardryx",
  "engram",
  "verdryx",
  "mockryx",
  "qryx",
];
