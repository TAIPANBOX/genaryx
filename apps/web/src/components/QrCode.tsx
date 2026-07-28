import { useMemo } from "react";
import { renderQr } from "../lib/qr";

/**
 * Renders `value` as a scannable QR code. The vendored
 * `qrcodegen` encoder (`lib/vendor/qrcodegen.ts`) produces a module matrix,
 * folded here into ONE SVG `<path>` covering every dark module (the
 * standard efficient QR-as-SVG technique: one `Mx,yh1v1h-1z` subpath per
 * dark module, filled with the default non-zero winding rule so adjacent
 * modules never visually merge or cancel) rather than one `<rect>` per
 * module - a ~50-module-per-side QR can have well over a thousand dark
 * modules, and one path node is far cheaper for the DOM than that many
 * elements.
 *
 * Colors are hardcoded white background / black modules, NEVER the app's
 * theme CSS variables: a QR code must keep strong light/dark contrast to
 * scan at all, and this app's dark theme's own palette would risk rendering
 * light-on-light or otherwise low-contrast modules.
 */
export function QrCode({
  value,
  size = 220,
  label = "QR code",
}: {
  value: string;
  size?: number;
  /** What the code encodes, for screen readers. Callers know; this file does not. */
  label?: string;
}) {
  const { size: modules, pathData } = useMemo(() => renderQr(value), [value]);
  // A 4-module quiet zone on every side (the QR spec's minimum) so a camera's
  // own edge detection has margin to lock onto the symbol.
  const quiet = 4;
  const viewBoxSize = modules + quiet * 2;

  return (
    <svg
      viewBox={`0 0 ${viewBoxSize} ${viewBoxSize}`}
      width={size}
      height={size}
      role="img"
      aria-label={label}
      style={{ background: "#fff", borderRadius: 8, display: "block" }}
    >
      <g transform={`translate(${quiet},${quiet})`}>
        <path d={pathData} fill="#000" />
      </g>
    </svg>
  );
}
