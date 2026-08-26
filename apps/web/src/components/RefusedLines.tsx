import { useEffect, useState } from "react";
import { fetchQuarantine, type QuarantinePanel, type QuarantineReason } from "../lib/quarantine";
import { useConsoleStateVersion } from "../lib/consoleState";
import { formatTimestamp } from "../lib/format";

/**
 * The strip above the Bus Explorer saying what this bus refused.
 *
 * # WHY IT RENDERS WHEN THERE IS NOTHING TO REPORT
 *
 * A banner that only appears on trouble teaches nobody that the check exists,
 * and an absent banner is indistinguishable from a check that stopped running.
 * The clean state is one faint line; the unhappy state is loud. That is the
 * same distinction the rest of this console is built on: "nothing was refused"
 * and "nobody looked" must not render the same.
 *
 * It renders NOTHING only when there is no backend to ask, because then there
 * is no claim to make either way.
 */
export function RefusedLines() {
  const [panel, setPanel] = useState<QuarantinePanel | null>(null);
  // Re-read the moment anything app-wide changes what the box would answer,
  // instead of waiting out the slow poll below. On a real box that is a
  // lifecycle command; in the demo it is the storyline switch.
  const version = useConsoleStateVersion();

  useEffect(() => {
    let alive = true;
    const load = async () => {
      const p = await fetchQuarantine();
      if (alive) setPanel(p);
    };
    void load();
    // Slower than the event feed on purpose. A refusal is a producer's
    // configuration, which changes when somebody deploys, not per second.
    const t = setInterval(() => void load(), 30_000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [version]);

  if (!panel) return null;

  // Could not look. The one thing this must never do is fall through to the
  // calm line below.
  if (!panel.measured) {
    return (
      <Strip tone="warn">
        <span style={{ fontWeight: 600 }}>envelope check did not run.</span>{" "}
        {panel.note ?? "The store could not be read."}
      </Strip>
    );
  }

  if (panel.total === 0) {
    return (
      <Strip tone="calm">
        envelope: every line this bus read was accepted
      </Strip>
    );
  }

  return (
    <Strip tone="warn">
      <div style={{ fontWeight: 600, marginBottom: 4 }}>
        {panel.total.toLocaleString("en-US")} line(s) refused by the envelope and NOT on the bus
      </div>
      <div style={{ opacity: 0.85, marginBottom: 6 }}>{panel.note}</div>
      <ul style={{ margin: 0, paddingLeft: 0, listStyle: "none" }}>
        {panel.reasons.map((r) => (
          <ReasonItem key={r.reason} reason={r} />
        ))}
      </ul>
    </Strip>
  );
}

/**
 * One refusal reason, with the two things an operator needs in order to act on
 * it: where a refused line is on disk, and WHEN this last happened.
 *
 * The time is the half that was missing. A count on its own cannot tell a
 * producer that is still broken from one that was fixed an hour ago and left
 * its scar in the store, and both look identical for as long as the store
 * holds the lines. `last_ts` came down the wire the whole time
 * (`crates/core/src/store.rs`, an `Option<String>`) and nothing rendered it.
 *
 * A null `last_ts` says so rather than rendering nothing: an absent line in a
 * list like this reads as recent to anybody scanning it.
 */
export function ReasonItem({ reason: r }: { reason: QuarantineReason }) {
  return (
    <li style={{ marginTop: 6 }}>
      <span style={{ fontWeight: 600 }}>{r.count.toLocaleString("en-US")}x</span> {r.reason}
      <div style={{ opacity: 0.7 }}>
        {r.last_ts ? `last seen ${formatTimestamp(r.last_ts)}` : "last seen: not recorded"}
      </div>
      {r.example_file ? (
        <div style={{ opacity: 0.7 }}>
          {r.example_file}
          {r.example_offset !== null ? ` @ ${r.example_offset}` : ""}
        </div>
      ) : null}
      {/* The head of one refused line. Enough to recognize which
          producer, which is what somebody needs to go and fix it. */}
      {r.raw_excerpt ? (
        <div
          style={{
            opacity: 0.6,
            overflowX: "auto",
            whiteSpace: "nowrap",
            maxWidth: "100%",
          }}
        >
          {r.raw_excerpt}
        </div>
      ) : null}
    </li>
  );
}

/// The warn tone matches `StatsView`'s own `Banner` (`--panel-2` behind
/// `--fg`), because two banners in one console saying the same kind of thing
/// two different ways is how a reader learns to skip one. The amber rule down
/// the left is the only addition, and it is what separates this from the calm
/// line at a glance rather than by reading it.
export function Strip({ tone, children }: { tone: "warn" | "calm"; children: React.ReactNode }) {
  const warn = tone === "warn";
  return (
    <div
      className="mono px-4 shrink-0"
      style={{
        fontSize: warn ? 11.5 : 10.5,
        paddingTop: warn ? 8 : 4,
        paddingBottom: warn ? 8 : 4,
        lineHeight: 1.7,
        color: warn ? "var(--fg)" : "var(--faint)",
        background: warn ? "var(--panel-2)" : "transparent",
        borderLeft: warn ? "2px solid var(--amber)" : "2px solid transparent",
        borderBottom: "1px solid var(--line-2)",
      }}
    >
      {children}
    </div>
  );
}
