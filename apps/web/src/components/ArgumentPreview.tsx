import type { ReactNode } from "react";
import { visibleText } from "../lib/policyExport";
import type { PreviewDocument, PreviewNode, PreviewPart } from "../policyTypes";

/**
 * What a person deciding a held call is shown of its arguments (CLAUDE.md
 * invariant 48): wardryx's preview document, a tree of typed nodes it built from
 * the bytes the grant is bound to, with secrets and personal data already
 * hidden. Rendered with text nodes only: nothing here is markup, nothing is a
 * link, and every character that would not read as it looks is written out by
 * code point. The frame says whose words these are, because an argument can
 * carry a sentence addressed to the person deciding.
 */
export function ArgumentPreview({ document, pvd }: { document: PreviewDocument; pvd: string | null }) {
  return (
    <div
      className="flex flex-col gap-1.5"
      data-testid="argument-preview"
      style={{ border: "1px solid var(--line)", borderRadius: 6, padding: "8px 10px", background: "var(--panel)" }}
    >
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        Data from the agent, not instructions. Secrets and personal data are hidden by wardryx (rules {document.ruleset}).
      </span>
      <div className="mono text-[12px]" style={{ color: "var(--fg)", whiteSpace: "pre-wrap", wordBreak: "break-all" }}>
        <Value node={document.root} depth={0} />
      </div>
      {pvd && (
        <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }} title="The digest a grant of this call names">
          preview {pvd}
        </span>
      )}
    </div>
  );
}

const MAX_RENDER_DEPTH = 12;

function Badge({ children, tone }: { children: ReactNode; tone: string }) {
  return (
    <span className="badge" style={{ color: tone, borderColor: tone }}>
      {children}
    </span>
  );
}

function Parts({ parts }: { parts: PreviewPart[] }) {
  return (
    <>
      {parts.map((p, i) => {
        if (p.hidden !== undefined) {
          return (
            <Badge key={i} tone="var(--sev-medium)">
              hidden: {visibleText(p.hidden)}
            </Badge>
          );
        }
        if (p.vault !== undefined) {
          return (
            <Badge key={i} tone="var(--dim)">
              vault secret {visibleText(p.vault)}
            </Badge>
          );
        }
        return <span key={i}>{visibleText(p.text ?? "")}</span>;
      })}
    </>
  );
}

function Value({ node, depth }: { node: PreviewNode; depth: number }): ReactNode {
  if (depth > MAX_RENDER_DEPTH) return <span>...</span>;
  switch (node.k) {
    case "object":
      return (
        <span>
          {"{"}
          {node.duplicate_keys && <Badge tone="var(--sev-high)">repeated key</Badge>}
          {(node.members ?? []).map((m, i) => (
            <div key={i} style={{ paddingLeft: 14 }}>
              <Value node={m.key} depth={depth + 1} />: <Value node={m.value} depth={depth + 1} />
            </div>
          ))}
          {node.omitted ? <div style={{ paddingLeft: 14 }}>... {node.omitted} more members</div> : null}
          {"}"}
        </span>
      );
    case "array":
      return (
        <span>
          {"["}
          {(node.items ?? []).map((it, i) => (
            <div key={i} style={{ paddingLeft: 14 }}>
              <Value node={it} depth={depth + 1} />
            </div>
          ))}
          {node.omitted ? <div style={{ paddingLeft: 14 }}>... {node.omitted} more items</div> : null}
          {"]"}
        </span>
      );
    case "string":
      return (
        <span>
          {'"'}
          <Parts parts={node.parts ?? []} />
          {node.cut && `... (${node.runes ?? "?"} characters${node.sha256 ? `, sha256 ${node.sha256}` : ""})`}
          {'"'}
          {(node.flags ?? []).map((f) => (
            <Badge key={f} tone="var(--sev-high)">
              {visibleText(f)}
            </Badge>
          ))}
        </span>
      );
    case "number":
      return <span>{visibleText(node.lexeme ?? "")}{node.cut ? "..." : ""}</span>;
    case "bool":
      return <span>{node.value ? "true" : "false"}</span>;
    case "null":
      return <span>null</span>;
    case "hidden":
      return (
        <Badge tone="var(--sev-medium)">
          hidden {visibleText(node.type ?? "value")}: {visibleText(node.rule ?? "")}
        </Badge>
      );
    case "vault_handle":
      return <Badge tone="var(--dim)">vault secret {visibleText(node.name ?? "")}</Badge>;
    case "blob":
      return (
        <Badge tone="var(--dim)">
          {visibleText(node.mime ?? "binary")}, {node.bytes ?? 0} bytes, sha256 {visibleText(node.sha256 ?? "")}
        </Badge>
      );
    case "elided":
      return (
        <span style={{ color: "var(--faint)" }}>
          ... cut ({visibleText(node.reason ?? "")}, {node.count ?? 0})
        </span>
      );
    default:
      return <span style={{ color: "var(--faint)" }}>unknown node</span>;
  }
}
