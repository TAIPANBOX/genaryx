import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { downloadCsv, downloadJson, type ExportMeta } from "../lib/download";
import { decideApproval, describePolicyError, fetchApprovals, fetchPolicies } from "../lib/policy";
import {
  APPROVALS_EXPORT_COLUMNS,
  approvalsExportMeta,
  approvalsExportRows,
  POLICIES_EXPORT_COLUMNS,
  policiesExportMeta,
  policiesExportRows,
} from "../lib/policyExport";
import { usePolicyStatus } from "../lib/usePolicyStatus";
import { useSession } from "../lib/useSession";
import type { Approval, Decision, PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";
import { ApprovalsInbox, type GrantedToken } from "./ApprovalsInbox";
import { DecisionStream } from "./DecisionStream";
import { PolicyList } from "./PolicyList";
import { FreshBadge } from "./FreshBadge";
import { Hero, HeroBand, KpiTile, Section } from "./dash";

/** Same feels-alive refresh cadence as `MoneyView.tsx`/`OverviewView.tsx` -
 * not a live push (the Approvals Inbox and Policy list are plain reads),
 * just a periodic re-fetch plus an always-on refetch right after any
 * decision settles. */
const REFRESH_INTERVAL_MS = 20_000;

/** Local-day comparison for the hero's "decided today" tile - no timezone
 * conversion, matches how an operator reads their own wall clock. */
function isToday(iso: string): boolean {
  const d = new Date(iso);
  const now = new Date();
  return d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
}

/**
 * The two save buttons that sit in a Section's header.
 *
 * A component rather than four copies of the same pair, and it takes the meta
 * as a thunk on purpose: `takenAt` has to be the moment of the click, not the
 * moment of the last render, or a file taken at 16:40 claims to be the state
 * of the world at 16:12.
 */
function ExportButtons<T>({
  basename,
  columns,
  rows,
  meta,
  title,
}: {
  basename: string;
  columns: { key: keyof T & string; header: string }[];
  rows: () => T[];
  meta: () => ExportMeta;
  title: string;
}) {
  const style = {
    background: "var(--panel-2)",
    color: "var(--dim)",
    border: "1px solid var(--line)",
  } as const;
  return (
    <span className="flex items-center gap-2">
      <button
        type="button"
        className="mono text-[11.5px] px-3 py-1 rounded"
        style={style}
        title={title}
        onClick={() => downloadCsv(`${basename}.csv`, columns, rows(), meta())}
      >
        Export CSV
      </button>
      <button
        type="button"
        className="mono text-[11.5px] px-3 py-1 rounded"
        style={style}
        title={title}
        onClick={() => downloadJson(`${basename}.json`, rows(), meta())}
      >
        Export JSON
      </button>
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

/**
 * Wave-3 deep-link props (docs/PHASE2.md "Actionable notifications"),
 * threaded straight through to `ApprovalsInbox.tsx` - `AppShell.tsx` owns
 * the actual state (`focusApprovalId`/`mutedKeys`) since both need to
 * survive independently of which view is currently active (a mute toggled
 * here must still suppress a notification raised while the operator is on
 * a different view; a notification raised while on a different view must
 * still be able to focus a row here once the operator switches over).
 */
export interface PolicyViewProps {
  focusApprovalId: string | null;
  mutedKeys: ReadonlySet<string>;
  onToggleMuteAgent: (agentId: string) => void;
  /** Phase-3 wave-3 deep link (docs/PHASE3.md W3): opens the Agent 360 card
   * for an `agent_id`, threaded down to `DecisionStream`/`ApprovalsInbox`. */
  onOpenAgent: (agentId: string) => void;
}

export function PolicyView({ focusApprovalId, mutedKeys, onToggleMuteAgent, onOpenAgent }: PolicyViewProps) {
  const status = usePolicyStatus();
  const ready = status?.state === "ready";
  const session = useSession();

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
      // Sets the SAME `error` state `refresh()` above uses, so a rejection
      // (most notably a 403 `role_required` refusal for an approver trying
      // an admin-only command, or a viewer trying to decide at all -
      // docs/CONSOLE-IDP.md) renders through the existing banner instead of
      // vanishing as an unhandled rejection. Still rethrown: `ConfirmButton`'s
      // Grant/Deny ceremony resets its pending state either way, but the
      // reject/resolve distinction is kept honest for any future caller that
      // branches on it, matching `MoneyView.tsx`'s identical contract.
      try {
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
      } catch (err) {
        setError(err as PolicyError);
        throw err;
      }
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

  const pendingCount = (approvals ?? []).filter((a) => a.pending).length;
  const decidedTodayCount = (approvals ?? []).filter((a) => a.decided_at !== null && isToday(a.decided_at)).length;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <span className="chip" style={cssVar("dot", "var(--src-wardryx)")}>
        <span className="dot" aria-hidden="true" />
        {status.source.source === "taipan" ? `taipan up · ${status.source.name}` : "env fallback"} &middot;{" "}
        {status.wardryx_url}
      </span>

      {notice && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--mint)" }}>
          {notice}
        </div>
      )}

      {error && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describePolicyError(error, session?.role)}
        </div>
      )}

      {approvals === null || policies === null ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
          loading policy plane...
        </div>
      ) : (
        <HeroBand
          hero={
            <Hero
              cap="Policy · approvals awaiting decision"
              value={pendingCount.toLocaleString("en-US")}
              sub={<>{decidedTodayCount} decided today</>}
            />
          }
          tiles={
            <>
              <KpiTile label="Decided today" value={decidedTodayCount.toLocaleString("en-US")} sub="grants + denies" />
              <KpiTile label="Policies" value={policies.length.toLocaleString("en-US")} sub="read-only · editor in v1" />
            </>
          }
        />
      )}

      <Section title="Decision Stream" right={<FreshBadge variant="live" />}>
        <DecisionStream onOpenAgent={onOpenAgent} />
      </Section>

      <Section
        title="Approvals Inbox"
        right={
          <span className="flex items-center gap-3">
            {approvals !== null && approvals.length > 0 && (
              <ExportButtons
                basename="genaryx-approval-trail"
                columns={APPROVALS_EXPORT_COLUMNS}
                rows={() => approvalsExportRows(approvals)}
                title={`The whole trail: ${approvals.length} approval(s), pending and decided, with the model and org the inbox shows and the reason the plane gave`}
                meta={() =>
                  approvalsExportMeta({
                    total: approvals.length,
                    pending: pendingCount,
                    environment: window.location.host || "unknown",
                    takenAt: new Date().toISOString(),
                  })
                }
              />
            )}
            <FreshBadge variant="auto" detail="20s" />
          </span>
        }
      >
        {approvals === null ? (
          <Loading />
        ) : (
          <ApprovalsInbox
            approvals={approvals}
            onDecide={handleDecide}
            grantedToken={grantedToken}
            onDismissToken={() => setGrantedToken(null)}
            focusApprovalId={focusApprovalId}
            mutedKeys={mutedKeys}
            onToggleMuteAgent={onToggleMuteAgent}
            onOpenAgent={onOpenAgent}
          />
        )}
      </Section>

      <Section
        title="Policies"
        right={
          <span className="flex items-center gap-3">
            {policies !== null && policies.length > 0 && (
              <ExportButtons
                basename="genaryx-policy-inventory"
                columns={POLICIES_EXPORT_COLUMNS}
                rows={() => policiesExportRows(policies)}
                title="The policy STORE only. Rules loaded from a wardryx -policy file are enforced and are not in this list, and the file says so."
                meta={() =>
                  policiesExportMeta({
                    total: policies.length,
                    policyVersion: latestPolicyVersion,
                    environment: window.location.host || "unknown",
                    takenAt: new Date().toISOString(),
                  })
                }
              />
            )}
            <FreshBadge variant="auto" detail="20s" />
          </span>
        }
      >
        {policies === null ? <Loading /> : <PolicyList policies={policies} policyVersion={latestPolicyVersion} />}
      </Section>
    </div>
  );
}
