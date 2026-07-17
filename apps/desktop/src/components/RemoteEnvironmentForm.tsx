import { useCallback, useEffect, useState } from "react";
import { describeRemoteError, setRemoteEnvironment } from "../lib/remote";
import { blankRemoteEnvironment } from "../remoteTypes";
import type { RemoteEnvironment, RemoteError, RemoteStatus } from "../remoteTypes";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
  width: "100%",
} as const;

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11px]" style={{ color: "var(--dim)" }}>
        {label}
      </span>
      {children}
    </label>
  );
}

function SubHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 10.5, letterSpacing: "0.09em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

/**
 * Section 2 (docs/PHASE4.md W4 position 2): the operator-defined remote
 * environment - the WG peer, the tunnel local/peer IPs, the SSH target, and
 * the `wireguard-go` binary path. Saving replaces whatever was there before
 * (`remote_set_environment` resets the SSH client/tunnel/tail - see the Rust
 * module's doc comment) - a deliberate, explicit action, never inferred.
 *
 * Pre-fills `wireguard_go_bin` from `defaultWireguardGoBin` ONCE, on first
 * load with nothing saved yet; never overwrites an operator's own edit on a
 * later status re-render (mirrors `EvidenceView.tsx`'s identical "prefill
 * once" `useEffect` discipline).
 */
export function RemoteEnvironmentForm({
  environment,
  defaultWireguardGoBin,
  onSaved,
}: {
  environment: RemoteEnvironment | null;
  defaultWireguardGoBin: string | null;
  onSaved: (status: RemoteStatus) => void;
}) {
  const [form, setForm] = useState<RemoteEnvironment>(() => environment ?? blankRemoteEnvironment());
  const [allowedIpsText, setAllowedIpsText] = useState(() => (environment?.wg_allowed_ips ?? []).join(", "));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<RemoteError | null>(null);
  const [savedAtMs, setSavedAtMs] = useState<number | null>(null);
  const [prefilled, setPrefilled] = useState(false);

  useEffect(() => {
    if (prefilled || environment !== null) return;
    if (defaultWireguardGoBin) {
      setForm((f) => ({ ...f, wireguard_go_bin: defaultWireguardGoBin }));
      setPrefilled(true);
    }
  }, [prefilled, environment, defaultWireguardGoBin]);

  const set = <K extends keyof RemoteEnvironment>(key: K, value: RemoteEnvironment[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const canSave =
    form.name.trim().length > 0 &&
    form.wg_peer_public_key_hex.trim().length > 0 &&
    form.wg_endpoint.trim().length > 0 &&
    form.wg_local_ip.trim().length > 0 &&
    form.wg_peer_ip.trim().length > 0 &&
    form.ssh_host.trim().length > 0 &&
    form.ssh_user.trim().length > 0 &&
    form.ssh_identity_file.trim().length > 0 &&
    form.ssh_pinned_host_key.trim().length > 0;

  const onSave = useCallback(async () => {
    if (!canSave || saving) return;
    setSaving(true);
    setError(null);
    try {
      const allowedIps = allowedIpsText
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      const status = await setRemoteEnvironment({ ...form, wg_allowed_ips: allowedIps });
      setSavedAtMs(Date.now());
      onSaved(status);
    } catch (err) {
      setError(err as RemoteError);
    } finally {
      setSaving(false);
    }
  }, [canSave, saving, allowedIpsText, form, onSaved]);

  return (
    <div className="panel px-4 py-3 flex flex-col gap-3" style={{ background: "var(--panel-2)" }}>
      <Field label="environment name">
        <input
          className="mono"
          style={FIELD_STYLE}
          value={form.name}
          onChange={(e) => set("name", e.target.value)}
          placeholder="hetzner-campaign-1"
          spellCheck={false}
        />
      </Field>

      <SubHeader title="WireGuard peer (the client-hosted box)" />
      <div className="grid gap-2.5" style={{ gridTemplateColumns: "1fr 1fr" }}>
        <Field label="peer public key (hex)">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={form.wg_peer_public_key_hex}
            onChange={(e) => set("wg_peer_public_key_hex", e.target.value)}
            placeholder="64 hex chars"
            spellCheck={false}
          />
        </Field>
        <Field label="peer endpoint (host:port)">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={form.wg_endpoint}
            onChange={(e) => set("wg_endpoint", e.target.value)}
            placeholder="203.0.113.9:51820"
            spellCheck={false}
          />
        </Field>
        <Field label="allowed IPs (comma-separated)">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={allowedIpsText}
            onChange={(e) => setAllowedIpsText(e.target.value)}
            placeholder="10.9.0.1/32"
            spellCheck={false}
          />
        </Field>
        <Field label="persistent keepalive (secs, optional)">
          <input
            className="mono"
            style={FIELD_STYLE}
            type="number"
            min={0}
            value={form.wg_persistent_keepalive ?? ""}
            onChange={(e) => set("wg_persistent_keepalive", e.target.value === "" ? null : Number(e.target.value))}
            placeholder="25"
          />
        </Field>
        <Field label="tunnel local IP (this console)">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={form.wg_local_ip}
            onChange={(e) => set("wg_local_ip", e.target.value)}
            placeholder="10.9.0.2"
            spellCheck={false}
          />
        </Field>
        <Field label="tunnel peer IP (the box)">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={form.wg_peer_ip}
            onChange={(e) => set("wg_peer_ip", e.target.value)}
            placeholder="10.9.0.1"
            spellCheck={false}
          />
        </Field>
        <Field label="listen port (optional, blank = ephemeral)">
          <input
            className="mono"
            style={FIELD_STYLE}
            type="number"
            min={0}
            max={65535}
            value={form.wg_listen_port ?? ""}
            onChange={(e) => set("wg_listen_port", e.target.value === "" ? null : Number(e.target.value))}
            placeholder="blank = ephemeral"
          />
        </Field>
        <Field label="wireguard-go binary path">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={form.wireguard_go_bin}
            onChange={(e) => set("wireguard_go_bin", e.target.value)}
            placeholder={defaultWireguardGoBin ?? "no default resolved - set a path"}
            spellCheck={false}
          />
        </Field>
      </div>

      <SubHeader title="SSH target (ops: reachability, descriptor read, log tail)" />
      <div className="grid gap-2.5" style={{ gridTemplateColumns: "1fr 90px 1fr" }}>
        <Field label="host">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={form.ssh_host}
            onChange={(e) => set("ssh_host", e.target.value)}
            placeholder="203.0.113.9"
            spellCheck={false}
          />
        </Field>
        <Field label="port">
          <input
            className="mono"
            style={FIELD_STYLE}
            type="number"
            min={1}
            max={65535}
            value={form.ssh_port}
            onChange={(e) => set("ssh_port", Number(e.target.value))}
          />
        </Field>
        <Field label="user">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={form.ssh_user}
            onChange={(e) => set("ssh_user", e.target.value)}
            placeholder="root"
            spellCheck={false}
          />
        </Field>
      </div>
      <Field label="identity file (private key path - never generated or uploaded here)">
        <input
          className="mono"
          style={FIELD_STYLE}
          value={form.ssh_identity_file}
          onChange={(e) => set("ssh_identity_file", e.target.value)}
          placeholder="/Users/you/.ssh/hetzner-campaign-1"
          spellCheck={false}
        />
      </Field>
      <Field label="pinned host key (ssh-keyscan output - the ONLY host key this console will trust)">
        <input
          className="mono"
          style={FIELD_STYLE}
          value={form.ssh_pinned_host_key}
          onChange={(e) => set("ssh_pinned_host_key", e.target.value)}
          placeholder="ssh-ed25519 AAAAC3Nza..."
          spellCheck={false}
        />
      </Field>

      <div className="flex items-center gap-3 flex-wrap pt-1">
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onSave()}
          disabled={saving || !canSave}
        >
          {saving ? "Saving..." : "Save environment"}
        </button>
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          {savedAtMs !== null
            ? `saved · ${new Date(savedAtMs).toLocaleTimeString()} - resets any live tunnel/SSH connection below`
            : "resets any live tunnel/SSH connection below - kept in memory only, for this session"}
        </span>
      </div>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel)", color: "var(--sev-high)" }}>
          {describeRemoteError(error)}
        </div>
      )}
    </div>
  );
}
