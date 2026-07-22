import { useState } from "react";
import { invokeBackend } from "../lib/transport";
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
 */

interface Provider {
  id: string;
  label: string;
  local: boolean;
  defaultModel: string;
  defaultBase?: string;
  keyLabel?: string;
}

const PROVIDERS: Provider[] = [
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

export function FelyxConnectCard({ onConnected }: { onConnected?: () => void }) {
  const win = useWindowControls();
  const close = () => win?.close();
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
    if (!model.trim()) {
      setError("Pick a model.");
      return;
    }
    if (!provider.local && !apiKey.trim()) {
      setError(`${provider.label} needs an API key.`);
      return;
    }
    setSaving(true);
    try {
      await invokeBackend("copilot_connect", {
        provider: provider.id,
        model: model.trim(),
        base_url: baseUrl.trim() || null,
        api_key: provider.local ? null : apiKey.trim(),
        allow_non_local_endpoints: !provider.local,
        max_usd_per_day: Number(maxUsd) || 5,
        local: provider.local,
      });
      onConnected?.();
      close();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save. This box may not support connecting Felyx yet.");
    } finally {
      setSaving(false);
    }
  };

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
