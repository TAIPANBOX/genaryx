import { useEffect, useMemo, useState } from "react";
import { fetchIncidents, describeMoneyError } from "../lib/money";
import { fetchAlerts } from "../lib/identity";
import { fetchRecentEvents } from "../lib/recentEvents";
import { hasBackend, subscribeBackend } from "../lib/transport";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { useIdentityStatus } from "../lib/useIdentityStatus";
import { usePostureData } from "../lib/usePostureData";
import {
  aggregateIncidents,
  busCoverage,
  busPlaneLabel,
  busPlaneView,
  filterIncidents,
  incidentPlane,
  type UnifiedIncident,
  planesPresent,
  INCIDENT_SOURCE_LABEL,
  INCIDENT_SOURCE_VIEW,
} from "../lib/incidents";
import type { Incident, MoneyError } from "../moneyTypes";
import type { IdryxAlert } from "../identityTypes";
import type { UiEvent } from "../types";
import type { ViewId } from "../lib/views";
import { sevColor } from "../lib/dashData";
import { Feed, Section, type FeedItem } from "./dash";

/** The same window the Overview card reads. Kept identical on purpose: two
 * surfaces over one question that disagreed about how far back they looked
 * would be worse than either alone. */
const BUS_FETCH_LIMIT = 500;

const SEVERITY_CHIPS = ["critical", "high", "medium", "low", "info"] as const;

/**
 * The whole trouble stream, on its own tab.
 *
 * The Overview card answers "is anything on fire" in ten rows. This answers
 * "show me all of it", which is a different question and was one the console
 * could not answer at all: the card is capped, and a capped card is where an
 * operator stops rather than where they start.
 *
 * `@yurii` 2026-08-26, asking for it: "має бути вкладка, куди можна зайти,
 * було б подивитись все, як воно є. Не тільки на цій картці, на Овервью."
 *
 * Everything here is a read some other view already performs, aggregated by
 * `lib/incidents.ts` exactly as the card does, and drilled by
 * `lib/useIncidentDrill.tsx` exactly as the card does. This view owns the
 * FILTERS and nothing else, which is why it is short: a second aggregation
 * would be a second answer to one question.
 */
export function AnomaliesView({
  onSelectView,
  onOpenIncident,
}: {
  onSelectView: (view: ViewId) => void;
  /** Hand the row up to the shell, which owns the overlay layer. The view
   * does not open the card itself: Incident 360 opens Agent 360 beside it,
   * and a card opened from inside a tab cannot outlive a tab switch. */
  onOpenIncident: (row: UnifiedIncident) => void;
}) {
  const moneyStatus = useMoneyStatus();
  const ready = moneyStatus?.state === "ready";
  const identityStatus = useIdentityStatus();
  const identityReady = identityStatus?.state === "ready";
  const posture = usePostureData();
  const postureFindings = useMemo(
    () => [...posture.stackFindings, ...posture.identityFindings, ...posture.connectionFindings],
    [posture.stackFindings, posture.identityFindings, posture.connectionFindings],
  );

  const [moneyIncidents, setMoneyIncidents] = useState<Incident[]>([]);
  const [moneyError, setMoneyError] = useState<MoneyError | null>(null);
  const [identityAlerts, setIdentityAlerts] = useState<IdryxAlert[]>([]);
  const [busEvents, setBusEvents] = useState<UiEvent[]>([]);

  const [planes, setPlanes] = useState<string[]>([]);
  const [severities, setSeverities] = useState<string[]>([]);
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (!ready) return;
    let cancelled = false;
    void fetchIncidents()
      .then((rows) => {
        if (!cancelled) setMoneyIncidents(rows);
      })
      .catch((e: unknown) => {
        // Named rather than swallowed: a source that could not be read is not
        // a source with nothing to report, and the footer below says which.
        if (!cancelled) setMoneyError(e as MoneyError);
      });
    return () => {
      cancelled = true;
    };
  }, [ready]);

  useEffect(() => {
    if (!identityReady) return;
    let cancelled = false;
    void fetchAlerts().then((rows) => {
      if (!cancelled) setIdentityAlerts(rows);
    });
    return () => {
      cancelled = true;
    };
  }, [identityReady]);

  useEffect(() => {
    let cancelled = false;
    void fetchRecentEvents(BUS_FETCH_LIMIT).then((res) => {
      if (!cancelled) setBusEvents(res.events);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!hasBackend()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void subscribeBackend<UiEvent>("bus:event", (payload) => {
      setBusEvents((prev) => [payload, ...prev].slice(0, BUS_FETCH_LIMIT));
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const all = useMemo(
    () => aggregateIncidents({ moneyIncidents, identityAlerts, busEvents, postureFindings }),
    [moneyIncidents, identityAlerts, busEvents, postureFindings],
  );
  const available = useMemo(() => planesPresent(all), [all]);
  const rows = useMemo(
    () => filterIncidents(all, { planes, severities, query }),
    [all, planes, severities, query],
  );
  const coverage = useMemo(() => busCoverage(busEvents, BUS_FETCH_LIMIT), [busEvents]);

  const toggle = (list: string[], set: (v: string[]) => void, value: string) =>
    set(list.includes(value) ? list.filter((v) => v !== value) : [...list, value]);

  const items: FeedItem[] = rows.map((row) => {
    const chipLabel = row.source === "bus" ? busPlaneLabel(row.raw) : INCIDENT_SOURCE_LABEL[row.source];
    const chipView = row.source === "bus" ? busPlaneView(row.raw) : INCIDENT_SOURCE_VIEW[row.source];
    return {
      key: row.id,
      color: sevColor(row.severity),
      onClick: () => onOpenIncident(row),
      title: (
        <span className="flex items-center gap-2">
          <button
            type="button"
            className="chip"
            style={{ cursor: "pointer" }}
            title={`Open the ${chipView} tab`}
            onClick={(e) => {
              e.stopPropagation();
              onSelectView(chipView);
            }}
          >
            {chipLabel}
          </button>
          <span className="truncate">{row.title}</span>
        </span>
      ),
      sub: row.detail,
      value: row.occurrences !== undefined && row.occurrences > 1 ? `${row.occurrences}×` : undefined,
    };
  });

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <Section
        title="Anomalies"
        right={
          <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
            {rows.length} of {all.length} shown
          </span>
        }
      >
        <div className="flex items-center gap-2 flex-wrap" style={{ marginBottom: 10 }}>
          {available.map((plane) => (
            <button
              key={plane}
              type="button"
              className="chip"
              style={{ cursor: "pointer", opacity: planes.length === 0 || planes.includes(plane) ? 1 : 0.4 }}
              onClick={() => toggle(planes, setPlanes, plane)}
            >
              {plane}
            </button>
          ))}
          <span style={{ width: 10 }} />
          {SEVERITY_CHIPS.map((sev) => (
            <button
              key={sev}
              type="button"
              className="chip"
              style={{
                cursor: "pointer",
                opacity: severities.length === 0 || severities.includes(sev) ? 1 : 0.4,
                borderColor: sevColor(sev),
              }}
              onClick={() => toggle(severities, setSeverities, sev)}
            >
              {sev}
            </button>
          ))}
          <input
            className="mono"
            placeholder="filter by agent or text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{
              flex: "1 1 160px",
              minWidth: 140,
              fontSize: 11,
              padding: "3px 8px",
              background: "transparent",
              border: "1px solid var(--line)",
              borderRadius: 4,
              color: "inherit",
            }}
          />
        </div>

        <Feed
          items={items}
          empty={
            all.length === 0
              ? "no incidents from money, identity, posture, or any plane on the bus"
              : "no row matches these filters"
          }
        />

        {/* The same sentence the Overview card carries, for the same reason: a
            quiet stream and a blind one look identical, and this view is where
            an operator goes when they expect to see everything. */}
        <div className="mono" style={{ fontSize: 10, color: "var(--faint)", marginTop: 8 }}>
          {coverage.read === 0
            ? "no events read off the bus, so no plane is accounted for here"
            : `${coverage.incidentRows} of ${coverage.read} bus event(s) are incidents` +
              `${coverage.truncated ? `, capped at ${coverage.limit}` : ""}` +
              ` · on the bus: ${coverage.planes.join(", ")}`}
          {moneyError ? ` · money incidents unread: ${describeMoneyError(moneyError)}` : ""}
        </div>
      </Section>
    </div>
  );
}

/** Exported for the tab's own count badge, so the nav can say how many rows
 * are waiting without this view being mounted. Kept beside the view rather
 * than in `lib/` because it is about presentation, not aggregation. */
export function anomalyPlaneOf(row: Parameters<typeof incidentPlane>[0]): string {
  return incidentPlane(row);
}
