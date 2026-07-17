/** One Overview tile: a label, a big tabular number, and an optional
 * secondary line. Reused by the Money view's savings breakdown too. */
export function StatTile({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: string;
  sub?: string;
  tone?: string;
}) {
  return (
    <div className="panel px-4 py-3 flex flex-col gap-1.5 min-w-0" style={{ background: "var(--panel-2)" }}>
      <span
        className="mono"
        style={{ fontSize: 10, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}
      >
        {label}
      </span>
      <span
        className="mono tabular truncate"
        style={{ fontSize: 22, fontWeight: 650, color: tone ?? "var(--fg)" }}
        title={value}
      >
        {value}
      </span>
      {sub && (
        <span className="text-[11px] truncate" style={{ color: "var(--dim)" }} title={sub}>
          {sub}
        </span>
      )}
    </div>
  );
}
