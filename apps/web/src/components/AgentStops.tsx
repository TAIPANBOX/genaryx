import { useEffect, useState } from "react";
import { fetchAgentStops, stopSummary, type StopEntry, type StopsPanel } from "../lib/agentStops";

/**
 * Every time this agent was stopped: when, by whom, and why.
 *
 * # WHY THIS IS A LIST AND NOT A COUNT
 *
 * The Statistics tab already answers "how many", and a count cannot tell
 * twenty-six stops in one afternoon from twenty-six across a quarter. This is
 * the section that says which, and it is the only place in the console where
 * "who pulled the switch" is readable per event rather than as a column total.
 *
 * # THE LIMIT IT STATES RATHER THAN HIDES
 *
 * An operator freeze is enforced by writing an ordinary deny-all policy, so the
 * refusals that follow it arrive as plain `policy_deny` naming nobody. The
 * freeze itself appears here with its operator; its consequences appear as
 * system stops. Without saying that, a reader counts one operator action and
 * concludes a person barely touched this agent.
 */
export function AgentStops({ agentId }: { agentId: string }) {
  const [panel, setPanel] = useState<StopsPanel | null | "loading">("loading");
  const [openTs, setOpenTs] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setPanel("loading");
    setOpenTs(null);
    void (async () => {
      const p = await fetchAgentStops(agentId);
      if (alive) setPanel(p);
    })();
    return () => {
      alive = false;
    };
  }, [agentId]);

  if (panel === "loading") return <Note>loading...</Note>;
  // A failed call, not an answer. The box's own "nothing stored" is a measured
  // panel with total 0, and the two must not render the same.
  if (panel === null) {
    return <Note>the box did not answer, so this agent's stops are not shown.</Note>;
  }
  if (!panel.measured) {
    return <Note>{panel.note ?? "the event store could not be read."}</Note>;
  }
  if (panel.total === 0) {
    return <Note>nothing on the bus has stopped this agent.</Note>;
  }

  return (
    <div className="flex flex-col gap-2">
      <p className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6, margin: 0 }}>
        <span style={{ color: "var(--fg)" }}>
          {panel.total} stop{panel.total === 1 ? "" : "s"}
        </span>
        , {panel.by_operator} named a person.
      </p>

      <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
        {panel.entries.map((e) => (
          <div key={`${e.ts}-${e.type_}`}>
            <button
              type="button"
              onClick={() => setOpenTs(openTs === e.ts ? null : e.ts)}
              className="w-full grid items-center gap-3 px-3 py-1.5 bus-row text-left"
              style={{
                gridTemplateColumns: "132px 128px 1fr 16px",
                background: "transparent",
                border: "none",
                cursor: "pointer",
              }}
              title="Open this stop"
            >
              <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
                {shortTs(e.ts)}
              </span>
              <span
                className="mono text-[10.5px]"
                style={{ color: e.by_operator ? "var(--amber)" : "var(--dim)" }}
              >
                {e.type_}
              </span>
              <span className="mono text-[10.5px] truncate" style={{ color: "var(--dim)" }}>
                {stopSummary(e)}
              </span>
              <span className="mono text-[10px]" style={{ color: "var(--faint)" }}>
                {openTs === e.ts ? "-" : "+"}
              </span>
            </button>
            {openTs === e.ts ? <Detail entry={e} /> : null}
          </div>
        ))}
      </div>

      {/* The note carries what the list cannot show, and it is not optional:
          a freeze's own refusals are plain policy_deny naming nobody, so a
          reader who counts only the operator rows concludes a person barely
          touched this agent. */}
      {panel.note ? (
        <span className="text-[10.5px]" style={{ color: "var(--faint)", lineHeight: 1.55 }}>
          {panel.note}
        </span>
      ) : null}
    </div>
  );
}

function Detail({ entry }: { entry: StopEntry }) {
  return (
    <div
      className="px-3 py-2 flex flex-col gap-1"
      style={{ background: "var(--panel-2)", borderTop: "1px solid var(--line-2)" }}
    >
      <Field label="when" value={entry.ts} />
      <Field label="type" value={entry.type_} />
      <Field label="from" value={entry.source} />
      <Field
        label="by"
        value={entry.actor ?? "the services, no person named"}
        dim={!entry.actor}
      />
      <Field
        label="reason"
        value={entry.reason ?? "not recorded by the producer"}
        dim={!entry.reason}
      />
    </div>
  );
}

function Field({ label, value, dim = false }: { label: string; value: string; dim?: boolean }) {
  return (
    <div className="flex items-baseline gap-2 min-w-0">
      <span
        className="text-[9.5px] uppercase tracking-wider"
        style={{ color: "var(--faint)", minWidth: 52 }}
      >
        {label}
      </span>
      <span
        className="mono text-[11px] break-all"
        style={{ color: dim ? "var(--faint)" : "var(--dim)" }}
      >
        {value}
      </span>
    </div>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[11px]" style={{ color: "var(--faint)" }}>
      {children}
    </span>
  );
}

/** `2026-08-09T10:05:00Z` to `Aug 09 10:05:00`. The full timestamp is one click
 * away in the detail, so the row shows what a person scans by. */
function shortTs(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleString("en-US", {
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}
