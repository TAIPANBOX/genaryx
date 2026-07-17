import { cssVar } from "../lib/cssVars";
import type { MoneyError } from "../moneyTypes";

/**
 * Renders a `ConnectorError::PlanRequired` rejection as an upsell, never as
 * an error toast (spec). The upgrade URL is shown as plain selectable text
 * rather than a clickable link: this app runs inside a Tauri webview, whose
 * external-link handling is not something this task can verify without
 * launching the GUI, so showing the URL as text is the choice that cannot
 * silently fail to open anything.
 */
export function UpsellBanner({ error }: { error: Extract<MoneyError, { kind: "plan_required" }> }) {
  return (
    <div
      className="panel px-4 py-3 flex items-center gap-3"
      style={{
        background: "color-mix(in srgb, var(--sev-medium) 10%, var(--panel-2))",
        borderColor: "color-mix(in srgb, var(--sev-medium) 45%, var(--line-2))",
      }}
    >
      <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
        upgrade
      </span>
      <div className="flex flex-col min-w-0 gap-0.5">
        <span className="text-[12.5px]" style={{ color: "var(--fg)" }}>
          {error.feature} is not available on {error.org}&rsquo;s current plan.
        </span>
        <span className="mono text-[11px] truncate" style={{ color: "var(--dim)" }} title={error.upgrade_url}>
          {error.upgrade_url}
        </span>
      </div>
    </div>
  );
}
