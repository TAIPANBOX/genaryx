import { useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { getScenario, onScenarioChange, setScenario, type DemoScenario } from "./scenario";

/**
 * The persistent "you are in the demo" cluster: a small, fixed corner pill
 * group, shown from funnel step 2 onward, never on step 1's sign-in mimic,
 * which stays a clean look-alike of the real gate (`SignInStep.tsx` carries
 * its own small disclosure line instead, see that file).
 *
 * Three controls sharing one floating strip:
 * - a plain "Demo, simulated data" pill, always the same;
 * - the calm/incident switcher the mock world simulator reacts to
 *   (`scenario.ts`), reflecting whichever side is live and staying in sync
 *   even if something else ever calls `setScenario` (`onScenarioChange`);
 * - "Reset demo", a full `location.reload()`. Refresh is a clean slate by
 *   design (`scenario.ts`'s own module state resets the same way on
 *   reload), so this needs no explicit teardown of its own.
 *
 * Fixed position, not a portal: this renders as a plain sibling of the
 * console tree from `DemoFunnel.tsx` (outside `.app` entirely), so there is
 * no ancestor that could turn `position: fixed` into anything other than
 * viewport-relative.
 */
export function DemoControls() {
  const [scenario, setLocalScenario] = useState<DemoScenario>(() => getScenario());

  useEffect(() => onScenarioChange(() => setLocalScenario(getScenario())), []);

  return (
    <div
      className="flex items-center gap-2"
      style={{
        position: "fixed",
        right: 14,
        bottom: 14,
        zIndex: 60,
        padding: "7px 8px 7px 10px",
        borderRadius: 999,
        background: "var(--panel)",
        border: "1px solid var(--line-2)",
        boxShadow: "0 18px 48px rgba(28, 20, 8, 0.26), 0 4px 12px rgba(28, 20, 8, 0.16)",
      }}
    >
      <span
        className="chip"
        style={cssVar("dot", "var(--sev-medium)")}
        title="Every number here is generated locally. Nothing leaves your browser."
      >
        <span className="dot" aria-hidden="true" />
        Demo, simulated data
      </span>

      <div
        className="flex items-center"
        style={{ gap: 2, padding: 2, borderRadius: 999, background: "var(--panel-2)", border: "1px solid var(--line)" }}
      >
        <ScenarioButton
          label="Calm"
          active={scenario === "calm"}
          tone="var(--sev-low)"
          onClick={() => setScenario("calm")}
        />
        <ScenarioButton
          label="Incident"
          active={scenario === "incident"}
          tone="var(--sev-high)"
          onClick={() => setScenario("incident")}
        />
      </div>

      <button
        type="button"
        className="icon-btn"
        title="Reset demo: back to the start, clean slate"
        aria-label="Reset demo"
        onClick={() => window.location.reload()}
      >
        <ResetIcon />
      </button>
    </div>
  );
}

/** One side of the calm/incident segmented control. Plain buttons rather
 * than radio inputs: this mirrors `AppHeader.tsx`'s own `NavItemButton`
 * active/inactive treatment (filled `--panel-3` pill for the active side,
 * dim/faint text otherwise) rather than introducing a new control shape. */
function ScenarioButton({
  label,
  active,
  tone,
  onClick,
}: {
  label: string;
  active: boolean;
  tone: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className="mono"
      style={{
        fontSize: 10.5,
        fontWeight: 600,
        letterSpacing: "0.04em",
        padding: "5px 10px",
        borderRadius: 999,
        border: "1px solid transparent",
        background: active ? "var(--panel-3)" : "transparent",
        color: active ? tone : "var(--faint)",
        cursor: "pointer",
      }}
    >
      {label}
    </button>
  );
}

function ResetIcon() {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" aria-hidden="true">
      <path
        d="M4 4v6h6M20 20v-6h-6M5.5 15a7 7 0 1 0 1.3-8.2L4 9.5M18.5 9a7 7 0 0 1-1.3 8.2L20 14.5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
