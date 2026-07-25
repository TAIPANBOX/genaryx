import type { FormEvent } from "react";
import { useEffect, useMemo, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatTimestamp, formatUsd } from "../lib/format";
import { fetchRuns } from "../lib/money";
import { REPLAY_SPEEDS, useReplayClock, type ReplayClock } from "../lib/useReplayClock";
import { fetchRunEvents } from "../lib/replay";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import type { Run } from "../moneyTypes";
import type { UiEvent } from "../types";
import { EventRow } from "./EventRow";

/** Comfortably above the demo generator's biggest single run (a handful of
 * events); large enough that a real production run's whole history still
 * fits in one fetch, matching `Agent360.tsx`'s `EVENTS_LIMIT` rationale. */
const EVENTS_LIMIT = 2_000;

/** The picker renders at most this many rows. A month-scale money plane holds
 * tens of thousands of runs; mapping all of them to DOM rows freezes the tab
 * (that is the "loading..." hang). Show the most recent N here and let the
 * run-ID field below reach any run by id - `run_events` looks it up directly. */
const PICKER_LIMIT = 150;

/** Stable empty-array reference so `useReplayClock`'s `[events]` reset
 * effect never sees a "new" list on every render while nothing has loaded
 * yet - see that hook's own doc comment. */
const EMPTY_EVENTS: readonly UiEvent[] = [];

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

function Empty({ children }: { children: string }) {
  return (
    <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
      {children}
    </div>
  );
}

const PICKER_COLUMNS = "1fr 210px 90px 150px 80px";

function RunPickerRow({ run, onReplay }: { run: Run; onReplay: (runId: string) => void }) {
  return (
    <div className="grid items-center gap-3 px-4 py-2.5 bus-row" style={{ gridTemplateColumns: PICKER_COLUMNS }}>
      <span className="mono truncate text-[12px]" title={run.run_id} style={{ color: "var(--fg)" }}>
        {run.run_id}
      </span>
      <span className="mono truncate text-[11.5px]" title={run.agent_id} style={{ color: "var(--dim)" }}>
        {run.agent_id || "-"}
      </span>
      <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
        {formatUsd(run.spent_usd)}
      </span>
      <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
        {formatTimestamp(run.last_seen)}
      </span>
      <span className="flex justify-end">
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => onReplay(run.run_id)}
        >
          Replay
        </button>
      </span>
    </div>
  );
}

/**
 * The run picker shown when `RunReplayView` has no run selected yet: the
 * Money runs list (`fetchRuns`, same read `MoneyView`/`RunsTable` already
 * make) as the curated primary source, plus a manual run-id field as a
 * direct-entry fallback. The manual field matters beyond convenience: Money
 * is backed by a live TokenFuse Cloud pairing that may not be connected in
 * every environment, while `run_events` reads the console's own local event
 * Store (always seeded, even offline) - so typing an id (e.g. a demo run
 * like `demo-run-000`) keeps Replay usable with no Money plane at all, and
 * doubles as the "run_id passed via deep-link" entry point in a build with
 * no clickable deep-link handy.
 */
function RunPicker({ onReplay }: { onReplay: (runId: string) => void }) {
  const moneyStatus = useMoneyStatus();
  const ready = moneyStatus?.state === "ready";

  const [runs, setRuns] = useState<Run[] | null>(null);
  const [manualId, setManualId] = useState("");

  // Most-recent-first, capped, computed once per runs load (not per keystroke).
  const shown = useMemo(
    () => (runs ? [...runs].sort((a, b) => (b.last_seen ?? "").localeCompare(a.last_seen ?? "")).slice(0, PICKER_LIMIT) : null),
    [runs],
  );

  useEffect(() => {
    if (!ready) return;
    let cancelled = false;
    void fetchRuns()
      .then((r) => {
        if (!cancelled) setRuns(r);
      })
      .catch(() => {
        if (!cancelled) setRuns([]);
      });
    return () => {
      cancelled = true;
    };
  }, [ready]);

  const submitManual = (e: FormEvent) => {
    e.preventDefault();
    const id = manualId.trim();
    if (id) onReplay(id);
  };

  return (
    <div className="flex flex-col gap-6">
      <section className="flex flex-col gap-2">
        <SectionHeader title="Pick a run - from Money" />
        {!ready ? (
          <Empty>
            {!moneyStatus || moneyStatus.state === "bootstrapping"
              ? "connecting to the money plane..."
              : "money plane not connected - no curated runs list; use a run ID directly below."}
          </Empty>
        ) : runs === null ? (
          <Empty>loading...</Empty>
        ) : runs.length === 0 ? (
          <Empty>no runs yet.</Empty>
        ) : (
          <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
            <div
              className="grid gap-3 px-4 py-2"
              style={{ gridTemplateColumns: PICKER_COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
            >
              {["run", "agent", "spent", "last seen", ""].map((label) => (
                <span
                  key={label || "spacer"}
                  className="mono"
                  style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
                >
                  {label}
                </span>
              ))}
            </div>
            {runs.length > (shown?.length ?? 0) && (
              <div
                className="px-4 py-2 mono text-[11px]"
                style={{ color: "var(--faint)", borderBottom: "1px solid var(--line-2)" }}
              >
                showing the {shown?.length ?? 0} most recent of {runs.length.toLocaleString("en-US")} runs - type a run ID below for any other
              </div>
            )}
            {(shown ?? []).map((r) => (
              <RunPickerRow key={r.run_id} run={r} onReplay={onReplay} />
            ))}
          </div>
        )}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Or enter a run ID directly" />
        <form className="flex items-center gap-2" onSubmit={submitManual}>
          <input
            type="text"
            value={manualId}
            onChange={(e) => setManualId(e.target.value)}
            placeholder="e.g. demo-run-000"
            className="mono"
            style={{
              width: 280,
              fontSize: 11.5,
              background: "var(--panel-2)",
              border: "1px solid var(--line-2)",
              borderRadius: 6,
              padding: "5px 8px",
              color: "var(--fg)",
            }}
          />
          <button
            type="submit"
            className="icon-btn"
            style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
            disabled={manualId.trim() === ""}
          >
            Replay
          </button>
        </form>
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          Looks up the run directly in the console's own event store (Store::events_for_run) - works even without a
          connected Money plane.
        </span>
      </section>
    </div>
  );
}

function speedLabel(speed: number): string {
  return `${speed}x`;
}

function PlaybackControls({ clock, count }: { clock: ReplayClock; count: number }) {
  return (
    <div className="flex flex-wrap items-center gap-3">
      {clock.atEnd ? (
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 12px", fontSize: 12 }}
          onClick={clock.restart}
        >
          Restart
        </button>
      ) : (
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 12px", fontSize: 12 }}
          onClick={clock.toggle}
          disabled={count === 0}
          aria-label={clock.playing ? "Pause" : "Play"}
        >
          {clock.playing ? "Pause" : "Play"}
        </button>
      )}
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
        onClick={clock.reset}
        disabled={count === 0 || (clock.revealedCount === 0 && !clock.playing)}
      >
        Reset
      </button>

      <input
        type="range"
        min={0}
        max={count}
        step={1}
        value={clock.revealedCount}
        onChange={(e) => clock.seek(Number(e.target.value))}
        style={{ flex: 1, minWidth: 160, accentColor: "var(--sev-medium)" }}
        aria-label="Scrub position"
        disabled={count === 0}
      />

      <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)", minWidth: 60, textAlign: "right" }}>
        {clock.revealedCount} / {count}
      </span>

      <span className="inline-flex items-center gap-1" role="group" aria-label="Playback speed">
        {REPLAY_SPEEDS.map((s) => {
          const active = clock.speed === s;
          return (
            <button
              key={s}
              type="button"
              className="icon-btn"
              aria-pressed={active}
              style={{
                width: "auto",
                padding: "0 8px",
                fontSize: 10.5,
                color: active ? "var(--fg)" : "var(--dim)",
                borderColor: active ? "var(--line-2)" : undefined,
                background: active ? "var(--panel-3)" : undefined,
              }}
              onClick={() => clock.setSpeed(s)}
            >
              {speedLabel(s)}
            </button>
          );
        })}
      </span>
    </div>
  );
}

/**
 * One run's playback: fetches its events once (`fetchRunEvents`), then hands
 * them to [`useReplayClock`] and renders whatever is currently revealed
 * through the same `EventRow` the Bus Explorer uses (so a replayed event
 * looks exactly like it would live, including the same raw-NDJSON/expand
 * detail). Revealed events render oldest-first, top to bottom, matching
 * `events_for_run`'s own chronological contract - reading top-down IS
 * watching the run happen.
 */
function RunPlayback({
  runId,
  onOpenAgent,
  onPickDifferentRun,
}: {
  runId: string;
  onOpenAgent: (agentId: string) => void;
  onPickDifferentRun: () => void;
}) {
  const [events, setEvents] = useState<UiEvent[] | null>(null);
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(new Set());

  useEffect(() => {
    let cancelled = false;
    setEvents(null);
    setExpanded(new Set());
    void fetchRunEvents(runId, EVENTS_LIMIT).then((e) => {
      if (!cancelled) setEvents(e);
    });
    return () => {
      cancelled = true;
    };
  }, [runId]);

  const clock = useReplayClock(events ?? EMPTY_EVENTS);

  const toggleExpand = (id: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const count = events?.length ?? 0;
  const visible = (events ?? []).slice(0, clock.revealedCount);
  const cursorTs = clock.revealedCount > 0 && events ? events[clock.revealedCount - 1].ts : null;
  // Every event of one run shares that run's acting agent (`replay.rs`'s own
  // grounded test asserts this against the demo generator); the first
  // revealed - or, before anything is revealed, the first fetched - event's
  // `agent_id` is enough to offer an Agent 360 deep link without waiting for
  // playback to start.
  const runAgentId = events && events.length > 0 ? events[0].agent_id : null;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-qryx)")}>
          <span className="dot" aria-hidden="true" />
          run {runId}
        </span>
        {runAgentId && (
          <button
            type="button"
            className="chip"
            style={{ cursor: "pointer" }}
            title={`Open Agent 360 for ${runAgentId}`}
            onClick={() => onOpenAgent(runAgentId)}
          >
            {runAgentId}
          </button>
        )}
        {cursorTs && (
          <span className="chip" style={cssVar("dot", "var(--faint)")}>
            <span className="dot" aria-hidden="true" />
            at {formatTimestamp(cursorTs)}
          </span>
        )}
        <div className="flex-1" />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={onPickDifferentRun}
        >
          Pick a different run
        </button>
      </div>

      {events === null ? (
        <Empty>loading...</Empty>
      ) : events.length === 0 ? (
        <Empty>no events for this run.</Empty>
      ) : (
        <>
          <PlaybackControls clock={clock} count={count} />
          <div className="panel thin-scroll" style={{ background: "var(--panel)", overflow: "auto", maxHeight: "56vh" }}>
            {visible.length === 0 ? (
              <Empty>press Play or drag the scrubber to reveal this run's timeline.</Empty>
            ) : (
              visible.map((e) => <EventRow key={e.id} event={e} expanded={expanded.has(e.id)} onToggle={() => toggleExpand(e.id)} />)
            )}
          </div>
        </>
      )}
    </div>
  );
}

/**
 * Run Replay (docs/PHASE3.md W4, position 5): pick a run - from the Money
 * runs list, a manual run ID, or a `presetRunId` handed in from a deep-link
 * entry point (a run row's Replay button, or Agent 360's Money section) -
 * then step through its events in the order they happened: play/pause, a
 * scrub slider (event i of N, with the current position's timestamp shown
 * alongside), and a speed control, the same scrub/speed mental model the
 * it-rat2 site sims use. `presetRunId` only SEEDS the initial selection (the
 * component owns `selectedRunId` after that); `AppShell.tsx` remounts this
 * component via `key={replayRunId}` whenever a NEW entry point fires, so a
 * fresh preset always takes effect.
 */
export function RunReplayView({
  presetRunId,
  onOpenAgent,
}: {
  presetRunId: string | null;
  onOpenAgent: (agentId: string) => void;
}) {
  const [selectedRunId, setSelectedRunId] = useState<string | null>(presetRunId);

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-col gap-1">
        <span style={{ fontSize: 13, color: "var(--fg)" }}>Run Replay</span>
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          Step through one run's events in the order they happened.
        </span>
      </div>

      {selectedRunId === null ? (
        <RunPicker onReplay={setSelectedRunId} />
      ) : (
        <RunPlayback runId={selectedRunId} onOpenAgent={onOpenAgent} onPickDifferentRun={() => setSelectedRunId(null)} />
      )}
    </div>
  );
}
