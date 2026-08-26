import type { CryptoError } from "../cryptoTypes";
import { describeCryptoError } from "../lib/crypto";
import { cbomComponents } from "../lib/cryptoExport";

const COLUMNS = "1fr 130px 110px 1fr 200px";

/**
 * CBOM inventory (docs/PHASE4.md W1 position 3): the crypto components from
 * `scan_cbom`'s CycloneDX `components[]` array, rendered as a table. The
 * connector keeps this untyped (CycloneDX is a large external schema), so
 * this component reads it tolerantly - an unrecognized/missing field renders
 * as "-", never a fabricated value.
 */
export function CryptoCbomTable({
  value,
  loading,
  error,
}: {
  value: unknown;
  loading: boolean;
  error: CryptoError | null;
}) {
  if (loading) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        scanning...
      </div>
    );
  }
  if (error) {
    return (
      <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
        {describeCryptoError(error)}
      </div>
    );
  }
  if (value === null) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        scan a target to see its crypto-component inventory.
      </div>
    );
  }

  const components = cbomComponents(value);
  if (components.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no crypto components found in the last CBOM scan.
      </div>
    );
  }

  return (
    <div style={{ overflowX: "auto" }}>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["component", "type", "version", "crypto asset type", "primitive / parameter set"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {components.map((c, idx) => (
        <div key={`${c.name ?? "component"}-${idx}`} className="grid items-center gap-3 px-5 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
          <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={c.name}>
            {c.name ?? "(unnamed)"}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
            {c.type ?? "-"}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
            {c.version ?? "-"}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
            {c.cryptoProperties?.assetType ?? "-"}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--faint)" }}>
            {c.cryptoProperties?.algorithmProperties?.primitive ?? "-"}
            {c.cryptoProperties?.algorithmProperties?.parameterSetIdentifier
              ? ` / ${c.cryptoProperties.algorithmProperties.parameterSetIdentifier}`
              : ""}
          </span>
        </div>
      ))}
    </div>
  );
}
