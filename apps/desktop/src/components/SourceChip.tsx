import { cssVar } from "../lib/cssVars";
import { SOURCES, type SourceId } from "../types";

const SOURCE_VAR: Record<SourceId, string> = {
  tokenfuse: "var(--src-tokenfuse)",
  wardryx: "var(--src-wardryx)",
  engram: "var(--src-engram)",
  verdryx: "var(--src-verdryx)",
  mockryx: "var(--src-mockryx)",
  qryx: "var(--src-qryx)",
};

function isKnownSource(value: string): value is SourceId {
  return (SOURCES as readonly string[]).includes(value);
}

/** Source chip: a colored dot (the same accent each service uses on its
 * it-rat2 page) plus its name, monospace. Falls back to a neutral dot for
 * any source outside the six known emitting planes. */
export function SourceChip({ source }: { source: string }) {
  const tone = isKnownSource(source) ? SOURCE_VAR[source] : "var(--faint)";
  return (
    <span className="chip" style={cssVar("dot", tone)}>
      <span className="dot" aria-hidden="true" />
      {source}
    </span>
  );
}
