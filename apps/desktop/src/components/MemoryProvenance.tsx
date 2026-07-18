import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeMemoryError, fetchWhy, forget } from "../lib/memory";
import type { EngramForgetResult, EngramProvenance, MemoryError } from "../memoryTypes";
import { ConfirmButton } from "./ConfirmButton";

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-[11px] shrink-0" style={{ color: "var(--faint)" }}>
        {label}
      </span>
      <span
        className="mono tabular truncate text-[12px]"
        style={{ color: "var(--fg)" }}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

/**
 * Why / provenance (docs/PHASE4.md W2 Memory position 3): selecting a
 * memory id runs `why` and branches on `kind` - a semantic fact shows the
 * triple + extraction chain; an episodic memory shows content +
 * encoding/access metadata. An unknown id is the connector's own honest
 * "memory not found" Tool error, shown as such (never a fabricated empty
 * result - `describeMemoryError`'s `mcp` branch surfaces it verbatim).
 *
 * Also hosts the optional, irreversible `forget` admin action
 * (docs/PHASE4.md W2 Memory position 6), gated behind `ConfirmButton`'s
 * plain (non-break-glass) confirm ceremony: `EngramClient::forget` takes no
 * `reason` field to collect one for (unlike Money's kill/set-budget), so a
 * break-glass justification modal here would collect text that goes nowhere
 * - a clear "irreversible" warning plus an explicit Confirm/Cancel step is
 * the honest amount of friction.
 */
export function MemoryProvenance({
  memoryId,
  onForgotten,
}: {
  memoryId: string | null;
  onForgotten: (memoryId: string) => void;
}) {
  const [provenance, setProvenance] = useState<EngramProvenance | null>(null);
  const [error, setError] = useState<MemoryError | null>(null);
  const [loading, setLoading] = useState(false);
  const [forgetResult, setForgetResult] = useState<EngramForgetResult | null>(null);
  const [forgetError, setForgetError] = useState<MemoryError | null>(null);

  useEffect(() => {
    setForgetResult(null);
    setForgetError(null);
    if (!memoryId) {
      setProvenance(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchWhy(memoryId)
      .then((p) => {
        if (!cancelled) setProvenance(p);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setProvenance(null);
          setError(err as MemoryError);
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [memoryId]);

  const onForget = useCallback(async () => {
    if (!memoryId) return;
    try {
      const result = await forget(memoryId);
      setForgetResult(result);
      setForgetError(null);
      onForgotten(memoryId);
    } catch (err) {
      setForgetError(err as MemoryError);
    }
  }, [memoryId, onForgotten]);

  if (!memoryId) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        select a memory above to see its provenance.
      </div>
    );
  }

  if (loading) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        loading provenance...
      </div>
    );
  }

  if (error) {
    return (
      <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
        {describeMemoryError(error)}
      </div>
    );
  }

  if (!provenance) return null;

  return (
    <div className="d-card px-4 py-3.5 flex flex-col gap-2.5">
      <div className="flex items-center justify-between gap-2">
        <span
          className="badge"
          style={cssVar(
            "tone",
            provenance.kind === "semantic" ? "var(--src-verdryx)" : "var(--src-engram)",
          )}
        >
          {provenance.kind}
        </span>
        <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
          {provenance.id}
        </span>
      </div>

      {provenance.kind === "semantic" ? (
        <div className="flex flex-col gap-1.5">
          <Field label="subject" value={provenance.subject} />
          <Field label="predicate" value={provenance.predicate} />
          <Field label="object" value={provenance.object} />
          <Field label="confidence" value={provenance.confidence.toFixed(3)} />
          <Field label="valid from" value={provenance.valid_from} />
          <Field label="valid to" value={provenance.valid_to ?? "still valid"} />
          <Field label="recorded at" value={provenance.recorded_at} />
          <Field label="extracted from" value={provenance.extracted_from ?? "-"} />
          <Field label="reflection run" value={provenance.extracted_by_reflection_run ?? "-"} />
          <Field label="extraction model" value={provenance.extraction_model ?? "-"} />
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          <Field label="content" value={provenance.content} />
          <Field label="timestamp" value={provenance.timestamp} />
          <Field label="actors" value={provenance.actors.length > 0 ? provenance.actors.join(", ") : "-"} />
          <Field label="tags" value={provenance.tags.length > 0 ? provenance.tags.join(", ") : "-"} />
          <Field
            label="salience"
            value={provenance.salience !== null ? provenance.salience.toFixed(3) : "n/a"}
          />
          <Field
            label="emotional valence"
            value={provenance.emotional_valence !== null ? provenance.emotional_valence.toFixed(3) : "n/a"}
          />
          <Field
            label="importance score"
            value={provenance.importance_score !== null ? provenance.importance_score.toFixed(3) : "n/a"}
          />
          <Field label="summary of" value={provenance.summary_of ?? "-"} />
          <Field label="agent" value={provenance.agent_id ?? "-"} />
          <Field label="access count" value={String(provenance.access_count)} />
          <Field label="last accessed" value={provenance.last_accessed ?? "never"} />
          {provenance.note && <Field label="note" value={provenance.note} />}
        </div>
      )}

      <div className="flex items-center gap-2 pt-2" style={{ borderTop: "1px solid var(--line-2)" }}>
        <span className="text-[11px] flex-1" style={{ color: "var(--faint)" }}>
          Forget permanently deletes this memory from Engram. Irreversible.
        </span>
        <ConfirmButton
          label="Forget"
          confirmLabel="Permanently delete"
          pendingLabel="Deleting..."
          onConfirm={onForget}
        />
      </div>
      {forgetError && (
        <span className="mono text-[11.5px]" style={{ color: "var(--sev-high)" }}>
          {describeMemoryError(forgetError)}
        </span>
      )}
      {forgetResult && (
        <span
          className="mono text-[11.5px]"
          style={{ color: forgetResult.deleted ? "var(--sev-low)" : "var(--sev-high)" }}
        >
          {forgetResult.deleted ? "Deleted" : "Not deleted"} - {forgetResult.kind} {forgetResult.id}
        </span>
      )}
    </div>
  );
}
