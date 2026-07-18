/** Hand-rolled SVG area+line sparkline - no chart dependency, themed through
 * CSS variables so it tracks the active light/dark theme. Stretches to fill
 * its container width; the stroke stays crisp via `vectorEffect`. */
export function Sparkline({
  values,
  stroke = "var(--amber)",
  fill = "color-mix(in srgb, var(--amber) 20%, transparent)",
  dot = "var(--ember)",
  height = 72,
}: {
  values: number[];
  stroke?: string;
  fill?: string;
  dot?: string;
  height?: number;
}) {
  if (values.length < 2) {
    return <div className="d-spark" style={{ height }} aria-hidden="true" />;
  }
  const W = 600;
  const H = 100;
  const pad = 8;
  const max = Math.max(1, ...values);
  const pts = values.map((v, i) => {
    const x = (i / (values.length - 1)) * W;
    const y = H - (v / max) * (H - pad * 2) - pad;
    return [x, y] as const;
  });
  const line = pts.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  const [lx, ly] = pts[pts.length - 1];
  return (
    <svg
      className="d-spark"
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
      style={{ height }}
      aria-hidden="true"
    >
      <polygon points={`0,${H} ${line} ${W},${H}`} fill={fill} />
      <polyline
        points={line}
        fill="none"
        stroke={stroke}
        strokeWidth={2.5}
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
      <circle cx={lx} cy={ly} r={3.5} fill={dot} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}
