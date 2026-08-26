import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatHm, formatTimestamp } from "../lib/format";
import {
  fetchRoutinesHistory,
  fetchRoutinesStatus,
  latestDetailLine,
  latestRelativeTime,
  recordStatusTone,
  ROUTINE_STATUS_LABEL,
  ROUTINE_STATUS_TONE,
  sortRoutinesWorstFirst,
  toUiStatus,
} from "../lib/routines";
import {
  ROUTINE_HISTORY_EXPORT_COLUMNS,
  routineHistoryExportMeta,
  routineHistoryExportRows,
} from "../lib/cryptoExport";
import { ExportBar } from "../lib/cryptoExportBar";
import { downloadCsv, downloadJson } from "../lib/download";
import type { RoutineSummaryDto, RoutinesHistoryDto, RoutinesStatusDto } from "../routinesTypes";
import { FreshBadge } from "./FreshBadge";
import { Hero, HeroBand, KpiTile, Section } from "./dash";

const SUMMARY_COLUMNS = "150px 80px 100px 100px 1fr";
const HISTORY_COLUMNS = "90px 150px 150px 60px 1fr";

/** Age-based tick, same rationale as `lib/usePostureData.ts`'s `NOW_TICK_MS`
 * / `IdentityView.tsx`'s Credentials-card interval: drives the "last run"
 * relative-time column without needing a fresh fetch just to re-evaluate
 * staleness. */
const NOW_TICK_MS = 5_000;

function Loading() {
  return (
    <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
      loading...
    </div>
  );
}

/**
 * Honest empty state for when `$STACK_UP_HOME/routines` does not exist yet -
 * mirrors `IdentityView.tsx`/`QualityView.tsx`'s own `*EmptyState`
 * convention. Routines are scheduled and run by stack-up's own
 * `routines.sh` on the box, NOT by this console (see `RoutinesView`'s own
 * doc comment for the read-only non-goal stated plainly) - this explains
 * that up front rather than showing a bare "not found".
 */
function RoutinesEmptyState({ routinesDir }: { routinesDir: string }) {
  return (
    <div className="flex-1 min-h-0 flex items-center justify-center px-6">
      <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 560 }}>
        <span style={{ fontSize: 13, color: "var(--fg)" }}>No routines directory yet</span>
        <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
          Routines run ON THE BOX via stack-up&apos;s own <span style={{ color: "var(--fg)" }}>routines.sh</span>, on
          a schedule you install with{" "}
          <span style={{ color: "var(--fg)" }}>./routines.sh install</span>. This console never installs,
          uninstalls, or runs a routine itself - it only surfaces what <span style={{ color: "var(--fg)" }}>
            routines.sh
          </span>{" "}
          already recorded. Once{" "}
          <span className="mono" style={{ color: "var(--fg)" }}>
            {routinesDir}
          </span>{" "}
          exists (after the first scheduled run, or <span style={{ color: "var(--fg)" }}>routines.sh run &lt;name&gt;</span>{" "}
          by hand), this tab shows it.
        </span>
      </div>
    </div>
  );
}

/** One routine's status chip - worst-first tone via `lib/routines.ts`'s
 * shared classification, so this can never disagree with the table's own
 * sort order. `title` surfaces the read failure message honestly when the
 * status file exists but could not be parsed. */
function RoutineStatusChip({ row }: { row: RoutineSummaryDto }) {
  const status = toUiStatus(row);
  return (
    <span className="chip" style={cssVar("dot", ROUTINE_STATUS_TONE[status])} title={row.latest_error ?? undefined}>
      <span className="dot" aria-hidden="true" />
      {ROUTINE_STATUS_LABEL[status]}
    </span>
  );
}

/**
 * The five-routine summary table (I7b), worst-first via
 * `sortRoutinesWorstFirst`. Clicking a row selects it for the history panel
 * below - mirrors `QualityRunsList.tsx`'s identical click-to-select
 * convention.
 */
function RoutinesSummaryTable({
  routines,
  selectedName,
  onSelect,
  nowMs,
}: {
  routines: RoutineSummaryDto[];
  selectedName: string | null;
  onSelect: (name: string) => void;
  nowMs: number;
}) {
  const sorted = sortRoutinesWorstFirst(routines);
  return (
    <div style={{ overflowX: "auto" }}>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: SUMMARY_COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["routine", "timer", "status", "last run", "detail"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {sorted.map((row) => {
        const active = row.name === selectedName;
        const detail = row.latest_error ?? (row.latest ? latestDetailLine(row.latest) : "never run");
        return (
          <button
            key={row.name}
            type="button"
            onClick={() => onSelect(row.name)}
            className="grid items-center gap-3 px-5 py-2.5 bus-row w-full text-left"
            style={{
              gridTemplateColumns: SUMMARY_COLUMNS,
              background: active ? "color-mix(in srgb, var(--accent) 8%, transparent)" : "transparent",
              border: "none",
              cursor: "pointer",
            }}
          >
            <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={row.name}>
              {row.name}
            </span>
            <span className="chip" style={cssVar("dot", row.installed ? "var(--mint)" : "var(--faint)")}>
              <span className="dot" aria-hidden="true" />
              {row.installed ? "yes" : "no"}
            </span>
            <RoutineStatusChip row={row} />
            <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }}>
              {latestRelativeTime(row.latest, nowMs)}
            </span>
            <span className="truncate text-[11.5px]" style={{ color: "var(--dim)" }} title={detail}>
              {detail}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/**
 * One routine's run history, newest first - mirrors `QualityRunDetail.tsx`'s
 * "select something above to see it here" shape. Surfaces `skipped_lines`
 * (malformed `history.ndjson` lines) honestly rather than silently
 * truncating, and distinguishes "this routine has no runs yet" from
 * "history.ndjson does not exist at all".
 */
function RoutineHistoryPanel({
  routine,
  history,
  loading,
}: {
  routine: string | null;
  history: RoutinesHistoryDto | null;
  loading: boolean;
}) {
  if (!routine) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        select a routine above to see its run history.
      </div>
    );
  }
  if (loading || history === null) {
    return <Loading />;
  }
  return (
    <div className="flex flex-col gap-2">
      {history.skipped_lines > 0 && (
        <div className="mono px-5 pt-2 text-[11px]" style={{ color: "var(--sev-medium)" }}>
          {history.skipped_lines} line{history.skipped_lines === 1 ? "" : "s"} in history.ndjson could not be parsed
          and were skipped - the history below may be incomplete.
        </div>
      )}
      {history.records.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          {history.history_file_exists
            ? `no recorded runs of ${routine} yet.`
            : "history.ndjson does not exist yet - no routine has ever run on this box."}
        </div>
      ) : (
        <div style={{ overflowX: "auto" }}>
          <div
            className="grid gap-3 px-5 py-2"
            style={{ gridTemplateColumns: HISTORY_COLUMNS, borderBottom: "1px solid var(--line)" }}
          >
            {["status", "started", "finished", "exit", "detail"].map((label) => (
              <span
                key={label}
                className="mono"
                style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
              >
                {label}
              </span>
            ))}
          </div>
          {history.records.map((r, idx) => (
            <div
              key={`${routine}-${r.started_at}-${idx}`}
              className="grid items-center gap-3 px-5 py-2.5 bus-row"
              style={{ gridTemplateColumns: HISTORY_COLUMNS }}
            >
              <span className="chip" style={cssVar("dot", recordStatusTone(r.status))}>
                <span className="dot" aria-hidden="true" />
                {r.status}
              </span>
              <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }}>
                {formatTimestamp(r.started_at)}
              </span>
              <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }}>
                {formatTimestamp(r.finished_at)}
              </span>
              <span
                className="mono tabular text-[12px]"
                style={{ color: r.exit_code === 0 ? "var(--dim)" : "var(--sev-high)" }}
              >
                {r.exit_code}
              </span>
              <span className="truncate text-[11.5px]" style={{ color: "var(--dim)" }} title={latestDetailLine(r)}>
                {latestDetailLine(r)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * The Routines tab (I7b): a read-only surface over what stack-up's
 * `routines.sh` already recorded under `$STACK_UP_HOME/routines/` - five
 * scheduled governance routines (focus-export, qryx-trend, verdryx-drift,
 * idryx-detect, mockryx-drill), each showing whether it is installed as an
 * OS timer and its run history, worst-first.
 *
 * **Non-goal, stated plainly (matches `crates/api/src/routines`'s own
 * module doc)**: this console does NOT install, uninstall, or run a
 * routine. That stays the operator's own `routines.sh` on the box. This tab
 * only surfaces the schedule state and recorded history.
 *
 * No live polling (mirrors `IdentityView.tsx`'s "load once, explicit
 * Refresh" idryx sections, not `QualityView.tsx`'s 60s poll): a plain file
 * read on demand is the FreshBadge's own `snapshot` variant, not `auto`/
 * `window` - see `FreshBadge.tsx`'s doc comment for the grammar.
 */
export function RoutinesView() {
  const [status, setStatus] = useState<RoutinesStatusDto | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [asOfMs, setAsOfMs] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const [selectedRoutine, setSelectedRoutine] = useState<string | null>(null);
  const [history, setHistory] = useState<RoutinesHistoryDto | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);

  const loadStatus = useCallback(async () => {
    try {
      const s = await fetchRoutinesStatus();
      setStatus(s);
      setStatusError(null);
      setAsOfMs(Date.now());
      // Keep the current selection if it is still a real routine; otherwise
      // default to the worst-ranked one, so the operator lands on the
      // routine most worth looking at first - mirrors `QualityView.tsx`'s
      // identical "keep selection if valid, else pick the most relevant
      // row" pattern.
      setSelectedRoutine((prev) => {
        if (prev && s.routines.some((r) => r.name === prev)) return prev;
        return sortRoutinesWorstFirst(s.routines)[0]?.name ?? null;
      });
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  useEffect(() => {
    const id = window.setInterval(() => setNowMs(Date.now()), NOW_TICK_MS);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    if (!selectedRoutine) {
      setHistory(null);
      setHistoryError(null);
      return;
    }
    let cancelled = false;
    setHistoryLoading(true);
    setHistoryError(null);
    fetchRoutinesHistory(selectedRoutine)
      .then((h) => {
        if (!cancelled) setHistory(h);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setHistory(null);
        setHistoryError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setHistoryLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedRoutine]);

  const hhmm = asOfMs !== null ? formatHm(asOfMs) : undefined;
  const environment = window.location.host || "unknown";

  if (status === null) {
    return statusError ? (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {statusError}
        </div>
      </div>
    ) : (
      <Loading />
    );
  }

  if (!status.routines_dir_exists) {
    return (
      <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-2">
          <FreshBadge variant="snapshot" detail={hhmm} title="Resolved $STACK_UP_HOME/routines - see below" />
          <div className="flex-1" />
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
            onClick={() => void loadStatus()}
          >
            Refresh
          </button>
        </div>
        <RoutinesEmptyState routinesDir={status.routines_dir} />
      </div>
    );
  }

  const sortedRoutines = sortRoutinesWorstFirst(status.routines);
  const worst = sortedRoutines[0];
  const worstUiStatus = toUiStatus(worst);
  const issueCount = status.routines.filter((r) => {
    const s = toUiStatus(r);
    return s === "error" || s === "unreadable" || s === "findings";
  }).length;
  const installedCount = status.routines.filter((r) => r.installed).length;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--faint)")} title={status.routines_dir}>
          <span className="dot" aria-hidden="true" />
          {status.routines_dir}
        </span>
        <FreshBadge
          variant="snapshot"
          detail={hhmm}
          title="Reads $STACK_UP_HOME/routines fresh on load and on Refresh - not a live stream"
        />
        <div className="flex-1" />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => void loadStatus()}
        >
          Refresh
        </button>
      </div>

      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        Read-only: routines run on the box via stack-up&apos;s own routines.sh, on its own schedule. This tab
        surfaces what it already recorded; it does not install, uninstall, or run a routine itself.
      </span>

      {statusError && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {statusError}
        </div>
      )}

      <HeroBand
        hero={
          <Hero
            cap="Routines · scheduled governance"
            value={`${installedCount}/${status.routines.length}`}
            sub={<>installed as timers</>}
          />
        }
        tiles={
          <>
            <KpiTile
              label="Needs attention"
              value={issueCount.toLocaleString("en-US")}
              tone={issueCount > 0 ? "var(--sev-high)" : "var(--mint)"}
              sub="error · unreadable · findings"
            />
            <KpiTile
              label="Worst status"
              value={ROUTINE_STATUS_LABEL[worstUiStatus]}
              tone={ROUTINE_STATUS_TONE[worstUiStatus]}
              sub={worst.name}
            />
          </>
        }
      />

      <Section title="Routines" right={<FreshBadge variant="snapshot" detail={hhmm} />}>
        <RoutinesSummaryTable
          routines={status.routines}
          selectedName={selectedRoutine}
          onSelect={setSelectedRoutine}
          nowMs={nowMs}
        />
      </Section>

      <Section
        title={selectedRoutine ? `History · ${selectedRoutine}` : "History"}
        right={
          <span className="inline-flex items-center gap-2">
            {/* One routine's history, and the file says so: the server's
                200-record default and any unparseable line go into its
                caveats, so a file that stops at 200 runs cannot be read as
                the whole history. */}
            <ExportBar
              label={`the recorded runs of ${selectedRoutine ?? "this routine"}`}
              disabledHint="select a routine with recorded runs"
              disabled={selectedRoutine === null || history === null || history.records.length === 0}
              onCsv={() =>
                selectedRoutine &&
                history &&
                downloadCsv(
                  `genaryx-routine-history-${selectedRoutine}.csv`,
                  ROUTINE_HISTORY_EXPORT_COLUMNS,
                  routineHistoryExportRows(history.records),
                  routineHistoryExportMeta(selectedRoutine, history, new Date().toISOString(), environment),
                )
              }
              onJson={() =>
                selectedRoutine &&
                history &&
                downloadJson(
                  `genaryx-routine-history-${selectedRoutine}.json`,
                  routineHistoryExportRows(history.records),
                  routineHistoryExportMeta(selectedRoutine, history, new Date().toISOString(), environment),
                )
              }
            />
            <FreshBadge variant="snapshot" detail={hhmm} />
          </span>
        }
      >
        {historyError && (
          <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
            {historyError}
          </div>
        )}
        <RoutineHistoryPanel routine={selectedRoutine} history={history} loading={historyLoading} />
      </Section>
    </div>
  );
}
