import { useCallback, useRef, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { usePopover } from "../lib/popover";
import type { ConsoleRole } from "../lib/session";
import { useSession } from "../lib/useSession";
import type { ViewId } from "../lib/views";
import { NAV_SECTIONS } from "../lib/views";
import { PasskeySettings } from "./PasskeySettings";
import { RemoteWgOperatorPopoverCard } from "./RemoteWgOperatorCard";

/** Persisted, drag-resizable rail width (Yurii, 2026-07-24), alongside the
 * existing collapse/expand toggle - collapse is a binary owned by
 * `AppShell.tsx` (it also gates the rest of the layout), width is a plain
 * continuous value nothing outside this component reads, so it lives here
 * instead of being threaded through another prop pair. Same clamp-and-persist
 * shape as `WatchDock.tsx`'s own width, on the opposite edge of the screen. */
const RAIL_WIDTH_KEY = "genaryx.railWidth";
const RAIL_MIN_WIDTH = 140;
const RAIL_MAX_WIDTH = 360;
const RAIL_DEFAULT_WIDTH = 194;

function clampWidth(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function readStoredWidth(key: string, fallback: number, min: number, max: number): number {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? clampWidth(parsed, min, max) : fallback;
  } catch {
    return fallback;
  }
}

function writeStoredWidth(key: string, value: number): void {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    // Storage unavailable - the resize still applies for the rest of this
    // session via React state, it just will not survive a reload, same
    // best-effort tolerance every other localStorage write in this app has.
  }
}

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

/** Points left when the rail is expanded (click to collapse it that way) and
 * right when collapsed (click to expand it back out) - the same directional
 * convention `WatchDock.tsx` uses for its own collapse chevron on the
 * opposite edge of the screen. */
function CollapseIcon({ collapsed }: { collapsed: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" aria-hidden="true">
      <path
        d={collapsed ? "M9 5l7 7-7 7" : "M15 5l-7 7 7 7"}
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
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
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "5px 8px", fontSize: 10.5, justifyContent: "flex-start" }}
        title="Reach this console from your own laptop or phone over WireGuard, instead of SSH"
        onClick={(e) =>
          open(<RemoteWgOperatorPopoverCard />, { anchor: e.currentTarget.getBoundingClientRect(), width: 460 })
        }
      >
        Connect this machine
      </button>
    </div>
  );
}

/** One nav row, in either the full expanded form (icon-less label, full
 * width, trailing alert-count pill) or the collapsed form (a small square
 * button showing just the label's first letter, full label moved to
 * `title`/`aria-label` so the item stays identifiable on hover and to
 * assistive tech, alert count shrunk to a corner dot so it still registers
 * without the room a number needs). Split out of the plain expanded button
 * that used to live inline here purely to keep that branch readable now that
 * there are two of them. */
function NavItemButton({
  item,
  active,
  alertCount,
  collapsed,
  onSelectView,
}: {
  item: { id: ViewId; label: string };
  active: boolean;
  alertCount: number;
  collapsed: boolean;
  onSelectView: (view: ViewId) => void;
}) {
  if (collapsed) {
    return (
      <button
        type="button"
        onClick={() => onSelectView(item.id)}
        aria-current={active ? "page" : undefined}
        aria-label={item.label}
        title={item.label}
        className="mono flex items-center justify-center relative"
        style={{
          fontSize: 11,
          width: 34,
          height: 30,
          borderRadius: 8,
          border: `1px solid ${active ? "var(--line-2)" : "transparent"}`,
          background: active ? "var(--panel-3)" : "transparent",
          color: active ? "var(--fg)" : "var(--dim)",
          cursor: "pointer",
          textTransform: "uppercase",
        }}
      >
        {item.label.charAt(0)}
        {alertCount > 0 && (
          <span
            aria-hidden="true"
            style={{
              position: "absolute",
              top: 2,
              right: 2,
              width: 7,
              height: 7,
              borderRadius: "50%",
              background: "var(--sev-medium)",
            }}
          />
        )}
      </button>
    );
  }
  return (
    <button
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
}

/** The drag handle itself: a thin, invisible-until-hover strip on the rail's
 * inner (right) edge - `WatchDock.tsx` renders the mirror image of this on
 * its own inner (left) edge. Pure pointer-events plumbing (no drag math of
 * its own); `onPointerDown` captures the pointer on the handle element
 * itself so move/up keep firing even once the cursor leaves the thin strip
 * mid-drag, exactly as a native scrollbar/resizer would. */
function RailResizeHandle({
  onPointerDown,
  onPointerMove,
  onPointerUp,
}: {
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerUp: (e: React.PointerEvent<HTMLDivElement>) => void;
}) {
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize navigation rail"
      title="Drag to resize"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      style={{
        position: "absolute",
        top: 0,
        bottom: 0,
        right: 0,
        width: 6,
        cursor: "col-resize",
        zIndex: 1,
        touchAction: "none",
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = "color-mix(in srgb, var(--iris) 35%, transparent)")}
      onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
    />
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
 *
 * `railCollapsed`/`onToggleRail` (Yurii, 2026-07-24: "collapse/expand
 * control"): collapsed state is owned by `AppShell.tsx` (persisted to
 * localStorage there), this component only renders it. Collapsed, the rail
 * narrows to a ~52px strip: brand glyph and the toggle stay, section labels
 * disappear, and each nav item shrinks to a small first-letter square (full
 * label still reachable via its tooltip and `aria-label` - see
 * `NavItemButton` above) so navigation keeps working without the width. The
 * session badge and Passkeys action are dropped when collapsed (there is no
 * legible way to show a role/username/action row in 52px); the theme toggle
 * stays as an icon-only button.
 *
 * `railWidth` (Yurii, 2026-07-24: "resizable in addition to collapsible"):
 * unlike `railCollapsed`, this width is owned and persisted right here
 * (`genaryx.railWidth`, clamped [`RAIL_MIN_WIDTH`, `RAIL_MAX_WIDTH`]) rather
 * than threaded through `AppShell.tsx` - nothing outside this component reads
 * the number, so there is no reason to lift it. Dragging [`RailResizeHandle`]
 * on the rail's inner (right) edge updates it live; only collapsing hides the
 * handle, matching `WatchDock.tsx`'s identical width/handle pair on the
 * opposite edge of the screen.
 */
export function AppHeader({
  view,
  onSelectView,
  theme,
  onToggleTheme,
  policyAlertCount,
  railCollapsed,
  onToggleRail,
}: {
  view: ViewId;
  onSelectView: (view: ViewId) => void;
  theme: "dark" | "light";
  onToggleTheme: () => void;
  policyAlertCount: number;
  railCollapsed: boolean;
  onToggleRail: () => void;
}) {
  const [railWidth, setRailWidth] = useState<number>(() =>
    readStoredWidth(RAIL_WIDTH_KEY, RAIL_DEFAULT_WIDTH, RAIL_MIN_WIDTH, RAIL_MAX_WIDTH),
  );
  // `dragging` (Yurii, 2026-07-24) suppresses the width transition below
  // while a drag is live, so the rail tracks the pointer 1:1 instead of
  // visibly lagging behind it through a 0.16s ease - real state (not a ref)
  // since the render itself reads it. The start point/width live in a plain
  // ref: written on every pointermove (dozens of times per drag) and never
  // read by render, so a ref avoids re-running this whole component's render
  // once per pixel of movement the way state would.
  const [dragging, setDragging] = useState(false);
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  const onHandlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      dragRef.current = { startX: e.clientX, startWidth: railWidth };
      setDragging(true);
    },
    [railWidth],
  );
  const onHandlePointerMove = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    // Dragging right (positive delta) widens the rail - its handle sits on
    // the RIGHT inner edge, so moving toward the main content grows it.
    setRailWidth(clampWidth(drag.startWidth + (e.clientX - drag.startX), RAIL_MIN_WIDTH, RAIL_MAX_WIDTH));
  }, []);
  const onHandlePointerUp = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragRef.current) return;
    dragRef.current = null;
    setDragging(false);
    setRailWidth((w) => {
      writeStoredWidth(RAIL_WIDTH_KEY, w);
      return w;
    });
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // Already released (e.g. the pointer left the window mid-drag) -
      // nothing left to clean up.
    }
  }, []);

  return (
    <nav
      className="flex flex-col shrink-0"
      aria-label="Views"
      style={{
        position: "relative",
        width: railCollapsed ? 52 : railWidth,
        height: "100%",
        borderRight: "1px solid var(--line)",
        background: "color-mix(in srgb, var(--panel) 55%, transparent)",
        backdropFilter: "blur(12px) saturate(1.2)",
        WebkitBackdropFilter: "blur(12px) saturate(1.2)",
        transition: dragging ? undefined : "width 0.16s ease",
        overflow: "hidden",
      }}
    >
      {!railCollapsed && (
        <RailResizeHandle onPointerDown={onHandlePointerDown} onPointerMove={onHandlePointerMove} onPointerUp={onHandlePointerUp} />
      )}
      <div
        className={railCollapsed ? "flex flex-col items-center gap-2 px-2 shrink-0" : "flex items-center gap-2.5 px-4 shrink-0"}
        style={{ height: railCollapsed ? "auto" : 60, paddingTop: railCollapsed ? 12 : undefined, paddingBottom: railCollapsed ? 10 : undefined }}
      >
        <BrandMark />
        {!railCollapsed && (
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
        )}
        {!railCollapsed && <div className="flex-1" />}
        <button
          type="button"
          className="icon-btn"
          style={{ width: 24, height: 24, flexShrink: 0 }}
          onClick={onToggleRail}
          aria-label={railCollapsed ? "Expand navigation" : "Collapse navigation"}
          title={railCollapsed ? "Expand navigation" : "Collapse navigation"}
        >
          <CollapseIcon collapsed={railCollapsed} />
        </button>
      </div>

      <div
        className="flex-1 min-h-0 overflow-y-auto px-2.5 pb-2"
        style={{ display: "flex", flexDirection: "column", gap: railCollapsed ? 10 : 12 }}
      >
        {NAV_SECTIONS.map((section) => (
          <div key={section.label} className={railCollapsed ? "flex flex-col items-center" : "flex flex-col"} style={{ gap: 2 }}>
            {!railCollapsed && (
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
            )}
            {railCollapsed && (
              <div
                aria-hidden="true"
                title={section.label}
                style={{ width: 18, height: 1, background: "var(--line)", margin: "2px 0 4px" }}
              />
            )}
            {section.items.map((item) => (
              <NavItemButton
                key={item.id}
                item={item}
                active={item.id === view}
                alertCount={item.id === "policy" ? policyAlertCount : 0}
                collapsed={railCollapsed}
                onSelectView={onSelectView}
              />
            ))}
          </div>
        ))}
      </div>

      <div
        className={railCollapsed ? "flex flex-col items-center gap-2 px-2 py-3 shrink-0" : "flex flex-col gap-2 px-3 py-3 shrink-0"}
        style={{ borderTop: "1px solid var(--line)" }}
      >
        {!railCollapsed && <SessionBadge />}
        <button
          type="button"
          className="icon-btn"
          style={
            railCollapsed
              ? { width: 28, height: 28, padding: 0 }
              : { width: "auto", padding: "5px 8px", fontSize: 10.5, justifyContent: "flex-start", gap: 8 }
          }
          onClick={onToggleTheme}
          aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
        >
          {theme === "dark" ? <SunIcon /> : <MoonIcon />}
          {!railCollapsed && (
            <span className="mono" style={{ fontSize: 10.5, color: "var(--dim)" }}>
              {theme === "dark" ? "Light" : "Dark"}
            </span>
          )}
        </button>
      </div>
    </nav>
  );
}
