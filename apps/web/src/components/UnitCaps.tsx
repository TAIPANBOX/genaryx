import { useCallback, useEffect, useState } from "react";
import { describeMoneyError, fetchUnitCaps, setUnitBudget } from "../lib/money";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { notifyConsoleStateChanged } from "../lib/consoleState";
import { capLabel, monthSpentLabel, UNIT_CAP_PILL, UNIT_CAPS_CAVEAT, unitCapsCaption } from "../lib/unitCaps";
import type { MoneyError, UnitCaps } from "../moneyTypes";
import { BudgetEditor } from "./BudgetEditor";
import { FuseBar } from "./FuseBar";
import { Section } from "./dash";

const REFRESH_INTERVAL_MS = 20_000;

const COLS = "minmax(150px, 1fr) 200px 70px 120px 170px";

/**
 * Unit month caps: every business unit, its central monthly cap, what it has
 * spent this month, and where it stands. Pure props, so every state renders
 * in a test without a backend.
 */
export function UnitCapsBody({
  data,
  error,
  onSetCap,
}: {
  data: UnitCaps | null;
  error: MoneyError | null;
  onSetCap: (unit: string, budgetUsd: number, reason: string) => Promise<void>;
}) {
  if (error) {
    return (
      <div className="mono" style={{ fontSize: 11.5, color: "var(--sev-high)", padding: "14px 20px" }}>
        unit caps could not be read: {describeMoneyError(error)}
      </div>
    );
  }
  if (data === null) {
    return (
      <div className="mono" style={{ fontSize: 12, color: "var(--faint)", padding: "14px 20px" }}>
        reading unit caps...
      </div>
    );
  }
  return (
    <div>
      {data.units.length === 0 ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)", padding: "14px 20px" }}>
          no unit has spent this month and no central cap is set.
        </div>
      ) : (
        <div style={{ overflowX: "auto", contain: "inline-size" }}>
          <div style={{ minWidth: 760 }}>
            <div className="d-th" style={{ gridTemplateColumns: COLS }}>
              <span>unit</span>
              <span>spent this month / cap</span>
              <span className="r">calls</span>
              <span>state</span>
              <span className="r"> </span>
            </div>
            {data.units.map((u) => {
              const pill = UNIT_CAP_PILL[u.state];
              return (
                <div className="d-tr" style={{ gridTemplateColumns: COLS }} key={u.unit}>
                  <div className="mono" style={{ fontSize: 12 }} title={u.unit}>
                    {u.unit}
                  </div>
                  <div className="d-spentcell">
                    <div className="amt">
                      <span>{monthSpentLabel(u)}</span>
                      <span className="cap">{capLabel(u, data)}</span>
                    </div>
                    {u.fraction !== null && <FuseBar fraction={u.fraction} />}
                  </div>
                  <div className="d-num cell-r">{u.month_calls !== null ? u.month_calls.toLocaleString("en-US") : "-"}</div>
                  <div>
                    <span className={`d-pill ${pill.cls}`} title={pill.title}>
                      {pill.label}
                    </span>
                  </div>
                  <div className="d-acts">
                    {u.cappable ? (
                      <BudgetEditor runId={u.unit} currentUsd={u.cap_usd} onSubmit={onSetCap} subject="unit" />
                    ) : (
                      <span
                        className="mono"
                        style={{ fontSize: 10.5, color: "var(--faint)" }}
                        title="Spend whose run resolved no unit. No gateway can apply a unit cap to it."
                      >
                        unmapped spend
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}
      {data.notes.map((n) => (
        <div key={n} className="mono" style={{ fontSize: 11, color: "var(--sev-medium)", padding: "6px 20px 0" }}>
          {n}
        </div>
      ))}
      <div className="mono" style={{ fontSize: 10.5, color: "var(--faint)", padding: "8px 20px 12px", lineHeight: 1.5 }}>
        {UNIT_CAPS_CAVEAT}
      </div>
    </div>
  );
}

/** The Money tab's unit caps section: reads `money_units` every 20 s and sets
 * a cap through the passkey ceremony. */
export function UnitCapsSection() {
  const status = useMoneyStatus();
  const ready = status?.state === "ready";
  const [data, setData] = useState<UnitCaps | null>(null);
  const [error, setError] = useState<MoneyError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setData(await fetchUnitCaps());
      setError(null);
    } catch (err) {
      setError(err as MoneyError);
    }
  }, []);

  useEffect(() => {
    if (!ready) return;
    void load();
    const id = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [ready, load]);

  const onSetCap = useCallback(
    async (unit: string, budgetUsd: number, reason: string) => {
      try {
        const outcome = await setUnitBudget(unit, budgetUsd, reason);
        setNotice(outcome.summary);
        notifyConsoleStateChanged();
        await load();
      } catch (err) {
        setNotice(describeMoneyError(err as MoneyError));
        throw err;
      }
    },
    [load],
  );

  return (
    <Section title="Unit month caps" right={data ? unitCapsCaption(data) : undefined}>
      {notice && (
        <div className="mono" style={{ fontSize: 11.5, color: "var(--dim)", padding: "6px 20px" }}>
          {notice}
        </div>
      )}
      <UnitCapsBody data={data} error={error} onSetCap={onSetCap} />
    </Section>
  );
}
