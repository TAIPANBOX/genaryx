/**
 * Who a bus event is filed under, which is not always the `agent_id` on its
 * envelope.
 *
 * Every event on the bus is about the agent its envelope names, with one
 * exception: TokenFuse's `identity_mismatch`. The gateway writes it when a
 * caller presented a credential that may not speak as the agent it claimed,
 * and it puts the CLAIMED id on the envelope and the credential that actually
 * called in `data.key_id`. So the envelope names exactly the agent that did
 * not make the call, and reading it as the subject made an impersonation
 * attempt a high-severity incident against its victim.
 *
 * The rule is the console's (`genaryx_core::attribution` in Rust, which the
 * Statistics counts, the stop list and the profile read) and TokenFuse's own
 * for its FOCUS export: a refused identity is filed under `key:<key_id>`,
 * never under the agent it claimed, and under `key:(none)` when the gateway
 * ran without client keys. This file is the TypeScript copy of that one rule,
 * kept to the same inputs and outputs; `attribution.test.ts` holds it to the
 * same cases the Rust tests use.
 */
import type { UiEvent } from "../types";

export const IDENTITY_MISMATCH = "identity_mismatch";
export const KEY_PREFIX = "key:";
export const NO_KEY = "key:(none)";

type EventLike = Pick<UiEvent, "type" | "agent_id" | "data">;

/** The key an identity refusal names, or null for any other event and for a
 * refusal that carried no readable key. Only a non-empty string names a key. */
export function refusedKey(e: Pick<UiEvent, "type" | "data">): string | null {
  if (e.type !== IDENTITY_MISMATCH) return null;
  const data = e.data;
  if (!data || typeof data !== "object" || Array.isArray(data)) return null;
  const key = (data as Record<string, unknown>).key_id;
  return typeof key === "string" && key !== "" ? key : null;
}

/** A call the gateway refused for its CLIENT KEY, not for the agent it
 * claimed: TokenFuse writes it as `identity_mismatch` with `data.reason`
 * `missing_key` (no key presented) or `unknown_key` (a key it does not hold,
 * named only by `data.key_fingerprint`, `sha256:` and 16 hex digits of the
 * presented value, never the key), rate-limited per key and claimed agent,
 * with `data.unreported_since_last` counting what the limit held back
 * (tokenfuse invariant 105). Null for every other event, a real identity
 * mismatch included. Optional: a gateway older than that writes no such
 * event, and nothing here depends on one arriving.
 *
 * It is filed like any identity refusal (no `key_id`, so `key:(none)`): the
 * rule in `filedUnder` is the Rust and SQL rule too, and stays one rule. What
 * changes is the sentence, the incident's title and its grouping, which name
 * the cause and the fingerprint. */
export function keyRefusal(
  e: Pick<UiEvent, "type" | "data">,
): { reason: "missing_key" | "unknown_key"; fingerprint: string | null; heldBack: number | null } | null {
  if (e.type !== IDENTITY_MISMATCH) return null;
  const data = e.data;
  if (!data || typeof data !== "object" || Array.isArray(data)) return null;
  const d = data as Record<string, unknown>;
  if (d.reason !== "missing_key" && d.reason !== "unknown_key") return null;
  const fp = d.key_fingerprint;
  const fingerprint = typeof fp === "string" && /^sha256:[0-9a-f]{1,64}$/.test(fp) ? fp : null;
  const held = d.unreported_since_last;
  const heldBack = typeof held === "number" && Number.isFinite(held) && held >= 0 ? held : null;
  return { reason: d.reason, fingerprint: d.reason === "unknown_key" ? fingerprint : null, heldBack };
}

/** How a refused client key reads: "no client key" or "an unknown key
 * sha256:..." (or "an unknown key" when the fingerprint is unreadable). */
function refusedKeyPhrase(r: { reason: "missing_key" | "unknown_key"; fingerprint: string | null }): string {
  if (r.reason === "missing_key") return "no client key";
  return r.fingerprint ? `an unknown key ${r.fingerprint}` : "an unknown key";
}

/** Who this event is filed under: its envelope `agent_id`, or, for an
 * identity refusal, the key that made the call. */
export function filedUnder(e: EventLike): string {
  if (e.type !== IDENTITY_MISMATCH) return e.agent_id;
  const key = refusedKey(e);
  return key === null ? NO_KEY : `${KEY_PREFIX}${key}`;
}

/** True for an identity refusal, whose envelope `agent_id` is a claim. */
export function isIdentityRefusal(e: Pick<UiEvent, "type">): boolean {
  return e.type === IDENTITY_MISMATCH;
}

/** True for a subject that is a credential, not an agent. Such a subject has
 * no Agent 360 card, no owner and no team: opening one would show an empty
 * card that reads as an agent with no history. */
export function isKeySubject(subject: string): boolean {
  return subject.startsWith(KEY_PREFIX);
}

/** The sentence an identity refusal is described by, naming both halves:
 * who it claimed to be and which key it actually was. Null for any other
 * event. */
export function claimSentence(e: EventLike): string | null {
  if (!isIdentityRefusal(e)) return null;
  const claimed = e.agent_id || "no agent";
  const refusal = keyRefusal(e);
  if (refusal) return `claimed ${claimed} with ${refusedKeyPhrase(refusal)}, refused before the call`;
  const key = refusedKey(e);
  return key === null ? `claimed ${claimed} with no key` : `claimed ${claimed} with key ${key}`;
}

/** How one bus event reads on an agent's own event feed (Agent 360).
 *
 * The feed is read by envelope `agent_id`, so an identity refusal that CLAIMED
 * this agent lands on it. It stays, since somebody trying to be this agent is
 * worth seeing on its card, but as a claim by the key that was refused, never
 * as the agent's own activity: `own` is false and the text names the key.
 * Every other event reads as its own type, as before. */
export function agentFeedLine(e: EventLike, viewer: string): { text: string; own: boolean } {
  if (isIdentityRefusal(e) && e.agent_id === viewer) {
    const refusal = keyRefusal(e);
    if (refusal) return { text: `${e.type}: claimed with ${refusedKeyPhrase(refusal)}, refused`, own: false };
    const key = refusedKey(e);
    const by = key === null ? "claimed with no key" : `claimed by key ${key}`;
    return { text: `${e.type}: ${by}, refused`, own: false };
  }
  return { text: e.type, own: true };
}
