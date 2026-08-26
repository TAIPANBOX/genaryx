import { useState } from "react";
import { hasBackend, invokeBackend, isWebShell } from "../lib/transport";
import { useWindowControls, PopoverHeader } from "../lib/popover";

/**
 * Connect Felyx to a model.
 *
 * The Copilot tab used to show only a chat box and a "no provider configured"
 * line, with nowhere to actually pick a provider or enter a key (Yurii's
 * complaint, 2026-07-22). This is that missing form: choose one of the
 * providers the backend already models (Anthropic, OpenAI-compatible,
 * OpenRouter, Ollama, LM Studio), give a key for the cloud ones, set a daily
 * spend cap, and save.
 *
 * It states the residency truth plainly, because that is the whole point of
 * this panel: a local runtime (Ollama / LM Studio) keeps every prompt on this
 * machine; a cloud provider is a deliberate BYO-key opt-in where prompts and
 * the key leave the box. Felyx still only reads and proposes, never acts,
 * whichever provider is chosen.
 *
 * HONESTY: only the preview backend answers `copilot_connect`. The command is
 * named in exactly two places in this repository, [`planConnect`]'s dispatch
 * below and `lib/mockPreview.ts`'s own switch, and in no crate, doc, script or
 * provisioning file. A real box refuses it twice over: an unclassified command
 * name fails closed to Admin in `crates/web/src/roles.rs`'s `required_role`,
 * so a non-admin is refused 403 at the chokepoint, and an admin reaches the
 * fallback arm of `crates/web/src/dispatch.rs` and is refused 404 "unknown
 * command". So this card is preview-only, in the same sense and the same words
 * as `lib/agentRecord.ts`, `lib/agentActions.ts` and `lib/entityRecords.ts`:
 * [`felyxConnectSupport`] says which box this is, and `CopilotView` does not
 * offer the card at all on a real one.
 *
 * That is a stronger statement than "not built yet", and the difference is the
 * point. A box's provider IS configurable, just not from a browser.
 * `crates/api/src/copilot/state.rs`'s `config_from_env` reads the whole
 * surface off the process environment at bootstrap
 * (`GENARYX_COPILOT_PROVIDER`, `_MODEL`, `_BASE_URL`, `_ALLOW_REMOTE`), and
 * the key is deliberately never a value it holds: `_API_KEY_REF` names an
 * `env:` or `file:` location, "a pointer to a secret, resolved at use, never
 * stored in the config value" (`crates/copilot/src/config.rs`). A console-side
 * `copilot_connect` taking the raw key this form collects would be writing a
 * credential through the browser into a design built to hold only a reference
 * to one. That is a security surface and a decision for a human to take on
 * purpose, not a gap to close on the way past.
 *
 * Until somebody takes it, the form must not SEND either, and that is the
 * sharper half. The 403 or the 404 arrives only AFTER the request body has
 * crossed the wire, so a card that merely reported the failure had already
 * handed the operator's API key to a box with nowhere to put it.
 * [`planConnect`] therefore refuses before `invokeBackend` is called at all.
 */

export interface Provider {
  id: string;
  label: string;
  local: boolean;
  defaultModel: string;
  defaultBase?: string;
  keyLabel?: string;
}

export const PROVIDERS: Provider[] = [
  { id: "anthropic", label: "Anthropic", local: false, defaultModel: "claude-haiku-4-5", keyLabel: "Anthropic API key" },
  { id: "openai_compat", label: "OpenAI", local: false, defaultModel: "gpt-4o-mini", keyLabel: "OpenAI API key" },
  { id: "openrouter", label: "OpenRouter", local: false, defaultModel: "meta-llama/llama-3.1-70b-instruct", keyLabel: "OpenRouter API key" },
  { id: "ollama", label: "Ollama", local: true, defaultModel: "qwen2.5:7b-instruct", defaultBase: "http://127.0.0.1:11434/v1" },
  { id: "lmstudio", label: "LM Studio", local: true, defaultModel: "local-model", defaultBase: "http://127.0.0.1:1234/v1" },
];

const inputStyle = {
  width: "100%",
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "8px 10px",
  fontSize: 12.5,
  color: "var(--fg)",
} as const;

function Label({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[10.5px] uppercase tracking-wider" style={{ color: "var(--faint)", marginBottom: 4, marginTop: 10 }}>
      {children}
    </div>
  );
}

/** Whether THIS build can connect Felyx at all, and if not, what is true
 * instead. Two refusals rather than one boolean: "there is no box" and "this
 * box does not take its provider from a browser" are different facts, and an
 * operator can act on the second.
 *
 * Three strings because a refusal is read in two places at two sizes.
 * [`short`] is what a banner that has just dropped a button can afford; a
 * control that vanishes with nothing in its place is its own small dishonesty,
 * and the operator is left to guess whether the console is broken or the
 * feature is elsewhere. [`reason`] and [`detail`] are the full account, for
 * the card and for the banner that has room. */
export type FelyxConnectSupport =
  | { supported: true }
  | { supported: false; short: string; reason: string; detail: string };

/** A `genaryx-web` box. Everything in the detail is checkable on the box
 * itself, which is the test of a refusal worth printing: it names the
 * variables `config_from_env` reads and says which of them carries a location
 * rather than a secret. */
const NOT_FROM_A_BROWSER: FelyxConnectSupport = {
  supported: false,
  short: "Set on the box, not from the console.",
  reason: "This console does not configure Felyx's provider on a real box.",
  detail:
    "A genaryx-web box has no copilot_connect command. It reads its provider, model and endpoint from the process environment it starts with: GENARYX_COPILOT_PROVIDER, GENARYX_COPILOT_MODEL, GENARYX_COPILOT_BASE_URL and GENARYX_COPILOT_ALLOW_REMOTE. The key is not one of them. GENARYX_COPILOT_API_KEY_REF names where the key lives, as env:VARIABLE or file:/path, and the box reads it at the moment of use. Set those on the box and restart it.",
};

/** A bare `vite preview`: no `VITE_GENARYX_API` and not the mock build, so
 * there is no backend of either kind behind this page. */
const NO_BOX_AT_ALL: FelyxConnectSupport = {
  supported: false,
  short: "No box behind this page.",
  reason: "There is no box to connect Felyx on.",
  detail:
    "This build has no backend of either kind: no VITE_GENARYX_API pointing at a genaryx-web, and not the mock preview. There is no provider here to configure.",
};

/** Which of the three builds this is, decided from the transport rather than
 * from a build flag this module would have to keep in step: a web shell is a
 * real box, a backend that is not a web shell is the mock preview, and neither
 * is a bare preview. See this module's doc comment for why only the preview
 * answers `copilot_connect`. */
export function felyxConnectSupport(): FelyxConnectSupport {
  if (isWebShell()) return NOT_FROM_A_BROWSER;
  if (!hasBackend()) return NO_BOX_AT_ALL;
  return { supported: true };
}

/** The form's values, exactly as the inputs hold them (all strings but the
 * chosen provider), so [`planConnect`] can be driven without a DOM. */
export interface ConnectForm {
  provider: Provider;
  model: string;
  baseUrl: string;
  apiKey: string;
  maxUsd: string;
}

export type ConnectPlan =
  | { send: true; args: Record<string, unknown> }
  | { send: false; error: string };

/**
 * Everything the Connect button decides, with no side effect: whether this box
 * can take the request at all, whether the form is complete, and the exact
 * argument object to send if both hold.
 *
 * The support check comes FIRST and on purpose. A refusal built after the
 * request is a refusal that has already sent the key (see the module doc), so
 * on an unsupported box this returns the reason and the args are never
 * constructed, which is also why no `error` here ever quotes a field value.
 */
export function planConnect(form: ConnectForm, support: FelyxConnectSupport): ConnectPlan {
  if (!support.supported) {
    return { send: false, error: `${support.reason} ${support.detail}` };
  }
  if (!form.model.trim()) {
    return { send: false, error: "Pick a model." };
  }
  if (!form.provider.local && !form.apiKey.trim()) {
    return { send: false, error: `${form.provider.label} needs an API key.` };
  }
  return {
    send: true,
    args: {
      provider: form.provider.id,
      model: form.model.trim(),
      base_url: form.baseUrl.trim() || null,
      api_key: form.provider.local ? null : form.apiKey.trim(),
      allow_non_local_endpoints: !form.provider.local,
      // A cap the operator cannot read back is worse than no cap, so an
      // unparseable one falls back to the default this card states, never to
      // NaN on the wire.
      max_usd_per_day: Number(form.maxUsd) || 5,
      local: form.provider.local,
    },
  };
}

/**
 * What to show when the dispatch itself was refused.
 *
 * `invokeBackend` rejects with the box's OWN structured error body for every
 * non-2xx, never an `Error`, so an `instanceof Error` test alone falls through
 * to whatever the fallback says and the operator never sees the words the box
 * used. This prefers the box's `error` string, and where there is none it says
 * only what is certainly true: the request was refused and nothing was saved.
 * It never guesses at a cause, which is what "may not support this yet" was.
 */
export function refusalText(cause: unknown): string {
  const nothing = "Nothing was saved, and the provider is unchanged.";
  if (cause && typeof cause === "object" && !(cause instanceof Error)) {
    const said = (cause as { error?: unknown }).error;
    if (typeof said === "string" && said.trim().length > 0) {
      return `The box refused this: ${said}. ${nothing}`;
    }
  }
  if (cause instanceof Error && cause.message.trim().length > 0) {
    return `${cause.message}. ${nothing}`;
  }
  return `The box refused this. ${nothing}`;
}

export function FelyxConnectCard({ onConnected }: { onConnected?: () => void }) {
  const win = useWindowControls();
  const close = () => win?.close();
  const support = felyxConnectSupport();
  const [providerId, setProviderId] = useState<string>("ollama");
  const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];
  const [model, setModel] = useState(provider.defaultModel);
  const [baseUrl, setBaseUrl] = useState(provider.defaultBase ?? "");
  const [apiKey, setApiKey] = useState("");
  const [maxUsd, setMaxUsd] = useState("5");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pick = (p: Provider) => {
    setProviderId(p.id);
    setModel(p.defaultModel);
    setBaseUrl(p.defaultBase ?? "");
    setApiKey("");
    setError(null);
  };

  const save = async () => {
    setError(null);
    const plan = planConnect({ provider, model, baseUrl, apiKey, maxUsd }, support);
    if (!plan.send) {
      setError(plan.error);
      return;
    }
    setSaving(true);
    try {
      await invokeBackend("copilot_connect", plan.args);
      onConnected?.();
      close();
    } catch (e) {
      setError(refusalText(e));
    } finally {
      setSaving(false);
    }
  };

  // Defence in depth. `CopilotView` already withholds the button that opens
  // this card on a box that cannot take it, so reaching here means some other
  // caller opened it anyway; a preview-only surface that turns up on a real
  // box has to say so itself rather than rely on whoever opened it.
  if (!support.supported) {
    return (
      <div className="flex flex-col">
        <PopoverHeader kicker="Copilot" title="Connect Felyx" onClose={close} />
        <div style={{ padding: "0 16px 16px" }}>
          <div
            className="text-[10.5px] uppercase tracking-wider"
            style={{ color: "var(--amber)", marginTop: 2 }}
          >
            preview only
          </div>
          <div className="text-[12.5px]" style={{ color: "var(--fg)", marginTop: 8 }}>
            {support.reason}
          </div>
          <div className="text-[11.5px]" style={{ color: "var(--dim)", marginTop: 8, lineHeight: 1.55 }}>
            {support.detail}
          </div>
          <button
            type="button"
            onClick={close}
            className="text-[12.5px]"
            style={{
              marginTop: 14,
              padding: "8px 14px",
              borderRadius: 8,
              cursor: "pointer",
              border: "1px solid var(--line-2)",
              background: "var(--panel)",
              color: "var(--dim)",
            }}
          >
            Close
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Copilot" title="Connect Felyx" onClose={close} />

      <div style={{ padding: "0 16px 14px" }}>
        <Label>Provider</Label>
        <div className="flex flex-wrap gap-1.5">
          {PROVIDERS.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={() => pick(p)}
              className="text-[12px]"
              style={{
                padding: "6px 11px",
                borderRadius: 8,
                cursor: "pointer",
                border: `1px solid ${p.id === providerId ? "var(--iris)" : "var(--line-2)"}`,
                background: p.id === providerId ? "color-mix(in srgb, var(--iris) 16%, transparent)" : "var(--panel)",
                color: p.id === providerId ? "var(--fg)" : "var(--dim)",
              }}
            >
              {p.label}
            </button>
          ))}
        </div>

        {/* The residency truth, restated for the chosen provider. */}
        <div
          className="text-[11.5px]"
          style={{
            marginTop: 10,
            padding: "8px 10px",
            borderRadius: 8,
            background: "var(--panel)",
            border: `1px solid color-mix(in srgb, ${provider.local ? "var(--mint)" : "var(--amber)"} 30%, var(--line))`,
            color: provider.local ? "var(--mint)" : "var(--amber)",
          }}
        >
          {provider.local
            ? "Local runtime: every prompt stays on this machine. No key needed."
            : "Cloud provider: prompts and your key leave the box. A deliberate BYO-key opt-in."}
        </div>

        <Label>Model</Label>
        <input style={inputStyle} value={model} onChange={(e) => setModel(e.target.value)} placeholder="model id" />

        {(provider.local || baseUrl) && (
          <>
            <Label>Base URL</Label>
            <input style={inputStyle} value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="http://127.0.0.1:11434/v1" />
          </>
        )}

        {!provider.local && (
          <>
            <Label>{provider.keyLabel ?? "API key"}</Label>
            <input
              style={inputStyle}
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="sk-..."
              autoComplete="off"
            />
          </>
        )}

        <Label>Daily spend cap (USD)</Label>
        <input style={inputStyle} value={maxUsd} onChange={(e) => setMaxUsd(e.target.value)} inputMode="decimal" placeholder="5" />

        <div className="text-[11px]" style={{ color: "var(--faint)", marginTop: 8 }}>
          Felyx reads and recommends only. It holds no signing key, so it can never press a button itself; every change still needs a human to approve and sign it.
        </div>

        {error && (
          <div className="text-[11.5px]" style={{ color: "var(--sev-high)", marginTop: 8 }}>
            {error}
          </div>
        )}

        <div className="flex items-center gap-2" style={{ marginTop: 14 }}>
          <button
            type="button"
            onClick={() => void save()}
            disabled={saving}
            className="text-[12.5px]"
            style={{
              padding: "8px 16px",
              borderRadius: 8,
              cursor: saving ? "default" : "pointer",
              border: "1px solid var(--iris)",
              background: "color-mix(in srgb, var(--iris) 20%, transparent)",
              color: "var(--fg)",
              opacity: saving ? 0.6 : 1,
            }}
          >
            {saving ? "Connecting..." : "Connect Felyx"}
          </button>
          <button
            type="button"
            onClick={close}
            className="text-[12.5px]"
            style={{ padding: "8px 14px", borderRadius: 8, cursor: "pointer", border: "1px solid var(--line-2)", background: "var(--panel)", color: "var(--dim)" }}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
