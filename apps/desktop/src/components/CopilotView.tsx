import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CopilotAnswer,
  CopilotExplainRequest,
  CopilotStatus,
  CopilotToolInvocation,
} from "../copilotTypes";
import { askCopilot, describeCopilotError, explainIncident, fetchCopilotStatus } from "../lib/copilot";
import { cssVar } from "../lib/cssVars";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "9px 12px",
  fontSize: 12.5,
  color: "var(--fg)",
} as const;

interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  text: string;
  /** Present only on a successful assistant answer - the evidence surface
   * `ToolTraceSection` renders. `undefined` (not an empty array) for a user
   * message or an error note, so the collapsible section never appears on
   * either. */
  toolTrace?: CopilotToolInvocation[];
  /** Set when this "assistant" message is actually `copilot_ask`'s
   * rejection rendered inline (e.g. `CopilotError::NoProvider`'s message) -
   * an honest note, never a crash, styled distinctly from a real answer. */
  isNote?: boolean;
}

/**
 * The residency banner (Phase 6, C0 - docs/PHASE6.md, itrat-console/13
 * D13.2): the one thing every screen of this panel must make impossible to
 * miss - whether a question just typed here can leave this machine at all.
 * Three honest states, never blended: still checking, no provider
 * configured (muted), a local provider (mint - "nothing leaves this box"),
 * or a remote BYO-key provider (amber - a deliberate, explicit opt-in per
 * `genaryx-copilot`'s residency gate, `crates/copilot/src/residency.rs`).
 */
function ResidencyBanner({ status }: { status: CopilotStatus | null }) {
  if (!status) {
    return (
      <div className="d-card px-4 py-3 mono" style={{ fontSize: 12, color: "var(--faint)" }}>
        checking copilot status...
      </div>
    );
  }

  if (!status.enabled) {
    return (
      <div className="d-card px-4 py-3 flex items-center gap-2.5">
        <span
          aria-hidden="true"
          style={{ width: 8, height: 8, borderRadius: "50%", background: "var(--faint)", flex: "0 0 auto" }}
        />
        <span className="text-[12.5px]" style={{ color: "var(--dim)" }}>
          No provider configured{status.disabled_reason ? ` - ${status.disabled_reason}` : ""}
        </span>
      </div>
    );
  }

  const local = status.local === true;
  const tone = local ? "var(--mint)" : "var(--amber)";
  return (
    <div
      className="d-card px-4 py-3 flex items-center gap-2.5"
      style={{ borderColor: `color-mix(in srgb, ${tone} 30%, var(--line))` }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 8,
          height: 8,
          borderRadius: "50%",
          background: tone,
          boxShadow: `0 0 8px ${tone}`,
          flex: "0 0 auto",
        }}
      />
      <span className="mono text-[12.5px]" style={{ color: tone }}>
        {local
          ? `Local: ${status.model ?? "unknown model"} via ${status.provider ?? "unknown provider"} on this machine`
          : `Remote: ${status.provider ?? "unknown provider"} (BYO key)`}
      </span>
    </div>
  );
}

/** The evidence surface (D13.6): every tool Felyx's loop actually ran for
 * this answer, collapsed by default so a plain-text answer stays the focus,
 * one click away for an operator who wants to check the numbers behind it. */
function ToolTraceSection({ trace }: { trace: CopilotToolInvocation[] }) {
  if (trace.length === 0) return null;
  return (
    <details className="mt-2">
      <summary
        className="mono"
        style={{
          fontSize: 10.5,
          letterSpacing: "0.07em",
          textTransform: "uppercase",
          color: "var(--faint)",
          cursor: "pointer",
        }}
      >
        tools used ({trace.length})
      </summary>
      <div className="flex flex-col gap-1.5 mt-2">
        {trace.map((t, idx) => (
          <div key={`${t.name}-${idx}`} className="flex items-start gap-2" style={{ fontSize: 11 }}>
            <span className="badge" style={cssVar("tone", t.ok ? "var(--sev-low)" : "var(--sev-high)")}>
              {t.ok ? "ok" : "fail"}
            </span>
            <span className="mono" style={{ color: "var(--fg)", flex: "0 0 auto", whiteSpace: "nowrap" }}>
              {t.name}
            </span>
            <span className="mono truncate" style={{ color: "var(--faint)" }} title={t.result_preview}>
              {t.result_preview}
            </span>
          </div>
        ))}
      </div>
    </details>
  );
}

function MessageBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";
  return (
    <div className="flex" style={{ justifyContent: isUser ? "flex-end" : "flex-start" }}>
      <div
        className="d-card px-3.5 py-2.5"
        style={{
          maxWidth: "72%",
          background: isUser
            ? "color-mix(in srgb, var(--iris) 12%, var(--panel-2))"
            : message.isNote
              ? "color-mix(in srgb, var(--amber) 8%, var(--panel-2))"
              : "var(--panel-2)",
          borderColor: message.isNote ? "color-mix(in srgb, var(--amber) 30%, var(--line))" : "var(--line)",
        }}
      >
        <span
          className="text-[12.5px]"
          style={{ color: message.isNote ? "var(--dim)" : "var(--fg)", lineHeight: 1.6, whiteSpace: "pre-wrap" }}
        >
          {message.text}
        </span>
        {!isUser && message.toolTrace && <ToolTraceSection trace={message.toolTrace} />}
      </div>
    </div>
  );
}

/**
 * The Copilot panel (Phase 6, C0 - docs/PHASE6.md, itrat-console/13 D13): a
 * chat pane over Felyx, the read-only analyst copilot. A residency banner
 * (pinned, [`ResidencyBanner`]) always tells the operator where inference
 * runs before they type anything; a scrollable transcript below holds every
 * question and answer for this session (in-memory only - there is no
 * `copilot_history` command, and there should not be one until this crate's
 * "propose" tier lands, C2); a pinned composer at the bottom sends one
 * question at a time through [`askCopilot`].
 *
 * C0 ships the read path only (`crates/copilot/src/lib.rs`'s own doc
 * comment): with today's default config (`provider = "none"`, no LLM on
 * this box) every question resolves to `CopilotError::NoProvider`'s message,
 * rendered here as an assistant note rather than a toast or a crash - the
 * panel is fully usable and honest about its own C0 state without a real
 * provider ever being configured.
 *
 * C1 (docs/PHASE6-C1.md) adds `explainRequest`: an "Explain with Felyx"
 * hand-off from a sibling view (the Money panel's Incidents feed), threaded
 * down from `AppShell`'s own state - see `copilotTypes.ts`'s
 * `CopilotExplainRequest` doc comment. This view is unmounted whenever the
 * operator navigates away (`AppShell` only renders it while
 * `view === "copilot"`), so a pending request is simply picked up by the
 * effect below the moment this component (re)mounts.
 */
export function CopilotView({
  explainRequest,
  onExplainRequestHandled,
}: {
  explainRequest: CopilotExplainRequest | null;
  onExplainRequestHandled: () => void;
}) {
  const [status, setStatus] = useState<CopilotStatus | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const nextId = useRef(0);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    void fetchCopilotStatus().then(setStatus);
  }, []);

  // Keep the transcript pinned to the newest message, mirroring any chat
  // surface's baseline expectation - runs after every append (a new
  // question, a new answer, or a new error note all count).
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages]);

  // "Explain with Felyx" hand-off (C1): fires once per `explainRequest.nonce`
  // - which in practice means once per mount, since the button that sets it
  // lives on a different view this component is never simultaneously
  // rendered alongside (see this component's own doc comment). Appends the
  // synthetic question first (so the transcript reads like the operator
  // asked it), then runs the exact same fetch/append/error-note shape
  // `send()` below uses, sharing `sending` so the composer disables the same
  // way for either kind of request. `cancelled` guards the state updates (not
  // the `onExplainRequestHandled()` call itself, which must always fire so a
  // later, unrelated remount of this view never re-triggers the same
  // request) in case the operator navigates away before the round trip
  // finishes.
  useEffect(() => {
    if (!explainRequest) return;
    const { incidentId } = explainRequest;
    let cancelled = false;

    setMessages((m) => [
      ...m,
      { id: nextId.current++, role: "user", text: `Explain incident \`${incidentId}\`` },
    ]);
    setSending(true);

    void (async () => {
      try {
        const answer: CopilotAnswer = await explainIncident(incidentId);
        if (!cancelled) {
          setMessages((m) => [
            ...m,
            { id: nextId.current++, role: "assistant", text: answer.text, toolTrace: answer.tool_trace },
          ]);
        }
      } catch (err) {
        if (!cancelled) {
          setMessages((m) => [
            ...m,
            { id: nextId.current++, role: "assistant", text: describeCopilotError(err), isNote: true },
          ]);
        }
      } finally {
        if (!cancelled) setSending(false);
        onExplainRequestHandled();
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [explainRequest, onExplainRequestHandled]);

  const send = useCallback(async () => {
    const question = input.trim();
    if (!question || sending) return;

    setMessages((m) => [...m, { id: nextId.current++, role: "user", text: question }]);
    setInput("");
    setSending(true);
    try {
      const answer: CopilotAnswer = await askCopilot(question);
      setMessages((m) => [
        ...m,
        { id: nextId.current++, role: "assistant", text: answer.text, toolTrace: answer.tool_trace },
      ]);
    } catch (err) {
      // e.g. CopilotError::NoProvider's message with today's default config
      // - an honest note about why there is no answer, never a crash.
      setMessages((m) => [
        ...m,
        { id: nextId.current++, role: "assistant", text: describeCopilotError(err), isNote: true },
      ]);
    } finally {
      setSending(false);
    }
  }, [input, sending]);

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="px-5 pt-4 pb-2 shrink-0">
        <ResidencyBanner status={status} />
      </div>

      <div ref={scrollRef} className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-2 flex flex-col gap-3">
        {messages.length === 0 ? (
          <div className="flex-1 min-h-0 flex items-center justify-center">
            <span className="mono text-[12px] text-center" style={{ color: "var(--faint)", maxWidth: 420 }}>
              Ask Felyx about your agent fleet - spend, alerts, runs, approvals. Felyx can read and recommend, never
              act: any change still needs a human to approve and sign it.
            </span>
          </div>
        ) : (
          messages.map((m) => <MessageBubble key={m.id} message={m} />)
        )}
      </div>

      <div className="d-card mx-5 mb-4 mt-2 px-3 py-3 flex items-center gap-2 shrink-0">
        <input
          className="mono flex-1"
          style={FIELD_STYLE}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Ask Felyx..."
          spellCheck={false}
          disabled={sending}
          onKeyDown={(e) => {
            if (e.key === "Enter") void send();
          }}
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void send()}
          disabled={sending || input.trim().length === 0}
        >
          {sending ? "Asking..." : "Send"}
        </button>
      </div>
    </div>
  );
}
