/** Money figures: 2 decimals normally, 4 for sub-dollar amounts (TokenFuse
 * spend is routinely sub-cent - `$0.0012`, not `$0.00`). Callers pair this
 * with the `mono tabular` classes so a column of these lines up. */
export function formatUsd(value: number): string {
  const decimals = value !== 0 && Math.abs(value) < 1 ? 4 : 2;
  return `$${value.toFixed(decimals)}`;
}

/** Compact "Jul 16 14:32:05" clock for table rows - same
 * hours:minutes:seconds precision as `EventRow.tsx`'s `formatClock`, plus a
 * date since Money/Overview data is not assumed to all be from today. */
export function formatTimestamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const datePart = d.toLocaleDateString(undefined, { month: "short", day: "2-digit" });
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  return `${datePart} ${hh}:${mm}:${ss}`;
}
