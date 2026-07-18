/** The "fuse" heat bar - the TokenFuse budget-health metaphor. A filled track
 * whose colour reads mint (healthy) -> amber (warming) -> ember (over) unless a
 * fixed `tone` is given. Pure CSS (`.d-fuse` in index.css), no dependency. */
export function FuseBar({
  fraction,
  tone,
}: {
  fraction: number;
  tone?: "mint" | "amber" | "ember" | "iris";
}) {
  const cls = tone ?? (fraction >= 1 ? "ember" : fraction >= 0.8 ? "amber" : "mint");
  const pct = Math.min(100, Math.max(0, fraction * 100));
  return (
    <div className={`d-fuse ${cls}`}>
      <i style={{ width: `${pct}%` }} />
    </div>
  );
}
