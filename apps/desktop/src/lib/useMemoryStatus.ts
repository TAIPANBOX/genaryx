import { useEffect, useState } from "react";
import { fetchMemoryStatus } from "./memory";
import type { MemoryStatus } from "../memoryTypes";

/** Poll cadence while the backend is still resolving (`state:"bootstrapping"`,
 * see `src-tauri/src/memory/state.rs`'s spawn-then-handshake bootstrap).
 * Mirrors `lib/useIdentityStatus.ts`'s identical constant. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - lines up with
 * `McpStdioClient::DEFAULT_TIMEOUT` (60s), the ceiling `EngramClient::spawn`
 * itself can legitimately take on a slow-starting `engram-mcp` process (a
 * cold Python interpreter + import cost) - see `memory::state`'s module doc
 * for why there is no separate, shorter outer timeout on the Rust side. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Memory view: fetches `memory_status`
 * once, and re-polls every [`POLL_MS`] while the backend is still
 * bootstrapping. Settles (stops polling) the moment the state is anything
 * other than "bootstrapping". Structurally identical to
 * `lib/useIdentityStatus.ts`'s `useIdentityStatus`, duplicated rather than
 * generalized over every panel since `MemoryStatus`/`IdentityStatus` are
 * independent Rust-side types that only happen to share this shape today.
 */
export function useMemoryStatus(): MemoryStatus | null {
  const [status, setStatus] = useState<MemoryStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchMemoryStatus();
      if (cancelled) return;
      setStatus(next);
      pollCount += 1;
      if (next.state === "bootstrapping" && pollCount < MAX_POLLS) {
        timer = setTimeout(() => void tick(), POLL_MS);
      }
    };
    void tick();

    return () => {
      cancelled = true;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, []);

  return status;
}
