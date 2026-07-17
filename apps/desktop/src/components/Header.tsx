import { cssVar } from "../lib/cssVars";
import type { EventsSource } from "../lib/recentEvents";

/**
 * Bus Explorer status strip: live event count + data-source chip. Used to be
 * the whole app's header (brand mark + theme toggle included); now that the
 * Bus Explorer is one of three views under `AppShell`/`AppHeader`, this is
 * scoped down to just what is specific to the Bus view itself.
 */
export function BusStatusBar({ count, source }: { count: number; source: EventsSource }) {
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

      {source === "mock" && (
        <span
          className="chip"
          style={cssVar("dot", "var(--sev-medium)")}
          title="No Tauri runtime detected (or the recent_events command failed): showing bundled mock data."
        >
          <span className="dot" aria-hidden="true" />
          mock data
        </span>
      )}

      <span className="mono tabular" style={{ fontSize: 12.5, color: "var(--fg)" }}>
        {count}
        <span style={{ color: "var(--faint)", marginLeft: 6, fontSize: 11 }}>events live</span>
      </span>
    </div>
  );
}
