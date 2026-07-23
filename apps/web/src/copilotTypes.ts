/**
 * Copilot wire types (Phase 6, C0). Mirrors the Rust DTOs in
 * `src-tauri/src/copilot/commands.rs` field-for-field, same convention
 * `identityTypes.ts`/`policyTypes.ts` follow for their own panels.
 *
 * Unlike every other panel's status type, `CopilotStatus` is a flat
 * interface, not a `state`-tagged union: the copilot has no environment to
 * discover and no reachability to probe (see `commands.rs`'s module doc for
 * why), so there is nothing besides "is it enabled, and with what
 * residency" to render.
 */
export interface CopilotStatus {
  enabled: boolean;
  provider: string | null;
  model: string | null;
  endpoint: string | null;
  local: boolean | null;
  disabled_reason: string | null;
}

/** Mirrors `genaryx_copilot::provider::Usage` (already `Serialize`, crosses
 * the Tauri IPC boundary as-is - see `lib/copilot.ts`'s doc comment). */
export interface CopilotUsage {
  prompt_tokens: number;
  completion_tokens: number;
}

/** Mirrors `genaryx_copilot::agent::ToolInvocation` (already `Serialize`) -
 * one tool call Felyx's loop executed, the evidence surface rendered next to
 * the model's own text so a claim is always checkable, never just trusted. */
export interface CopilotToolInvocation {
  name: string;
  ok: boolean;
  result_preview: string;
}

/** Mirrors `genaryx_copilot::action::ActionKind` (`#[serde(rename_all =
 * "snake_case")]`) - the four kinds of action Felyx may PROPOSE (C2,
 * docs/PHASE6-C2.md). Each maps to an EXISTING human-signed mutation the
 * shell already implements; the copilot crate holds no signer and never
 * calls those paths itself - see `ProposedAction`'s doc comment and
 * `CopilotView.tsx`'s `runApproval`. */
export type ProposedActionKind = "kill" | "budget" | "grant_deny" | "rescan";

/** Mirrors `genaryx_copilot::action::ProposedAction` (already `Serialize`) -
 * a recommendation with its evidence, never an executed mutation
 * (`crates/copilot/src/action.rs`'s own doc comment: "There is deliberately
 * no `Act` here"). The shell renders this as an approve/dismiss card
 * (`CopilotView.tsx`'s `ProposalCard`); clicking Approve routes into the
 * SAME signed ceremony a manual click on the Money/Policy/Identity panel
 * already triggers - this type is display data until then. */
export interface ProposedAction {
  kind: ProposedActionKind;
  /** The subject: a run id, approval id, agent id, etc. - `""` for a
   * fleet-wide `rescan` proposal with no specific target. */
  target: string;
  /** Action parameters: `{"usd_cap": number}` for `budget`,
   * `{"verdict": "grant"|"deny"}` for `grant_deny`, `{}` otherwise. Typed
   * loosely (mirrors the Rust side's own untyped `serde_json::Value`) - read
   * via the kind-specific accessors in `CopilotView.tsx` rather than assumed
   * shape-checked here. */
  params: Record<string, unknown>;
  /** Why the copilot proposes this, in one or two sentences. */
  rationale: string;
  /** The model's self-reported confidence in `[0, 1]`. */
  confidence: number;
  /** Source row ids backing the rationale, rendered verbatim (run ids,
   * incident ids, store event ids) so a claim is always checkable. */
  evidence_refs: string[];
  /** C2 Wardryx pre-check (side-effect-free): governing policy targets, read
   * from `list_policies` when Wardryx is configured. Empty when Wardryx is
   * absent or nothing matched - render nothing, never a fabricated "no
   * policy" claim. */
  policy_context: string[];
}

/** Mirrors `genaryx_copilot::agent::Answer` (already `Serialize`) - the
 * finished answer `copilot_ask` returns on success: the model's text, every
 * tool it ran along the way, any actions it PROPOSED (C2 - rendered as
 * approve/dismiss cards, nothing has happened yet), and accumulated token
 * usage. */
export interface CopilotAnswer {
  text: string;
  tool_trace: CopilotToolInvocation[];
  proposals: ProposedAction[];
  usage: CopilotUsage;
}

/** A pending "Explain with Felyx" hand-off (C1, docs/PHASE6-C1.md) from a
 * sibling view - e.g. the Money panel's Incidents feed - into the Copilot
 * pane. Not a Rust-side mirror: this is pure frontend plumbing, owned by
 * `AppShell` and consumed by `CopilotView.tsx`'s effect, the "shared prop"
 * hand-off the C1 contract calls for. `nonce` exists purely so two requests
 * for the SAME incident (the operator re-clicks "Explain" after already
 * reading the first answer) still register as a new request - a React
 * effect dependency compares by value, so an unchanged `incidentId` alone
 * would not re-fire. */
export interface CopilotExplainRequest {
  nonce: number;
  incidentId: string;
}
