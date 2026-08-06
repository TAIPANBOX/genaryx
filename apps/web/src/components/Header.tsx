import type { BusMode } from "../lib/busStatus";
import { cssVar } from "../lib/cssVars";
import type { EventsSource } from "../lib/recentEvents";

/**
 * The chip that says whether these events are real.
 *
 * Deliberately loud in demo mode and quiet in live mode: a fabricated stream
 * shown without a warning is the failure worth preventing, while a correct
 * live console should not shout about being correct. It names the
 * environment when live, so an operator with several of them can see which
 * one is on screen without opening a settings panel.
 *
 * Renders nothing when the mode is unknown (no backend to ask), rather
 * than guessing.
 */
function BusModeChip({ mode }: { mode: BusMode | null }) {
  if (!mode) return null;

  if (mode.kind === "demo") {
    return (
      <span
        className="chip"
        style={cssVar("dot", "var(--sev-medium)")}
        title={`No environment found under ~/.taipan/environments, so these events are generated, not real. Fixtures in ${mode.dir}.`}
      >
        <span className="dot" aria-hidden="true" />
        demo data
      </span>
    );
  }

  if (mode.kind === "unavailable") {
    return (
      <span
        className="chip"
        style={cssVar("dot", "var(--sev-high)")}
        title={`The event bus could not be opened: ${mode.reason}`}
      >
        <span className="dot" aria-hidden="true" />
        bus unavailable
      </span>
    );
  }

  return (
    <span
      className="chip"
      style={cssVar("dot", "var(--sev-info)")}
      title={`Tailing the real event files of environment "${mode.env}" at ${mode.dir}.`}
    >
      <span className="dot" aria-hidden="true" />
      {mode.env}
    </span>
  );
}

/**
 * Bus Explorer status strip: live event count + data-source chips. Used to be
 * the whole app's header (brand mark + theme toggle included); now that the
 * Bus Explorer is one of three views under `AppShell`/`AppHeader`, this is
 * scoped down to just what is specific to the Bus view itself.
 */
export function BusStatusBar({
  count,
  source,
  mode,
}: {
  count: number;
  source: EventsSource;
  mode: BusMode | null;
}) {
  return (
    <div
      className="flex items-center gap-3 px-4 shrink-0"
      style={{ height: 40, borderBottom: "1px solid var(--line)", background: "var(--panel-2)" }}
    >
      <span
        className="mono"
        style={{ fontSize: 10, letterSpacing: "0.14em", textTransform: "uppercase", color: "var(--faint)" }}
      >
        Bus Explorer
      </span>

      <div className="flex-1" />

      <BusModeChip mode={mode} />

      {source === "mock" && (
        <span
          className="chip"
          style={cssVar("dot", "var(--sev-medium)")}
          title="No backend is configured, so these rows are bundled sample data, not this box's records."
        >
          <span className="dot" aria-hidden="true" />
          mock data
        </span>
      )}

      {/* A real backend that did not answer. Distinct from the mock chip on
          purpose: the two used to be the same state, and that is what let a
          failing box show a fixture stream under a "mock data" label. There
          are no rows behind this one, and that is the honest answer. */}
      {source === "error" && (
        <span
          className="chip"
          style={cssVar("dot", "var(--sev-high)")}
          title="The box did not answer recent_events. No events are shown, because there are none to show."
        >
          <span className="dot" aria-hidden="true" />
          no answer from the box
        </span>
      )}

      <span className="mono tabular" style={{ fontSize: 12.5, color: "var(--fg)" }}>
        {count}
        <span style={{ color: "var(--faint)", marginLeft: 6, fontSize: 11 }}>events live</span>
      </span>
    </div>
  );
}
