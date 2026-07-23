import { useMemo } from "react";
import { cssVar } from "../lib/cssVars";
import { sortAccessRowsWorstFirst, type AccessRow, type PermissionRollup, type PolicyOverlay } from "../lib/access";

const COLUMNS = "1fr 70px 70px 110px 110px 100px 100px 110px 150px";

/** "Unused" cell (I5 spec, verbatim): a badge showing the unused-permission
 * count, escalated (`--sev-high`) when an unused ADMIN permission is among
 * them, calm (`--sev-medium`) for a plain unused count, mint for a clean
 * 0/0, and - the honesty gate - neutral/faint whenever this identity carries
 * no usage signal at all, regardless of how many permissions are nominally
 * `used === false` (see `lib/access.ts`'s `PermissionRollup.hasUsageSignal`
 * doc comment: idryx's own `least_privilege` detector stays silent in
 * exactly this case, and this table must not look more assured than that). */
function UnusedBadge({ rollup }: { rollup: PermissionRollup }) {
  if (!rollup.hasUsageSignal) {
    return (
      <span
        className="badge"
        style={cssVar("tone", "var(--faint)")}
        title="idryx has recorded no usage signal for this identity's permissions - an unused count here would not be a meaningful least-privilege signal."
      >
        {rollup.unused.length} &middot; no signal
      </span>
    );
  }
  if (rollup.adminUnused.length > 0) {
    return (
      <span
        className="badge"
        style={cssVar("tone", "var(--sev-high)")}
        title={`unused admin permission(s): ${rollup.adminUnused.map((p) => p.name).join(", ")}`}
      >
        {rollup.unused.length}
      </span>
    );
  }
  if (rollup.unused.length > 0) {
    return (
      <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
        {rollup.unused.length}
      </span>
    );
  }
  return (
    <span className="badge" style={cssVar("tone", "var(--mint)")}>
      0
    </span>
  );
}

/** Shared "the wardryx column group could not be confirmed" cell - never a
 * bare `0`/`-` that would read the same as "checked, and there is none". */
function PlaneUnavailableCell() {
  return (
    <span
      className="mono text-[11px]"
      style={{ color: "var(--faint)" }}
      title="policy plane unavailable - could not confirm this agent's Wardryx overlay"
    >
      unavailable
    </span>
  );
}

function DomainsCell({ overlay }: { overlay: PolicyOverlay }) {
  const eff = overlay.allowDomains.effective;
  if (eff.kind === "unrestricted") {
    return (
      <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
        unrestricted
      </span>
    );
  }
  const contradiction = eff.domains.length === 0;
  return (
    <span
      className="mono tabular text-[12px]"
      style={{ color: contradiction ? "var(--sev-high)" : "var(--fg)" }}
      title={
        contradiction
          ? "matched policies each restrict domains but share none in common - every domain-declaring action from this agent is denied"
          : eff.domains.join(", ")
      }
    >
      {eff.domains.length}
    </span>
  );
}

function FlagsCell({ overlay }: { overlay: PolicyOverlay }) {
  if (!overlay.denyIfUnattested && overlay.maxSteps === null) {
    return <span style={{ color: "var(--faint)" }}>-</span>;
  }
  return (
    <div className="flex items-center gap-1.5">
      {overlay.denyIfUnattested && (
        <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
          unattested
        </span>
      )}
      {overlay.maxSteps !== null && (
        <span className="mono text-[11px]" style={{ color: "var(--dim)" }}>
          max {overlay.maxSteps}
        </span>
      )}
    </div>
  );
}

/**
 * The Access matrix's fleet table (I5): one row per agent identity, worst
 * first ({@link sortAccessRowsWorstFirst} - shadow desc, then unused-admin
 * desc, then unused desc), mirroring `CredentialsKeysTable.tsx`'s own
 * worst-first convention. Clicking a row opens the existing Agent 360
 * overlay for that agent, the same deep link `IdentityList.tsx` already
 * wires its id column to. Every `AccessRow.policy === null` cell (the
 * denied-tools/domains/flags group) renders {@link PlaneUnavailableCell}
 * instead of a bare zero - `rows` is expected to be uniformly all-`null` or
 * all-populated for that field (one shared policies fetch), never a mix.
 */
export function AccessMatrixTable({
  rows,
  onOpenAgent,
}: {
  rows: readonly AccessRow[];
  /** Phase-3 wave-3 deep link (docs/PHASE3.md W3): opens the Agent 360 card
   * for a row's agent id - same prop every other table in this view already
   * takes. */
  onOpenAgent: (agentId: string) => void;
}) {
  const sorted = useMemo(() => sortAccessRowsWorstFirst(rows), [rows]);

  if (sorted.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no agent identities in this snapshot.
      </div>
    );
  }

  return (
    <div style={{ overflowX: "auto" }}>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["agent", "granted", "used", "unused", "mcp sanctioned", "mcp shadow", "denied tools", "domains", "flags"].map(
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
      {sorted.map((row) => (
        <div
          key={row.identity.id}
          className="grid items-center gap-3 px-5 py-2.5 bus-row"
          style={{ gridTemplateColumns: COLUMNS, cursor: "pointer" }}
          role="button"
          tabIndex={0}
          title={`Open Agent 360 for ${row.identity.id}`}
          onClick={() => onOpenAgent(row.identity.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              onOpenAgent(row.identity.id);
            }
          }}
        >
          <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={row.identity.id}>
            {row.identity.id}
          </span>
          <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
            {row.permissions.granted}
          </span>
          <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
            {row.permissions.used}
          </span>
          <UnusedBadge rollup={row.permissions} />
          <span
            className="mono tabular text-[12px]"
            style={{ color: "var(--dim)" }}
            title={row.mcpReach.sanctionedServers.map((s) => s.serverId).join(", ") || undefined}
          >
            {row.mcpReach.sanctionedTools.length}
          </span>
          <span
            className="mono tabular text-[12px]"
            style={{ color: row.mcpReach.shadowTools.length > 0 ? "var(--sev-high)" : "var(--dim)" }}
            title={
              [
                row.mcpReach.shadowServers.map((s) => s.serverId).join(", "),
                row.agentShadowToolAlertCount > 0 ? `${row.agentShadowToolAlertCount} agent_shadow_tool alert(s)` : "",
              ]
                .filter(Boolean)
                .join(" - ") || undefined
            }
          >
            {row.mcpReach.shadowTools.length}
          </span>
          {row.policy === null ? (
            <PlaneUnavailableCell />
          ) : (
            <span
              className="mono tabular text-[12px]"
              style={{ color: "var(--dim)" }}
              title={row.policy.overlay.deniedTools.join(", ") || undefined}
            >
              {row.policy.overlay.deniedTools.length}
            </span>
          )}
          {row.policy === null ? <PlaneUnavailableCell /> : <DomainsCell overlay={row.policy.overlay} />}
          {row.policy === null ? <PlaneUnavailableCell /> : <FlagsCell overlay={row.policy.overlay} />}
        </div>
      ))}
    </div>
  );
}
