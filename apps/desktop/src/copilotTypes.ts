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

/** Mirrors `genaryx_copilot::agent::Answer` (already `Serialize`) - the
 * finished answer `copilot_ask` returns on success: the model's text, every
 * tool it ran along the way, and accumulated token usage. */
export interface CopilotAnswer {
  text: string;
  tool_trace: CopilotToolInvocation[];
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
