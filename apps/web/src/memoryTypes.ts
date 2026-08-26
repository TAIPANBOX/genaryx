/**
 * Memory wire types. Mirrors the Rust DTOs in
 * `crates/api/src/memory/commands.rs` and `crates/api/src/memory/env.rs`
 * field-for-field (same convention `qualityTypes.ts`/`cryptoTypes.ts` follow
 * for their own panels), including the exact serde tag/rename_all shape of
 * every enum so `invokeBackend<T>(...)` results type-check honestly instead of
 * being cast.
 *
 * `EngramStats`/`EngramCounts`/`EngramMemory`/`EngramProvenance`/
 * `EngramForgetResult` mirror `genaryx_connectors::Engram*`
 * (`crates/connectors/src/engram.rs`) directly - those Rust types already
 * derive `Serialize` and cross the genaryx-web JSON boundary as-is, so these
 * interfaces exist only so the frontend has names/types for the exact same
 * wire shape, not because the Rust side re-wraps anything. Every `Option<T>`
 * field there serializes as `T | null` (always present, never omitted -
 * `#[serde(default)]` only affects deserialization), matching every field
 * below marked `| null`.
 */

/** Mirrors `memory::env::EnvSource` (`#[serde(tag = "source", rename_all = "snake_case")]`). */
export type EnvSource = { source: "taipan"; name: string } | { source: "well_known" };

/** Mirrors `memory::commands::MemoryStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). Unlike Quality,
 * `unreachable` here means "resolved an engram-mcp binary + a real `.engram`
 * store, but spawning the process or the MCP handshake failed" - see
 * `memory::state`'s doc comment. */
export type MemoryStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | {
      state: "unreachable";
      source: EnvSource;
      engram_mcp_bin: string;
      db_path: string;
      reason: string;
    }
  | { state: "ready"; source: EnvSource; engram_mcp_bin: string; db_path: string };

/** Mirrors `memory::commands::MemoryError` (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type MemoryError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "unreachable"; reason: string }
  | { kind: "mcp"; message: string };

/** Mirrors `genaryx_connectors::EngramCounts` (`stats`'s `counts`).
 * `procedural` is ALWAYS `0` in this Engram version (the store implements
 * only episodic + semantic) - render it labeled "not implemented", never a
 * real zero. */
export interface EngramCounts {
  episodic: number;
  semantic: number;
  procedural: number;
}

/** Mirrors `genaryx_connectors::EngramStats` (`stats`'s return). */
export interface EngramStats {
  /** The effective agent scope these counts are for - `null` only if the
   * server itself has no default agent configured (this console never sets
   * one at spawn time, see `memory::state`'s doc comment, so this is
   * commonly `null`). */
  agent_id: string | null;
  counts: EngramCounts;
  vector_index_size: number;
  facts_total: number;
  facts_active: number;
  facts_superseded: number;
  entities: number;
  reflections: number;
  db_path: string;
  /** `null` for an in-memory store or a file that does not exist yet -
   * render "in-memory / n/a", never a fabricated `0`. */
  db_size_bytes: number | null;
}

/** Mirrors `genaryx_connectors::EngramMemory` (`recall`'s per-hit shape).
 * Ranked by relevance; render in the order the array arrives (already
 * most-relevant first). */
export interface EngramMemory {
  id: string;
  content: string;
  score: number;
  importance: number;
  /** UTC ISO-8601 encoding time. */
  timestamp: string;
  actors: string[];
  tags: string[];
}

/** Mirrors `genaryx_connectors::EngramProvenance`
 * (`#[serde(tag = "kind", rename_all = "lowercase")]`) - `why`'s return. Two
 * shapes discriminated by `kind`: a semantic fact (with its extraction
 * chain) or an episodic observation (with encoding + access metadata). */
export type EngramProvenance =
  | {
      kind: "semantic";
      id: string;
      subject: string;
      predicate: string;
      object: string;
      confidence: number;
      valid_from: string;
      valid_to: string | null;
      recorded_at: string;
      extracted_from: string | null;
      extracted_by_reflection_run: string | null;
      extraction_model: string | null;
    }
  | {
      kind: "episodic";
      id: string;
      content: string;
      timestamp: string;
      actors: string[];
      tags: string[];
      salience: number | null;
      emotional_valence: number | null;
      importance_score: number | null;
      /** The episode ids this episode summarizes. A LIST on the wire, always
       * present (`Vec<String>` with `#[serde(default)]`, and
       * `Episode.summary_of: list[str]` upstream), never a scalar. This was
       * `string | null` here until 2026-08-26, which is the same mistake the
       * backend was already burned by: `crates/connectors/src/engram.rs`
       * carries a comment recording that typing it as a single string made
       * every episodic `why` fail to deserialize, caught live against real
       * `engram-mcp`. The console-side copy of the error was quieter and
       * lasted longer, because TypeScript does not check the wire: `[]` is
       * not nullish, so `summary_of ?? "-"` never once fired, and a
       * two-episode list rendered as the run-together token `ep-1,ep-2`.
       * An empty list means this episode summarizes nothing, which is a fact
       * about the episode, not a missing value. */
      summary_of: string[];
      agent_id: string | null;
      access_count: number;
      last_accessed: string | null;
      note: string;
    };

/** Mirrors `genaryx_connectors::EngramForgetResult` (`forget`'s return). */
export interface EngramForgetResult {
  id: string;
  /** `episodic` or `semantic` - which store the id was found in. */
  kind: string;
  deleted: boolean;
}

/** `recall`'s `mode` argument - see `EngramClient::recall`'s doc comment.
 * NOT enum-validated on the MCP wire (an unknown mode silently behaves as
 * `cosine`), but this console only ever sends one of these three. */
export const RECALL_MODES = ["cosine", "spreading", "hybrid"] as const;
export type RecallMode = (typeof RECALL_MODES)[number];
