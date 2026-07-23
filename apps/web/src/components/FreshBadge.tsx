/** FreshBadge - the freshness-grammar pill every dashboard section header
 * wears in its `right` slot (`dash.tsx`'s `Section`). Six states, same
 * grammar everywhere: `STATE` or `STATE · detail`.
 *
 * - `live`     push stream (bus/SSE) - mint, dot pulses via CSS animation.
 * - `auto`     REST poll on a schedule - iris, e.g. `AUTO · 20s`.
 * - `snapshot` stands until an explicit Refresh/Rescan - dim, e.g.
 *              `SNAPSHOT · 14:32`.
 * - `onDemand` only moves when the operator triggers it (Scan/Run/Build) -
 *              faint, e.g. `ON-DEMAND · 13:05` or bare `ON-DEMAND` before
 *              anything has ever run.
 * - `window`   accumulated aggregate over a period - amber, e.g.
 *              `WINDOW · history`.
 * - `paused`   stream buffering, not losing - amber, no pulse, e.g.
 *              `PAUSED · 47`.
 *
 * Colors come only from the existing `--mint`/`--iris`/`--amber`/`--dim`/
 * `--faint` tokens (`index.css`); the dot uses `currentColor` so it always
 * matches the tone class. The live pulse goes static automatically under
 * the app-wide `prefers-reduced-motion` reset already in `index.css`
 * (`* { animation: none !important; }`), so no separate handling is needed
 * here. */
import type { KeyboardEvent, ReactNode } from "react";

export type FreshVariant = "live" | "auto" | "snapshot" | "onDemand" | "window" | "paused";

const LABEL: Record<FreshVariant, string> = {
  live: "LIVE",
  auto: "AUTO",
  snapshot: "SNAPSHOT",
  onDemand: "ON-DEMAND",
  window: "WINDOW",
  paused: "PAUSED",
};

const TONE: Record<FreshVariant, string> = {
  live: "mint",
  auto: "iris",
  snapshot: "dim",
  onDemand: "faint",
  window: "amber",
  paused: "amber",
};

function keyActivate(onClick: () => void) {
  return (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onClick();
    }
  };
}

export function FreshBadge({
  variant,
  detail,
  title,
  onClick,
}: {
  variant: FreshVariant;
  /** Trailing detail after the " · " separator - a period ("20s"), a clock
   * ("14:32"), a window name ("history"), or a buffered-event count ("47").
   * Omit for a bare label (e.g. plain "ON-DEMAND" before anything has ever
   * run). */
  detail?: ReactNode;
  title?: string;
  onClick?: () => void;
}) {
  return (
    <span
      className={`d-fresh ${TONE[variant]}${onClick ? " clk" : ""}`}
      title={title}
      onClick={onClick}
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={onClick ? keyActivate(onClick) : undefined}
    >
      <span className={`dot${variant === "live" ? " pulse" : ""}`} aria-hidden="true" />
      {LABEL[variant]}
      {detail != null && <span className="d-fresh-detail"> · {detail}</span>}
    </span>
  );
}
