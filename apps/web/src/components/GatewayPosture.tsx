import { useEffect, useMemo, useState } from "react";
import { describeCredentialsError, fetchGatewayPosture, type CredentialsError, type GatewayPosture } from "../lib/credentials";
import { useCredentialsStatus } from "../lib/useCredentialsStatus";

/** What a switch the gateway did not report reads, and what the whole
 * settings block reads on a gateway without the route. Never "off". */
export const NOT_REPORTED = "not reported by this gateway version";

const REFRESH_INTERVAL_MS = 60_000;

function Row({ label, value, title }: { label: string; value: string; title?: string }) {
  const muted = value === NOT_REPORTED;
  return (
    <div className="flex items-baseline justify-between" style={{ gap: 12, padding: "3px 0", borderBottom: "1px solid var(--line)" }}>
      <span style={{ fontSize: 12, color: "var(--dim)" }}>{label}</span>
      <span className="mono" style={{ fontSize: 11.5, color: muted ? "var(--faint)" : "var(--fg)", textAlign: "right" }} title={title}>
        {value}
      </span>
    </div>
  );
}

function list(items: string[], none: string, max = 12): string {
  if (items.length === 0) return none;
  const shown = items.slice(0, max).join(", ");
  return items.length > max ? `${shown} (+${items.length - max})` : shown;
}

/**
 * The gateway's posture: the switches it runs with, its keys and identity
 * map, and the book it prices calls with. Pure props, so every state renders
 * in a test.
 */
export function GatewayPostureBody({ data, error }: { data: GatewayPosture | null; error: CredentialsError | null }) {
  if (error) {
    return (
      <div className="mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
        gateway posture could not be read: {describeCredentialsError(error)}
      </div>
    );
  }
  if (data === null) {
    return (
      <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
        reading the gateway's settings...
      </div>
    );
  }
  const book = data.price_book;
  return (
    <div className="flex flex-col" style={{ gap: 14 }}>
      <div className="panel px-3 py-2" style={{ background: "var(--panel-2)" }}>
        <div className="mono" style={{ fontSize: 10.5, color: "var(--faint)", marginBottom: 4 }}>
          switches in force · {data.gateway_url}
        </div>
        {data.settings === null ? (
          <div className="mono" style={{ fontSize: 11.5, color: "var(--faint)" }}>
            the settings read failed (see below)
          </div>
        ) : !data.settings.reported ? (
          <div className="mono" style={{ fontSize: 11.5, color: "var(--faint)" }}>
            {NOT_REPORTED}: this gateway has no GET /v1/settings, so its firewall mode, PII mode, run budget ceiling
            and wire are not shown here. That is not the same as off.
          </div>
        ) : (
          data.settings.items.map((s) => <Row key={s.key} label={s.label} value={s.value ?? NOT_REPORTED} />)
        )}
      </div>

      <div className="panel px-3 py-2" style={{ background: "var(--panel-2)" }}>
        <div className="mono" style={{ fontSize: 10.5, color: "var(--faint)", marginBottom: 4 }}>client keys · identity map</div>
        {data.keys === null ? (
          <div className="mono" style={{ fontSize: 11.5, color: "var(--faint)" }}>the keys read failed (see below)</div>
        ) : (
          <>
            <Row label="Identity check" value={data.keys.identity_strict} />
            <Row label="Identity map" value={data.keys.identity_map_configured ? "configured" : "none configured"} />
            <Row
              label="Client keys configured"
              value={`${data.keys.configured_keys} of ${data.keys.keys_listed} listed`}
              title="Keys in the gateway's client-key list right now, of every key its report names. Details in Identity > Credentials."
            />
            <Row label="Refused for no or a wrong key since start" value={String(data.keys.unauthorized_attempts)} />
          </>
        )}
      </div>

      <div className="panel px-3 py-2" style={{ background: "var(--panel-2)" }}>
        <div className="mono" style={{ fontSize: 10.5, color: "var(--faint)", marginBottom: 4 }}>price book · tiers</div>
        {book === null ? (
          <div className="mono" style={{ fontSize: 11.5, color: "var(--faint)" }}>{NOT_REPORTED} (see below)</div>
        ) : (
          <>
            <Row label="Rows" value={`${book.rows} (${book.built_in_rows} built in)`} />
            <Row label="Operator file" value={book.operator_file ? "applied" : "none"} />
            <Row label="Overridden" value={list(book.overridden_models, "none")} />
            <Row label="Added" value={list(book.added_models, "none")} />
            <Row
              label="Fallback rate (USD per M tokens)"
              value={
                book.fallback_input_per_mtok_usd !== null && book.fallback_output_per_mtok_usd !== null
                  ? `${book.fallback_input_per_mtok_usd} in / ${book.fallback_output_per_mtok_usd} out`
                  : "none"
              }
              title="What a model id with no row in the book is charged at: a gateway estimate, not a provider bill."
            />
            <Row
              label="Charged at the fallback since start"
              value={list(book.fallback_models_seen, "no model id")}
              title={`${book.fallback_models_seen_total} model id(s) in all`}
            />
            <Row
              label="Prompt-length tiers"
              value={
                book.tiers === null
                  ? NOT_REPORTED
                  : book.tiers.length === 0
                    ? "none"
                    : book.tiers
                        .map((t) => `${t.model} above ${t.above_prompt_tokens.toLocaleString("en-US")} tokens: ${t.input_per_mtok_usd}/${t.output_per_mtok_usd}`)
                        .join(" · ")
              }
            />
            {book.tiers_dropped_models.length > 0 && (
              <Row label="Tiers dropped by the operator file" value={list(book.tiers_dropped_models, "none")} />
            )}
          </>
        )}
      </div>

      {data.notes.map((n) => (
        <div key={n} className="mono" style={{ fontSize: 11, color: "var(--sev-medium)" }}>
          {n}
        </div>
      ))}
    </div>
  );
}

/** The load step apart from React, so a test can drive it with a fake. */
export function postureLoader(deps: {
  fetch: () => Promise<GatewayPosture>;
  onData: (data: GatewayPosture) => void;
  onError: (error: CredentialsError | null) => void;
}) {
  return async () => {
    try {
      deps.onData(await deps.fetch());
      deps.onError(null);
    } catch (err) {
      deps.onError(err as CredentialsError);
    }
  };
}

/** The Posture view's "Gateway settings" section. */
export function GatewayPostureSection() {
  const status = useCredentialsStatus();
  const ready = status?.state === "ready";
  const [data, setData] = useState<GatewayPosture | null>(null);
  const [error, setError] = useState<CredentialsError | null>(null);
  const load = useMemo(() => postureLoader({ fetch: fetchGatewayPosture, onData: setData, onError: setError }), []);

  useEffect(() => {
    if (!ready) return;
    void load();
    const id = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [ready, load]);

  if (!ready) {
    return (
      <div className="mono" style={{ fontSize: 11.5, color: "var(--faint)" }}>
        {status === null
          ? "connecting to the gateway..."
          : status.state === "unreachable"
            ? `could not reach the gateway: ${status.reason}`
            : status.state === "no_environment"
              ? "no gateway configured in this environment"
              : "connecting to the gateway..."}
      </div>
    );
  }
  return <GatewayPostureBody data={data} error={error} />;
}
