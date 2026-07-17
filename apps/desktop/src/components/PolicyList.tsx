import { cssVar } from "../lib/cssVars";
import { formatTimestamp, formatUsd } from "../lib/format";
import type { PolicyRecord } from "../policyTypes";

const COLUMNS = "110px 1fr 1fr 1fr 90px 90px 80px 90px 150px";

/**
 * The Policy view: `WardryxClient::list_policies()`'s read-only list
 * (PHASE2.md Wave 2: "Read-only in MVP - the guarded PUT/DELETE editor is
 * v1"), plus the set-level `policy_version` chip. `GET /v1/policies` itself
 * carries no such field (a bare array, nowhere to put a set-level value -
 * see `policy::commands::policy_list_policies`'s doc comment); `policyVersion`
 * here is the caller's best-effort derivation from the most recently
 * requested approval, honestly labeled when unknown rather than blank.
 */
export function PolicyList({ policies, policyVersion }: { policies: PolicyRecord[]; policyVersion: string | null }) {
  return (
    <div className="flex flex-col gap-2">
      <span className="chip" style={cssVar("dot", "var(--src-wardryx)")}>
        <span className="dot" aria-hidden="true" />
        policy_version {policyVersion ?? "unknown (no approval decided yet)"}
      </span>

      {policies.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no policies configured.
        </div>
      ) : (
        <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
          <div
            className="grid gap-3 px-4 py-2"
            style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
          >
            {["id", "target", "deny tool", "allow domains", "human >$", "deny >$", "steps", "unattested", "updated"].map(
              (label) => (
                <span
                  key={label}
                  className="mono"
                  style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
                >
                  {label}
                </span>
              ),
            )}
          </div>
          {policies.map((p) => (
            <div key={p.id} className="grid items-center gap-3 px-4 py-2.5 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
              <span className="mono truncate text-[12px]" title={p.id} style={{ color: "var(--fg)" }}>
                {p.id}
              </span>
              <span className="mono truncate text-[11.5px]" title={p.target} style={{ color: "var(--dim)" }}>
                {p.target}
              </span>
              <span
                className="mono truncate text-[11px]"
                style={{ color: "var(--dim)" }}
                title={p.deny_tool.join(", ") || undefined}
              >
                {p.deny_tool.length > 0 ? p.deny_tool.join(", ") : "-"}
              </span>
              <span
                className="mono truncate text-[11px]"
                style={{ color: "var(--dim)" }}
                title={p.allow_domains.join(", ") || undefined}
              >
                {p.allow_domains.length > 0 ? p.allow_domains.join(", ") : "-"}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
                {p.require_human_above_usd > 0 ? formatUsd(p.require_human_above_usd) : "-"}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
                {p.deny_above_usd > 0 ? formatUsd(p.deny_above_usd) : "-"}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
                {p.max_steps > 0 ? p.max_steps : "-"}
              </span>
              <span>
                {p.deny_if_unattested ? (
                  <span className="badge" style={cssVar("tone", "var(--sev-high)")}>
                    yes
                  </span>
                ) : (
                  <span style={{ color: "var(--faint)" }}>-</span>
                )}
              </span>
              <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
                {p.updated_at ? formatTimestamp(p.updated_at) : "-"}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
