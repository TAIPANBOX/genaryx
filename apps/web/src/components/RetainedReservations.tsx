import { useEffect, useState } from "react";
import { describeCredentialsError, fetchGatewayRetainedRuns } from "../lib/credentials";
import type { CredentialsError, CredentialsStatus, GatewayRetained } from "../lib/credentials";
import { useCredentialsStatus } from "../lib/useCredentialsStatus";
import { formatUsd } from "../lib/format";
import { Section, Feed } from "./dash";
import type { FeedItem } from "./dash";

/** Same cadence as `MoneyView`'s own runs/incidents/savings refresh - the
 * gateway's held reservations change as calls resolve, same reasoning. */
const REFRESH_INTERVAL_MS = 20_000;

/** `data.runs` -> `Feed`'s row shape - factored out so it is testable without
 * a DOM (same rationale `lib/quality.ts`'s `formatQualityMean` gives). */
export function retainedFeedItems(data: GatewayRetained): FeedItem[] {
  return data.runs.map((r) => ({
    key: r.run_id,
    color: "var(--sev-high)",
    title: r.run_id,
    sub: `${r.retained} reservation${r.retained === 1 ? "" : "s"} held`,
    value: formatUsd(r.retained_usd),
  }));
}

/**
 * The pure rendering half of [`RetainedReservationsSection`]: explicit props
 * in, JSX out, no hooks - so every state (bootstrapping, no gateway
 * configured, unreachable, a read error, loading, and a real report) can be
 * rendered and asserted on directly. `RetainedReservationsSection` below
 * supplies these from `useCredentialsStatus` and its own fetch/poll effect;
 * this component never fetches anything itself.
 */
export function RetainedReservationsBody({
  status,
  data,
  error,
}: {
  status: CredentialsStatus | null;
  data: GatewayRetained | null;
  error: CredentialsError | null;
}) {
  // Not configured / not reachable: say so, never show a 0 as if measured.
  if (!status || status.state === "bootstrapping") {
    return (
      <Section title="Retained reservations">
        <div className="mono px-4 py-4" style={{ fontSize: 11.5, color: "var(--faint)" }}>
          connecting to the gateway...
        </div>
      </Section>
    );
  }
  if (status.state === "no_environment") {
    return (
      <Section title="Retained reservations">
        <div className="mono px-4 py-4" style={{ fontSize: 11.5, color: "var(--faint)" }}>
          no gateway configured for this environment - retained reservations cannot be read.
        </div>
      </Section>
    );
  }
  if (status.state === "unreachable") {
    return (
      <Section title="Retained reservations">
        <div className="mono px-4 py-4" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          gateway unreachable: {status.reason}
        </div>
      </Section>
    );
  }

  if (error) {
    return (
      <Section title="Retained reservations">
        <div className="mono px-4 py-4" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeCredentialsError(error)}
        </div>
      </Section>
    );
  }

  if (data === null) {
    return (
      <Section title="Retained reservations">
        <div className="mono px-4 py-4" style={{ fontSize: 11.5, color: "var(--faint)" }}>
          loading...
        </div>
      </Section>
    );
  }

  return (
    <Section
      title="Retained reservations"
      right={
        <span className="mono" style={{ fontSize: 11, color: "var(--faint)" }}>
          {formatUsd(data.total_retained_usd)} held
        </span>
      }
    >
      {/* The explanation lives in the body, not the header's right slot: the
          rail card is narrow and a long right-hand label ran past its edge. */}
      <div className="mono px-4 pt-2" style={{ fontSize: 10.5, color: "var(--faint)", lineHeight: 1.5 }}>
        Money held after unknown outcome: the gateway keeps it open, neither spent nor released,
        after a call whose outcome it could not confirm.
      </div>
      <Feed items={retainedFeedItems(data)} empty="no reservations currently held" />
    </Section>
  );
}

/**
 * "Retained reservations" (tokenfuse invariant 50): money the gateway held
 * open after a call whose outcome it never learned - deliberately neither
 * released nor counted spent. Before this, the Money tab showed `spent_usd`
 * and nothing about what was held back.
 *
 * Independent of `MoneyView`'s own Cloud-backed state: this reads the
 * GATEWAY's `GET /v1/runs` through the Credentials plane
 * (`TOKENFUSE_GATEWAY_ADMIN_KEY`, the same admin key `CredentialsKeysTable`
 * already presents), which can be ready, not configured, or unreachable
 * independently of whether the Money/Cloud plane is. A gateway that is not
 * configured or not reachable says so here, in its own words - never a `0`
 * that would look like a measured "nothing retained".
 */
export function RetainedReservationsSection() {
  const credentialsStatus = useCredentialsStatus();
  const gatewayReady = credentialsStatus?.state === "ready";

  const [data, setData] = useState<GatewayRetained | null>(null);
  const [error, setError] = useState<CredentialsError | null>(null);

  useEffect(() => {
    if (!gatewayReady) {
      setData(null);
      setError(null);
      return;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const d = await fetchGatewayRetainedRuns();
        if (!cancelled) {
          setData(d);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) setError(err as CredentialsError);
      }
    };
    void load();
    const id = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [gatewayReady]);

  return <RetainedReservationsBody status={credentialsStatus} data={data} error={error} />;
}
