import { useCallback, useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import type { UiEvent } from "../types";
import { WindowControlsProvider, useDraggableCard } from "../lib/popover";
import { EventDetailCard } from "./EventDetailCard";
import { SeverityBadge } from "./SeverityBadge";
import { SourceChip } from "./SourceChip";

/**
 * Pin one row out of a moving feed.
 *
 * Yurii's interaction (2026-07-22): tapping a row in a live stream lifts THAT
 * row above the flow and freezes it exactly where it was tapped, with a shadow,
 * while new rows keep arriving and scrolling underneath it, and a detail window
 * opens beside it above everything. This renders that: a transparent full-screen
 * layer (so the feed stays visible and keeps moving under it, and a click on the
 * empty area releases the pin), the lifted row clone fixed at its original
 * on-screen rect, and the event's detail card placed next to it.
 *
 * It is its own overlay rather than the generic popover because the lifted row
 * and the card share one dismissal: Escape, a click on the backdrop, or the
 * card's close button all release the pin together.
 */

const GAP = 10;
const MARGIN = 12;

export function PinnedEventOverlay({
  event,
  rect,
  onClose,
  onOpenAgent,
}: {
  event: UiEvent;
  rect: DOMRect;
  onClose: () => void;
  onOpenAgent?: (agentId: string, rect: DOMRect) => void;
}) {
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Place the card beside the pinned row (right if there is room, else left),
  // then let it be dragged and kept on screen like every other popover card.
  const place = useCallback(
    (cw: number) => {
      let left = rect.right + GAP;
      if (left + cw > window.innerWidth - MARGIN) left = rect.left - GAP - cw;
      return { left, top: rect.top };
    },
    [rect],
  );
  const { pos, dragHandleProps } = useDraggableCard(cardRef, place);

  return createPortal(
    <div
      style={{ position: "fixed", inset: 0, zIndex: 1000, background: "transparent" }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      {/* The lifted row, frozen where it was tapped, riding above the flow. */}
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          position: "fixed",
          left: rect.left,
          top: rect.top,
          width: rect.width,
          minHeight: rect.height,
          zIndex: 1001,
          background: "var(--panel-2)",
          borderRadius: 8,
          border: "1px solid var(--iris)",
          boxShadow: "0 14px 34px color-mix(in srgb, var(--ink) 60%, transparent)",
        }}
      >
        <div
          className="grid items-center gap-3 px-4 py-2"
          style={{ gridTemplateColumns: "84px 108px 190px 1fr 108px" }}
        >
          <SeverityBadge severity={event.severity} />
          <SourceChip source={event.source} />
          <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={event.type}>
            {event.type}
          </span>
          <span className="mono tabular truncate text-[12px]" style={{ color: "var(--dim)" }} title={event.agent_id}>
            {event.agent_id}
          </span>
          <span className="mono tabular text-[11.5px] text-right" style={{ color: "var(--faint)" }}>
            pinned
          </span>
        </div>
      </div>

      {/* The detail, beside the pinned row, above everything. */}
      <div
        ref={cardRef}
        className="gx-popover thin-scroll"
        onClick={(e) => e.stopPropagation()}
        style={{
          position: "fixed",
          width: 420,
          maxWidth: "94vw",
          maxHeight: "80vh",
          overflowY: "auto",
          zIndex: 1002,
          background: "var(--bg)",
          border: "1px solid var(--line-2)",
          borderRadius: 12,
          boxShadow: "0 24px 60px color-mix(in srgb, var(--ink) 55%, transparent)",
          ...(pos ? { left: pos.left, top: pos.top } : { left: -9999, top: 0 }),
        }}
      >
        <WindowControlsProvider value={{ close: onClose, dragHandleProps }}>
          <EventDetailCard event={event} onClose={onClose} onOpenAgent={onOpenAgent} />
        </WindowControlsProvider>
      </div>
    </div>,
    document.body,
  );
}
