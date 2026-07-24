import { useMemo } from "react";

interface Token {
  text: string;
  cls: "jk" | "js" | "jn" | "jb" | "jz" | "jp";
}

// Matches, in order of preference: a quoted string (optionally followed by
// a colon, i.e. an object key), then true/false/null, then a number.
// Everything between matches (braces, brackets, commas, whitespace) is
// plain punctuation.
const TOKEN_RE =
  /"(?:\\u[0-9a-fA-F]{4}|\\[^u]|[^\\"])*"(\s*:)?|\btrue\b|\bfalse\b|\bnull\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;

/** Tokenize pretty-printed JSON for the tiny hand-rolled highlighter below.
 * No dependency: `JSON.stringify(value, null, 2)` in, a flat token list
 * out; every token renders as a plain React text node (never raw HTML), so
 * there is no injection surface. */
function tokenize(json: string): Token[] {
  const tokens: Token[] = [];
  let last = 0;
  for (const m of json.matchAll(TOKEN_RE)) {
    const start = m.index ?? 0;
    if (start > last) {
      tokens.push({ text: json.slice(last, start), cls: "jp" });
    }
    const text = m[0];
    let cls: Token["cls"] = "jn";
    if (text.startsWith('"')) {
      cls = m[1] ? "jk" : "js";
    } else if (text === "true" || text === "false") {
      cls = "jb";
    } else if (text === "null") {
      cls = "jz";
    }
    tokens.push({ text, cls });
    last = start + text.length;
  }
  if (last < json.length) {
    tokens.push({ text: json.slice(last), cls: "jp" });
  }
  return tokens;
}

/** Pretty-printed, lightly syntax-colored JSON. Used for both the `data`
 * payload and the full raw NDJSON line in the row expand panel. */
export function JsonPreview({ value }: { value: unknown }) {
  const json = useMemo(() => {
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return String(value);
    }
  }, [value]);
  const tokens = useMemo(() => tokenize(json), [json]);

  return (
    <pre className="json-pre mono thin-scroll">
      {tokens.map((t, i) => (
        <span key={i} className={t.cls}>
          {t.text}
        </span>
      ))}
    </pre>
  );
}
