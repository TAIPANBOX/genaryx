import { AppShell } from "./components/AppShell";
import { WebGate } from "./components/WebGate";
import { DemoFunnel } from "./demo/DemoFunnel";
// `MOCK` from `mockPreview.ts` directly, not `lib/transport.ts`: transport.ts
// imports it too but never re-exports it, so this is the one public source
// for it, same as `transport.ts`'s own import.
import { MOCK } from "./lib/mockPreview";
import { PopoverProvider } from "./lib/popover";

/** The real console, unchanged: `WebGate` is the sign-in gate on a genuine
 * `genaryx-web` deployment, and passes straight through when there is none
 * to sign in to (`isWebShell()` false: a bare preview, or this build's own
 * `MOCK` transport). Split out so the demo path below can wrap it without
 * changing what it renders. */
function Console() {
  return (
    <WebGate>
      <PopoverProvider>
        <AppShell />
      </PopoverProvider>
    </WebGate>
  );
}

/**
 * App root.
 *
 * Two builds share this file (see `lib/transport.ts`'s own doc comment):
 * the real product (`VITE_GENARYX_API` set, or a bare preview with neither
 * flag) renders `Console` exactly as it always has, untouched by anything
 * below. The `VITE_GENARYX_MOCK` build additionally wraps it in the
 * it-rat.com "Live demo" funnel (`demo/DemoFunnel.tsx`): a sign-in mimic and
 * a "Connect this machine" theater step before the SAME `Console` the real
 * product renders, gated on the same `MOCK` flag `transport.ts` and
 * `mockPreview.ts` already key their own routing off of.
 */
export default function App() {
  if (!MOCK) return <Console />;
  return (
    <DemoFunnel>
      <Console />
    </DemoFunnel>
  );
}
