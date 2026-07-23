import { qrcodegen } from "./vendor/qrcodegen";

const { QrCode } = qrcodegen;

export interface QrRender {
  /** Modules per side (odd, `version * 4 + 17`). */
  size: number;
  /** One SVG `<path>` `d` attribute covering every DARK module, one unit
   * per module - see `components/QrCode.tsx` for why this is a single path
   * rather than one `<rect>` per module. */
  pathData: string;
}

/**
 * Render `text` as a QR code, dependency-free (docs/PHASE5.md W2: "use a
 * dependency-free QR approach or a tiny vendored generator, keep it
 * self-contained") via the vendored `qrcodegen` library
 * (`lib/vendor/qrcodegen.ts`, Project Nayuki, MIT). Error correction level
 * MEDIUM (~15% tolerance): enough headroom for a phone camera scan of a
 * desktop-screen QR without inflating the symbol size the way
 * QUARTILE/HIGH would for the Pocket panel's ~150-char
 * `genaryx-pocket://pair/v1?...` content. Version is chosen automatically
 * (smallest that fits, `QrCode.encodeText`'s own contract).
 */
export function renderQr(text: string): QrRender {
  const qr = QrCode.encodeText(text, QrCode.Ecc.MEDIUM);
  const segments: string[] = [];
  for (let y = 0; y < qr.size; y++) {
    for (let x = 0; x < qr.size; x++) {
      if (qr.getModule(x, y)) segments.push(`M${x},${y}h1v1h-1z`);
    }
  }
  return { size: qr.size, pathData: segments.join("") };
}
