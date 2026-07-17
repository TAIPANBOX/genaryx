import { useCallback, useState } from "react";
import { formatTimestamp } from "../lib/format";
import { describeMemoryError, recall } from "../lib/memory";
import { RECALL_MODES } from "../memoryTypes";
import type { EngramMemory, MemoryError, RecallMode } from "../memoryTypes";

const COLUMNS = "1fr 80px 90px 150px 1fr 1fr";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
} as const;

/**
 * Recall (docs/PHASE4.md W2 Memory position 2): a query box (+ mode
 * selector cosine/spreading/hybrid, + limit) that runs `recall` on demand
 * and shows the ranked memories, most relevant first (the array already
 * arrives in that order - see `EngramMemory`'s doc comment). Never runs on
 * its own; results are labeled "as of last query". Selecting a row hands
 * its id up to `MemoryProvenance` via `onSelect`.
 */
export function MemoryRecall({
  agentId,
  selectedId,
  onSelect,
}: {
  agentId: string;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<RecallMode>("cosine");
  const [limit, setLimit] = useState(5);
  const [results, setResults] = useState<EngramMemory[] | null>(null);
  const [error, setError] = useState<MemoryError | null>(null);
  const [loading, setLoading] = useState(false);
  const [queriedAtMs, setQueriedAtMs] = useState<number | null>(null);

  const runQuery = useCallback(async () => {
    if (query.trim().length === 0) return;
    setLoading(true);
    setError(null);
    try {
      const hits = await recall(query, limit, mode, agentId);
      setResults(hits);
      setQueriedAtMs(Date.now());
    } catch (err) {
      setError(err as MemoryError);
    } finally {
      setLoading(false);
    }
  }, [query, limit, mode, agentId]);

  return (
    <div className="flex flex-col gap-3">
      <div
        className="panel px-4 py-3 flex flex-wrap items-center gap-2"
        style={{ background: "var(--panel-2)" }}
      >
        <input
          className="mono flex-1"
          style={{ ...FIELD_STYLE, minWidth: 160 }}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="query text"
          spellCheck={false}
          onKeyDown={(e) => {
            if (e.key === "Enter") void runQuery();
          }}
        />
        <select
          className="mono"
          style={FIELD_STYLE}
          value={mode}
          onChange={(e) => setMode(e.target.value as RecallMode)}
          aria-label="recall mode"
        >
          {RECALL_MODES.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <input
          type="number"
          min={1}
          max={50}
          value={limit}
          onChange={(e) => setLimit(Math.min(50, Math.max(1, Number(e.target.value) || 1)))}
          className="mono tabular"
          style={{ ...FIELD_STYLE, width: 56 }}
          aria-label="limit"
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 12px", fontSize: 11 }}
          onClick={() => void runQuery()}
          disabled={loading || query.trim().length === 0}
        >
          {loading ? "Searching..." : "Recall"}
        </button>
      </div>

      {error && (
        <div
          className="panel px-3 py-2 mono text-[11.5px]"
          style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}
        >
          {describeMemoryError(error)}
        </div>
      )}

      {results === null ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          run a query to see ranked memories.
        </div>
      ) : results.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no memories matched.
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            as of last query
            {queriedAtMs !== null ? ` · ${new Date(queriedAtMs).toLocaleTimeString()}` : ""}
          </span>
          <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
            <div
              className="grid gap-3 px-4 py-2"
              style={{
                gridTemplateColumns: COLUMNS,
                borderBottom: "1px solid var(--line-2)",
                background: "var(--panel-2)",
              }}
            >
              {["content", "score", "importance", "timestamp", "actors", "tags"].map((label) => (
                <span
                  key={label}
                  className="mono"
                  style={{
                    fontSize: 10,
                    letterSpacing: "0.08em",
                    textTransform: "uppercase",
                    color: "var(--faint)",
                  }}
                >
                  {label}
                </span>
              ))}
            </div>
            {results.map((m) => {
              const active = m.id === selectedId;
              return (
                <button
                  key={m.id}
                  type="button"
                  onClick={() => onSelect(m.id)}
                  className="grid items-center gap-3 px-4 py-2.5 bus-row w-full text-left"
                  style={{
                    gridTemplateColumns: COLUMNS,
                    background: active
                      ? "color-mix(in srgb, var(--accent) 8%, transparent)"
                      : "transparent",
                    border: "none",
                    cursor: "pointer",
                  }}
                >
                  <span
                    className="truncate text-[12px]"
                    title={`${m.id}: ${m.content}`}
                    style={{ color: "var(--fg)" }}
                  >
                    {m.content}
                  </span>
                  <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
                    {m.score.toFixed(3)}
                  </span>
                  <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
                    {m.importance.toFixed(3)}
                  </span>
                  <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
                    {formatTimestamp(m.timestamp)}
                  </span>
                  <span
                    className="mono truncate text-[11px]"
                    style={{ color: "var(--faint)" }}
                    title={m.actors.join(", ") || undefined}
                  >
                    {m.actors.length > 0 ? m.actors.join(", ") : "-"}
                  </span>
                  <span
                    className="mono truncate text-[11px]"
                    style={{ color: "var(--faint)" }}
                    title={m.tags.join(", ") || undefined}
                  >
                    {m.tags.length > 0 ? m.tags.join(", ") : "-"}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
