/**
 * Pure add/remove/validate/dedup helpers for the onboard generate form's
 * "Filesystem access" section (docs/ONBOARD.md). Mirrors the backend's own
 * rules client-side (`crates/api/src/onboard/commands.rs`'s
 * `validate_filesystem_scopes`) so the operator sees a problem BEFORE
 * submitting - the backend stays the source of truth; nothing here ever
 * blocks a request from reaching it, it only decides whether the form's own
 * Generate button is enabled (see `OnboardView.tsx`).
 *
 * `FsScopeRow` is the UI's own shape: a stable `id` for React list keys,
 * since two rows can transiently share the same blank or duplicate path
 * while being edited. `FsScope` (`onboardTypes.ts`) is the wire shape the
 * backend actually accepts - {@link toFsScopes} converts one to the other.
 * Every function here is a plain function of its arguments (no module-level
 * state, no DOM, no backend call) - `id` generation is the caller's job
 * (`OnboardView.tsx` owns a counter), so this module stays trivially testable.
 */
import type { FsScope, FsScopeMode } from "../onboardTypes";

export interface FsScopeRow {
  id: string;
  path: string;
  mode: FsScopeMode;
}

/** A fresh, blank row for the given `id` - default mode `"read"`, the more
 * conservative of the two (a write scope is the one an operator should have
 * to opt into deliberately). */
export function blankFsScopeRow(id: string): FsScopeRow {
  return { id, path: "", mode: "read" };
}

/** Append one fresh blank row - the "+ Add folder" button's whole job. */
export function addFsScopeRow(rows: readonly FsScopeRow[], id: string): FsScopeRow[] {
  return [...rows, blankFsScopeRow(id)];
}

/** Drop the row with this `id` - the "x" button's whole job. Every other row
 * is returned unchanged, in its original order. */
export function removeFsScopeRow(rows: readonly FsScopeRow[], id: string): FsScopeRow[] {
  return rows.filter((r) => r.id !== id);
}

/** Replace one row's `path`, leaving its `mode` and every other row alone. */
export function setFsScopePath(rows: readonly FsScopeRow[], id: string, path: string): FsScopeRow[] {
  return rows.map((r) => (r.id === id ? { ...r, path } : r));
}

/** Replace one row's `mode`, leaving its `path` and every other row alone. */
export function setFsScopeMode(rows: readonly FsScopeRow[], id: string, mode: FsScopeMode): FsScopeRow[] {
  return rows.map((r) => (r.id === id ? { ...r, mode } : r));
}

/** The same normalization the backend applies before its own empty/dedup
 * checks (`path.trim()`) - kept as one function so every check below agrees
 * on what "the path" means. */
function trimmedPath(row: FsScopeRow): string {
  return row.path.trim();
}

/** `true` for a row whose path is blank after trimming - mirrors the
 * backend's "non-empty after trim" rule. */
export function isEmptyPath(row: FsScopeRow): boolean {
  return trimmedPath(row).length === 0;
}

/** `true` when at least one row's path is blank after trimming. */
export function hasEmptyPath(rows: readonly FsScopeRow[]): boolean {
  return rows.some((r) => isEmptyPath(r));
}

/** The set of trimmed paths that appear on more than one row - mirrors the
 * backend's dedup rule (exact string match after trim). Blank paths are
 * excluded: an empty path is already flagged by {@link isEmptyPath}/
 * {@link hasEmptyPath}, and counting blanks as "duplicates of each other"
 * would just be a confusing second warning for the same underlying problem. */
export function duplicatePaths(rows: readonly FsScopeRow[]): Set<string> {
  const seen = new Set<string>();
  const dupes = new Set<string>();
  for (const row of rows) {
    const p = trimmedPath(row);
    if (p.length === 0) continue;
    if (seen.has(p)) dupes.add(p);
    seen.add(p);
  }
  return dupes;
}

/** `true` when every row has a non-empty path and no path is duplicated -
 * the Generate button's own gate. Mirrors the backend's two filesystem
 * refusals so the operator sees the same problem before submitting, not
 * after (the backend re-validates regardless; this is a hint, not a lock). */
export function fsScopesAreValid(rows: readonly FsScopeRow[]): boolean {
  return !hasEmptyPath(rows) && duplicatePaths(rows).size === 0;
}

/** Rows -> the wire shape the backend accepts, trimmed and in declared
 * order - `onboard_generate`'s `filesystem` request field. */
export function toFsScopes(rows: readonly FsScopeRow[]): FsScope[] {
  return rows.map((r) => ({ path: trimmedPath(r), mode: r.mode }));
}
