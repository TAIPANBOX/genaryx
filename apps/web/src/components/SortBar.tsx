/**
 * A compact sort control for any list or the live feed. Each key is a toggle:
 * clicking the active key flips direction, clicking another switches to it. The
 * caller owns the state and does the actual sorting, so one small control drives
 * every list (runs, bus events, agents in a unit) with the dimensions that
 * make sense for what it shows.
 */

export interface SortOption {
  key: string;
  label: string;
}

export type SortDir = "asc" | "desc";

export function SortBar({
  options,
  active,
  dir,
  onChange,
}: {
  options: SortOption[];
  active: string;
  dir: SortDir;
  onChange: (key: string, dir: SortDir) => void;
}) {
  return (
    <div className="flex items-center gap-1.5 flex-wrap">
      <span className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        sort
      </span>
      {options.map((o) => {
        const on = o.key === active;
        return (
          <button
            key={o.key}
            type="button"
            onClick={() => onChange(o.key, on ? (dir === "desc" ? "asc" : "desc") : "desc")}
            className="text-[11px]"
            style={{
              padding: "3px 8px",
              borderRadius: 6,
              cursor: "pointer",
              border: `1px solid ${on ? "var(--iris)" : "var(--line-2)"}`,
              background: on ? "color-mix(in srgb, var(--iris) 14%, transparent)" : "transparent",
              color: on ? "var(--fg)" : "var(--dim)",
              whiteSpace: "nowrap",
            }}
          >
            {o.label}
            {on ? (dir === "desc" ? " ▾" : " ▴") : ""}
          </button>
        );
      })}
    </div>
  );
}
