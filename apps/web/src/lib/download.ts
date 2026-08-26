/**
 * Saving a table out of the console, as CSV or JSON.
 *
 * # WHY THE PROVENANCE BLOCK IS NOT OPTIONAL
 *
 * A file that leaves this console stops being a view and becomes a document.
 * Somebody will paste it into a deck, and by then nobody can tell which box it
 * came from, what window it covered, or how much of it the console could not
 * attribute. Every export here therefore carries a `meta` block, and the CSV
 * writes it as leading `#` lines rather than dropping it: a spreadsheet shows
 * those as ordinary rows, which is exactly what should happen. A silently
 * bare table is the failure mode this module exists to prevent.
 *
 * # WHY THIS ONE IS SHARED AND THE OTHER THREE ARE NOT
 *
 * This is not the console's only download, and it is the newest of four.
 * `lib/evidence.ts` saves a signed evidence pack, `lib/remote.ts` an issued
 * operator WireGuard config, and `demo/wgDemoConfig.ts` the demo's copy of
 * one, each through the same Blob and `<a download>` shape as
 * [`triggerDownload`] below. This module is the only one that saves a TABLE,
 * and the only one carrying the provenance block above, and those two facts
 * are the same fact: a signed zip and a `.conf` are single artefacts that name
 * themselves and carry their own signature or their own peer, while a table of
 * numbers says nothing at all about where it came from. So the other three are
 * deliberately left alone rather than folded in here, and a view saving a
 * TABLE goes through this helper rather than growing a fifth private copy.
 *
 * The sentence this replaced said the reverse: that no other module in
 * `apps/web/src` reached for `createObjectURL` at all. Three did, and all
 * three calls were already in the tree on the day this file was written
 * (`git log -S 'URL.createObjectURL('`: evidence.ts 2026-07-23, remote.ts and
 * wgDemoConfig.ts 2026-07-25, this file 2026-08-10). Same shape as the faults
 * CLAUDE.md's invariant 7 collects: the claim was true of the intent, and
 * nothing ever ran it against the tree. `download.test.ts` runs it now, which
 * is why the retraction above describes the old sentence instead of quoting
 * it.
 */

/** Provenance for one export. Every field is something a reader of the file,
 * days later and elsewhere, cannot recover on their own. */
export interface ExportMeta {
  /** What the file contains, in a few words. */
  subject: string;
  /** Which console/environment produced it. */
  environment: string;
  /** When it was taken (ISO 8601). */
  takenAt: string;
  /** One line per data window, e.g. "spend: TokenFuse Cloud, its own
   * retention" and "counts: this console's bus, since it started". Separate
   * lines because they are separate windows, and a single "period" field would
   * force a false answer. */
  windows: string[];
  /** Anything the console could not attribute, stated rather than omitted. */
  caveats?: string[];
}

function metaLines(meta: ExportMeta): string[] {
  const lines = [
    `subject: ${meta.subject}`,
    `environment: ${meta.environment}`,
    `taken_at: ${meta.takenAt}`,
    ...meta.windows.map((w) => `window: ${w}`),
    ...(meta.caveats ?? []).map((c) => `caveat: ${c}`),
  ];
  return lines;
}

/** RFC 4180 quoting: wrap in quotes when the value carries a comma, a quote or
 * a newline, and double any embedded quote.
 *
 * `null` and `undefined` become an EMPTY cell, never the strings "null" or
 * "0". An empty cell reads as "not recorded", which is what those values mean
 * here; a 0 would be a measurement nobody took. */
function csvCell(value: unknown): string {
  if (value === null || value === undefined) return "";
  const s = String(value);
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function triggerDownload(filename: string, mime: string, body: string): void {
  const url = URL.createObjectURL(new Blob([body], { type: `${mime};charset=utf-8` }));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Revoked on the next tick rather than immediately: Safari has been known to
  // cancel an in-flight download when the object URL is revoked synchronously.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

/** Build the CSV text (exported for tests: the file writing itself needs a DOM,
 * the quoting and the provenance block do not). */
export function toCsv<T>(
  columns: { key: keyof T & string; header: string }[],
  rows: T[],
  meta: ExportMeta,
): string {
  const head = metaLines(meta).map((l) => `# ${l}`);
  const header = columns.map((c) => csvCell(c.header)).join(",");
  const body = rows.map((r) => columns.map((c) => csvCell(r[c.key])).join(","));
  return [...head, header, ...body].join("\n") + "\n";
}

/** Build the JSON text (exported for the same reason as [`toCsv`]). */
export function toJson<T>(rows: T[], meta: ExportMeta): string {
  return JSON.stringify({ meta, rows }, null, 2) + "\n";
}

export function downloadCsv<T>(
  filename: string,
  columns: { key: keyof T & string; header: string }[],
  rows: T[],
  meta: ExportMeta,
): void {
  triggerDownload(filename, "text/csv", toCsv(columns, rows, meta));
}

export function downloadJson<T>(filename: string, rows: T[], meta: ExportMeta): void {
  triggerDownload(filename, "application/json", toJson(rows, meta));
}
