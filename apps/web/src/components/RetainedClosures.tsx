import { useCallback, useEffect, useState } from "react";
import { formatUsd } from "../lib/format";
import {
  buildCloseArgs,
  closeRetained,
  describeRetainedError,
  listRetained,
} from "../lib/retainedClosure";
import type {
  CloseForm,
  CloseOutcome,
  EvidenceKind,
  RetainedError,
  RetainedReservation,
} from "../lib/retainedClosure";
import { Section } from "./dash";

const REFRESH_INTERVAL_MS = 20_000;

const inputStyle = {
  background: "var(--panel-2)",
  color: "var(--text)",
  border: "1px solid var(--line)",
  fontSize: 11.5,
} as const;

function isReservationList(d: unknown): d is RetainedReservation[] {
  return (
    Array.isArray(d) &&
    d.every(
      (r) =>
        typeof r === "object" &&
        r !== null &&
        typeof (r as Record<string, unknown>).reservation_id === "number" &&
        typeof (r as Record<string, unknown>).revision === "string" &&
        typeof (r as Record<string, unknown>).run_id === "string" &&
        typeof (r as Record<string, unknown>).amount_microusd === "number",
    )
  );
}

/**
 * The Close form for one reservation: evidence kind, the observed cost in
 * micro-USD (observed only), and a reference to the evidence. Submitting
 * runs the passkey ceremony (`closeRetained` -> `invokeWithCeremony`); a form
 * the gateway would refuse is refused here first and never asks for a key.
 */
function CloseRow({
  row,
  onClose,
}: {
  row: RetainedReservation;
  onClose: (row: RetainedReservation, form: CloseForm) => Promise<string | null>;
}) {
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<EvidenceKind>("observed");
  const [observed, setObserved] = useState("");
  const [ref, setRef] = useState("");
  const [pending, setPending] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const submit = async () => {
    setPending(true);
    setProblem(null);
    const why = await onClose(row, { evidenceKind: kind, observedMicrousd: observed, evidenceRef: ref });
    setPending(false);
    if (why) setProblem(why);
    else setOpen(false);
  };

  return (
    <div className="px-4 py-2" style={{ borderTop: "1px solid var(--line)" }}>
      <div className="flex items-center justify-between gap-2">
        <div className="mono min-w-0" style={{ fontSize: 11.5 }}>
          <div style={{ color: "var(--text)" }} title={`revision ${row.revision}`}>
            #{row.reservation_id} · {row.run_id}
          </div>
          <div style={{ color: "var(--faint)", fontSize: 10.5 }}>
            {formatUsd(row.amount_microusd / 1_000_000)} held{row.unit ? ` · ${row.unit}` : ""}
            {row.model ? ` · ${row.model}` : ""}
          </div>
        </div>
        {!open && (
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 10px", fontSize: 11, color: "var(--sev-high)" }}
            onClick={() => setOpen(true)}
          >
            Close
          </button>
        )}
      </div>
      {open && (
        <form
          className="mt-2 flex flex-col gap-1.5"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <label className="mono flex items-center gap-2" style={{ fontSize: 11 }}>
            evidence
            <select
              value={kind}
              onChange={(e) => setKind(e.target.value as EvidenceKind)}
              className="rounded px-1 py-0.5"
              style={inputStyle}
            >
              <option value="observed">observed cost</option>
              <option value="no_charge">no charge</option>
            </select>
          </label>
          {kind === "observed" && (
            <input
              inputMode="numeric"
              placeholder="observed micro-USD, e.g. 4210"
              value={observed}
              onChange={(e) => setObserved(e.target.value)}
              className="mono rounded px-2 py-1"
              style={inputStyle}
            />
          )}
          <input
            placeholder="evidence reference (invoice line, usage record)"
            value={ref}
            maxLength={512}
            onChange={(e) => setRef(e.target.value)}
            className="mono rounded px-2 py-1"
            style={inputStyle}
          />
          {problem && (
            <div className="mono" style={{ fontSize: 10.5, color: "var(--sev-high)" }}>
              {problem}
            </div>
          )}
          <div className="flex gap-1.5">
            <button
              type="submit"
              className="icon-btn"
              disabled={pending}
              style={{ width: "auto", padding: "0 10px", fontSize: 11, color: "var(--sev-high)" }}
            >
              {pending ? "Confirming..." : "Confirm with passkey"}
            </button>
            <button
              type="button"
              className="icon-btn"
              disabled={pending}
              style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
              onClick={() => setOpen(false)}
            >
              Cancel
            </button>
          </div>
        </form>
      )}
    </div>
  );
}

/**
 * The pure rendering half: explicit props in, JSX out, so every state can be
 * rendered and asserted on without a gateway.
 */
export function RetainedClosuresBody({
  rows,
  error,
  lastClosed,
  onClose,
}: {
  rows: unknown;
  error: RetainedError | null;
  lastClosed: CloseOutcome | null;
  onClose: (row: RetainedReservation, form: CloseForm) => Promise<string | null>;
}) {
  const note = (text: string, color = "var(--faint)") => (
    <Section title="Close a retained reservation">
      <div className="mono px-4 py-4" style={{ fontSize: 11.5, color }}>
        {text}
      </div>
    </Section>
  );
  if (error) return note(describeRetainedError(error), "var(--sev-high)");
  if (rows === null) return note("loading...");
  if (!isReservationList(rows)) return note("the gateway's retained list could not be read.", "var(--sev-high)");

  return (
    <Section title="Close a retained reservation" right={<span className="mono" style={{ fontSize: 11, color: "var(--faint)" }}>{rows.length} open</span>}>
      <div className="mono px-4 pt-2 pb-1" style={{ fontSize: 10.5, color: "var(--faint)", lineHeight: 1.5 }}>
        Closing settles a reservation at the cost the evidence shows, or releases it when nothing was billed. It needs
        your passkey, and every attempt is recorded in the console journal under your name.
      </div>
      {lastClosed && (
        <div className="mono px-4 pb-1" style={{ fontSize: 10.5, color: "var(--dim)" }}>
          closed {lastClosed.replayed ? "(already closed by this operation) " : ""}
          as {lastClosed.operation_id}
          {lastClosed.settled_microusd !== null ? `, settled ${lastClosed.settled_microusd} micro-USD` : ""}
          {lastClosed.bus_recorded ? "" : ` - NOT journaled: ${lastClosed.bus_error ?? "unknown"}`}
        </div>
      )}
      {rows.length === 0 ? (
        <div className="mono px-4 py-3" style={{ fontSize: 11.5, color: "var(--faint)" }}>
          no retained reservations to close
        </div>
      ) : (
        rows.map((r) => <CloseRow key={`${r.reservation_id}-${r.revision}`} row={r} onClose={onClose} />)
      )}
    </Section>
  );
}

/** D8 in the Money panel: the gateway's retained reservations, each with a
 * Close action that runs the passkey ceremony and takes the evidence. */
export function RetainedClosuresSection() {
  const [rows, setRows] = useState<RetainedReservation[] | null>(null);
  const [error, setError] = useState<RetainedError | null>(null);
  const [lastClosed, setLastClosed] = useState<CloseOutcome | null>(null);

  const load = useCallback(async () => {
    try {
      setRows(await listRetained());
      setError(null);
    } catch (err) {
      setError(err as RetainedError);
    }
  }, []);

  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [load]);

  const onClose = useCallback(
    async (row: RetainedReservation, form: CloseForm): Promise<string | null> => {
      const built = buildCloseArgs(row, form);
      if (!built.ok) return built.reason;
      try {
        setLastClosed(await closeRetained(built.args));
        void load();
        return null;
      } catch (err) {
        return describeRetainedError(err as RetainedError);
      }
    },
    [load],
  );

  return <RetainedClosuresBody rows={rows} error={error} lastClosed={lastClosed} onClose={onClose} />;
}
