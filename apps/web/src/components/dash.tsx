/** Genaryx dashboard kit - the shared primitives every panel composes into a
 * modern, readable, interactive dashboard (hero + KPI tiles + bars + feeds),
 * styled by the `.d-*` classes in index.css and the hand-rolled `Sparkline`
 * and `FuseBar`. No chart dependency; theme-aware via CSS variables. */
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { Sparkline } from "./Sparkline";
import { FuseBar } from "./FuseBar";

export { Sparkline, FuseBar };

export type FuseTone = "mint" | "amber" | "ember" | "iris";

/** A titled gradient card with a section header (title + optional right note). */
export function Section({
  title,
  right,
  children,
}: {
  title: string;
  right?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="d-card">
      <div className="d-sechead">
        <span className="t">{title}</span>
        {right != null && <span className="r">{right}</span>}
      </div>
      {children}
    </div>
  );
}

/** The 2-up (or n-up) KPI tile grid. */
export function KpiGrid({ children, cols = 2 }: { children: ReactNode; cols?: number }) {
  return (
    <div
      className="d-tiles"
      style={cols !== 2 ? { gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` } : undefined}
    >
      {children}
    </div>
  );
}

/** One KPI tile: uppercase label, big tabular number, optional sub-line. When
 * `onClick` is set the whole tile is a drill-down target (it receives the
 * tile's on-screen rect so a popover can open anchored beside it) and reads as
 * clickable to both mouse and keyboard. */
export function KpiTile({
  label,
  value,
  sub,
  tone,
  onClick,
}: {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  tone?: string;
  onClick?: (rect: DOMRect) => void;
}) {
  return (
    <div
      className={"d-card d-tile" + (onClick ? " clk" : "")}
      style={onClick ? { cursor: "pointer" } : undefined}
      onClick={onClick ? (e) => onClick(e.currentTarget.getBoundingClientRect()) : undefined}
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={onClick ? rowKeyDown(onClick) : undefined}
    >
      <span className="k">{label}</span>
      <span className="n" style={tone ? { color: tone } : undefined}>
        {value}
      </span>
      {sub != null && <span className="s">{sub}</span>}
    </div>
  );
}

/** The hero card: a caption, a big headline number, an optional right-aligned
 * secondary figure, a spend/activity sparkline, an optional fuse bar, and an
 * optional two-part note beneath. */
export function Hero({
  cap,
  value,
  sub,
  series,
  seriesStroke,
  seriesFill,
  fuseFraction,
  fuseTone,
  noteLeft,
  noteRight,
}: {
  cap: ReactNode;
  value: ReactNode;
  sub?: ReactNode;
  series?: number[];
  seriesStroke?: string;
  seriesFill?: string;
  fuseFraction?: number;
  fuseTone?: FuseTone;
  noteLeft?: ReactNode;
  noteRight?: ReactNode;
}) {
  return (
    <div className="d-card d-hero">
      <div className="d-cap">{cap}</div>
      <div className="d-heroline">
        <span className="d-heronum">{value}</span>
        {sub != null && <span className="d-herosub">{sub}</span>}
      </div>
      {series && <Sparkline values={series} stroke={seriesStroke} fill={seriesFill} />}
      {fuseFraction !== undefined && <FuseBar fraction={fuseFraction} tone={fuseTone} />}
      {(noteLeft != null || noteRight != null) && (
        <div className="d-heronote">
          <span>{noteLeft}</span>
          <span>{noteRight}</span>
        </div>
      )}
    </div>
  );
}

export interface BarItem {
  key: string;
  label: ReactNode;
  sub?: ReactNode;
  fraction: number;
  tone?: FuseTone;
  value: ReactNode;
  /** An optional small state badge shown beside the label - today a
   * STOPPED/FROZEN/KILLED lifecycle badge on a blocked agent's spend bar, so
   * the Overview "spend by agent" reflects the same state as everywhere else.
   * Omitted for a live row. */
  badge?: ReactNode;
  /** Drill-down: opens the object's detail in a popover anchored to this row
   * (never switches tab). Receives the row's on-screen rect for placement. */
  onClick?: (rect: DOMRect) => void;
}

/** Keyboard-activates a clickable dashboard row (Enter/Space), so drill-downs
 * are reachable without a mouse. Passes the row element's rect so the popover
 * opens beside the row for keyboard users too, not in a corner. */
function rowKeyDown(onClick: (rect: DOMRect) => void) {
  return (e: ReactKeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onClick((e.currentTarget as HTMLElement).getBoundingClientRect());
    }
  };
}

/** Ranked horizontal fuse-bars (spend by agent, findings by kind, ...). */
export function Bars({ items, empty = "no data" }: { items: BarItem[]; empty?: string }) {
  if (items.length === 0) {
    return (
      <div className="mono" style={{ fontSize: 12, color: "var(--faint)", padding: "18px 20px" }}>
        {empty}
      </div>
    );
  }
  return (
    <div className="d-bars">
      {items.map((it) => (
        <div
          className={"d-bar" + (it.onClick ? " clk" : "")}
          key={it.key}
          onClick={it.onClick ? (e) => it.onClick!(e.currentTarget.getBoundingClientRect()) : undefined}
          role={it.onClick ? "button" : undefined}
          tabIndex={it.onClick ? 0 : undefined}
          onKeyDown={it.onClick ? rowKeyDown(it.onClick) : undefined}
        >
          <div className="lbl">
            <div className="nm flex items-center gap-1.5">
              <span className="truncate">{it.label}</span>
              {it.badge}
            </div>
            {it.sub != null && <div className="tm">{it.sub}</div>}
          </div>
          <FuseBar fraction={it.fraction} tone={it.tone ?? "amber"} />
          <span className="amt">{it.value}</span>
        </div>
      ))}
    </div>
  );
}

export interface FeedItem {
  key: string;
  color: string;
  title: ReactNode;
  sub?: ReactNode;
  value?: ReactNode;
  valueColor?: string;
  action?: ReactNode;
  /** Drill-down: opens the object's detail in a popover anchored to this row
   * (never switches tab). Receives the row's on-screen rect for placement. */
  onClick?: (rect: DOMRect) => void;
}

/** A vertical feed of dot + title/sub + right-aligned value/action rows
 * (incidents, alerts, decisions, approvals). Rows with `onClick` drill into
 * the object; an `action` control inside stops the row click from firing. */
export function Feed({ items, empty = "nothing here" }: { items: FeedItem[]; empty?: string }) {
  if (items.length === 0) {
    return (
      <div
        className="mono"
        style={{ fontSize: 12, color: "var(--faint)", padding: "28px 20px", textAlign: "center" }}
      >
        {empty}
      </div>
    );
  }
  return (
    <>
      {items.map((it) => (
        <div
          className={"d-arow" + (it.onClick ? " clk" : "")}
          key={it.key}
          onClick={it.onClick ? (e) => it.onClick!(e.currentTarget.getBoundingClientRect()) : undefined}
          role={it.onClick ? "button" : undefined}
          tabIndex={it.onClick ? 0 : undefined}
          onKeyDown={it.onClick ? rowKeyDown(it.onClick) : undefined}
        >
          <span className="dot" style={{ color: it.color }} />
          <div className="tx">
            <div className="m">{it.title}</div>
            {it.sub != null && <div className="s">{it.sub}</div>}
          </div>
          {it.value != null && (
            <span className="oc" style={it.valueColor ? { color: it.valueColor } : undefined}>
              {it.value}
            </span>
          )}
          {it.action != null && (
            <span onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
              {it.action}
            </span>
          )}
        </div>
      ))}
    </>
  );
}

export interface CompItem {
  key: string;
  label: string;
  value: number;
  total: number;
  tone: FuseTone;
  valueText: string;
}

/** Stacked composition rows: label, amount, and share-of-total as a fuse bar. */
export function Composition({ items }: { items: CompItem[] }) {
  return (
    <div className="d-comp">
      {items.map((it) => {
        const frac = it.total > 0 ? it.value / it.total : 0;
        return (
          <div className="row" key={it.key}>
            <div className="top">
              <span className="lbl">{it.label}</span>
              <span className="val">
                {it.valueText} · {Math.round(frac * 100)}%
              </span>
            </div>
            <FuseBar fraction={frac} tone={it.tone} />
          </div>
        );
      })}
    </div>
  );
}

/** Standard two-column dashboard body: a wide primary column + a fixed rail. */
export function DashMain({ primary, rail }: { primary: ReactNode; rail: ReactNode }) {
  return (
    <div className="d-main">
      <div className="flex flex-col gap-4">{primary}</div>
      <div className="d-rail">{rail}</div>
    </div>
  );
}

/** The hero band: a wide hero card beside a KPI tile grid. */
export function HeroBand({ hero, tiles }: { hero: ReactNode; tiles: ReactNode }) {
  return (
    <div className="d-band">
      {hero}
      <div className="d-tiles">{tiles}</div>
    </div>
  );
}
