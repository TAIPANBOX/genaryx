/**
 * The words for a unit's standing against its central month cap, said once so
 * the Money section and its tests agree. See `crates/api/src/money/units.rs`
 * for where the numbers come from and what they cannot see.
 */

import type { UnitCap, UnitCapState, UnitCaps } from "../moneyTypes";

/** The pill for each state: its words and the `d-pill` class that colours it. */
export const UNIT_CAP_PILL: Record<UnitCapState, { label: string; cls: string; title: string }> = {
  over_cap: {
    label: "over cap",
    cls: "over",
    title: "Spent this month at or above the central cap: every gateway refuses the unit's next call (unit_budget_exceeded).",
  },
  near_cap: { label: "near cap", cls: "near", title: "At or above 80% of the central cap this month." },
  under_cap: { label: "under cap", cls: "live", title: "Under 80% of the central cap this month." },
  no_cap: {
    label: "no central cap",
    cls: "dead",
    title: "The Cloud holds no central cap for this unit. A cap written only in a gateway's identity map file is enforced there and does not reach the Cloud, so it cannot be shown here.",
  },
  cap_unknown: {
    label: "cap unknown",
    cls: "dead",
    title: "The Cloud's cap map could not be read, so whether this unit has a cap is unknown. Not the same as no cap.",
  },
  spend_unknown: {
    label: "spend unknown",
    cls: "near",
    title: "A central cap is set, and the Cloud has no month spend for this unit (or GET /v1/units did not answer).",
  },
};

/** A dollar figure that keeps a micro-dollar cap readable: the forge lab's
 * USD 0.00005 cap printed as $0.0001 by the two-or-four-decimal formatter. */
export function capUsd(value: number): string {
  if (value === 0) return "$0.00";
  const abs = Math.abs(value);
  const decimals = abs < 0.0001 ? 6 : abs < 1 ? 4 : 2;
  return `$${value.toFixed(decimals)}`;
}

/** The cap cell: the cap, or what its absence means. */
export function capLabel(row: UnitCap, data: UnitCaps): string {
  if (row.cap_usd !== null) {
    return row.cap_usd <= 0 ? `${capUsd(row.cap_usd)} · refuses every call` : capUsd(row.cap_usd);
  }
  return data.caps_read ? "none" : "unknown";
}

/** The spend cell: month-to-date, or "not reported", never a zero. */
export function monthSpentLabel(row: UnitCap): string {
  return row.month_spent_usd !== null ? capUsd(row.month_spent_usd) : "not reported";
}

/** The section's right-hand caption. */
export function unitCapsCaption(data: UnitCaps): string {
  const over = data.units.filter((u) => u.state === "over_cap").length;
  const month = data.month ? `month ${data.month} (UTC)` : "month not reported";
  return `${data.units.length} unit(s) · ${over} over cap · ${month}`;
}

/** What the section cannot see, printed under the table. */
export const UNIT_CAPS_CAVEAT =
  "Central caps only: set here or on the control plane, polled by every gateway of the org within seconds. A cap written only in a gateway's identity map file is enforced by that gateway and does not reach the Cloud. Month spend is the Cloud's fold of what the gateways pushed, so it can trail a gateway's own counter. The Cloud has no route that removes a central cap.";
