import type { CSSProperties } from "react";

/**
 * `index.css` reads CSS custom properties (`--tone`, `--dot`, ...) that
 * `CSSProperties` does not model. Rather than casting `as CSSProperties` at
 * every call site, components go through this one narrow, named helper.
 */
export type VarStyle = CSSProperties & Record<`--${string}`, string | undefined>;

export function cssVar(name: string, value: string): VarStyle {
  return { [`--${name}`]: value } as VarStyle;
}
