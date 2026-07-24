import type { IdentityStatus } from "../identityTypes";
import { cssVar } from "../lib/cssVars";
import { describePolicyError } from "../lib/policy";
import type { FindingState, PostureFinding } from "../lib/posture";
import { usePostureData } from "../lib/usePostureData";
import type { MoneyStatus } from "../moneyTypes";
import type { PolicyStatus } from "../policyTypes";
import { SeverityBadge } from "./SeverityBadge";

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

function moneyStatusLabel(status: MoneyStatus | null): string {
  if (!status) return "connecting...";
  switch (status.state) {
    case "bootstrapping":
      return "connecting...";
    case "no_environment":
      return "no environment";
    case "pairing_failed":
      return "pairing failed";
    case "ready":
      return status.source.source === "taipan" ? `taipan up . ${status.source.name}` : "env fallback";
  }
}

function policyStatusLabel(status: PolicyStatus | null): string {
  if (!status) return "connecting...";
  switch (status.state) {
    case "bootstrapping":
      return "connecting...";
    case "no_environment":
      return "no policy plane";
    case "unreachable":
      return "unreachable";
    case "ready":
      return status.source.source === "taipan" ? `taipan up . ${status.source.name}` : "env fallback";
  }
}

/** Phase-3 W4 addition: a third plane chip, identity-flavored - mirrors
 * `moneyStatusLabel`/`policyStatusLabel` exactly. Unlike those two,
 * `IdentityStatus`'s `Ready.source` has only ever the one `"taipan"`
 * variant (idryx has no bearer of its own to gate an env-var fallback on,
 * see `identityTypes.ts`), so there is no second branch to label. */
function identityStatusLabel(status: IdentityStatus | null): string {
  if (!status) return "connecting...";
  switch (status.state) {
    case "bootstrapping":
      return "connecting...";
    case "no_environment":
      return "no identity plane";
    case "unreachable":
      return "unreachable";
    case "ready":
      return `taipan up . ${status.source.name}`;
  }
}

/** Small connectivity chip, one per plane Posture reads from - context for
 * WHY a given zond below might read "n/a" (that plane not ready yet)
 * without duplicating either panel's own full empty-state treatment
 * (`MoneyEmptyState`/`PolicyEmptyState` already own that). */
function PlaneChip({ label, ready, text }: { label: string; ready: boolean; text: string }) {
  return (
    <span className="chip" style={cssVar("dot", ready ? "var(--sev-low)" : "var(--faint)")}>
      <span className="dot" aria-hidden="true" />
      {label}: {text}
    </span>
  );
}

/** The finding's status badge: its real (PHASE2.md-assigned) severity when
 * it has actually fired, a calm "OK" when the check passed, or a neutral
 * "n/a" while the signal it needs is not available yet - never a fake
 * severity for the latter two, matching `SeverityBadge`'s own "never look
 * more assured than the data actually is" spirit. */
function FindingStatusBadge({ state, severity }: { state: FindingState; severity: PostureFinding["severity"] }) {
  if (state === "triggered") return <SeverityBadge severity={severity} />;
  if (state === "unknown") {
    return (
      <span className="badge" style={cssVar("tone", "var(--faint)")}>
        n/a
      </span>
    );
  }
  return (
    <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
      OK
    </span>
  );
}

function FindingRow({ finding }: { finding: PostureFinding }) {
  return (
    <div className="panel px-3 py-2.5 flex flex-col gap-2" style={{ background: "var(--panel-2)" }}>
      <div className="flex items-center gap-3">
        <FindingStatusBadge state={finding.state} severity={finding.severity} />
        <span className="text-[12.5px]" style={{ color: "var(--fg)" }}>
          {finding.title}
        </span>
      </div>
      <span className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6 }}>
        {finding.whyItMatters}
      </span>
      {finding.state === "triggered" && (
        <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
          fix: <span style={{ color: "var(--fg)" }}>{finding.howToFix}</span>
        </span>
      )}
    </div>
  );
}

/**
 * Posture-lite (docs/PHASE2.md Wave 3) + Posture full (docs/PHASE3.md W4,
 * position 6) + I3's "Connection & credential health" group: a read-only
 * sidebar view listing the 4 v0 stack-sanity zonds, the 5 identity-plane
 * zonds, and the 11 connection/credential-health zonds
 * (`lib/posture.ts`'s `computeStackPostureFindings`/
 * `computeIdentityPostureFindings`/`computeConnectionHealthFindings`),
 * computed entirely from signals this app already fetches elsewhere - the
 * resolved env sources (same status hooks the Policy/Money/Identity/
 * Quality/Crypto/Memory/Drills/Copilot/Remote/Overview views already use),
 * `policy_list_policies()`/`policy_list_approvals()` (same commands
 * `PolicyView.tsx`'s own Policies/Approvals sections call),
 * `identity_list_identities`/`identity_list_alerts` (same commands
 * `IdentityView.tsx`/`Agent360.tsx` already call), `money_runs` (same
 * command `MoneyView.tsx`/`OverviewView.tsx` already call), and the live bus
 * (the same `fetchRecentEvents` + `bus:event` listener pattern
 * `DecisionStream.tsx`/`BusExplorer.tsx` already follow, unfiltered here
 * since "schema mix" and "bus stale" are properties of the WHOLE bus, not
 * just the wardryx slice). No new backend command, no new connector call.
 *
 * All of that data-fetching now lives in `lib/usePostureData.ts` (extracted
 * from this component during I3 so `OverviewView.tsx`'s Incident Center, I2,
 * can consume the SAME live findings instead of re-deriving its own copy);
 * this component is just that hook's renderer.
 *
 * Deliberately never gated behind a single-plane "ready" check the way
 * `PolicyView`/`OverviewView`/`IdentityView` are: Posture's whole point is
 * reading across multiple, independently-failing planes at once, so a down
 * Wardryx or a not-yet-connected idryx must never blank the whole panel -
 * each zond just reports its own honest `unknown` state until the signal it
 * specifically needs is available (see `posture.ts`'s doc comment).
 */
export function PostureView() {
  const { moneyStatus, policyStatus, identityStatus, policiesError, stackFindings, identityFindings, connectionFindings } =
    usePostureData();

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <PlaneChip label="money" ready={moneyStatus?.state === "ready"} text={moneyStatusLabel(moneyStatus)} />
        <PlaneChip label="policy" ready={policyStatus?.state === "ready"} text={policyStatusLabel(policyStatus)} />
        <PlaneChip label="identity" ready={identityStatus?.state === "ready"} text={identityStatusLabel(identityStatus)} />
      </div>

      {policiesError && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describePolicyError(policiesError)}
        </div>
      )}

      <section className="flex flex-col gap-2">
        <SectionHeader title="Stack posture" />
        <div className="flex flex-col gap-2">
          {stackFindings.map((f) => (
            <FindingRow key={f.id} finding={f} />
          ))}
        </div>
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Identity + Wardryx admin" />
        <div className="flex flex-col gap-2">
          {identityFindings.map((f) => (
            <FindingRow key={f.id} finding={f} />
          ))}
        </div>
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Connection & credential health" />
        <div className="flex flex-col gap-2">
          {connectionFindings.map((f) => (
            <FindingRow key={f.id} finding={f} />
          ))}
        </div>
      </section>
    </div>
  );
}
