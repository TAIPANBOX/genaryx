import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeMemoryError, fetchStats } from "../lib/memory";
import { useMemoryStatus } from "../lib/useMemoryStatus";
import type { EngramStats, MemoryError, MemoryStatus } from "../memoryTypes";
import { MemoryProvenance } from "./MemoryProvenance";
import { MemoryRecall } from "./MemoryRecall";
import { MemoryStats as StoreStats } from "./MemoryStats";
import { MemoryTimeline } from "./MemoryTimeline";

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

function Loading() {
  return (
    <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
      loading...
    </div>
  );
}

/**
 * Shared "not ready yet" rendering for the Memory view - mirrors
 * `QualityView.tsx`'s local `QualityEmptyState`'s honest, distinct states
 * (never a generic spinner-forever or error toast), Engram-flavored: still
 * spawning `engram-mcp`, no memory plane configured (no binary and/or no
 * real `.engram` store resolved - see `memory::env`'s doc comment for why
 * those are not distinguished here), or a resolved binary+db pair whose
 * spawn/handshake failed.
 */
function MemoryEmptyState({ status }: { status: MemoryStatus | null }) {
  if (!status || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          starting engram-mcp...
        </div>
      </div>
    );
  }

  if (status.state === "no_environment") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 560 }}>
          <span style={{ fontSize: 13, color: "var(--fg)" }}>No memory plane found</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
            Engram needs BOTH <span style={{ color: "var(--fg)" }}>engram-mcp</span> (on{" "}
            <span style={{ color: "var(--fg)" }}>PATH</span>, at{" "}
            <span style={{ color: "var(--fg)" }}>~/.taipan/bin/engram-mcp</span>, or in a{" "}
            <span style={{ color: "var(--fg)" }}>~/Development/engram/.venv</span> checkout) AND a real,
            already-written <span style={{ color: "var(--fg)" }}>.engram</span> store. Park (or symlink) the store
            at <span style={{ color: "var(--fg)" }}>~/.taipan/.engram</span> for the console to auto-discover it.
          </span>
        </div>
      </div>
    );
  }

  if (status.state === "unreachable") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 560 }}>
          <span style={{ fontSize: 13, color: "var(--sev-high)" }}>Could not start engram-mcp</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.engram_mcp_bin || "(no binary resolved)"} &middot; {status.db_path || "(no store resolved)"}
          </span>
          <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.reason}
          </span>
        </div>
      </div>
    );
  }

  // `status.state === "ready"`: callers only render this component when NOT
  // ready, so this branch is unreachable in practice.
  return null;
}

/**
 * The Memory panel (docs/PHASE4.md W2): store stats, an on-demand recall
 * query, why/provenance for a selected memory (plus the irreversible
 * `forget` action), and a live timeline - over the ONE long-lived
 * `engram-mcp` connection `src-tauri/src/memory/state.rs` spawns once and
 * keeps alive for the app's whole lifetime. Mirrors `QualityView.tsx`'s
 * overall shape (status hook, empty state, section layout, no periodic
 * auto-refresh - `engram-mcp`'s store only changes when an agent writes to
 * it externally, so a timer here would mostly be a no-op; the explicit
 * Refresh button re-reads current stats instead).
 *
 * `agentId` is one shared "agent scope" field (rather than one per section):
 * `EngramClient::stats`/`recall` both accept the SAME kind of optional
 * `agent_id` scope, and this console never picks a default at spawn time
 * (see `memory::state`'s module doc) - so this one field is the whole
 * scoping surface both Store and Recall read from, letting an operator
 * inspect a specific agent's memory without restarting anything.
 */
export function MemoryView({ onOpenAgent }: { onOpenAgent: (agentId: string) => void }) {
  const status = useMemoryStatus();
  const ready = status?.state === "ready";

  const [agentId, setAgentId] = useState("");
  const [stats, setStats] = useState<EngramStats | null>(null);
  const [statsError, setStatsError] = useState<MemoryError | null>(null);
  const [asOfMs, setAsOfMs] = useState<number | null>(null);

  const [selectedId, setSelectedId] = useState<string | null>(null);

  const loadStats = useCallback(async () => {
    if (!ready) return;
    try {
      const s = await fetchStats(agentId);
      setStats(s);
      setAsOfMs(Date.now());
      setStatsError(null);
    } catch (err) {
      setStatsError(err as MemoryError);
    }
  }, [ready, agentId]);

  useEffect(() => {
    void loadStats();
  }, [loadStats]);

  const onForgotten = useCallback(() => {
    setSelectedId(null);
    void loadStats();
  }, [loadStats]);

  if (!ready) {
    return <MemoryEmptyState status={status} />;
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-engram)")}>
          <span className="dot" aria-hidden="true" />
          {status.engram_mcp_bin} &middot; {status.db_path}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          as of load{asOfMs !== null ? ` · fetched ${new Date(asOfMs).toLocaleTimeString()}` : ""}
        </span>
        <div className="flex-1" />
        <input
          className="mono"
          style={{
            background: "var(--panel-2)",
            border: "1px solid var(--line-2)",
            borderRadius: 8,
            padding: "5px 9px",
            fontSize: 11.5,
            color: "var(--fg)",
            width: 220,
          }}
          value={agentId}
          onChange={(e) => setAgentId(e.target.value)}
          placeholder="agent scope (optional)"
          spellCheck={false}
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => void loadStats()}
        >
          Refresh
        </button>
      </div>

      {statsError && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describeMemoryError(statsError)}
        </div>
      )}

      <section className="flex flex-col gap-2">
        <SectionHeader title="Store" />
        {stats === null ? <Loading /> : <StoreStats stats={stats} />}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Recall" />
        <MemoryRecall agentId={agentId} selectedId={selectedId} onSelect={setSelectedId} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Why / Provenance" />
        <MemoryProvenance memoryId={selectedId} onForgotten={onForgotten} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Timeline" />
        <MemoryTimeline onOpenAgent={onOpenAgent} />
      </section>
    </div>
  );
}
