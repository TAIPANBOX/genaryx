/**
 * The CSV/JSON pair that saves one table out of the Crypto, Quality or
 * Routines panel.
 *
 * It lives beside `cryptoExport.ts` rather than in `components/` because it is
 * the visible half of that module: the rows and the provenance block are
 * decided there, and this is the two buttons that hand them to
 * `lib/download.ts`. Three panels need the identical pair, and three copies of
 * a button that must stay disabled under exactly the same condition is how the
 * fourth copy ends up enabled over an empty table.
 *
 * `disabled` is not cosmetic. `lib/download.ts` writes the provenance block
 * whether or not there are rows, so a click with nothing loaded produces a
 * file whose entire content is a header describing rows it does not have.
 */

/** @param label what the file holds, for the button's own tooltip. */
export function ExportBar({
  onCsv,
  onJson,
  disabled,
  label,
  disabledHint,
}: {
  onCsv: () => void;
  onJson: () => void;
  disabled: boolean;
  label: string;
  disabledHint: string;
}) {
  return (
    <span className="inline-flex items-center gap-1">
      {(["CSV", "JSON"] as const).map((kind) => (
        <button
          key={kind}
          type="button"
          className="mono text-[10.5px] px-2 py-0.5 rounded"
          disabled={disabled}
          onClick={kind === "CSV" ? onCsv : onJson}
          title={disabled ? disabledHint : `Save ${label} as ${kind}, with its provenance block`}
          style={{
            background: "var(--panel-2)",
            color: disabled ? "var(--faint)" : "var(--dim)",
            border: "1px solid var(--line)",
            cursor: disabled ? "default" : "pointer",
          }}
        >
          {kind}
        </button>
      ))}
    </span>
  );
}
