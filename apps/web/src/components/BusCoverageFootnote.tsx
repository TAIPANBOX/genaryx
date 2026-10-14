import type { BusCoverage } from "../lib/incidents";

/**
 * The footnote under an incident list: what the bus read could and could not
 * account for. Shared by the Overview's Incident center and the Incidents view,
 * since a quiet card and a blind one look identical without it. It says "on the
 * bus", not "heard from": identity rows arrive through the identity API, not the
 * bus. `extra` appends a sentence about a source the bus does not carry.
 */
export function BusCoverageFootnote({ coverage, extra }: { coverage: BusCoverage; extra?: string }) {
  return (
    <div className="mono px-5" style={{ fontSize: 10, color: "var(--faint)", marginTop: 8 }}>
      {coverage.read === 0
        ? "no events read off the bus, so no plane is accounted for here"
        : `${coverage.incidentRows} of ${coverage.read} bus event(s) are incidents` +
          `${coverage.truncated ? `, capped at ${coverage.limit}` : ""}` +
          ` · on the bus: ${coverage.planes.join(", ")}`}
      {extra ?? ""}
    </div>
  );
}
