/**
 * A small floating-window manager for the console.
 *
 * Yurii's model (2026-07-22): detail cards are not one-at-a-time popovers, they
 * are independent windows. You can open several at once, from different tabs,
 * and they all stay put and visible together (they live above the whole app, so
 * switching tabs never closes them). Each window can be dragged by its header,
 * resized horizontally and vertically, maximised to fill the screen and
 * restored, brought to the front by clicking it, and closed only by the X in
 * its corner, exactly like an ordinary window. There is no outside-click or
 * Escape dismissal, so a window never vanishes while you are reading it.
 *
 * The manager owns geometry, stacking, and dismissal, and knows nothing about
 * what a card contains. `usePopover().open(node, { anchor })` adds a window;
 * inside a card, `useWindowControls()` gives that card its own close, maximise,
 * and drag handle (wired through `PopoverHeader`).
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { createPortal } from "react-dom";

const GAP = 10;
const MARGIN = 12;
const DEFAULT_W = 380;
const MIN_W = 280;
const MIN_H = 160;

export interface PopoverOptions {
  anchor?: DOMRect;
  width?: number;
  /** Kept for source compatibility; every card is a movable window now, so a
   * modal is just a window opened near screen centre. */
  modal?: boolean;
  key?: string;
}

interface WindowState {
  id: number;
  node: ReactNode;
  anchor?: DOMRect;
  width: number;
  z: number;
}

interface PopoverApi {
  /** Add a window. Returns its id. */
  open: (node: ReactNode, opts?: PopoverOptions) => number;
  /** Close a specific window, or all of them when no id is given. */
  close: (id?: number) => void;
}

const PopoverContext = createContext<PopoverApi | null>(null);

export function usePopover(): PopoverApi {
  const ctx = useContext(PopoverContext);
  if (!ctx) throw new Error("usePopover must be used inside <PopoverProvider>");
  return ctx;
}

/** Per-window controls handed to the card inside: close it, toggle its
 * maximised state, and the pointer handler that drags it (attached to the
 * header). Null when a card is rendered outside a window (it just shows a
 * static header). */
export interface WindowControls {
  close: () => void;
  dragHandleProps: { onPointerDown: (e: ReactPointerEvent) => void };
  toggleMaximize?: () => void;
  maximized?: boolean;
}
const WindowContext = createContext<WindowControls | null>(null);
export function useWindowControls(): WindowControls | null {
  return useContext(WindowContext);
}
export function WindowControlsProvider({ value, children }: { value: WindowControls; children: ReactNode }) {
  return <WindowContext.Provider value={value}>{children}</WindowContext.Provider>;
}

interface Pos {
  left: number;
  top: number;
}

/** Keep a box fully inside the viewport with a margin. */
function clampBox(left: number, top: number, cw: number, ch: number): Pos {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const maxLeft = Math.max(MARGIN, vw - cw - MARGIN);
  const maxTop = Math.max(MARGIN, vh - ch - MARGIN);
  return {
    left: Math.min(Math.max(left, MARGIN), maxLeft),
    top: Math.min(Math.max(top, MARGIN), maxTop),
  };
}

/**
 * Drag + viewport-clamp for a floating card that is NOT a managed window (the
 * Bus Explorer's pinned-event card). Kept so that card drags and re-clamps
 * exactly like the managed windows do.
 */
export function useDraggableCard(
  cardRef: React.RefObject<HTMLDivElement | null>,
  place: (cw: number, ch: number) => Pos | null,
) {
  const [pos, setPos] = useState<Pos | null>(null);
  const posRef = useRef<Pos | null>(null);
  const draggedRef = useRef(false);
  const dragRef = useRef<{ sx: number; sy: number; ol: number; ot: number } | null>(null);
  const placeRef = useRef(place);
  placeRef.current = place;

  const setBoth = useCallback((p: Pos) => {
    posRef.current = p;
    setPos(p);
  }, []);

  const reflow = useCallback(() => {
    const card = cardRef.current;
    if (!card) return;
    const cw = card.offsetWidth;
    const ch = card.offsetHeight;
    if (draggedRef.current && posRef.current) {
      setBoth(clampBox(posRef.current.left, posRef.current.top, cw, ch));
    } else {
      const p = placeRef.current(cw, ch);
      if (p) setBoth(clampBox(p.left, p.top, cw, ch));
    }
  }, [cardRef, setBoth]);

  useLayoutEffect(() => {
    reflow();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const card = cardRef.current;
    if (!card) return;
    const ro = new ResizeObserver(() => reflow());
    ro.observe(card);
    window.addEventListener("resize", reflow);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", reflow);
    };
  }, [cardRef, reflow]);

  const onPointerDown = useCallback((e: ReactPointerEvent) => {
    const start = posRef.current;
    if (!start) return;
    draggedRef.current = true;
    dragRef.current = { sx: e.clientX, sy: e.clientY, ol: start.left, ot: start.top };
    const onMove = (ev: PointerEvent) => {
      const d = dragRef.current;
      const card = cardRef.current;
      if (!d || !card) return;
      setBoth(clampBox(d.ol + (ev.clientX - d.sx), d.ot + (ev.clientY - d.sy), card.offsetWidth, card.offsetHeight));
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    e.preventDefault();
  }, [cardRef, setBoth]);

  return { pos, dragHandleProps: { onPointerDown } };
}

export function PopoverProvider({ children }: { children: ReactNode }) {
  const [windows, setWindows] = useState<WindowState[]>([]);
  const seq = useRef(0);
  const topZ = useRef(1000);

  const open = useCallback((node: ReactNode, opts: PopoverOptions = {}): number => {
    seq.current += 1;
    topZ.current += 1;
    const id = seq.current;
    setWindows((ws) => [
      ...ws,
      { id, node, anchor: opts.anchor, width: opts.width ?? DEFAULT_W, z: topZ.current },
    ]);
    return id;
  }, []);

  const close = useCallback((id?: number) => {
    setWindows((ws) => (id === undefined ? [] : ws.filter((w) => w.id !== id)));
  }, []);

  const focus = useCallback((id: number) => {
    topZ.current += 1;
    const z = topZ.current;
    setWindows((ws) => ws.map((w) => (w.id === id ? { ...w, z } : w)));
  }, []);

  const api = useMemo<PopoverApi>(() => ({ open, close }), [open, close]);

  return (
    <PopoverContext.Provider value={api}>
      {children}
      {windows.map((w) => (
        <FloatingWindow key={w.id} win={w} onClose={() => close(w.id)} onFocus={() => focus(w.id)} />
      ))}
    </PopoverContext.Provider>
  );
}

function FloatingWindow({
  win,
  onClose,
  onFocus,
}: {
  win: WindowState;
  onClose: () => void;
  onFocus: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<Pos | null>(null);
  const [size, setSize] = useState<{ w: number; h: number | null }>({ w: win.width, h: null });
  const [maximized, setMaximized] = useState(false);
  const movedRef = useRef(false);
  const posRef = useRef<Pos | null>(null);
  const setBoth = (p: Pos) => {
    posRef.current = p;
    setPos(p);
  };

  // First placement: beside the anchor, clamped. Re-clamp on content growth so
  // a window whose card loaded more data never spills off the bottom.
  const reflow = useCallback(() => {
    const el = ref.current;
    if (!el || maximized) return;
    const cw = el.offsetWidth;
    const ch = el.offsetHeight;
    if (movedRef.current && posRef.current) {
      setBoth(clampBox(posRef.current.left, posRef.current.top, cw, ch));
    } else if (win.anchor) {
      let left = win.anchor.right + GAP;
      if (left + cw > window.innerWidth - MARGIN) left = win.anchor.left - GAP - cw;
      setBoth(clampBox(left, win.anchor.top, cw, ch));
    } else {
      setBoth(clampBox((window.innerWidth - cw) / 2, (window.innerHeight - ch) / 2, cw, ch));
    }
  }, [maximized, win.anchor]);

  useLayoutEffect(() => {
    reflow();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => reflow());
    ro.observe(el);
    window.addEventListener("resize", reflow);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", reflow);
    };
  }, [reflow]);

  const onDragPointerDown = useCallback((e: ReactPointerEvent) => {
    if (maximized) return;
    const start = posRef.current;
    if (!start) return;
    movedRef.current = true;
    onFocus();
    const s = { sx: e.clientX, sy: e.clientY, ol: start.left, ot: start.top };
    const onMove = (ev: PointerEvent) => {
      const el = ref.current;
      if (!el) return;
      setBoth(clampBox(s.ol + (ev.clientX - s.sx), s.ot + (ev.clientY - s.sy), el.offsetWidth, el.offsetHeight));
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    e.preventDefault();
  }, [maximized, onFocus]);

  const startResize = (dir: "e" | "s" | "se") => (e: ReactPointerEvent) => {
    if (maximized) return;
    e.preventDefault();
    e.stopPropagation();
    onFocus();
    const el = ref.current;
    const startW = size.w;
    const startH = size.h ?? (el ? el.offsetHeight : 320);
    const sx = e.clientX;
    const sy = e.clientY;
    const onMove = (ev: PointerEvent) => {
      const next: { w: number; h: number | null } = { w: startW, h: startH };
      if (dir === "e" || dir === "se") next.w = Math.max(MIN_W, startW + (ev.clientX - sx));
      if (dir === "s" || dir === "se") next.h = Math.max(MIN_H, startH + (ev.clientY - sy));
      setSize(next);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const controls: WindowControls = {
    close: onClose,
    dragHandleProps: { onPointerDown: onDragPointerDown },
    toggleMaximize: () => setMaximized((m) => !m),
    maximized,
  };

  const geom: React.CSSProperties = maximized
    ? { left: MARGIN, top: MARGIN, width: `calc(100vw - ${MARGIN * 2}px)`, height: `calc(100vh - ${MARGIN * 2}px)` }
    : {
        left: pos ? pos.left : -9999,
        top: pos ? pos.top : 0,
        width: size.w,
        maxWidth: "96vw",
        height: size.h ?? undefined,
      };

  const contentScrolls = maximized || size.h !== null;

  return createPortal(
    <div
      ref={ref}
      role="dialog"
      className="gx-popover"
      onPointerDown={onFocus}
      style={{
        position: "fixed",
        zIndex: win.z,
        background: "var(--panel)",
        border: "1px solid var(--line-2)",
        borderRadius: 12,
        boxShadow: "0 18px 48px rgba(28, 20, 8, 0.26), 0 4px 12px rgba(28, 20, 8, 0.16)",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
        ...geom,
      }}
    >
      <WindowControlsProvider value={controls}>
        <div
          className="thin-scroll"
          style={{
            flex: 1,
            minHeight: 0,
            overflowY: "auto",
            maxHeight: contentScrolls ? undefined : "82vh",
          }}
        >
          {win.node}
        </div>
      </WindowControlsProvider>

      {!maximized && (
        <>
          <div onPointerDown={startResize("e")} style={{ position: "absolute", top: 0, right: 0, width: 8, height: "100%", cursor: "ew-resize" }} />
          <div onPointerDown={startResize("s")} style={{ position: "absolute", left: 0, bottom: 0, width: "100%", height: 8, cursor: "ns-resize" }} />
          <div onPointerDown={startResize("se")} style={{ position: "absolute", right: 0, bottom: 0, width: 16, height: 16, cursor: "nwse-resize" }} />
        </>
      )}
    </div>,
    document.body,
  );
}

/** The card header, which is also the window's title bar: a grip, kicker and
 * title, a maximise/restore toggle, and a close X. Grabbing anywhere on it
 * (except the buttons) drags the window; the controls come from the window
 * context, so a card never has to wire its own. `onClose` is only a fallback
 * for a card shown outside a window. */
export function PopoverHeader({
  kicker,
  title,
  onClose,
}: {
  kicker?: string;
  title: string;
  onClose?: () => void;
}) {
  const win = useWindowControls();
  const close = win?.close ?? onClose ?? (() => {});
  const drag = win?.dragHandleProps;
  return (
    <div
      className="flex items-start gap-2"
      style={{
        padding: "12px 12px 8px 14px",
        cursor: drag ? "grab" : "default",
        userSelect: "none",
        position: "sticky",
        top: 0,
        background: "var(--panel)",
        zIndex: 1,
        borderBottom: "1px solid transparent",
      }}
      onPointerDown={drag?.onPointerDown}
    >
      {drag && (
        <span aria-hidden="true" className="mono" style={{ color: "var(--faint)", fontSize: 13, lineHeight: 1.3, marginTop: 1 }}>
          &#8942;&#8942;
        </span>
      )}
      <div className="flex flex-col gap-0.5 min-w-0">
        {kicker && (
          <span className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            {kicker}
          </span>
        )}
        <span className="text-[14px]" style={{ color: "var(--fg)", fontWeight: 600 }}>
          {title}
        </span>
      </div>
      <div className="flex-1" />
      {win?.toggleMaximize && (
        <button
          type="button"
          className="icon-btn"
          aria-label={win.maximized ? "Restore" : "Maximize"}
          title={win.maximized ? "Restore" : "Maximize"}
          onClick={win.toggleMaximize}
          onPointerDown={(e) => e.stopPropagation()}
          style={{ fontSize: 12 }}
        >
          {win.maximized ? "❐" : "⛶"}
        </button>
      )}
      <button
        type="button"
        className="icon-btn"
        aria-label="Close"
        onClick={close}
        onPointerDown={(e) => e.stopPropagation()}
        style={{ fontSize: 15 }}
      >
        &times;
      </button>
    </div>
  );
}
