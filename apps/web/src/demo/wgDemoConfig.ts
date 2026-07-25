/**
 * The inert WireGuard config shown in the demo funnel's "Connect this
 * machine" step (`ConnectStep.tsx`), pure theater, never a real peer.
 *
 * Every value here is deliberately non-functional:
 * - the keys are not valid WireGuard keys (they literally spell DEMO
 *   instead of encoding 32 random bytes), so pasting this into a real
 *   WireGuard client fails to connect rather than silently doing something
 *   surprising;
 * - the endpoint (203.0.113.10) is IANA's TEST-NET-3 (RFC 5737), reserved
 *   for documentation and guaranteed to never route anywhere on the public
 *   internet;
 * - the whole string only ever leaves this module as a downloaded `.conf`
 *   file and a QR code the operator explicitly asked for, both local-only
 *   (a Blob URL, an inline SVG), never a network call
 *   (`sandboxGuard.ts` would also block one regardless).
 */

const DEMO_PRIVATE_KEY = "DEMO0000000000000000000000000000000000000=";
const DEMO_PUBLIC_KEY = "DEMO1111111111111111111111111111111111111=";
/** TEST-NET-3 (RFC 5737): reserved for documentation, never a real host. */
const DEMO_ENDPOINT = "203.0.113.10:51820";
/** Same value as {@link DEMO_ENDPOINT}, exported for `ConnectStep.tsx`'s own
 * small "endpoint (demo only)" readout, so that label and the config below
 * can never drift apart. */
export const DEMO_ENDPOINT_LABEL = DEMO_ENDPOINT;

export const DEMO_WG_CONFIG = `[Interface]
# Genaryx Live Demo - inert placeholder, this connects to nothing.
PrivateKey = ${DEMO_PRIVATE_KEY}
Address = 10.0.0.2/32
DNS = 1.1.1.1

[Peer]
# Placeholder peer on TEST-NET-3 (RFC 5737): never a real, routable host.
PublicKey = ${DEMO_PUBLIC_KEY}
Endpoint = ${DEMO_ENDPOINT}
AllowedIPs = 0.0.0.0/0
PersistentKeepalive = 25
`;

/** Trigger a browser download of {@link DEMO_WG_CONFIG} as a local Blob,
 * mirroring `lib/remote.ts`'s `downloadWgOperatorConfig` (Blob + a
 * temporary `<a download>`, revoked right after the click). Reimplemented
 * here rather than imported, since that helper's signature is tied to the
 * real `RemoteWgOperatorConfig` DTO this demo has no backend to produce. */
export function downloadDemoWgConfig(): void {
  const blob = new Blob([DEMO_WG_CONFIG], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  try {
    const a = document.createElement("a");
    a.href = url;
    a.download = "genaryx-demo.conf";
    a.style.display = "none";
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  } finally {
    URL.revokeObjectURL(url);
  }
}
