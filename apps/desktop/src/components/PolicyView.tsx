import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { decideApproval, describePolicyError, fetchApprovals, fetchPolicies } from "../lib/policy";
import { usePolicyStatus } from "../lib/usePolicyStatus";
import type { Approval, Decision, PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";
import { ApprovalsInbox, type GrantedToken } from "./ApprovalsInbox";
import { DecisionStream } from "./DecisionStream";
import { PolicyList } from "./PolicyList";

/** Same feels-alive refresh cadence as `MoneyView.tsx`/`OverviewView.tsx` -
 * not a live push (the Approvals Inbox and Policy list are plain reads),
 * just a periodic re-fetch plus an always-on refetch right after any
 * decision settles. */
const REFRESH_INTERVAL_MS = 20_000;

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

function Loading() {
  return (
    <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
      loading...
    </div>
  );
}

/**
 * Shared "not ready yet" rendering for the Policy view - mirrors
 * `MoneyEmptyState.tsx`'s three honest, distinct states (never a generic
 * spinner-forever or error toast), Wardryx-flavored: still connecting, no
 * policy plane configured, or a resolved environment whose `GET /healthz`
 * check failed. Kept local to this file rather than a new
 * `PolicyEmptyState.tsx` (not in this track's named component list) - the
 * copy and layout deliberately match `MoneyEmptyState` closely.
 */
function PolicyEmptyState({ status }: { status: PolicyStatus | null }) {
  if (!status || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          connecting to a Wardryx policy plane...
        </div>
      </div>
    );
  }

  if (status.state === "no_environment") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 480 }}>
          <span style={{ fontSize: 13, color: "var(--fg)" }}>No policy plane found</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
            Run <span style={{ color: "var(--fg)" }}>taipan up --with wardryx</span> to start one, or set{" "}
            <span style={{ color: "var(--fg)" }}>WARDRYX_ADMIN_KEY</span> (and optionally{" "}
            <span style={{ color: "var(--fg)" }}>WARDRYX_URL</span>, default 127.0.0.1:8090) for a wardryx already
            running.
          </span>
        </div>
      </div>
    );
  }

  if (status.state === "unreachable") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 480 }}>
          <span style={{ fontSize: 13, color: "var(--sev-high)" }}>Could not reach wardryx</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.wardryx_url || "(no wardryx URL resolved)"}
          </span>
          <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.reason}
          </span>
        </div>
      </div>
    );
  }

  // `status.state === "ready"`: callers only render this component when NOT
  // ready, so this branch is unreachable in practice.
  return null;
}

export function PolicyView() {
  const status = usePolicyStatus();
  const ready = status?.state === "ready";

  const [approvals, setApprovals] = useState<Approval[] | null>(null);
  const [policies, setPolicies] = useState<PolicyRecord[] | null>(null);
  const [error, setError] = useState<PolicyError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [grantedToken, setGrantedToken] = useState<GrantedToken | null>(null);

  const refresh = useCallback(async () => {
    if (!ready) return;
    try {
      const [a, p] = await Promise.all([fetchApprovals(), fetchPolicies()]);
      setApprovals(a);
      setPolicies(p);
      setError(null);
    } catch (err) {
      setError(err as PolicyError);
    }
  }, [ready]);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [refresh]);

  const handleDecide = useCallback(
    async (id: string, decision: Decision) => {
      const outcome = await decideApproval(id, decision);
      setNotice(
        outcome.bus_recorded
          ? `${outcome.summary} - signed console_command recorded, visible in the Bus tab.`
          : `${outcome.summary} (bus journal not recorded: ${outcome.bus_error ?? "unknown reason"})`,
      );
      if (outcome.token) {
        setGrantedToken({ approvalId: id, outcome });
      }
      void refresh();
    },
    [refresh],
  );

  if (!ready) {
    return <PolicyEmptyState status={status} />;
  }

  // The set-level `policy_version` has no dedicated read (see
  // `PolicyList.tsx`'s doc comment): derive it from the most recently
  // requested approval. `list_approvals` is documented (and mirrored here
  // unmodified) to return ascending `requested_at` order, so the last
  // element is the most recent one without needing to re-sort.
  const latestPolicyVersion = approvals && approvals.length > 0 ? approvals[approvals.length - 1].policy_version : null;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <span className="chip" style={cssVar("dot", "var(--src-wardryx)")}>
        <span className="dot" aria-hidden="true" />
        {status.source.source === "taipan" ? `taipan up · ${status.source.name}` : "env fallback"} &middot;{" "}
        {status.wardryx_url}
      </span>

      {notice && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-low)" }}>
          {notice}
        </div>
      )}

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describePolicyError(error)}
        </div>
      )}

      <section className="flex flex-col gap-2">
        <SectionHeader title="Decision Stream" />
        <DecisionStream />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Approvals Inbox" />
        {approvals === null ? (
          <Loading />
        ) : (
          <ApprovalsInbox
            approvals={approvals}
            onDecide={handleDecide}
            grantedToken={grantedToken}
            onDismissToken={() => setGrantedToken(null)}
          />
        )}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Policies" />
        {policies === null ? <Loading /> : <PolicyList policies={policies} policyVersion={latestPolicyVersion} />}
      </section>
    </div>
  );
}
