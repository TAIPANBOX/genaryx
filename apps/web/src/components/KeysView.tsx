import { useEffect, useState } from "react";
import { fetchKeys, type KeysRead } from "../lib/keyCustody";
import { useConsoleStateVersion } from "../lib/consoleState";
import { formatTimestamp } from "../lib/format";
import { visibleText } from "../lib/policyExport";
import type { Alarm, CustodyPart, Entry, KeyName, WitnessPart } from "../keysTypes";

/**
 * Keys: what Keyryx custody holds and what its witness last signed (invariant 53).
 *
 * Read-only, from the two routes custody and the witness publish. It shows the
 * record and nothing else: no rotate, no revoke, no form. Rotating or revoking
 * a key is custody's command line on the box. Every string from the wire goes
 * through `visibleText` and renders as a text node. Every time arrives as UNIX
 * seconds and is shown by `formatTimestamp`.
 *
 * A part not named, a part that did not answer, and a part that answered with
 * nothing render differently, so an unreachable custody is never shown as an
 * empty inventory.
 */
export function KeysView() {
  const [read, setRead] = useState<KeysRead | null | undefined>(undefined);
  const version = useConsoleStateVersion();

  useEffect(() => {
    let alive = true;
    const load = async () => {
      const r = await fetchKeys();
      if (alive) setRead(r);
    };
    void load();
    const t = setInterval(() => void load(), 30_000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [version]);

  return (
    <div className="flex flex-col min-h-0 h-full overflow-auto" style={{ padding: "18px 22px", gap: 14 }}>
      <div>
        <div className="text-[11px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
          Assure
        </div>
        <h1 className="text-[20px]" style={{ color: "var(--fg)", fontWeight: 600, margin: "2px 0 4px" }}>
          Keys
        </h1>
        <p className="text-[12px]" style={{ color: "var(--dim)", margin: 0, maxWidth: 760 }}>
          What Keyryx custody holds and what its witness last signed, read from their own read-only routes.
          Rotating or revoking a key is custody's command line on the box; nothing here changes a key.
        </p>
      </div>
      <Body read={read} />
    </div>
  );
}

function Note({ tone, children, testId }: { tone: "warn" | "calm" | "bad"; children: React.ReactNode; testId?: string }) {
  const edge = tone === "bad" ? "var(--sev-high)" : tone === "warn" ? "var(--amber)" : "var(--line-2)";
  return (
    <div
      data-testid={testId}
      className="text-[12px]"
      style={{
        padding: "8px 12px",
        borderLeft: `2px solid ${edge}`,
        background: tone === "calm" ? "transparent" : "var(--panel-2)",
        color: tone === "calm" ? "var(--dim)" : "var(--fg)",
        lineHeight: 1.6,
      }}
    >
      {children}
    </div>
  );
}

function Body({ read }: { read: KeysRead | null | undefined }) {
  if (read === undefined) return <Note tone="calm">reading custody and the witness...</Note>;
  if (read === null) return <Note tone="calm">No box to ask: this build has no backend.</Note>;
  if (!read.ok) {
    return (
      <Note tone="warn" testId="keys-error">
        The box did not answer for the keys: {visibleText(read.reason)}
      </Note>
    );
  }
  return (
    <>
      <Section title="Custody">
        <CustodySection part={read.panel.custody} />
      </Section>
      <Section title="Witness">
        <WitnessSection part={read.panel.witness} />
      </Section>
    </>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col" style={{ gap: 8 }}>
      <h2 className="text-[13px]" style={{ color: "var(--fg)", fontWeight: 600, margin: "6px 0 0" }}>
        {title}
      </h2>
      {children}
    </section>
  );
}

function when(s: number | null | undefined): string {
  if (s === null || s === undefined) return "";
  return formatTimestamp(new Date(s * 1000).toISOString());
}

function CustodySection({ part }: { part: CustodyPart }) {
  if (part.state === "not_configured") {
    return (
      <Note tone="warn" testId="keys-custody-not-configured">
        No Keyryx custody is named on this box, so the console cannot say what it holds. Set{" "}
        <span className="mono">{part.variable}</span> to custody's base URL.
      </Note>
    );
  }
  if (part.state === "refused") {
    return (
      <Note tone="warn" testId="keys-custody-refused">
        <span className="mono">{part.variable}</span> is not usable: {visibleText(part.reason)}
      </Note>
    );
  }
  if (part.state === "unreachable") {
    return (
      <Note tone="warn" testId="keys-custody-unreachable">
        Custody at <span className="mono">{visibleText(part.url)}</span> did not answer: {visibleText(part.reason)}.
        Nothing is shown as held while it cannot be read.
      </Note>
    );
  }

  return (
    <>
      <div className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.8 }}>
        <div data-testid="keys-custody-head">
          custody version {visibleText(part.version)}, {part.names.length} key name(s), {part.entries_total}{" "}
          {part.entries_total === 1 ? "inventory entry" : "inventory entries"}
        </div>
        <div style={{ color: "var(--faint)" }}>{visibleText(part.url)}</div>
      </div>
      {part.alarms.length > 0 && <Alarms alarms={part.alarms} total={part.alarms_total} />}
      {part.inventory_error !== null && (
        <Note tone="warn" testId="keys-inventory-error">
          Custody answered its state but not its inventory: {visibleText(part.inventory_error)}. The entries below
          are not the whole inventory.
        </Note>
      )}
      {part.names.length === 0 ? (
        <Note tone="calm">custody holds no key names.</Note>
      ) : (
        <NamesTable names={part.names} />
      )}
      {part.entries.length > 0 && <EntriesTable entries={part.entries} total={part.entries_total} />}
    </>
  );
}

function Alarms({ alarms, total }: { alarms: Alarm[]; total: number }) {
  return (
    <Note tone="bad" testId="keys-alarms">
      {alarms.map((a, i) => (
        <div key={i}>
          {visibleText(a.kind)} {visibleText(a.name)} {visibleText(a.kid)}: {visibleText(a.reason)}
        </div>
      ))}
      {total > alarms.length ? <div>newest {alarms.length} of {total}</div> : null}
    </Note>
  );
}

const cell: React.CSSProperties = { padding: "6px 10px", verticalAlign: "top", borderBottom: "1px solid var(--line-2)" };

function statesText(states: Record<string, number>): string {
  const entries = Object.entries(states);
  if (entries.length === 0) return "none";
  return entries.map(([k, n]) => `${visibleText(k)} ${n}`).join(", ");
}

function NamesTable({ names }: { names: KeyName[] }) {
  return (
    <div className="min-h-0 overflow-auto">
      <table data-testid="keys-names" className="text-[12px]" style={{ borderCollapse: "collapse", width: "100%", minWidth: 880 }}>
        <thead>
          <tr style={{ color: "var(--faint)", textAlign: "left" }}>
            {["name", "mode", "alg", "active key", "next key", "last rotation", "entries by state"].map((h) => (
              <th key={h} className="text-[10.5px] uppercase tracking-wider" style={{ ...cell, fontWeight: 500 }}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {names.map((n) => (
            <tr key={n.name} style={{ color: "var(--dim)" }}>
              <td className="mono" style={{ ...cell, color: "var(--fg)", overflowWrap: "anywhere" }}>
                {visibleText(n.name)}
              </td>
              <td style={cell}>{visibleText(n.mode)}</td>
              <td className="mono" style={cell}>{visibleText(n.alg)}</td>
              <td className="mono" style={{ ...cell, overflowWrap: "anywhere" }}>
                {n.active_kid ? visibleText(n.active_kid) : "none"}
                {n.active_since !== null ? <div style={{ color: "var(--faint)" }}>since {when(n.active_since)}</div> : null}
              </td>
              <td className="mono" style={{ ...cell, overflowWrap: "anywhere" }}>
                {n.next_kid ? visibleText(n.next_kid) : "none"}
                {n.next_activates_at !== null ? (
                  <div style={{ color: "var(--faint)" }}>activates {when(n.next_activates_at)}</div>
                ) : null}
              </td>
              <td className="mono" style={cell}>{n.last_rotation !== null ? when(n.last_rotation) : "none"}</td>
              <td style={cell}>
                {statesText(n.states)}
                {n.renew_due ? <div style={{ color: "var(--sev-high)" }}>renew due</div> : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function EntriesTable({ entries, total }: { entries: Entry[]; total: number }) {
  return (
    <div className="min-h-0 overflow-auto">
      {entries.length < total && (
        <div className="text-[11px]" style={{ color: "var(--faint)", paddingBottom: 6 }}>
          newest {entries.length} of {total}
        </div>
      )}
      <table data-testid="keys-entries" className="text-[12px]" style={{ borderCollapse: "collapse", width: "100%", minWidth: 720 }}>
        <thead>
          <tr style={{ color: "var(--faint)", textAlign: "left" }}>
            {["kid", "name", "state", "created", "retired or revoked"].map((h) => (
              <th key={h} className="text-[10.5px] uppercase tracking-wider" style={{ ...cell, fontWeight: 500 }}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {entries.map((e) => (
            <tr key={e.kid} style={{ color: "var(--dim)" }}>
              <td className="mono" style={{ ...cell, overflowWrap: "anywhere" }}>{visibleText(e.kid)}</td>
              <td className="mono" style={{ ...cell, overflowWrap: "anywhere" }}>{visibleText(e.name)}</td>
              <td style={cell}>{visibleText(e.state)}</td>
              <td className="mono" style={cell}>{when(e.created)}</td>
              <td className="mono" style={cell}>
                {e.revoked !== undefined
                  ? `revoked ${when(e.revoked)}`
                  : e.retired !== undefined
                    ? `retired ${when(e.retired)}`
                    : ""}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function WitnessSection({ part }: { part: WitnessPart }) {
  if (part.state === "not_configured") {
    return (
      <Note tone="warn" testId="keys-witness-not-configured">
        No Keyryx witness is named on this box, so the console cannot say what it last signed. Set{" "}
        <span className="mono">{part.variable}</span> to the witness's base URL.
      </Note>
    );
  }
  if (part.state === "refused") {
    return (
      <Note tone="warn" testId="keys-witness-refused">
        <span className="mono">{part.variable}</span> is not usable: {visibleText(part.reason)}
      </Note>
    );
  }
  if (part.state === "unreachable") {
    return (
      <Note tone="warn" testId="keys-witness-unreachable">
        The witness at <span className="mono">{visibleText(part.url)}</span> did not answer: {visibleText(part.reason)}.
      </Note>
    );
  }
  if (part.state === "no_head_yet") {
    return (
      <Note tone="calm" testId="keys-witness-no-head">
        The witness at <span className="mono">{visibleText(part.url)}</span> has signed no tree head yet.
      </Note>
    );
  }

  return (
    <div data-testid="keys-witness-head" className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.8 }}>
      <div>
        tree size {part.size}, root {visibleText(part.root)}, signed {when(part.at)}
      </div>
      <div>ES384 key {part.es384_kid ? visibleText(part.es384_kid) : "none"}</div>
      <div>ML-DSA-65 key {part.ml_dsa_65_kid ? visibleText(part.ml_dsa_65_kid) : "none"}</div>
      <div style={{ color: "var(--faint)" }}>{visibleText(part.url)}</div>
    </div>
  );
}
