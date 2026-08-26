import { useMemo, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { IDENTITY_TYPES, type IdryxIdentity, type IdryxRemediation } from "../identityTypes";

const COLUMNS = "130px 1fr 90px 130px 110px 80px 60px 70px 70px 150px 150px 120px 180px";

const HEADERS = [
  "type",
  "id",
  "source",
  "owner",
  "runtime",
  "privileged",
  "perms",
  "events",
  "alerts",
  "created",
  "last used",
  "idryx suggests",
  "on_behalf_of",
];

/** Every timestamp and free-text field on `IdryxIdentity` is
 * `#[serde(default)]` on a `String`, so an unrecorded one arrives as `""`.
 * A dash is that emptiness, and the legend below the filter chips says so on
 * the page: "-" under `last used` is idryx having no timestamp, which is a
 * weaker statement than the identity never having been used. */
function Absent() {
  return <span style={{ color: "var(--faint)" }}>-</span>;
}

function TextCell({ value, mono = true }: { value: string; mono?: boolean }) {
  if (value === "") return <Absent />;
  return (
    <span className={`${mono ? "mono " : ""}truncate text-[11.5px]`} title={value} style={{ color: "var(--dim)" }}>
      {value}
    </span>
  );
}

/**
 * One of idryx's own suggestions, opened under the row.
 *
 * `code` is rendered as what it is and nothing more. idryx sends it beside
 * every explanation and NOTHING in this console, in the connector, or in the
 * wire types establishes what it holds, so it is not labelled a fix command,
 * a policy document or a patch. It is shown verbatim, whole, with a line
 * saying the console does not interpret it. Guessing at it here would put a
 * sentence in an operator's head that no evidence supports.
 */
function SuggestionDetail({ label, suggestion }: { label: string; suggestion: IdryxRemediation }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <span className="badge" style={cssVar("tone", suggestion.kind === "rotation" ? "var(--sev-medium)" : "var(--sev-info)")}>
          {suggestion.kind || "(no kind)"}
        </span>
        <span className="mono text-[10px]" style={{ color: "var(--faint)" }}>
          {label}
        </span>
        <span className="mono text-[10px]" style={{ color: "var(--faint)" }}>
          {suggestion.created_at !== "" ? `created ${suggestion.created_at}` : "no created_at from idryx"}
        </span>
      </div>
      <span className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6 }}>
        {suggestion.explanation !== "" ? suggestion.explanation : "no explanation from idryx"}
      </span>
      {suggestion.code !== "" && (
        <>
          <span className="mono text-[10px]" style={{ color: "var(--faint)" }}>
            code, exactly as idryx sent it. This console does not interpret it and makes no claim about what it is for.
          </span>
          <pre
            className="mono text-[11px] px-3 py-2"
            style={{
              color: "var(--fg)",
              background: "var(--panel-2)",
              border: "1px solid var(--line)",
              borderRadius: 8,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              margin: 0,
            }}
          >
            {suggestion.code}
          </pre>
        </>
      )}
    </div>
  );
}

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
 *
 * `runtime`, `created`, `last_used` and idryx's two attached suggestions
 * (`remediation`, `rotation`) are on the wire and were rendered nowhere until
 * 2026-08-26. `last_used` in particular is what an access review is FOR: a
 * privileged service account last used eight months ago is the finding, and
 * this table could not show it. The suggestions open under the row rather
 * than into a tooltip, because `explanation` and `code` are both long enough
 * that a tooltip would truncate them and neither survives a screenshot.
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
  const [openId, setOpenId] = useState<string | null>(null);

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

      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        A dash is a field idryx sent empty, which means it recorded no value. An empty "last used" is not the same
        claim as never used.
      </span>

      {identities.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no identities in this snapshot.
        </div>
      ) : rows.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no identities match the selected type filter.
        </div>
      ) : (
        <div style={{ overflowX: "auto" }}>
          <div
            className="grid gap-3 px-5 py-2"
            style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
          >
            {HEADERS.map((label) => (
              <span
                key={label}
                className="mono"
                style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
              >
                {label}
              </span>
            ))}
          </div>
          {rows.map((i) => {
            const suggestions: [string, IdryxRemediation][] = [
              ...(i.remediation ? ([["remediation", i.remediation]] as [string, IdryxRemediation][]) : []),
              ...(i.rotation ? ([["rotation", i.rotation]] as [string, IdryxRemediation][]) : []),
            ];
            const open = openId === i.id;
            return (
              <div key={i.id}>
              <div className="grid items-center gap-3 px-5 py-2.5 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
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
                <TextCell value={i.owner} />
                <TextCell value={i.runtime} />
                <span>
                  {i.privileged ? (
                    <span className="badge" style={cssVar("tone", "var(--sev-high)")}>
                      yes
                    </span>
                  ) : (
                    <Absent />
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
                <TextCell value={i.created} />
                <TextCell value={i.last_used} />
                <span className="flex items-center gap-1">
                  {suggestions.length === 0 ? (
                    <Absent />
                  ) : (
                    <button
                      type="button"
                      className="chip"
                      aria-expanded={open}
                      style={{ ...cssVar("dot", "var(--sev-info)"), cursor: "pointer" }}
                      onClick={() => setOpenId(open ? null : i.id)}
                    >
                      <span className="dot" aria-hidden="true" />
                      {suggestions.map(([, s]) => s.kind || "(no kind)").join(" + ")}
                    </button>
                  )}
                </span>
                <span
                  className="mono truncate text-[11px]"
                  style={{ color: "var(--faint)" }}
                  title={i.on_behalf_of.length > 0 ? i.on_behalf_of.join(" -> ") : undefined}
                >
                  {i.on_behalf_of.length > 0 ? i.on_behalf_of.join(" -> ") : "-"}
                </span>
              </div>
              {open && suggestions.length > 0 && (
                <div className="px-5 py-3 flex flex-col gap-4" style={{ background: "var(--panel-2)", borderBottom: "1px solid var(--line)" }}>
                  {suggestions.map(([label, s]) => (
                    <SuggestionDetail key={label} label={label} suggestion={s} />
                  ))}
                </div>
              )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
