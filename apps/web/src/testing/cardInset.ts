/**
 * Where a piece of rendered text sits inside a dashboard card, read from the
 * markup alone.
 *
 * The unit tests run in node, which has no layout engine, so they read the
 * `renderToStaticMarkup` output: the inline styles and Tailwind inset classes a
 * component declares between the card (`.d-card`) and the text. The result is
 * the inset the component chose, in px. It cannot see the stylesheet (a chip's
 * own pill padding, a card's 1px border), so the browser check in each PR is
 * what measures pixels; this keeps the inset from regressing between them.
 */

const VOID = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"]);

// Group 1-3: an element tag; group 4: a run of text. Comments (React puts one
// between adjacent interpolations) are matched first and skipped.
const TOKEN = /<!--[\s\S]*?-->|<(\/?)([a-zA-Z][\w-]*)([^>]*)>|([^<]+)/g;

interface Frame {
  tag: string;
  classes: string[];
  style: string;
}

function attr(attrs: string, name: string): string {
  const m = new RegExp(`\\s${name}="([^"]*)"`).exec(attrs);
  return m ? m[1] : "";
}

function decode(s: string): string {
  return s
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/&#x2F;/g, "/")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

const px = (v: string): number => {
  const n = parseFloat(v);
  return Number.isFinite(n) ? n : 0;
};

/** Left inset declared by inline `padding` / `margin` (shorthand or longhand). */
function styleLeft(style: string): number {
  let left = 0;
  for (const decl of style.split(";")) {
    const i = decl.indexOf(":");
    if (i < 0) continue;
    const prop = decl.slice(0, i).trim();
    const parts = decl.slice(i + 1).trim().split(/\s+/);
    if (prop === "padding" || prop === "margin") {
      // 1 value: all sides; 2: vertical horizontal; 3: top horizontal bottom; 4: top right bottom left.
      left += px(parts.length === 4 ? parts[3] : parts.length >= 2 ? parts[1] : parts[0]);
    } else if (prop === "padding-left" || prop === "margin-left") {
      left += px(parts[0]);
    }
  }
  return left;
}

/** Left inset declared by Tailwind's spacing classes (`p-5`, `px-4`, `ml-2`, ...). */
function classLeft(classes: string[]): number {
  let left = 0;
  for (const c of classes) {
    const m = /^(p|px|pl|m|mx|ml)-(\d+(?:\.\d+)?)$/.exec(c);
    if (m) left += Number(m[2]) * 4;
  }
  return left;
}

function frameLeft(f: Frame): number {
  return classLeft(f.classes) + styleLeft(f.style);
}

/** The element stack, outermost first, at the start of the first text run that contains `text`. */
function stackAtText(html: string, text: string): Frame[] | null {
  const stack: Frame[] = [];
  const runs: { text: string; stack: Frame[] }[] = [];
  let buf = "";
  const flush = () => {
    if (buf !== "") runs.push({ text: decode(buf), stack: [...stack] });
    buf = "";
  };
  for (const m of html.matchAll(TOKEN)) {
    const [whole, close, tag, attrs, textRun] = m;
    if (whole.startsWith("<!--")) continue;
    if (textRun !== undefined) {
      buf += textRun;
      continue;
    }
    flush();
    if (close) {
      const i = stack.map((f) => f.tag).lastIndexOf(tag);
      if (i >= 0) stack.length = i;
      continue;
    }
    if (VOID.has(tag) || attrs.endsWith("/")) continue;
    stack.push({ tag, classes: attr(attrs, "class").split(/\s+/).filter(Boolean), style: attr(attrs, "style") });
  }
  flush();
  const hit = runs.find((r) => r.text.includes(text));
  return hit ? hit.stack : null;
}

/**
 * The left inset, in px, of `text` from the edge of the outermost `.d-card`
 * that holds it: the summed insets of every element between that card and the
 * text, the text's own element included.
 *
 * With `box`, the measure stops at the innermost element of that class holding
 * the text, so a bordered tile is measured by where it sits, not by its own
 * inner padding.
 */
export function insetOfText(html: string, text: string, box?: string): number {
  const stack = stackAtText(html, text);
  if (stack === null) throw new Error(`text not in the markup: ${JSON.stringify(text)}`);
  const cardIdx = stack.findIndex((f) => f.classes.includes("d-card"));
  if (cardIdx < 0) throw new Error(`no .d-card holds ${JSON.stringify(text)}`);
  let last = stack.length - 1;
  if (box !== undefined) {
    let boxIdx = -1;
    for (let i = cardIdx + 1; i < stack.length; i++) if (stack[i].classes.includes(box)) boxIdx = i;
    if (boxIdx < 0) throw new Error(`no .${box} holds ${JSON.stringify(text)}`);
    last = boxIdx - 1;
  }
  let inset = 0;
  for (let i = cardIdx + 1; i <= last; i++) inset += frameLeft(stack[i]);
  return inset;
}
