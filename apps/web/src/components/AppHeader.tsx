import { cssVar } from "../lib/cssVars";
import { usePopover } from "../lib/popover";
import type { ConsoleRole } from "../lib/session";
import { useSession } from "../lib/useSession";
import type { ViewId } from "../lib/views";
import { NAV_SECTIONS } from "../lib/views";
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
      className="flex flex-col gap-1.5"
      title={`Signed in as ${session.user}${session.method ? ` (${session.method} account)` : ""}`}
    >
      <div className="flex items-center gap-1.5">
        <span className="badge" style={cssVar("tone", ROLE_TONE[session.role])}>
          {session.role}
        </span>
        <span className="mono truncate" style={{ fontSize: 11.5, color: "var(--dim)", maxWidth: 118 }}>
          {session.user}
        </span>
        {session.method && (
          <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
            &middot; {session.method}
          </span>
        )}
      </div>
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "5px 8px", fontSize: 10.5, justifyContent: "flex-start" }}
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
 * Persistent app chrome, a LEFT RAIL (Yurii, 2026-07-24: eighteen views no
 * longer fit across the top): brand mark and title at the top, the view nav
 * grouped by [`NAV_SECTIONS`] (Operate / Investigate / Assure / Set up, most
 * used first and rare setup last), then the signed-in session badge, the
 * Passkeys action and the theme toggle pinned to the foot. Shown once
 * regardless of which view is active.
 *
 * `policyAlertCount` (docs/PHASE2.md Wave 3, "Actionable notifications"): a
 * small unread-count badge on the Policy nav item, owned by `AppShell.tsx`.
 * Clicking Policy while the badge shows a count always scrolls to and
 * highlights the relevant Approvals Inbox row (`AppShell.tsx`'s
 * `onSelectView`).
 *
 * `SessionBadge` (docs/CONSOLE-IDP.md, part 1 - IdP login + roles): the
 * signed-in user, role and sign-in method. Renders nothing in any session
 * without a role, so this rail stays honest when there is no console session.
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
    <nav
      className="flex flex-col shrink-0"
      aria-label="Views"
      style={{
        width: 194,
        height: "100%",
        borderRight: "1px solid var(--line)",
        background: "color-mix(in srgb, var(--panel) 55%, transparent)",
        backdropFilter: "blur(12px) saturate(1.2)",
        WebkitBackdropFilter: "blur(12px) saturate(1.2)",
      }}
    >
      <div className="flex items-center gap-2.5 px-4 shrink-0" style={{ height: 60 }}>
        <BrandMark />
        <div className="flex flex-col leading-none">
          <span style={{ fontFamily: "var(--font-d)", fontSize: 14, fontWeight: 750, color: "var(--fg)" }}>
            Genaryx
          </span>
          <span
            className="mono"
            style={{ fontSize: 9.5, letterSpacing: "0.14em", textTransform: "uppercase", color: "var(--faint)", marginTop: 3 }}
          >
            Control Room
          </span>
        </div>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-2.5 pb-2" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        {NAV_SECTIONS.map((section) => (
          <div key={section.label} className="flex flex-col" style={{ gap: 2 }}>
            <span
              className="mono px-2"
              style={{
                fontSize: 9,
                letterSpacing: "0.14em",
                textTransform: "uppercase",
                color: "var(--faint)",
                marginBottom: 3,
              }}
            >
              {section.label}
            </span>
            {section.items.map((item) => {
              const active = item.id === view;
              const alertCount = item.id === "policy" ? policyAlertCount : 0;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => onSelectView(item.id)}
                  aria-current={active ? "page" : undefined}
                  className="mono flex items-center"
                  style={{
                    fontSize: 12,
                    padding: "6.5px 10px",
                    borderRadius: 8,
                    border: `1px solid ${active ? "var(--line-2)" : "transparent"}`,
                    background: active ? "var(--panel-3)" : "transparent",
                    color: active ? "var(--fg)" : "var(--dim)",
                    cursor: "pointer",
                    textAlign: "left",
                    width: "100%",
                  }}
                >
                  <span className="flex-1">{item.label}</span>
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
          </div>
        ))}
      </div>

      <div
        className="flex flex-col gap-2 px-3 py-3 shrink-0"
        style={{ borderTop: "1px solid var(--line)" }}
      >
        <SessionBadge />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "5px 8px", fontSize: 10.5, justifyContent: "flex-start", gap: 8 }}
          onClick={onToggleTheme}
          aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
        >
          {theme === "dark" ? <SunIcon /> : <MoonIcon />}
          <span className="mono" style={{ fontSize: 10.5, color: "var(--dim)" }}>
            {theme === "dark" ? "Light" : "Dark"}
          </span>
        </button>
      </div>
    </nav>
  );
}
