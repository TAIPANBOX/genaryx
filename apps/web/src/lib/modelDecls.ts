/**
 * Pure add/remove/validate/dedup helpers for the onboard generate form's
 * "Declared models" section (docs/ONBOARD.md). Mirrors the backend's own
 * rules client-side (`crates/api/src/onboard/commands.rs`'s
 * `validate_model_decls`) so the operator sees a problem BEFORE submitting -
 * the backend stays the source of truth; nothing here ever blocks a request
 * from reaching it, it only decides whether the form's own Generate button
 * is enabled (see `OnboardView.tsx`). Mirrors `fsScopes.ts` field-for-field:
 * same "empty" and "duplicate" checks, same pure-function shape, same
 * control-character check left to the backend (rare/adversarial input; the
 * client-side gate only covers the two ordinary slip-ups, a blank field or a
 * repeated row).
 *
 * `ModelDeclRow` is the UI's own shape: a stable `id` for React list keys
 * (two rows can transiently share the same blank or duplicate declaration
 * while being edited), and `provider`/`model`/`endpoint` all as plain
 * strings (an empty string means "not set" - a text `<input>`'s only
 * vocabulary), where `ModelDecl` (`onboardTypes.ts`) is the wire shape the
 * backend actually accepts - {@link toModelDecls} converts one to the other,
 * turning a blank `model`/`endpoint` into an omitted field. Every function
 * here is a plain function of its arguments (no module-level state, no DOM,
 * no backend call) - `id` generation is the caller's job (`OnboardView.tsx`
 * owns a counter), so this module stays trivially testable.
 */
import type { ModelDecl } from "../onboardTypes";

export interface ModelDeclRow {
  id: string;
  provider: string;
  model: string;
  endpoint: string;
}

/** A fresh, blank row for the given `id` - every field starts empty; only
 * `provider` is ever required (agent-passport SPEC.md section 4.5). */
export function blankModelDeclRow(id: string): ModelDeclRow {
  return { id, provider: "", model: "", endpoint: "" };
}

/** Append one fresh blank row - the "+ Add model" button's whole job. */
export function addModelDeclRow(rows: readonly ModelDeclRow[], id: string): ModelDeclRow[] {
  return [...rows, blankModelDeclRow(id)];
}

/** Drop the row with this `id` - the "x" button's whole job. Every other row
 * is returned unchanged, in its original order. */
export function removeModelDeclRow(rows: readonly ModelDeclRow[], id: string): ModelDeclRow[] {
  return rows.filter((r) => r.id !== id);
}

/** Replace one row's `provider`, leaving `model`/`endpoint` and every other
 * row alone. */
export function setModelDeclProvider(
  rows: readonly ModelDeclRow[],
  id: string,
  provider: string,
): ModelDeclRow[] {
  return rows.map((r) => (r.id === id ? { ...r, provider } : r));
}

/** Replace one row's `model`, leaving `provider`/`endpoint` and every other
 * row alone. */
export function setModelDeclModel(
  rows: readonly ModelDeclRow[],
  id: string,
  model: string,
): ModelDeclRow[] {
  return rows.map((r) => (r.id === id ? { ...r, model } : r));
}

/** Replace one row's `endpoint`, leaving `provider`/`model` and every other
 * row alone. */
export function setModelDeclEndpoint(
  rows: readonly ModelDeclRow[],
  id: string,
  endpoint: string,
): ModelDeclRow[] {
  return rows.map((r) => (r.id === id ? { ...r, endpoint } : r));
}

/** The same normalization the backend applies before its own empty/dedup
 * checks (`provider.trim()`) - kept as one function so every check below
 * agrees on what "the provider" means. */
function trimmedProvider(row: ModelDeclRow): string {
  return row.provider.trim();
}

/** `true` for a row whose provider is blank after trimming - mirrors the
 * backend's "non-empty after trim" rule for `provider`. */
export function isEmptyProvider(row: ModelDeclRow): boolean {
  return trimmedProvider(row).length === 0;
}

/** `true` when at least one row's provider is blank after trimming. */
export function hasEmptyProvider(rows: readonly ModelDeclRow[]): boolean {
  return rows.some((r) => isEmptyProvider(r));
}

/** A stable key for a row's (provider, model, endpoint) triple after
 * trimming, with a blank `model`/`endpoint` collapsing to the same "not
 * provided" value an omitted field would have - mirrors the backend's own
 * tuple dedup key (`validate_model_decls`'s `(String, Option<String>,
 * Option<String>)`). Exported so the form can flag a specific row as the
 * duplicate it is, not just report that some duplicate exists somewhere
 * (mirrors how `fsScopes.ts`'s `duplicatePaths` lets a row check its own
 * trimmed path against the returned set - here the "path" is a triple, so
 * the row needs this function to compute its own comparable key). */
export function modelDeclKey(row: ModelDeclRow): string {
  const provider = trimmedProvider(row);
  const model = row.model.trim();
  const endpoint = row.endpoint.trim();
  return JSON.stringify([provider, model.length > 0 ? model : null, endpoint.length > 0 ? endpoint : null]);
}

/** The set of {@link modelDeclKey} values that appear on more than one row -
 * mirrors the backend's dedup rule (exact triple match after trim, blank
 * optional fields treated as absent). Rows with an empty provider are
 * excluded: an empty provider is already flagged by {@link isEmptyProvider}/
 * {@link hasEmptyProvider}, and counting several blank-provider rows as
 * "duplicates of each other" would just be a confusing second warning for
 * the same underlying problem (mirrors `duplicatePaths`'s identical
 * exclusion for a blank path). */
export function duplicateModelKeys(rows: readonly ModelDeclRow[]): Set<string> {
  const seen = new Set<string>();
  const dupes = new Set<string>();
  for (const row of rows) {
    if (isEmptyProvider(row)) continue;
    const key = modelDeclKey(row);
    if (seen.has(key)) dupes.add(key);
    seen.add(key);
  }
  return dupes;
}

/** `true` when every row has a non-empty provider and no
 * (provider, model, endpoint) triple is duplicated - the Generate button's
 * own gate. Mirrors the backend's two model-declaration refusals so the
 * operator sees the same problem before submitting, not after (the backend
 * re-validates regardless; this is a hint, not a lock). */
export function modelDeclsAreValid(rows: readonly ModelDeclRow[]): boolean {
  return !hasEmptyProvider(rows) && duplicateModelKeys(rows).size === 0;
}

/** Rows -> the wire shape the backend accepts, trimmed and in declared
 * order - `onboard_generate`'s `models` request field. A blank `model`/
 * `endpoint` becomes an omitted key, not an empty string, matching the
 * backend's own "blank means not provided" tolerance. */
export function toModelDecls(rows: readonly ModelDeclRow[]): ModelDecl[] {
  return rows.map((r) => {
    const model = r.model.trim();
    const endpoint = r.endpoint.trim();
    const decl: ModelDecl = { provider: trimmedProvider(r) };
    if (model.length > 0) decl.model = model;
    if (endpoint.length > 0) decl.endpoint = endpoint;
    return decl;
  });
}
