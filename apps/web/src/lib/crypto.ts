import { hasBackend, invokeBackend } from "./transport";
import type { CryptoError, CryptoStatus, EvidenceReport, NcscReport, VerifyOutcome } from "../cryptoTypes";

/** Thrown by every fetcher below when there is no backend to talk to -
 * mirrors `lib/identity.ts`'s identical `NO_ENVIRONMENT_ERROR` guard. */
const NO_ENVIRONMENT_ERROR: CryptoError = { kind: "no_environment" };

/** Normalize whatever `invokeBackend()` rejected with into a `CryptoError` - mirrors
 * `lib/quality.ts`'s `toQualityError`. */
function toCryptoError(err: unknown): CryptoError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as CryptoError;
  }
  return { kind: "qryx", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toCryptoError(err);
  }
}

/** Whole-panel connection state. Never throws: `crypto_status` itself never
 * fails (see `crypto::commands::crypto_status`'s doc comment), so the only
 * way this catches is a genuine IPC-transport failure - folded into the
 * same honest "no crypto plane" state a missing qryx binary would show. */
export async function fetchCryptoStatus(): Promise<CryptoStatus> {
  if (!hasBackend()) return { state: "no_environment" };
  try {
    return await invokeBackend<CryptoStatus>("crypto_status");
  } catch {
    return { state: "no_environment" };
  }
}

/** `qryx scan --format ncsc <path>` - the PQC readiness timeline. */
export const scanNcsc = (path: string): Promise<NcscReport> =>
  call<NcscReport>("crypto_scan_ncsc", { path });

/** `qryx scan --format cbom <path>` - the CycloneDX crypto-component
 * inventory, untyped (see `cryptoTypes.ts`'s doc comment). */
export const scanCbom = (path: string): Promise<unknown> => call<unknown>("crypto_scan_cbom", { path });

/** `qryx scan --format evidence <path>` - always unsigned for W1 (see
 * `crypto::commands::crypto_scan_evidence`'s doc comment). */
export const scanEvidence = (path: string): Promise<EvidenceReport> =>
  call<EvidenceReport>("crypto_scan_evidence", { path, sign_key: null });

/** `qryx verify-evidence <file>` - a saved evidence JSON file already on
 * disk, deliberately decoupled from `scanEvidence`'s in-memory result (see
 * `crypto::commands`'s module doc for why). */
export const verifyEvidence = (file: string): Promise<VerifyOutcome> =>
  call<VerifyOutcome>("crypto_verify_evidence", { file });

/** Human-readable text for any `CryptoError` - used for the plain error
 * banner. */
export function describeCryptoError(err: CryptoError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still resolving the qryx binary.";
    case "no_environment":
      return "No qryx binary found at ~/.taipan/bin/qryx.";
    case "qryx":
      return err.message;
  }
}
