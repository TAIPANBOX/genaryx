import { formatBytes } from "../lib/format";
import type { EngramStats } from "../memoryTypes";
import { StatTile } from "./StatTile";

/**
 * Store stats (docs/PHASE4.md W2 Memory position 1): episodic/semantic/
 * procedural counts (`procedural` labeled "not implemented in this Engram
 * version" - see `EngramCounts`'s doc comment - never a real `0`), fact
 * validity (active vs superseded), entities, reflections, vector-index
 * size, and the db path + size (`db_size_bytes: null` renders
 * "in-memory / n/a" via `formatBytes`, never a fabricated `0`).
 */
export function MemoryStats({ stats }: { stats: EngramStats }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(3, minmax(0, 1fr))" }}>
        <StatTile label="Episodic" value={String(stats.counts.episodic)} />
        <StatTile label="Semantic" value={String(stats.counts.semantic)} />
        <StatTile
          label="Procedural"
          value="not implemented"
          sub="this Engram version has no procedural store"
        />
      </div>
      <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(4, minmax(0, 1fr))" }}>
        <StatTile label="Facts active" value={String(stats.facts_active)} />
        <StatTile
          label="Facts superseded"
          value={String(stats.facts_superseded)}
          sub={`${stats.facts_total} total`}
        />
        <StatTile label="Entities" value={String(stats.entities)} />
        <StatTile label="Reflections" value={String(stats.reflections)} />
      </div>
      <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(2, minmax(0, 1fr))" }}>
        <StatTile label="Vector index size" value={String(stats.vector_index_size)} />
        <StatTile label="Store" value={formatBytes(stats.db_size_bytes)} sub={stats.db_path} />
      </div>
      <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
        scope: {stats.agent_id ?? "server default (none set)"}
      </span>
    </div>
  );
}
