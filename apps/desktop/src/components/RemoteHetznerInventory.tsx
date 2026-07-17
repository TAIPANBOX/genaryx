import { useCallback, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeRemoteError, listHetznerServers } from "../lib/remote";
import type { HetznerServer, RemoteError } from "../remoteTypes";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
} as const;

const COLUMNS = "70px 1fr 90px 130px 90px 60px 70px 110px";

function formatPrice(eur: number | null): string {
  if (eur === null) return "n/a";
  return `€${eur.toFixed(4)}/hr`;
}

/**
 * Section 1 (docs/PHASE4.md W4 position 1): a read-scoped Hetzner API token
 * + an optional label selector (default `managed-by=taipan`), "List
 * servers", and the resulting inventory table. STRICTLY READ-ONLY - there is
 * no create/delete affordance anywhere in this component, mirroring the
 * connector's own "no mutation method exists at all" guarantee
 * (`crates/connectors/src/hetzner.rs`). The token lives only in this
 * component's own local state - never persisted, never sent anywhere but
 * this one IPC call.
 */
export function RemoteHetznerInventory() {
  const [token, setToken] = useState("");
  const [labelSelector, setLabelSelector] = useState("");
  const [servers, setServers] = useState<HetznerServer[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<RemoteError | null>(null);
  const [listedAtMs, setListedAtMs] = useState<number | null>(null);

  const onList = useCallback(async () => {
    if (token.trim().length === 0 || loading) return;
    setLoading(true);
    setError(null);
    try {
      const rows = await listHetznerServers(token, labelSelector);
      setServers(rows);
      setListedAtMs(Date.now());
    } catch (err) {
      setError(err as RemoteError);
    } finally {
      setLoading(false);
    }
  }, [token, labelSelector, loading]);

  return (
    <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
          read-scoped API token
        </span>
        <input
          className="mono flex-1"
          style={{ ...FIELD_STYLE, minWidth: 160 }}
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="paste a read-scoped Hetzner Cloud API token"
          spellCheck={false}
          autoComplete="off"
        />
        <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
          label selector
        </span>
        <input
          className="mono"
          style={{ ...FIELD_STYLE, width: 200 }}
          value={labelSelector}
          onChange={(e) => setLabelSelector(e.target.value)}
          placeholder="managed-by=taipan"
          spellCheck={false}
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onList()}
          disabled={loading || token.trim().length === 0}
        >
          {loading ? "Listing..." : "List servers"}
        </button>
      </div>
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        read-only inventory - this console never creates, resizes, or deletes a Hetzner server; the token is used
        for this one request only and is never saved.
      </span>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel)", color: "var(--sev-high)" }}>
          {describeRemoteError(error)}
        </div>
      )}

      {servers !== null && (
        <div className="flex items-center gap-2">
          <span className="chip" style={cssVar("dot", "var(--faint)")}>
            <span className="dot" aria-hidden="true" />
            {listedAtMs !== null ? `as of last list · ${new Date(listedAtMs).toLocaleTimeString()}` : "no list yet"}
          </span>
        </div>
      )}

      {servers === null ? (
        <div className="px-1 py-4 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          list servers to see the campaign's boxes.
        </div>
      ) : servers.length === 0 ? (
        <div className="px-1 py-4 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no boxes found for this token/selector.
        </div>
      ) : (
        <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
          <div
            className="grid gap-3 px-4 py-2"
            style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-3)" }}
          >
            {["id", "name", "status", "ipv4", "type", "cores", "ram", "price/hr"].map((label) => (
              <span
                key={label}
                className="mono"
                style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
              >
                {label}
              </span>
            ))}
          </div>
          {servers.map((s) => (
            <div key={s.id} className="grid items-center gap-3 px-4 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
              <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
                {s.id}
              </span>
              <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={s.name}>
                {s.name}
              </span>
              <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.status}
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.ipv4 ?? "no public ip"}
              </span>
              <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.server_type}
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.cores}
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.memory_gb}G
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {formatPrice(s.price_hourly_eur)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
