import { useEffect, useMemo, useState } from "react";
import { describeMoneyError, fetchGateways } from "../lib/money";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { formatUsd } from "../lib/format";
import { ageLabel, gatewaysCaption, gatewayTitle, GATEWAYS_CAVEAT, lastHeardLabel } from "../lib/gateways";
import type { Gateways, MoneyError } from "../moneyTypes";
import { Section } from "./dash";

const REFRESH_INTERVAL_MS = 20_000;

/**
 * Which gateways the Cloud still hears from. A silent one leads, in the
 * colour of an incident: its runs, spend and kills stop moving through the
 * Cloud, and every other view goes quiet about it without saying why. Pure
 * props, so every state renders in a test.
 */
export function GatewaysBody({ data, error }: { data: Gateways | null; error: MoneyError | null }) {
  if (error) {
    return (
      <div className="mono" style={{ fontSize: 11.5, color: "var(--sev-high)", padding: "10px 0" }}>
        gateways could not be read: {describeMoneyError(error)}
      </div>
    );
  }
  if (data === null) {
    return (
      <div className="mono" style={{ fontSize: 12, color: "var(--faint)", padding: "10px 0" }}>
        reading gateways...
      </div>
    );
  }
  if (!data.reported) {
    return (
      <div className="mono" style={{ fontSize: 11.5, color: "var(--faint)", padding: "10px 0" }}>
        This Cloud has no GET /v1/gateways, so whether a gateway stopped reporting cannot be shown here. That is not
        the same as none having stopped.
      </div>
    );
  }
  return (
    <div className="flex flex-col" style={{ gap: 6 }}>
      {data.gateways.length === 0 ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)", padding: "10px 0" }}>
          no gateway has pushed to this Cloud yet.
        </div>
      ) : (
        data.gateways.map((g) => {
          const silent = g.state === "silent";
          return (
            <div
              key={g.site}
              className="flex items-center justify-between"
              style={{ gap: 10, padding: "6px 0", borderBottom: "1px solid var(--line)" }}
            >
              <div style={{ minWidth: 0 }}>
                <div className="mono" style={{ fontSize: 12, color: silent ? "var(--ember)" : "var(--fg)" }} title={g.site}>
                  {g.site}
                </div>
                <div className="mono" style={{ fontSize: 10.5, color: "var(--faint)" }}>
                  {lastHeardLabel(g)} · {g.calls.toLocaleString("en-US")} call(s) · {formatUsd(g.spent_usd)}
                </div>
              </div>
              <span className={`d-pill ${silent ? "over" : "live"}`} title={gatewayTitle(g, data.silent_after_secs)}>
                {silent ? `silent ${ageLabel(g.last_push_age_secs)}` : "reporting"}
              </span>
            </div>
          );
        })
      )}
      <div className="mono" style={{ fontSize: 10, color: "var(--faint)", lineHeight: 1.5, marginTop: 4 }}>
        {GATEWAYS_CAVEAT}
      </div>
    </div>
  );
}

/** The load step apart from React, so a test can drive it with a fake. */
export function gatewaysLoader(deps: {
  fetch: () => Promise<Gateways>;
  onData: (data: Gateways) => void;
  onError: (error: MoneyError | null) => void;
}) {
  return async () => {
    try {
      deps.onData(await deps.fetch());
      deps.onError(null);
    } catch (err) {
      deps.onError(err as MoneyError);
    }
  };
}

/** The Overview's Gateways section: reads `money_gateways` every 20 s. */
export function GatewaysSection() {
  const status = useMoneyStatus();
  const ready = status?.state === "ready";
  const [data, setData] = useState<Gateways | null>(null);
  const [error, setError] = useState<MoneyError | null>(null);
  const load = useMemo(() => gatewaysLoader({ fetch: fetchGateways, onData: setData, onError: setError }), []);

  useEffect(() => {
    if (!ready) return;
    void load();
    const id = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [ready, load]);

  return (
    <Section title="Gateways" right={data ? gatewaysCaption(data) : undefined}>
      <GatewaysBody data={data} error={error} />
    </Section>
  );
}
