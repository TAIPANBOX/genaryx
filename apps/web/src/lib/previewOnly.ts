/**
 * Commands only the mock preview answers (invariant 50).
 *
 * A real `genaryx-web` has no arm for any of these in
 * `crates/web/src/dispatch.rs`: they read or edit an owned agent, unit or user
 * record, or configure Felyx from the browser, and no store for either exists
 * on a box yet. Sent to a box, each one would come back 403 or 404 and the
 * caller would swallow it into `null`, so a control built on one did nothing
 * and said nothing.
 *
 * So `transport.invokeBackend` refuses every name here on a real box before
 * any request leaves the browser, with a {@link PreviewOnlyRefusal}, and the
 * controls built on them are hidden or marked there. The mock preview keeps
 * answering them, which is what the published demo shows.
 *
 * `scripts/backend-commands-are-handled.sh` reads this list, one quoted name
 * per line between the two markers below. It fails when the frontend names a
 * command that is neither a dispatch arm nor listed here, when a name listed
 * here gains a dispatch arm (the list went stale: drop it from here), and when
 * a name listed here is not answered by `mockPreview.ts` (then not even the
 * preview could answer it).
 */

// preview-only-commands:begin
export const PREVIEW_ONLY_COMMANDS = [
  "agent_record",
  "unit_record",
  "user_record",
  "org_directory",
  "agent_set_budget",
  "agent_reassign_unit",
  "agent_transfer_owner",
  "agent_set_behaviour",
  "copilot_connect",
] as const;
// preview-only-commands:end

export type PreviewOnlyCommand = (typeof PREVIEW_ONLY_COMMANDS)[number];

const LISTED: ReadonlySet<string> = new Set(PREVIEW_ONLY_COMMANDS);

export function isPreviewOnly(command: string): command is PreviewOnlyCommand {
  return LISTED.has(command);
}

/** What `invokeBackend` rejects with for a preview-only command on a real box.
 * Shaped like a box's own `{"error": ...}` body, so every plane normaliser's
 * string fallback can still print it. */
export interface PreviewOnlyRefusal {
  error: "preview_only";
  command: string;
}

export function previewOnlyRefusal(command: string): PreviewOnlyRefusal {
  return { error: "preview_only", command };
}

export function isPreviewOnlyRefusal(value: unknown): value is PreviewOnlyRefusal {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as { error?: unknown }).error === "preview_only" &&
    typeof (value as { command?: unknown }).command === "string"
  );
}
