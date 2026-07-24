import { hasBackend, invokeBackend } from "./transport";
import type {
  EngramForgetResult,
  EngramMemory,
  EngramProvenance,
  EngramStats,
  MemoryError,
  MemoryStatus,
  RecallMode,
} from "../memoryTypes";

/** Thrown by every fetcher below when there is no backend to talk to
 * (a plain `vite build`/browser preview) - mirrors `lib/quality.ts`'s
 * identical `NO_ENVIRONMENT_ERROR` guard: there is no mock memory plane to
 * fall back to, so this surfaces the same "no environment" state a real
 * engram-mcp-less box would show rather than inventing fake data. */
const NO_ENVIRONMENT_ERROR: MemoryError = { kind: "no_environment" };

/** Normalize whatever `invokeBackend()` rejected with into a `MemoryError`. genaryx-web
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `MemoryError` in disguise;
 * the fallback branch only matters for a transport-level failure. */
function toMemoryError(err: unknown): MemoryError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as MemoryError;
  }
  return { kind: "mcp", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toMemoryError(err);
  }
}

/** Whole-panel connection state. Never throws: with no backend (or on any
 * transport failure) it resolves to a renderable status instead - mirrors
 * `lib/quality.ts`'s `fetchQualityStatus` exactly. */
export async function fetchMemoryStatus(): Promise<MemoryStatus> {
  if (!hasBackend()) return { state: "no_environment" };
  try {
    return await invokeBackend<MemoryStatus>("memory_status");
  } catch (err) {
    return {
      state: "unreachable",
      source: { source: "well_known" },
      engram_mcp_bin: "",
      db_path: "",
      reason: err instanceof Error ? err.message : String(err),
    };
  }
}

/** `stats` - store-wide counts + fact validity + entities/reflections +
 * vector-index size + db path/size for `agentId` (empty/blank -> the
 * server's default scope). */
export const fetchStats = (agentId: string): Promise<EngramStats> =>
  call<EngramStats>("memory_stats", { agent_id: agentId.trim().length > 0 ? agentId : null });

/** `recall` - up to `limit` memories relevant to `query`, most relevant
 * first. Runs only on an explicit operator call, never automatically. */
export const recall = (
  query: string,
  limit: number,
  mode: RecallMode,
  agentId: string,
): Promise<EngramMemory[]> =>
  call<EngramMemory[]>("memory_recall", {
    query,
    limit,
    mode,
    agent_id: agentId.trim().length > 0 ? agentId : null,
  });

/** `why` - one memory's provenance. An unknown id rejects with a `mcp`-kind
 * `MemoryError` carrying the connector's own "memory not found" message -
 * never a fabricated empty result. */
export const fetchWhy = (memoryId: string): Promise<EngramProvenance> =>
  call<EngramProvenance>("memory_why", { memory_id: memoryId });

/** `forget` - permanently delete one memory. Irreversible; callers must
 * gate this behind their own confirm ceremony (see `MemoryProvenance.tsx`'s
 * use of `ConfirmButton`) - nothing here asks for confirmation on its own. */
export const forget = (memoryId: string): Promise<EngramForgetResult> =>
  call<EngramForgetResult>("memory_forget", { memory_id: memoryId });

/** Human-readable text for any `MemoryError` - used for the plain error
 * banner (mirrors `lib/quality.ts`'s `describeQualityError`). */
export function describeMemoryError(err: MemoryError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to an Engram memory plane.";
    case "no_environment":
      return "No Engram memory plane found.";
    case "unreachable":
      return `Could not start engram-mcp: ${err.reason}`;
    case "mcp":
      return err.message;
  }
}
