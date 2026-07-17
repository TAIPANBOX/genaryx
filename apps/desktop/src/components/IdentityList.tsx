import { useMemo, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { IDENTITY_TYPES, type IdryxIdentity } from "../identityTypes";

const COLUMNS = "130px 1fr 90px 130px 90px 60px 70px 70px 200px";

/** One accent per identity type, reusing the existing per-source palette
 * (`--src-*`) rather than inventing new CSS tokens - arbitrary but
 * consistent, matching how `cssVar()` lets any call site supply a color
 * without touching `index.css`. */
const TYPE_DOT: Record<string, string> = {
  human: "var(--src-engram)",
  service_account: "var(--src-verdryx)",
  key: "var(--src-mockryx)",
  agent: "var(--src-qryx)",
  mcp_server: "var(--src-tokenfuse)",
};

function TypeChip({ type }: { type: string }) {
  return (
    <span className="chip" style={cssVar("dot", TYPE_DOT[type] ?? "var(--faint)")}>
      <span className="dot" aria-hidden="true" />
      {type || "(unknown)"}
    </span>
  );
}

/** Type-filter chips: empty selection means no filter (show every type);
 * clicking a chip narrows the list to the union of selected types - same
 * "select to narrow" convention a faceted filter normally uses. */
function TypeFilterChips({
  active,
  onToggle,
}: {
  active: ReadonlySet<string>;
  onToggle: (t: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter by identity type">
      {IDENTITY_TYPES.map((t) => {
        const on = active.has(t);
        return (
          <button
            key={t}
            type="button"
            className="chip"
            style={{ ...cssVar("dot", TYPE_DOT[t] ?? "var(--faint)"), cursor: "pointer", opacity: on ? 1 : 0.45 }}
            aria-pressed={on}
            onClick={() => onToggle(t)}
          >
            <span className="dot" aria-hidden="true" />
            {t}
          </button>
        );
      })}
    </div>
  );
}

/**
 * The Identities list (docs/PHASE3.md W2): `GET /api/identities`, with
 * type-filter chips (human/service_account/key/agent/mcp_server). `events`/
 * `alerts` render as explicit labeled COUNTS, never as if they were
 * clickable object lists (idryx: "events and alerts are integer COUNTS,
 * not the objects" - `crates/connectors/src/idryx.rs`). `on_behalf_of`
 * renders root-first, exactly as idryx returns it.
 */
export function IdentityList({
  identities,
  onOpenAgent,
}: {
  identities: IdryxIdentity[];
  /** Phase-3 wave-3 deep link (docs/PHASE3.md W3): opens the Agent 360 card
   * for a row's `id`. */
  onOpenAgent: (agentId: string) => void;
}) {
  const [activeTypes, setActiveTypes] = useState<ReadonlySet<string>>(new Set());

  const toggleType = (t: string) => {
    setActiveTypes((prev) => {
      const next = new Set(prev);
      if (next.has(t)) next.delete(t);
      else next.add(t);
      return next;
    });
  };

  const rows = useMemo(
    () => (activeTypes.size === 0 ? identities : identities.filter((i) => activeTypes.has(i.type))),
    [identities, activeTypes],
  );

  return (
    <div className="flex flex-col gap-2">
      <TypeFilterChips active={activeTypes} onToggle={toggleType} />

      {identities.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no identities in this snapshot.
        </div>
      ) : rows.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no identities match the selected type filter.
        </div>
      ) : (
        <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
          <div
            className="grid gap-3 px-4 py-2"
            style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
          >
            {["type", "id", "source", "owner", "privileged", "perms", "events", "alerts", "on_behalf_of"].map(
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
          {rows.map((i) => (
            <div key={i.id} className="grid items-center gap-3 px-4 py-2.5 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
              <TypeChip type={i.type} />
              <button
                type="button"
                className="mono truncate text-[12px] text-left"
                title={`Open Agent 360 for ${i.id}`}
                style={{ color: "var(--fg)", background: "none", border: "none", padding: 0, cursor: "pointer" }}
                onClick={() => onOpenAgent(i.id)}
              >
                {i.id}
              </button>
              <span className="mono truncate text-[11px]" style={{ color: "var(--dim)" }}>
                {i.source}
              </span>
              <span className="mono truncate text-[11.5px]" title={i.owner || undefined} style={{ color: "var(--dim)" }}>
                {i.owner || "-"}
              </span>
              <span>
                {i.privileged ? (
                  <span className="badge" style={cssVar("tone", "var(--sev-high)")}>
                    yes
                  </span>
                ) : (
                  <span style={{ color: "var(--faint)" }}>-</span>
                )}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
                {i.permissions.length}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }} title="event count, not the objects">
                {i.events}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }} title="alert count, not the objects">
                {i.alerts}
              </span>
              <span
                className="mono truncate text-[11px]"
                style={{ color: "var(--faint)" }}
                title={i.on_behalf_of.length > 0 ? i.on_behalf_of.join(" -> ") : undefined}
              >
                {i.on_behalf_of.length > 0 ? i.on_behalf_of.join(" -> ") : "-"}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
