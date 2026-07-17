import type { MoneyStatus } from "../moneyTypes";

/**
 * Shared "not ready yet" rendering for the Overview and Money views: three
 * honest, distinct states (never a generic spinner-forever or error toast) -
 * still connecting, no environment configured, or a resolved environment
 * that failed to pair. `status === null` (the hook's pre-first-response
 * state) renders the same as "bootstrapping".
 */
export function MoneyEmptyState({ status }: { status: MoneyStatus | null }) {
  if (!status || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          connecting to a TokenFuse Cloud environment...
        </div>
      </div>
    );
  }

  if (status.state === "no_environment") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 460 }}>
          <span style={{ fontSize: 13, color: "var(--fg)" }}>No environment found</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
            Run <span style={{ color: "var(--fg)" }}>taipan up</span> to start a stack, or set{" "}
            <span style={{ color: "var(--fg)" }}>TOKENFUSE_CLOUD_ADMIN_KEY</span> for a Cloud already running at
            127.0.0.1:8080.
          </span>
        </div>
      </div>
    );
  }

  if (status.state === "pairing_failed") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 460 }}>
          <span style={{ fontSize: 13, color: "var(--sev-high)" }}>Could not pair with the Cloud</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.cloud_url || "(no Cloud URL resolved)"}
          </span>
          <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.reason}
          </span>
        </div>
      </div>
    );
  }

  // `status.state === "ready"`: callers only render this component when NOT
  // ready, so this branch is unreachable in practice - a blank pane rather
  // than a false "not ready" message if it is ever hit anyway.
  return null;
}
