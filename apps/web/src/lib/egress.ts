import { hasBackend, invokeBackend } from "./transport";
import type { EgressError, EgressPanel } from "../egressTypes";

/** Thrown when there is no backend to talk to (a plain `vite build` or a
 * browser preview), mirroring `lib/quality.ts`'s identical guard.
 *
 * There is deliberately NO mock egress data behind this. Every other panel that
 * falls back to mocks does so for a surface where a plausible shape helps a
 * reader see the layout; here a plausible shape is a list of web requests that
 * never happened, and this panel's whole job is to be believed about what
 * agents actually reached. */
const NO_ENVIRONMENT_ERROR: EgressError = { kind: "no_environment" };

function toEgressError(err: unknown): EgressError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as EgressError;
  }
  return { kind: "backend", message: err instanceof Error ? err.message : String(err) };
}

/** Recent egress activity, newest first.
 *
 * `limit` bounds the ROWS RETURNED. The backend reads a wider slice of the bus
 * and keeps the egress lines out of it, because a store busy with money-plane
 * traffic and one fetch an hour would otherwise show nothing. What was actually
 * read is stated in `panel.note`. */
export async function fetchEgress(limit = 100): Promise<EgressPanel> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  let raw: unknown;
  try {
    raw = await invokeBackend<unknown>("egress_recent", { limit });
  } catch (err) {
    throw toEgressError(err);
  }
  // An answer that is not a panel is an ERROR, never a panel-shaped nothing.
  //
  // The mock transport answers `null` for any command it does not know, and a
  // real backend can answer a body this build cannot read. Returning `null`
  // upward made the view hold "loading..." for ever, because a component that
  // has no panel yet and a component whose panel came back empty look
  // identical. Found by opening the view rather than by reading the code.
  if (!raw || typeof raw !== "object" || !("measured" in raw)) {
    throw {
      kind: "backend",
      message:
        "This build asked for the egress record and got an answer it could not read. " +
        "That is not a report that your agents made no web requests.",
    } as EgressError;
  }
  return raw as EgressPanel;
}

/** How many of the fetches shown were governed only at the navigation, as a
 * share, or null when there were no fetches at all.
 *
 * Null rather than 0, and the caller must render the difference. Zero per cent
 * of nothing is not a reassuring number, it is an absent one, and a dial
 * reading 0% beside "0 fetches" is the kind of thing an operator remembers as
 * "everything was fully enforced". */
export function navigationOnlyShare(panel: EgressPanel): number | null {
  if (panel.totals.fetched === 0) return null;
  return panel.totals.navigation_only / panel.totals.fetched;
}
