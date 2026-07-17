import { invoke, isTauri } from "@tauri-apps/api/core";
import type {
  EvidenceBuildRequest,
  EvidenceBuildResult,
  EvidenceError,
  EvidenceStatus,
} from "../evidenceTypes";

/** Thrown by every fetcher/mutator below when there is no Tauri runtime to
 * talk to - mirrors `lib/drills.ts`'s identical `NO_ENVIRONMENT_ERROR` guard.
 * Evidence has no "no_environment" status variant of its own (see
 * `evidenceTypes.ts`'s doc comment), so this folds into the same `build`
 * error shape every other IPC-transport failure does. */
const NO_TAURI_ERROR: EvidenceError = { kind: "build", message: "no Tauri runtime available" };

/** Normalize whatever `invoke()` rejected with into an `EvidenceError` -
 * mirrors `lib/drills.ts`'s `toDrillsError`. */
function toEvidenceError(err: unknown): EvidenceError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as EvidenceError;
  }
  return { kind: "build", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw NO_TAURI_ERROR;
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw toEvidenceError(err);
  }
}

/** Every source unresolved - the honest, settled fallback shape (see
 * `EVIDENCE_UNAVAILABLE`'s use sites below): NOT `bootstrapping`, which would
 * leave the panel showing "resolving..." forever outside a real Tauri
 * runtime (a plain `vite dev`/`vite preview` browser tab, or a genuine
 * IPC-transport failure) instead of settling like every sibling panel's own
 * `no_environment` fallback does. */
const EVIDENCE_UNAVAILABLE: EvidenceStatus = {
  state: "ready",
  qryx_available: false,
  qryx_bin: null,
  qryx_default_target: null,
  idryx_available: false,
  idryx_bin: null,
  idryx_load_sources: [],
  tokenfuse_available: false,
  tokenfuse_bin: null,
  tokenfuse_default_traces_dir: null,
};

/** Whole-panel local-tool availability. Never throws: `evidence_status`
 * itself never fails (see `evidence::commands::evidence_status`'s doc
 * comment), so the only way this catches is a genuine IPC-transport failure -
 * folded into the same honest "nothing resolved" shape a real fresh box with
 * no local tools would report, never a perpetual "resolving..." state. */
export async function fetchEvidenceStatus(): Promise<EvidenceStatus> {
  if (!isTauri()) return EVIDENCE_UNAVAILABLE;
  try {
    return await invoke<EvidenceStatus>("evidence_status");
  } catch {
    return EVIDENCE_UNAVAILABLE;
  }
}

/** `evidence_build` - assemble (and, when possible, sign) a pack from
 * whichever sources are checked, journal `console_evidence_built`, and
 * return the zip as base64 for the caller to save. Never auto-triggered,
 * only on an explicit "Build evidence pack" click. */
export const buildEvidence = (request: EvidenceBuildRequest): Promise<EvidenceBuildResult> =>
  call<EvidenceBuildResult>("evidence_build", { request });

/** Human-readable text for any `EvidenceError` - used for the plain error
 * banner, mirrors every sibling panel's `describe*Error`. */
export function describeEvidenceError(err: EvidenceError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still resolving the local evidence sources.";
    case "build":
      return err.message;
  }
}

/** Trigger a browser download of the built pack's zip bytes via a Blob + a
 * temporary `<a download>` - deliberately self-contained (no Tauri dialog
 * plugin, per this wave's scope). Decodes the base64 payload client-side. */
export function downloadEvidencePack(result: EvidenceBuildResult): void {
  const binary = atob(result.zip_base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  const blob = new Blob([bytes], { type: "application/zip" });
  const url = URL.createObjectURL(blob);
  try {
    const a = document.createElement("a");
    a.href = url;
    a.download = result.filename;
    a.style.display = "none";
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  } finally {
    URL.revokeObjectURL(url);
  }
}
