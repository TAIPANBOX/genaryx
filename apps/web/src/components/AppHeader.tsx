import { cssVar } from "../lib/cssVars";
import { usePopover } from "../lib/popover";
import type { ConsoleRole } from "../lib/session";
import { useSession } from "../lib/useSession";
import type { ViewId } from "../lib/views";
import { VIEWS } from "../lib/views";
import { PasskeySettings } from "./PasskeySettings";

/** The shared TAIPANBOX/IT-RAT bolt glyph (it-rat2 topbar brand mark),
 * inline SVG, no raster. */
function BrandMark() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <path d="M13.5 2 5 13.2h5.1L9.4 22l9-11.8h-5.3L13.5 2Z" fill="var(--sev-medium)" />
    </svg>
  );
}

function SunIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="4" stroke="currentColor" strokeWidth="2" />
      <path
        d="M12 2v2.5M12 19.5V22M4.2 4.2l1.8 1.8M18 18l1.8 1.8M2 12h2.5M19.5 12H22M4.2 19.8 6 18M18 6l1.8-1.8"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}

function MoonIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" aria-hidden="true">
      <path
        d="M20 14.5A8.5 8.5 0 1 1 9.5 4a7 7 0 0 0 10.5 10.5Z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Role -> badge tone, low to high privilege (docs/CONSOLE-IDP.md's three
 * console roles). Purely a privilege ladder, not a good/bad judgment, so it
 * borrows the same severity-scale variables the rest of the app already uses
 * for that kind of low-to-high read rather than inventing new ones. */
const ROLE_TONE: Record<ConsoleRole, string> = {
  viewer: "var(--sev-info)",
  approver: "var(--sev-medium)",
  admin: "var(--sev-high)",
};

/**
 * Who is signed in, and with what privilege - unobtrusive but always visible
 * once inside, so an operator never has to go hunting for either
 * (docs/CONSOLE-IDP.md: named audit actors + roles). Web-only: the desktop
 * shell has no console session (`useSession()` resolves to `null` there,
 * `WebGate.tsx`'s own gate never runs for it either), so this renders nothing
 * rather than an empty placeholder - the ONE guard that keeps a header shared
 * with the sessionless desktop shell honest.
 *
 * The "Passkeys" entry (D15 B3/2, docs/CONSOLE-IDP.md) opens `PasskeySettings`
 * as a popover window (`usePopover`, the same mechanism `UserCard`/
 * `AgentDetailCard` use) - it lives in this same signed-in-only block since
 * enrolling or listing a passkey is exactly as session-scoped as the badge
 * next to it.
 */
function SessionBadge() {
  const session = useSession();
  const { open } = usePopover();
  if (!session?.signed_in || !session.role || !session.user) return null;
  return (
    <div
      className="flex items-center gap-1.5"
      title={`Signed in as ${session.user}${session.method ? ` (${session.method} account)` : ""}`}
    >
      <span className="badge" style={cssVar("tone", ROLE_TONE[session.role])}>
        {session.role}
      </span>
      <span className="mono truncate" style={{ fontSize: 11.5, color: "var(--dim)", maxWidth: 140 }}>
        {session.user}
      </span>
      {session.method && (
        <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
          &middot; {session.method}
        </span>
      )}
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "0 8px", fontSize: 10.5 }}
        title="Enrolled passkeys - hardware-confirm kill/budget/approval actions"
        onClick={(e) =>
          open(<PasskeySettings />, { anchor: e.currentTarget.getBoundingClientRect(), width: 320 })
        }
      >
        Passkeys
      </button>
    </div>
  );
}

/**
 * Persistent app chrome: brand mark, view nav (Overview / Money / Policy /
 * Posture / Bus Explorer), the signed-in session badge, and the theme toggle
 * - shown once regardless of which view is active, replacing the Bus
 * Explorer's former standalone header now that it is one of several views
 * instead of the whole app (see `BusStatusBar` for what stayed behind,
 * scoped to the Bus view).
 *
 * `policyAlertCount` (docs/PHASE2.md Wave 3, "Actionable notifications"): a
 * small unread-count badge on the Policy nav item, owned by `AppShell.tsx`.
 * This IS the working half of the notification deep link on this desktop
 * build (see `lib/notifications.ts`'s doc comment for the grounded reason a
 * real OS notification-click callback does not fire here) - clicking Policy
 * while the badge shows a count always scrolls to and highlights the
 * relevant Approvals Inbox row (`AppShell.tsx`'s `onSelectView`).
 *
 * `SessionBadge` (docs/CONSOLE-IDP.md, part 1 - IdP login + roles): the
 * web build's signed-in user, role and sign-in method, next to the theme
 * toggle. Renders nothing in the desktop shell or in any web session without
 * a role, so this header stays the one shared component both shells mount
 * unmodified.
 */
export function AppHeader({
  view,
  onSelectView,
  theme,
  onToggleTheme,
  policyAlertCount,
}: {
  view: ViewId;
  onSelectView: (view: ViewId) => void;
  theme: "dark" | "light";
  onToggleTheme: () => void;
  policyAlertCount: number;
}) {
  return (
    <header
      className="flex items-center gap-3 px-4 shrink-0"
      style={{
        height: 52,
        borderBottom: "1px solid var(--line)",
        background: "color-mix(in srgb, var(--panel) 55%, transparent)",
        backdropFilter: "blur(12px) saturate(1.2)",
        WebkitBackdropFilter: "blur(12px) saturate(1.2)",
      }}
    >
      <BrandMark />
      <div className="flex flex-col leading-none">
        <span style={{ fontFamily: "var(--font-d)", fontSize: 14, fontWeight: 750, color: "var(--fg)" }}>
          Genaryx
        </span>
        <span
          className="mono"
          style={{ fontSize: 10, letterSpacing: "0.14em", textTransform: "uppercase", color: "var(--faint)", marginTop: 3 }}
        >
          Control Room
        </span>
      </div>

      <nav className="flex items-center gap-1 ml-3" aria-label="Views">
        {VIEWS.map((item) => {
          const active = item.id === view;
          const alertCount = item.id === "policy" ? policyAlertCount : 0;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => onSelectView(item.id)}
              aria-current={active ? "page" : undefined}
              className="mono inline-flex items-center gap-1.5"
              style={{
                fontSize: 11.5,
                padding: "6px 12px",
                borderRadius: 8,
                border: `1px solid ${active ? "var(--line-2)" : "transparent"}`,
                background: active ? "var(--panel-3)" : "transparent",
                color: active ? "var(--fg)" : "var(--dim)",
                cursor: "pointer",
              }}
            >
              {item.label}
              {alertCount > 0 && (
                <span
                  aria-label={`${alertCount} approval alert${alertCount === 1 ? "" : "s"} awaiting review`}
                  style={{
                    fontSize: 10,
                    fontWeight: 700,
                    lineHeight: 1,
                    padding: "2.5px 5.5px",
                    borderRadius: 999,
                    background: "var(--sev-medium)",
                    color: "var(--ink)",
                  }}
                >
                  {alertCount}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div className="flex-1" />

      <SessionBadge />

      <button
        type="button"
        className="icon-btn"
        onClick={onToggleTheme}
        aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
        title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
      >
        {theme === "dark" ? <SunIcon /> : <MoonIcon />}
      </button>
    </header>
  );
}
