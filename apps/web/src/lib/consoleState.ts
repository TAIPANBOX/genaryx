import { useEffect, useState } from "react";

/**
 * A tiny, app-wide "the console's manual-lifecycle state just changed" signal,
 * so a Stop/Freeze/Kill/Start/Unfreeze issued from ANY panel is reflected on
 * EVERY open panel within a beat, not only where it was clicked and not only
 * on that panel's own next poll.
 *
 * Deliberately mirrors `WatchDock.tsx`'s existing `WATCH_CHANGED_EVENT` pattern
 * (a plain `window` `Event`, a `useState` counter that bumps on it) rather than
 * pulling in a store dependency: the app already leans on this exact "dispatch
 * a bare event, views bump a version and refetch" shape for the watch dock's
 * pin list, so a second axis of the same shape stays consistent with what is
 * here.
 *
 * This is only ever the trigger to refetch: the numbers themselves still come
 * from the reads (`money_runs`, `agent_record`, ...), which the mock world
 * (`lib/mockPreview.ts`) drives from its one lifecycle store, and which a real
 * box answers from its own state. So on a real box where an unimplemented
 * command was a no-op, the refetch this fires simply re-reads the unchanged
 * state - honest, never a faked local mutation.
 */
export const CONSOLE_STATE_CHANGED_EVENT = "genaryx:console-state-changed";

/** Fire the signal after a successful lifecycle mutation. Called from the
 * command wrappers (`blockAgent`/`blockUnit`/`blockUser`/`killRun`) so EVERY
 * call site - the watch dock, the cards, the Money runs board - broadcasts
 * without repeating this itself. Best-effort and SSR-safe: no `window` (or a
 * dispatch that throws) simply means no one is listening. */
export function notifyConsoleStateChanged(): void {
  if (typeof window === "undefined") return;
  try {
    window.dispatchEvent(new Event(CONSOLE_STATE_CHANGED_EVENT));
  } catch {
    // No window / event constructor (SSR, a locked-down embed) - nothing to
    // notify, and never worth throwing a UI action over.
  }
}

/** A monotonic counter that bumps once per {@link CONSOLE_STATE_CHANGED_EVENT}.
 * Put it in a view's refetch-effect deps (alongside its own interval) so the
 * view re-reads the moment any lifecycle action lands, instead of waiting out
 * its poll cadence. Returns a number rather than a callback so a view opts in
 * with one line and no new subscription bookkeeping of its own. */
export function useConsoleStateVersion(): number {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const onChange = () => setVersion((v) => v + 1);
    window.addEventListener(CONSOLE_STATE_CHANGED_EVENT, onChange);
    return () => window.removeEventListener(CONSOLE_STATE_CHANGED_EVENT, onChange);
  }, []);
  return version;
}
