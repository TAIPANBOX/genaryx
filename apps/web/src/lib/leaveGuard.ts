/**
 * Asking before the page, or the console's view, is left while something on it
 * would be lost for good.
 *
 * One thing holds it today: an approval token on screen that has not expired and
 * has not been copied (the list of tokens in `components/ApprovalsInbox.tsx`).
 * Wardryx hands that token over once, so a reload, a closed tab, a typed address
 * or a change of view loses it with no way back. A reload or a closed tab raises
 * the browser's own question through `beforeunload`; a change of view inside the
 * console goes through `AppShell`'s `setView`, which asks {@link mayLeave}.
 *
 * This module holds no token and nothing about one, only how many holds there are
 * (CLAUDE.md invariant 46).
 */

import { useEffect } from "react";

/** What a change of view inside the console asks. For a reload or a closed tab the
 * browser writes its own words and shows none of these. */
export const LEAVE_QUESTION =
  "An approval token on this page is not copied yet, and it cannot be shown again. Leave anyway?";

const holds = new Set<symbol>();

function askBeforeUnload(event: BeforeUnloadEvent): void {
  event.preventDefault();
  // Browsers that predate preventDefault() here ask only when returnValue is set.
  event.returnValue = LEAVE_QUESTION;
}

/** Hold the page until the returned release is called. Holds stack: the page is
 * let go once every one of them is released. */
export function holdLeave(): () => void {
  const hold = Symbol("leave-hold");
  if (holds.size === 0) window.addEventListener("beforeunload", askBeforeUnload);
  holds.add(hold);
  return () => {
    if (holds.delete(hold) && holds.size === 0) window.removeEventListener("beforeunload", askBeforeUnload);
  };
}

/** Whether a change of view may go ahead: at once when nothing holds the page,
 * otherwise only when the person says so. */
export function mayLeave(): boolean {
  return holds.size === 0 || window.confirm(LEAVE_QUESTION);
}

/** Hold the page while `held` is true and the calling component is mounted. */
export function useLeaveGuard(held: boolean): void {
  useEffect(() => (held ? holdLeave() : undefined), [held]);
}
