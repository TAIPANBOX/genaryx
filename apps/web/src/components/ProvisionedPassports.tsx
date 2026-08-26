import type { ReactNode } from "react";
import { cssVar } from "../lib/cssVars";
import type { OnboardStatus, Provisioned } from "../onboardTypes";
import { Section } from "./dash";

/** The provisioned-passports table's column track. */
const PASSPORT_COLUMNS = "1fr 1fr 1fr 90px 90px 110px 90px";

/** Best-effort guess at a provisioned passport's `key_id`, for the "Verify"
 * per-row action's pre-fill (I6, docs/ADMISSION.md). `ProvisionedDto.file`
 * is `<passports_dir>/<path with '/' -> '-'>.json`
 * (`onboard::commands::onboard_generate`), and the DEFAULT `key_id` is that
 * SAME `path.replace('/', '-')` when the operator never overrode it at
 * generation time - so stripping the directory and the `.json` extension
 * off `file` recovers it exactly in the common (un-overridden) case. A
 * genuine guess, not an authority: a custom `key_id` override at generation
 * time makes this wrong, which is exactly why the Verify section's own key
 * id field stays freely editable rather than locked to this value. */
function guessKeyIdFromPassportFile(file: string): string {
  const base = file.split(/[/\\]/).pop() ?? file;
  return base.endsWith(".json") ? base.slice(0, -".json".length) : base;
}

/** One labelled line of declaration chips, under the passport's own row. */
function DeclarationLine({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span
        className="mono shrink-0"
        style={{
          fontSize: 10,
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          color: "var(--faint)",
        }}
      >
        {label}
      </span>
      {children}
    </div>
  );
}

/** One declaration, as it stands in the passport file. */
function Chip({ children, title }: { children: ReactNode; title: string }) {
  return (
    <span
      className="mono text-[11px]"
      title={title}
      style={{
        color: "var(--dim)",
        border: "1px solid var(--line-2)",
        borderRadius: 4,
        padding: "1px 6px",
        whiteSpace: "nowrap",
      }}
    >
      {children}
    </span>
  );
}

/** A field the passport file does not declare. Said out loud, never left as
 * an empty cell: a blank reads as a rendering fault, and this is a fact about
 * the file (the backend's `declared()` has already collapsed blank and absent
 * into the same `null`). */
function NotDeclared({ what }: { what: string }) {
  return <span style={{ color: "var(--faint)" }}>{what} not declared</span>;
}

/**
 * The declarations a passport file carries, under its own row: the
 * `filesystem` scopes and the `models` entries, in file order.
 *
 * `onboard_status` has always deserialized both arrays off disk and then read
 * only `.len()` off each, so the console had already parsed the words
 * `anthropic` and `claude-sonnet-4-5` and rendered "2 models". Nothing here
 * computes or infers anything: every chip is a string the backend sent.
 *
 * Three states, and the third is the one worth the code. A list that arrives
 * EMPTY means this passport declares nothing, and the row stays as quiet as
 * it was before. A list that does not arrive AT ALL, while its count says
 * there is something to show, means the source did not report the
 * declarations: an older genaryx-api, or `lib/mockPreview.ts`. That is not
 * the same statement, and an operator must not have to guess which one they
 * are looking at. This console's own api sends both lists or neither, so the
 * note speaks about the source rather than about one column.
 */
function PassportDeclarations({ passport }: { passport: Provisioned }) {
  const folders = passport.filesystem ?? [];
  const models = passport.models ?? [];
  const countOnly =
    (passport.filesystem === undefined && passport.filesystem_count > 0) ||
    (passport.models === undefined && passport.models_count > 0);
  if (folders.length === 0 && models.length === 0 && !countOnly) return null;
  return (
    // Indented and rule-led so it reads as hanging off the row above rather
    // than as a loose line between two rows: everything here is a statement
    // about ONE passport, and a misattributed declaration is worse than none.
    <div
      className="flex flex-col gap-1 pb-2 pl-3 ml-4 mr-4"
      style={{ borderLeft: "2px solid var(--line-2)" }}
    >
      {folders.length > 0 && (
        <DeclarationLine label="declared folders">
          {folders.map((scope, idx) => (
            <Chip
              key={`fs-${idx}-${scope.path ?? ""}`}
              title={`${scope.path ?? "path not declared"} (${scope.mode ?? "mode not declared"})`}
            >
              {scope.path ?? <NotDeclared what="path" />}
              <span style={{ color: "var(--faint)" }}>
                {" · "}
                {scope.mode ?? "mode not declared"}
              </span>
            </Chip>
          ))}
        </DeclarationLine>
      )}
      {models.length > 0 && (
        <DeclarationLine label="declared models">
          {models.map((decl, idx) => (
            <Chip
              key={`model-${idx}-${decl.provider ?? ""}-${decl.model ?? ""}`}
              title={[
                decl.provider ?? "provider not declared",
                decl.model ?? "model not declared",
                decl.endpoint ?? "endpoint not declared",
              ].join(" / ")}
            >
              {decl.provider ?? <NotDeclared what="provider" />}
              <span style={{ color: "var(--faint)" }}>{" · "}</span>
              {decl.model ?? <NotDeclared what="model" />}
              <span style={{ color: "var(--faint)" }}>{" · "}</span>
              {decl.endpoint ?? <NotDeclared what="endpoint" />}
            </Chip>
          ))}
        </DeclarationLine>
      )}
      {countOnly && (
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          this source reported the count only, not the declarations.
        </span>
      )}
    </div>
  );
}

/**
 * The "Provisioned passports" section: one row per passport file the backend
 * could read out of the staging dir, plus the tolerant "skipped" list for the
 * ones it could not.
 *
 * Split out of `OnboardView.tsx` (which owns the fetch, the generate form and
 * the Verify seed) so what the table SHOWS is a plain function of the status
 * it was handed, with no effect to run and nothing to mock:
 * `components/ProvisionedPassports.test.ts` renders it with
 * `renderToStaticMarkup` and reads the markup.
 */
export function ProvisionedPassports({
  status,
  onVerify,
}: {
  status: OnboardStatus;
  onVerify: (keyId: string, agentId: string) => void;
}) {
  return (
  <Section title="Provisioned passports">
    {status.passports.length === 0 ? (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no provisioned passports found in {status.passports_dir} yet.
      </div>
    ) : (
      <div style={{ overflowX: "auto" }}>
        <div
          className="grid gap-3 px-4 py-2"
          style={{
            gridTemplateColumns: PASSPORT_COLUMNS,
            borderBottom: "1px solid var(--line-2)",
            background: "var(--panel-3)",
          }}
        >
          {["agent id", "owner", "file", "folders", "models", "", ""].map((label, idx) => (
            <span
              key={`${label}-${idx}`}
              className="mono"
              style={{
                fontSize: 10,
                letterSpacing: "0.08em",
                textTransform: "uppercase",
                color: "var(--faint)",
              }}
            >
              {label}
            </span>
          ))}
        </div>
        {status.passports.map((p) => (
          <div key={p.file}>
          <div
            className="grid items-center gap-3 px-4 py-2 bus-row"
            style={{ gridTemplateColumns: PASSPORT_COLUMNS }}
          >
            <span
              className="mono truncate text-[11.5px]"
              title={p.agent_id}
              style={{ color: "var(--fg)" }}
            >
              {p.agent_id}
            </span>
            <span
              className="mono truncate text-[11.5px]"
              title={p.owner}
              style={{ color: "var(--dim)" }}
            >
              {p.owner}
            </span>
            <span
              className="mono truncate text-[11px]"
              title={p.file}
              style={{ color: "var(--faint)" }}
            >
              {p.file}
            </span>
            <span
              className="mono truncate text-[11px]"
              title={
                p.filesystem_count > 0
                  ? `${p.filesystem_count} declared filesystem scope${p.filesystem_count === 1 ? "" : "s"}`
                  : "no declared filesystem scopes"
              }
              style={{ color: "var(--faint)" }}
            >
              {p.filesystem_count > 0 ? `${p.filesystem_count} folder${p.filesystem_count === 1 ? "" : "s"}` : "-"}
            </span>
            <span
              className="mono truncate text-[11px]"
              title={
                p.models_count > 0
                  ? `${p.models_count} declared model${p.models_count === 1 ? "" : "s"}`
                  : "no declared models"
              }
              style={{ color: "var(--faint)" }}
            >
              {p.models_count > 0 ? `${p.models_count} model${p.models_count === 1 ? "" : "s"}` : "-"}
            </span>
            <span
              className="badge"
              style={cssVar("tone", p.in_map ? "var(--mint)" : "var(--sev-medium)")}
            >
              {p.in_map ? "in map" : "not in map"}
            </span>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
              onClick={() => onVerify(guessKeyIdFromPassportFile(p.file), p.agent_id)}
            >
              Verify
            </button>
          </div>
          <PassportDeclarations passport={p} />
          </div>
        ))}
      </div>
    )}
    {status.skipped.length > 0 && (
      <div className="px-4 py-2.5" style={{ borderTop: "1px solid var(--line-2)" }}>
        <details>
          <summary className="text-[11px]" style={{ color: "var(--faint)", cursor: "pointer" }}>
            {status.skipped.length} file{status.skipped.length === 1 ? "" : "s"} skipped (could
            not be parsed)
          </summary>
          <div className="flex flex-col gap-1 mt-1.5">
            {status.skipped.map((s) => (
              <div key={s.file} className="mono text-[11px]" style={{ color: "var(--dim)" }}>
                {s.file} - {s.reason}
              </div>
            ))}
          </div>
        </details>
      </div>
    )}
  </Section>
  );
}
