import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeMemoryError, fetchStats } from "../lib/memory";
import { useMemoryStatus } from "../lib/useMemoryStatus";
import { formatHm } from "../lib/format";
import type { EngramStats, MemoryError, MemoryStatus } from "../memoryTypes";
import { MemoryProvenance } from "./MemoryProvenance";
import { MemoryRecall } from "./MemoryRecall";
import { MemoryStats as StoreStats } from "./MemoryStats";
import { MemoryTimeline } from "./MemoryTimeline";
import { FreshBadge } from "./FreshBadge";
import { Hero, HeroBand, KpiTile, Section } from "./dash";

/** Store stats poll cadence for the `AUTO · 20s` section badge. The
 * original one-shot-on-mount fetch undersold itself here: `engram-mcp`'s
 * store can change from an agent writing to it at any time, same class of
 * gap Quality's `verdryx.db` had (design spec section 7 parity fix #4) - so
 * this view polls too, mirroring `QualityView.tsx`'s identical fix, to keep
 * the badge's promised cadence honest. */
const REFRESH_INTERVAL_MS = 20_000;

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
 * `engram-mcp` connection `crates/api/src/memory/state.rs` spawns once and
 * keeps alive for the app's whole lifetime. Mirrors `QualityView.tsx`'s
 * overall shape (status hook, empty state, section layout) including its
 * [`REFRESH_INTERVAL_MS`] poll: `engram-mcp`'s store can change from an
 * agent writing to it at any time, so Store stats poll on a schedule (badge
 * `AUTO · 20s`) on top of the explicit Refresh button.
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
  const [recallAtMs, setRecallAtMs] = useState<number | null>(null);

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
    const id = window.setInterval(() => void loadStats(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [loadStats]);

  const onForgotten = useCallback(() => {
    setSelectedId(null);
    void loadStats();
  }, [loadStats]);

  if (!ready) {
    return <MemoryEmptyState status={status} />;
  }

  const hhmm = asOfMs !== null ? formatHm(asOfMs) : undefined;
  const recallHhmm = recallAtMs !== null ? formatHm(recallAtMs) : undefined;
  const totalMemories = stats ? stats.counts.episodic + stats.counts.semantic : 0;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-engram)")}>
          <span className="dot" aria-hidden="true" />
          {status.engram_mcp_bin} &middot; {status.db_path}
        </span>
        <FreshBadge variant="auto" detail="20s" title={hhmm !== undefined ? `last read ${hhmm}` : undefined} />
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
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeMemoryError(statsError)}
        </div>
      )}

      {stats === null ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
          loading store stats...
        </div>
      ) : (
        <HeroBand
          hero={
            <Hero
              cap="Memory · engram store"
              value={totalMemories.toLocaleString("en-US")}
              sub={<>{stats.facts_active} facts active</>}
            />
          }
          tiles={
            <>
              <KpiTile
                label="Facts active"
                value={stats.facts_active.toLocaleString("en-US")}
                sub={`${stats.facts_total.toLocaleString("en-US")} total · ${stats.facts_superseded.toLocaleString("en-US")} superseded`}
              />
              <KpiTile
                label="Entities"
                value={stats.entities.toLocaleString("en-US")}
                sub={`${stats.reflections.toLocaleString("en-US")} reflections`}
              />
            </>
          }
        />
      )}

      <Section title="Store" right={<FreshBadge variant="auto" detail="20s" />}>
        {stats === null ? <Loading /> : <StoreStats stats={stats} />}
      </Section>

      <Section title="Recall" right={<FreshBadge variant="onDemand" detail={recallHhmm} />}>
        <MemoryRecall agentId={agentId} selectedId={selectedId} onSelect={setSelectedId} onQueried={setRecallAtMs} />
      </Section>

      <Section title="Why / Provenance" right={<FreshBadge variant="onDemand" />}>
        <MemoryProvenance memoryId={selectedId} onForgotten={onForgotten} />
      </Section>

      <Section title="Timeline" right={<FreshBadge variant="live" />}>
        <MemoryTimeline onOpenAgent={onOpenAgent} />
      </Section>
    </div>
  );
}
