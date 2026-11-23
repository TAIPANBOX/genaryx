import { useEffect, useState } from "react";
import { chainSentence, fetchNotifications, type NotificationsRead } from "../lib/sentJournal";
import { useConsoleStateVersion } from "../lib/consoleState";
import { formatTimestamp } from "../lib/format";
import { visibleText } from "../lib/policyExport";
import type { SentRecord } from "../notificationsTypes";

/**
 * Notifications: what heraldyx recorded sending, and to whom (invariant 51).
 *
 * Read-only, from heraldyx's own dispatch journal on this box. It shows the
 * record and nothing else: no resend, no test mail, no address book. Every
 * string in it is the journal's (recipients, the dedup key, a transport's
 * refusal), so each goes through `visibleText` and renders as a text node.
 *
 * Four states never render alike: no journal named on this box, a journal
 * that could not be read, an empty journal, and a journal with messages.
 */
export function NotificationsView() {
  const [read, setRead] = useState<NotificationsRead | null | undefined>(undefined);
  const version = useConsoleStateVersion();

  useEffect(() => {
    let alive = true;
    const load = async () => {
      const r = await fetchNotifications();
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
          Notifications
        </h1>
        <p className="text-[12px]" style={{ color: "var(--dim)", margin: 0, maxWidth: 760 }}>
          What heraldyx recorded sending, and to whom, read from its own journal on this box. Accepted
          means a mail server took the message, not that it reached a mailbox. Nothing here sends mail.
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

function Body({ read }: { read: NotificationsRead | null | undefined }) {
  if (read === undefined) return <Note tone="calm">reading the journal...</Note>;
  if (read === null) return <Note tone="calm">No box to ask: this build has no backend.</Note>;
  if (!read.ok) {
    return (
      <Note tone="warn" testId="notifications-error">
        The box did not answer for the journal: {visibleText(read.reason)}
      </Note>
    );
  }
  const p = read.panel;
  if (p.state === "not_configured") {
    return (
      <Note tone="warn" testId="notifications-not-configured">
        No heraldyx journal is named on this box, so the console cannot say what was sent. Mount
        heraldyx's state volume read-only and set <span className="mono">{p.variable}</span> to its{" "}
        <span className="mono">sent.ndjson</span>.
      </Note>
    );
  }
  if (p.state === "unreadable") {
    return (
      <Note tone="warn" testId="notifications-unreadable">
        The journal at <span className="mono">{visibleText(p.path)}</span> could not be read:{" "}
        {visibleText(p.reason)}
      </Note>
    );
  }

  const kinds = Object.entries(p.by_kind)
    .map(([k, n]) => `${visibleText(k || "(none)")} ${n}`)
    .join(", ");
  return (
    <>
      <div className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.8 }}>
        <div data-testid="notifications-summary">
          {p.total} message(s) recorded{kinds ? ` (${kinds})` : ""}: {p.accepted} accepted, {p.refused} refused
        </div>
        <div className="mono" style={{ color: "var(--faint)" }}>
          {visibleText(p.path)}
        </div>
      </div>
      <Note tone={p.chain.state === "broken" ? "bad" : "calm"} testId="notifications-chain">
        {chainSentence(p.chain, p.window_truncated)}
      </Note>
      {p.window_truncated && (
        <Note tone="calm">
          The journal is longer than one read: the counts are over its newest part.
        </Note>
      )}
      {p.skipped_total > 0 && (
        <Note tone="warn" testId="notifications-skipped">
          {p.skipped_total} line(s) are not heraldyx records and are not shown:{" "}
          {p.skipped.map((s) => `line ${s.line}, ${visibleText(s.reason)}`).join("; ")}
          {p.skipped_total > p.skipped.length ? "; ..." : ""}
        </Note>
      )}
      {p.total === 0 ? (
        <Note tone="calm" testId="notifications-empty">
          heraldyx has recorded no messages in this journal.
        </Note>
      ) : (
        <RecordsTable records={p.records} total={p.total} />
      )}
    </>
  );
}

const cell: React.CSSProperties = { padding: "6px 10px", verticalAlign: "top", borderBottom: "1px solid var(--line-2)" };

function RecordsTable({ records, total }: { records: SentRecord[]; total: number }) {
  return (
    <div className="min-h-0 overflow-auto">
      {records.length < total && (
        <div className="text-[11px]" style={{ color: "var(--faint)", paddingBottom: 6 }}>
          newest {records.length} of {total}
        </div>
      )}
      <table data-testid="notifications-table" className="text-[12px]" style={{ borderCollapse: "collapse", width: "100%", minWidth: 920 }}>
        <thead>
          <tr style={{ color: "var(--faint)", textAlign: "left" }}>
            {["when", "kind", "about", "written to", "transport", "outcome", "agent"].map((h) => (
              <th key={h} className="text-[10.5px] uppercase tracking-wider" style={{ ...cell, fontWeight: 500 }}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {records.map((r) => (
            <tr key={r.line} style={{ color: "var(--dim)" }}>
              <td className="mono" style={cell}>
                {formatTimestamp(r.ts)}
              </td>
              <td style={cell}>{visibleText(r.kind)}</td>
              <td className="mono" style={{ ...cell, overflowWrap: "anywhere" }}>
                {visibleText(r.about)}
              </td>
              <td className="mono" style={{ ...cell, overflowWrap: "anywhere" }}>
                {r.to.map(visibleText).join(", ")}
                {r.to_total > r.to.length ? ` and ${r.to_total - r.to.length} more` : ""}
              </td>
              <td style={cell}>{visibleText(r.transport)}</td>
              <td style={{ ...cell, color: r.outcome === "refused" ? "var(--sev-high)" : "var(--fg)" }}>
                {visibleText(r.outcome)}
                {r.error ? <div style={{ color: "var(--dim)" }}>{visibleText(r.error)}</div> : null}
              </td>
              <td className="mono" style={{ ...cell, overflowWrap: "anywhere" }}>
                {visibleText(r.agent_id)}
                {r.run_id ? <div style={{ color: "var(--faint)" }}>{visibleText(r.run_id)}</div> : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
