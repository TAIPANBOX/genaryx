import { useMemo } from "react";
import { cssVar } from "../lib/cssVars";
import {
  deriveKeyStatus,
  keyStatusRank,
  lastSeenLabel,
  totalCalls,
  type GatewayKeyEntry,
  type GatewayKeysReport,
  type KeyStatus,
} from "../lib/credentials";
import { callsBreakdown, keyWindowSentence, lastSeenSource, strictModeSentence } from "../lib/identityExport";
import { prettyUnit } from "../lib/views";

const COLUMNS = "120px 1fr 110px 1fr 100px 150px 150px";

/** One accent per key status - mirrors `IdentityList.tsx`'s `TYPE_DOT`
 * convention (an arbitrary but consistent existing-token mapping, no new CSS
 * added). The four "issue" statuses (see `lib/credentials.ts::isKeyIssue`)
 * read as `--sev-high`; `never-used`/`stale` are calmer notes, not alarms;
 * `active` is `--mint`, matching every other "all clear" chip in this app. */
const STATUS_TONE: Record<KeyStatus, string> = {
  removed: "var(--faint)",
  dangling: "var(--sev-high)",
  unbound: "var(--sev-high)",
  mismatching: "var(--sev-high)",
  "never-used": "var(--sev-info)",
  stale: "var(--sev-medium)",
  active: "var(--mint)",
};

const STATUS_LABEL: Record<KeyStatus, string> = {
  removed: "removed",
  dangling: "dangling",
  unbound: "unbound",
  mismatching: "mismatching",
  "never-used": "never used",
  stale: "stale",
  active: "active",
};

const STATUS_TITLE: Record<KeyStatus, string> = {
  removed: "not configured, not bound, but carries call history - a fully decommissioned key.",
  dangling: "the identity map still references this key_id, but it is no longer in TOKENFUSE_CLIENT_KEYS.",
  unbound: "configured (a real secret exists) but no identity-map binding matches it.",
  mismatching: "traffic on this key resolved to a different identity/unit than the map expects at least once.",
  "never-used": "configured, but zero calls recorded in either stats window.",
  stale: "last seen more than 7 days ago.",
  active: "no lifecycle concern detected.",
};

/** "created/age" cell: the stamped date plus, when it parses, a relative age
 * in the tooltip - `entry.created` is a plain `"YYYY-MM-DD"`
 * (`crates/api/src/onboard/commands.rs::today`), not a timestamp, so the
 * primary display is the date itself. */
function CreatedCell({ created, nowMs }: { created: string | null; nowMs: number }) {
  if (!created) {
    return <span style={{ color: "var(--faint)" }}>-</span>;
  }
  const parsed = Date.parse(created);
  const title = Number.isFinite(parsed) ? `created ${created}, ${Math.max(0, Math.round((nowMs - parsed) / 86_400_000))}d ago` : created;
  return (
    <span className="mono text-[11.5px]" title={title} style={{ color: "var(--dim)" }}>
      {created}
    </span>
  );
}

/**
 * The two report-level flags, as lines on the page rather than `title=`
 * tooltips, because both change what the columns below them MEAN.
 *
 * `history_available`: "last seen" and "calls" each merge `since_startup` with
 * `history`, and the gateway sends no history block at all when it has no
 * store. In that state a key in heavy use until the last restart shows 0 calls
 * and no last seen, and {@link deriveKeyStatus} derives "never used" for it.
 *
 * `strict_mode`: under `off` the gateway never counts an identity mismatch, so
 * the "mismatching" status cannot fire on anything this process saw. An
 * unchecked column and a clean one look the same.
 *
 * Both were disclosed only in a tooltip until now, and this table gets
 * screenshotted into access reviews. The sentences come from
 * `lib/identityExport.ts` so that the screenshot and a CSV saved from the same
 * table cannot drift apart.
 */
function ReportNotes({ report }: { report: GatewayKeysReport }) {
  return (
    <div className="px-5 pb-2 flex flex-col gap-1">
      <span
        className="text-[11px]"
        style={{ color: report.history_available ? "var(--faint)" : "var(--sev-medium)" }}
      >
        {keyWindowSentence(report)}
      </span>
      <span
        className="text-[11px]"
        style={{ color: report.strict_mode === "off" ? "var(--sev-medium)" : "var(--faint)" }}
      >
        {strictModeSentence(report)}
      </span>
    </div>
  );
}

/** What "never" is scoped to. The label itself comes from
 * `lib/credentials.ts::lastSeenLabel`, which says "never" whenever neither
 * window recorded a call, and on a gateway with no store that is a much
 * narrower statement than the word suggests. */
function lastSeenScope(entry: GatewayKeyEntry, report: GatewayKeysReport): string {
  const source = lastSeenSource(entry);
  if (source !== null) return source;
  return report.history_available ? "in neither window" : "not since gateway start";
}

/**
 * The Credentials card's key table (I15 "key lifecycle health"): one row per
 * `GatewayKeysReport.keys` entry, sorted worst-first by
 * {@link keyStatusRank}, status derived by {@link deriveKeyStatus}. Mirrors
 * `IdentityList.tsx`'s grid-row table shape. Callers only render this once
 * `report.keys.length > 0` (the empty state is the Credentials section's own
 * "no client keys configured" message, mirroring `IdentityView.tsx`'s
 * existing empty-state convention for Remediations).
 */
export function CredentialsKeysTable({
  report,
  nowMs,
}: {
  report: GatewayKeysReport;
  nowMs: number;
}) {
  const rows = useMemo(() => {
    return [...report.keys]
      .map((entry) => ({ entry, status: deriveKeyStatus(entry, report, nowMs) }))
      .sort((a, b) => keyStatusRank(a.status) - keyStatusRank(b.status) || a.entry.key_id.localeCompare(b.entry.key_id));
  }, [report, nowMs]);

  return (
    <>
      <ReportNotes report={report} />
      <div style={{ overflowX: "auto" }}>
        <div
          className="grid gap-3 px-5 py-2"
          style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
        >
          {["status", "key_id", "unit", "agents", "created", "last seen", "calls"].map((label) => (
            <span
              key={label}
              className="mono"
              style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
            >
              {label}
            </span>
          ))}
        </div>
        {rows.map(({ entry, status }) => (
          <div
            key={entry.key_id}
            className="grid items-center gap-3 px-5 py-2.5 bus-row"
            style={{ gridTemplateColumns: COLUMNS }}
          >
            <span className="chip" style={cssVar("dot", STATUS_TONE[status])} title={STATUS_TITLE[status]}>
              <span className="dot" aria-hidden="true" />
              {STATUS_LABEL[status]}
            </span>
            <span className="mono truncate text-[12px]" title={entry.key_id} style={{ color: "var(--fg)" }}>
              {entry.key_id}
            </span>
            <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
              {entry.unit ? prettyUnit(entry.unit) : "-"}
            </span>
            <span
              className="mono truncate text-[11px]"
              style={{ color: "var(--faint)" }}
              title={entry.agents.length > 0 ? entry.agents.join(", ") : undefined}
            >
              {entry.agents.length > 0 ? entry.agents.join(", ") : "-"}
            </span>
            <CreatedCell created={entry.created} nowMs={nowMs} />
            {/* Both of the next two cells print WHICH window their number came
                from underneath it, rather than in a tooltip. "3d ago" out of a
                stored trace and "3d ago" out of this process are different
                claims about a key, and a screenshot of this table is what ends
                up in the review. */}
            <span className="flex flex-col min-w-0">
              <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
                {lastSeenLabel(entry, nowMs)}
              </span>
              <span className="mono truncate text-[10px]" style={{ color: "var(--faint)" }}>
                {lastSeenScope(entry, report)}
              </span>
            </span>
            <span className="flex flex-col min-w-0">
              <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
                {totalCalls(entry).toLocaleString("en-US")}
              </span>
              <span className="mono truncate text-[10px]" style={{ color: "var(--faint)" }} title={callsBreakdown(entry, report)}>
                {callsBreakdown(entry, report)}
              </span>
            </span>
          </div>
        ))}
      </div>
    </>
  );
}
