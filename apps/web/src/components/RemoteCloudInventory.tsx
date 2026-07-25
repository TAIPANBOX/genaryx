import { useCallback, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeRemoteError, listCloudServers, listHetznerServers } from "../lib/remote";
import { useSession } from "../lib/useSession";
import type { CloudListOptions, CloudServer, HetznerServer, RemoteError } from "../remoteTypes";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
} as const;

const COLUMNS = "70px 1fr 90px 130px 90px 60px 70px 110px";
const CLOUD_COLUMNS = "1fr 130px 90px 130px 110px 100px";

type ProviderId =
  | "hetzner"
  | "aws"
  | "azure"
  | "gcp"
  | "ibmcloud"
  | "oracle"
  | "digitalocean"
  | "vultr"
  | "linode"
  | "scaleway"
  | "ionos"
  | "upcloud"
  | "exoscale"
  | "hostinger"
  | "fasthosts"
  | "ovh"
  | "contabo"
  | "kamatera"
  | "other";

interface ProviderMeta {
  id: ProviderId;
  label: string;
  /** true = this console ships a live read-only inventory connector for it. */
  builtIn: boolean;
  /** for providers listed with their own official CLI (no built-in connector yet). */
  cli?: { tool: string; auth: string; command: string; addressField: string };
  /** for providers with no standard public CLI: where to read the address in their own panel. */
  panel?: { where: string; addressField: string };
  /** the built-in read-only inventory connector id, when one exists for this
   * provider (AWS/GCP/Azure/IBM Cloud): the console lists servers via the
   * operator's own official CLI, exactly like Hetzner's token flow. */
  connector?: "aws" | "gcp" | "azure" | "ibmcloud";
}

/**
 * Nothing here is hardcoded to one host. The provider list is only a
 * convenience for reading an address; the actual connection (the environment
 * form below) is identical for every target and needs no provider at all.
 */
const PROVIDERS: ProviderMeta[] = [
  { id: "hetzner", label: "Hetzner Cloud", builtIn: true },
  {
    id: "aws",
    label: "AWS EC2",
    builtIn: false,
    connector: "aws",
    cli: {
      tool: "AWS CLI",
      auth: "aws configure",
      command:
        "aws ec2 describe-instances --query \"Reservations[].Instances[].{id:InstanceId,ip:PublicIpAddress,type:InstanceType,state:State.Name}\" --output table",
      addressField: "public IP",
    },
  },
  {
    id: "azure",
    label: "Microsoft Azure",
    builtIn: false,
    connector: "azure",
    cli: {
      tool: "Azure CLI",
      auth: "az login",
      command:
        "az vm list -d --query \"[].{name:name,ip:publicIps,size:hardwareProfile.vmSize,state:powerState}\" --output table",
      addressField: "public IP",
    },
  },
  {
    id: "gcp",
    label: "Google Cloud",
    builtIn: false,
    connector: "gcp",
    cli: {
      tool: "gcloud CLI",
      auth: "gcloud auth login",
      command:
        "gcloud compute instances list --format=\"table(name,networkInterfaces[0].accessConfigs[0].natIP,machineType.basename(),status)\"",
      addressField: "external IP",
    },
  },
  {
    id: "ibmcloud",
    label: "IBM Cloud",
    builtIn: false,
    connector: "ibmcloud",
    cli: {
      tool: "IBM Cloud CLI",
      auth: "ibmcloud login (or ibmcloud login --sso for a federated org)",
      command: "ibmcloud is instances --output json",
      addressField: "public IP",
    },
  },
  {
    id: "oracle",
    label: "Oracle Cloud (OCI)",
    builtIn: false,
    cli: {
      tool: "OCI CLI",
      auth: "oci setup config",
      command: "oci compute instance list --compartment-id COMPARTMENT_OCID --output table",
      addressField: "public IP",
    },
  },
  {
    id: "digitalocean",
    label: "DigitalOcean",
    builtIn: false,
    cli: {
      tool: "doctl",
      auth: "doctl auth init",
      command: "doctl compute droplet list --format ID,Name,PublicIPv4,Region,Status",
      addressField: "public IP",
    },
  },
  {
    id: "vultr",
    label: "Vultr",
    builtIn: false,
    cli: {
      tool: "vultr-cli",
      auth: "export VULTR_API_KEY=your-api-key",
      command: "vultr-cli instance list",
      addressField: "public IP",
    },
  },
  {
    id: "linode",
    label: "Linode / Akamai",
    builtIn: false,
    cli: {
      tool: "linode-cli",
      auth: "linode-cli configure",
      command: "linode-cli linodes list",
      addressField: "public IP",
    },
  },
  {
    id: "scaleway",
    label: "Scaleway",
    builtIn: false,
    cli: {
      tool: "scw",
      auth: "scw init",
      command: "scw instance server list",
      addressField: "public IP",
    },
  },
  {
    id: "ionos",
    label: "IONOS Cloud",
    builtIn: false,
    cli: {
      tool: "ionosctl",
      auth: "ionosctl login",
      command: "ionosctl server list --datacenter-id DATACENTER_ID",
      addressField: "public IP",
    },
  },
  {
    id: "upcloud",
    label: "UpCloud",
    builtIn: false,
    cli: {
      tool: "upctl",
      auth: "export UPCLOUD_USERNAME=you UPCLOUD_PASSWORD=secret",
      command: "upctl server list",
      addressField: "public IP",
    },
  },
  {
    id: "exoscale",
    label: "Exoscale",
    builtIn: false,
    cli: {
      tool: "exo",
      auth: "exo config",
      command: "exo compute instance list",
      addressField: "public IP",
    },
  },
  {
    id: "hostinger",
    label: "Hostinger",
    builtIn: false,
    panel: {
      addressField: "public IP",
      where: "In hPanel: VPS > your server > the Overview shows its public IP (SSH details are on the same page).",
    },
  },
  {
    id: "fasthosts",
    label: "Fasthosts",
    builtIn: false,
    panel: {
      addressField: "public IP",
      where: "In the Fasthosts CloudNX panel: your server > Network > the public IP is listed there.",
    },
  },
  {
    id: "ovh",
    label: "OVHcloud",
    builtIn: false,
    panel: {
      addressField: "public IP",
      where: "In the OVH Manager: Public Cloud or Bare Metal > your server > the dashboard shows its public IP.",
    },
  },
  {
    id: "contabo",
    label: "Contabo",
    builtIn: false,
    panel: {
      addressField: "public IP",
      where: "In the Contabo customer panel: Your Services > your VPS > the instance card shows its public IP.",
    },
  },
  {
    id: "kamatera",
    label: "Kamatera",
    builtIn: false,
    panel: {
      addressField: "public IP",
      where: "In the Kamatera console: your server > Server Details > the public IP is listed there.",
    },
  },
  { id: "other", label: "Other / on-prem / bare metal", builtIn: false },
];

function formatPrice(eur: number | null): string {
  if (eur === null) return "n/a";
  return `€${eur.toFixed(4)}/hr`;
}

/**
 * Provider-agnostic "grab an address" helper. The console reaches a
 * client-hosted stack over WireGuard + SSH, which does not care who hosts the
 * box - so no provider is baked in. Pick a provider only to look up an IP:
 *
 *  - Hetzner Cloud has a built-in, STRICTLY READ-ONLY inventory connector
 *    (`crates/connectors/src/hetzner.rs` - no create/resize/delete exists at
 *    all); paste a read-scoped token and list.
 *  - Providers with an official CLI (AWS, Azure, GCP, Oracle, DigitalOcean,
 *    Vultr, Linode, Scaleway, IONOS, UpCloud, Exoscale) show the authenticate
 *    + list commands; the console never holds their credentials or calls them.
 *  - Providers with no standard CLI (Hostinger, Fasthosts, OVHcloud, Contabo,
 *    Kamatera) point you at where the address lives in their own panel.
 *  - Other / on-prem needs no lookup step - type the box's address straight
 *    into the environment form below.
 *
 * Either way the connection itself is identical for every target.
 */
export function RemoteCloudInventory() {
  const [providerId, setProviderId] = useState<ProviderId>("hetzner");
  const provider = PROVIDERS.find((p) => p.id === providerId) ?? PROVIDERS[0];

  return (
    <div className="panel px-4 py-3 flex flex-col gap-3" style={{ background: "var(--panel-2)" }}>
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
          provider
        </span>
        <select
          className="mono"
          style={{ ...FIELD_STYLE, minWidth: 220 }}
          value={providerId}
          onChange={(e) => setProviderId(e.target.value as ProviderId)}
        >
          {PROVIDERS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
              {p.builtIn ? "  (built-in inventory)" : p.connector ? "  (live listing)" : ""}
            </option>
          ))}
        </select>
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          picking a provider only reads an address - it never changes how the connection works
        </span>
      </div>

      {provider.id === "hetzner" ? (
        <HetznerInventory />
      ) : provider.connector ? (
        <LiveCliInventory provider={provider} />
      ) : provider.cli ? (
        <CliInventory provider={provider} />
      ) : provider.panel ? (
        <PanelInventory provider={provider} />
      ) : (
        <ManualInventory />
      )}
    </div>
  );
}

/**
 * The one built-in connector: a read-scoped Hetzner API token + optional
 * label selector, "List servers", and the resulting inventory table. STRICTLY
 * READ-ONLY - no create/delete affordance exists anywhere, mirroring the
 * connector's own "no mutation method exists at all" guarantee. The token
 * lives only in local state - never persisted, never sent anywhere but this
 * one IPC call.
 */
function HetznerInventory() {
  const session = useSession();
  const [token, setToken] = useState("");
  const [labelSelector, setLabelSelector] = useState("");
  const [servers, setServers] = useState<HetznerServer[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<RemoteError | null>(null);
  const [listedAtMs, setListedAtMs] = useState<number | null>(null);

  const onList = useCallback(async () => {
    if (token.trim().length === 0 || loading) return;
    setLoading(true);
    setError(null);
    try {
      const rows = await listHetznerServers(token, labelSelector);
      setServers(rows);
      setListedAtMs(Date.now());
    } catch (err) {
      setError(err as RemoteError);
    } finally {
      setLoading(false);
    }
  }, [token, labelSelector, loading]);

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
          read-scoped API token
        </span>
        <input
          className="mono flex-1"
          style={{ ...FIELD_STYLE, minWidth: 160 }}
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="paste a read-scoped Hetzner Cloud API token"
          spellCheck={false}
          autoComplete="off"
        />
        <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
          label selector
        </span>
        <input
          className="mono"
          style={{ ...FIELD_STYLE, width: 200 }}
          value={labelSelector}
          onChange={(e) => setLabelSelector(e.target.value)}
          placeholder="managed-by=taipan"
          spellCheck={false}
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onList()}
          disabled={loading || token.trim().length === 0}
        >
          {loading ? "Listing..." : "List servers"}
        </button>
      </div>
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        read-only inventory - this console never creates, resizes, or deletes a server; the token is used for this
        one request only and is never saved.
      </span>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel)", color: "var(--sev-high)" }}>
          {describeRemoteError(error, session?.role)}
        </div>
      )}

      {servers !== null && (
        <div className="flex items-center gap-2">
          <span className="chip" style={cssVar("dot", "var(--faint)")}>
            <span className="dot" aria-hidden="true" />
            {listedAtMs !== null ? `as of last list · ${new Date(listedAtMs).toLocaleTimeString()}` : "no list yet"}
          </span>
        </div>
      )}

      {servers === null ? (
        <div className="px-1 py-4 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          list servers to see your boxes, then copy an address into the environment below.
        </div>
      ) : servers.length === 0 ? (
        <div className="px-1 py-4 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no boxes found for this token/selector.
        </div>
      ) : (
        <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
          <div
            className="grid gap-3 px-4 py-2"
            style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-3)" }}
          >
            {["id", "name", "status", "ipv4", "type", "cores", "ram", "price/hr"].map((label) => (
              <span
                key={label}
                className="mono"
                style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
              >
                {label}
              </span>
            ))}
          </div>
          {servers.map((s) => (
            <div key={s.id} className="grid items-center gap-3 px-4 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
              <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
                {s.id}
              </span>
              <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={s.name}>
                {s.name}
              </span>
              <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.status}
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.ipv4 ?? "no public ip"}
              </span>
              <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.server_type}
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.cores}
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {s.memory_gb}G
              </span>
              <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                {formatPrice(s.price_hourly_eur)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * A provider with a built-in, STRICTLY READ-ONLY inventory connector
 * (AWS / GCP / Azure): "List servers" runs the operator's own official CLI
 * (`aws`/`gcloud`/`az`) through the connector and shows the result inline,
 * exactly like Hetzner's token flow. The console never stores the provider's
 * credentials and only ever runs the describe/list command. An optional scope
 * (region / project / subscription) narrows the query; the exact command is
 * shown below as the honest "what this runs" and a manual fallback if the CLI
 * is not installed or authenticated on this machine.
 */
function LiveCliInventory({ provider }: { provider: ProviderMeta }) {
  const cli = provider.cli!;
  const connector = provider.connector!;
  const scopeLabel =
    connector === "aws"
      ? "region (optional)"
      : connector === "gcp"
        ? "project (optional)"
        : connector === "azure"
          ? "subscription (optional)"
          : "resource group (optional)";
  const scopePlaceholder =
    connector === "aws"
      ? "eu-central-1"
      : connector === "gcp"
        ? "my-project"
        : connector === "azure"
          ? "my-subscription"
          : "Default";
  const session = useSession();
  const [scope, setScope] = useState("");
  const [servers, setServers] = useState<CloudServer[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [listedAtMs, setListedAtMs] = useState<number | null>(null);

  const onList = useCallback(async () => {
    if (loading) return;
    setLoading(true);
    setError(null);
    try {
      const opts: CloudListOptions = {};
      const s = scope.trim();
      if (s) {
        if (connector === "aws") opts.region = s;
        else if (connector === "gcp") opts.project = s;
        else if (connector === "azure") opts.subscription = s;
        else opts.resource_group = s;
      }
      const rows = await listCloudServers(connector, opts);
      setServers(rows);
      setListedAtMs(Date.now());
    } catch (err) {
      setError(describeRemoteError(err as RemoteError, session?.role));
    } finally {
      setLoading(false);
    }
  }, [loading, scope, connector, session?.role]);

  return (
    <div className="flex flex-col gap-2.5">
      <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
        {provider.label} lists live through the {cli.tool} on this machine, like Hetzner. Authenticate the CLI once,
        then "List servers" and copy a box's {cli.addressField} into the environment below.
      </span>
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
          {scopeLabel}
        </span>
        <input
          className="mono"
          style={{ ...FIELD_STYLE, minWidth: 180 }}
          value={scope}
          onChange={(e) => setScope(e.target.value)}
          placeholder={scopePlaceholder}
          spellCheck={false}
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onList()}
          disabled={loading}
        >
          {loading ? "Listing..." : "List servers"}
        </button>
      </div>
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        read-only - this runs your CLI's describe/list only; the console never creates, resizes, or deletes a server,
        and never stores your {cli.tool} credentials.
      </span>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel)", color: "var(--sev-high)" }}>
          {error}
        </div>
      )}

      {servers !== null && (
        <div className="flex items-center gap-2">
          <span className="chip" style={cssVar("dot", "var(--faint)")}>
            <span className="dot" aria-hidden="true" />
            {listedAtMs !== null ? `as of last list · ${new Date(listedAtMs).toLocaleTimeString()}` : "no list yet"}
          </span>
        </div>
      )}

      {servers !== null &&
        (servers.length === 0 ? (
          <div className="px-1 py-3 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            no instances found for this account/scope.
          </div>
        ) : (
          <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
            <div
              className="grid gap-3 px-4 py-2"
              style={{ gridTemplateColumns: CLOUD_COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-3)" }}
            >
              {["name", "id", "status", "public ip", "type", "region"].map((label) => (
                <span
                  key={label}
                  className="mono"
                  style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
                >
                  {label}
                </span>
              ))}
            </div>
            {servers.map((s) => (
              <div key={s.id} className="grid items-center gap-3 px-4 py-2 bus-row" style={{ gridTemplateColumns: CLOUD_COLUMNS }}>
                <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={s.name}>
                  {s.name || "-"}
                </span>
                <span className="mono truncate text-[11px]" style={{ color: "var(--faint)" }} title={s.id}>
                  {s.id}
                </span>
                <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
                  {s.status}
                </span>
                <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                  {s.public_ip ?? "no public ip"}
                </span>
                <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
                  {s.server_type}
                </span>
                <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
                  {s.region}
                </span>
              </div>
            ))}
          </div>
        ))}

      <details>
        <summary className="text-[11px]" style={{ color: "var(--faint)", cursor: "pointer" }}>
          what "List servers" runs (or run it yourself and paste a box's {cli.addressField})
        </summary>
        <code
          className="mono"
          style={{ display: "block", marginTop: 6, background: "var(--panel)", border: "1px solid var(--line-2)", borderRadius: 8, padding: "7px 10px", fontSize: 11.5, color: "var(--dim)", whiteSpace: "pre-wrap", wordBreak: "break-word", lineHeight: 1.5 }}
        >
          {cli.command}
        </code>
      </details>
    </div>
  );
}

/**
 * A provider listed with its own official CLI (AWS, Azure, GCP, Oracle,
 * DigitalOcean, Vultr, Linode, Scaleway, IONOS, UpCloud, Exoscale). Honest by
 * design: the console holds none of these providers' credentials and never
 * calls their APIs. It shows the authenticate + list commands, then you paste
 * the box's address into the environment form below, exactly as for any other
 * target.
 */
function CliInventory({ provider }: { provider: ProviderMeta }) {
  const cli = provider.cli!;
  const [copied, setCopied] = useState(false);

  const copy = useCallback(() => {
    void navigator.clipboard?.writeText(cli.command).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => setCopied(false),
    );
  }, [cli.command]);

  return (
    <div className="flex flex-col gap-2.5">
      <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
        {provider.label} has no built-in inventory in this console yet. List your boxes with the {cli.tool}, then copy
        a box's {cli.addressField} into the environment below - the connection is identical for every provider.
      </span>

      <div>
        <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 4 }}>
          1 · authenticate ({cli.tool})
        </div>
        <code
          className="mono"
          style={{ display: "block", background: "var(--panel)", border: "1px solid var(--line-2)", borderRadius: 8, padding: "7px 10px", fontSize: 11.5, color: "var(--dim)" }}
        >
          {cli.auth}
        </code>
      </div>

      <div>
        <div className="flex items-center justify-between" style={{ paddingBottom: 4 }}>
          <span className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            2 · list your servers
          </span>
          <button
            type="button"
            className="mono"
            style={{ background: "none", border: "none", cursor: "pointer", fontSize: 10.5, color: copied ? "var(--mint)" : "var(--accent)" }}
            onClick={copy}
          >
            {copied ? "copied" : "copy"}
          </button>
        </div>
        <code
          className="mono"
          style={{ display: "block", background: "var(--panel)", border: "1px solid var(--line-2)", borderRadius: 8, padding: "7px 10px", fontSize: 11.5, color: "var(--fg)", whiteSpace: "pre-wrap", wordBreak: "break-word", lineHeight: 1.5 }}
        >
          {cli.command}
        </code>
      </div>

      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        3 · copy the box's {cli.addressField} (from the listing, or its detail page) into "peer endpoint" and "SSH
        host" in the environment form below. Genaryx never stores your {cli.tool} credentials or calls{" "}
        {provider.label} - it only connects to the address you paste.
      </span>
    </div>
  );
}

/**
 * A provider with no standard public CLI (Hostinger, Fasthosts, OVHcloud,
 * Contabo, Kamatera). Honest: the console never touches the provider at all -
 * it points you at where the address lives in that provider's own control
 * panel, then you paste it into the environment form below.
 */
function PanelInventory({ provider }: { provider: ProviderMeta }) {
  const panel = provider.panel!;
  return (
    <div className="flex flex-col gap-2 py-1">
      <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
        {provider.label} has no built-in inventory or standard CLI in this console. Read your box's {panel.addressField}{" "}
        from the {provider.label} control panel, then paste it into the environment below - the connection is identical
        for every provider.
      </span>
      <div
        className="text-[11.5px]"
        style={{ background: "var(--panel)", border: "1px solid var(--line-2)", borderRadius: 8, padding: "8px 11px", color: "var(--fg)", lineHeight: 1.5 }}
      >
        {panel.where}
      </div>
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        Genaryx never connects to {provider.label}; it reaches the box you enter over WireGuard + SSH, whoever hosts it.
      </span>
    </div>
  );
}

/**
 * On-prem / bare metal / anything else reachable over SSH: there is no lookup
 * step at all. You already know the address; type it into the environment
 * form below.
 */
function ManualInventory() {
  return (
    <div className="flex flex-col gap-1.5 py-1">
      <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
        No inventory step needed. Any host you can reach over SSH works - a cloud VM, a bare-metal server, or an
        on-prem box behind your own network.
      </span>
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        Enter the box's address and keys in the environment form below. The console reaches it over the same
        WireGuard + SSH transport, whoever hosts it.
      </span>
    </div>
  );
}
