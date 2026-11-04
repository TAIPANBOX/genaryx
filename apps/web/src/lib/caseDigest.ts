import type { CaseFact } from "../caseTypes";
import { sha256Hex } from "./sha256";

/** Whether a fact's digest is the sha256 of its row as `JSON.stringify` writes
 * it. The demo transport digests that way; a case from a real box is digested
 * by Rust over its own bytes, and is checked there, not here. */
export function digestOk(fact: Pick<CaseFact, "digest" | "row">): boolean {
  return fact.digest === `sha256:${sha256Hex(JSON.stringify(fact.row))}`;
}
