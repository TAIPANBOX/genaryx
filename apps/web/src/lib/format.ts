/** Money figures: 2 decimals normally, 4 for sub-dollar amounts (TokenFuse
 * spend is routinely sub-cent - `$0.0012`, not `$0.00`). Callers pair this
 * with the `mono tabular` classes so a column of these lines up. */
export function formatUsd(value: number): string {
  const decimals = value !== 0 && Math.abs(value) < 1 ? 4 : 2;
  return `$${value.toFixed(decimals)}`;
}

/** Compact 24-hour "hh:mm" clock - the freshness-badge detail format
 * (`FreshBadge.tsx`'s `snapshot`/`onDemand` variants: "SNAPSHOT · 14:32"),
 * deliberately shorter than `formatTimestamp`'s full table-row precision. */
export function formatHm(ms: number): string {
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "-";
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
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

/** A store's file size, or the Memory panel's honest "no real file" label.
 * `null` covers both an in-memory store and a file that has not been
 * created yet (`genaryx_connectors::EngramStats.db_size_bytes`'s doc
 * comment) - both render the same "in-memory / n/a", never a fabricated
 * `0`. */
export function formatBytes(bytes: number | null): string {
  if (bytes === null) return "in-memory / n/a";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(value < 10 ? 2 : 1)} ${units[unitIndex]}`;
}
