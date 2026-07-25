import { useState } from "react";
import { QrCode } from "../components/QrCode";
import { cssVar } from "../lib/cssVars";
import { DEMO_ENDPOINT_LABEL, DEMO_WG_CONFIG, downloadDemoWgConfig } from "./wgDemoConfig";

/**
 * Demo funnel step 2, "Connect this machine": theater for the real
 * WireGuard "Connect this machine" flow (`components/RemoteWgOperatorCard.tsx`,
 * opened from `AppHeader`'s session area). Same idea, a QR code plus a
 * downloadable `.conf`, but the config is a fixed, inert placeholder
 * (`wgDemoConfig.ts`) rather than a real peer issued by a box: there is no
 * box behind this build to issue one from.
 *
 * The QR reuses the console's own dependency-free encoder
 * (`components/QrCode.tsx` / `lib/qr.ts`, vendored `qrcodegen`) rather than
 * a second one duplicated under `src/demo/`: it is already in this bundle
 * (Pocket pairing uses it too), so importing it costs nothing extra and
 * produces a genuinely scannable code instead of a look-alike placeholder,
 * which is strictly more convincing "theater" for exactly zero extra
 * dependency weight.
 *
 * No dead end: both the "Enter console" button and the QR code itself
 * advance to step 3, matching how scanning the real QR with a WireGuard
 * client is "the" way through that real flow.
 */
export function ConnectStep({ onEnterConsole }: { onEnterConsole: () => void }) {
  const [downloaded, setDownloaded] = useState(false);

  return (
    <div className="flex min-h-screen items-center justify-center p-6" style={{ background: "var(--ink)" }}>
      <div className="panel flex w-full flex-col gap-4 p-6" style={{ maxWidth: 480, background: "var(--panel-2)" }}>
        <div className="flex flex-col gap-0.5">
          <span className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            Session
          </span>
          <span className="text-[15px] font-semibold" style={{ color: "var(--fg)" }}>
            Connect this machine
          </span>
        </div>

        <p className="text-[12.5px] leading-relaxed" style={{ color: "var(--dim)" }}>
          On a real box this issues your laptop or phone a WireGuard peer, so you can reach the console over the
          tunnel instead of SSH. Here it is simulated: scan the QR, or download the config, to see the shape of it,
          then continue in.
        </p>

        <div className="flex flex-wrap items-start gap-5">
          <button
            type="button"
            onClick={onEnterConsole}
            aria-label="Simulate scanning this QR code and continue to the console"
            className="flex flex-col items-center gap-1.5 rounded-lg border-0 bg-transparent p-0"
            style={{ cursor: "pointer" }}
          >
            <QrCode value={DEMO_WG_CONFIG} size={176} />
            <span className="mono text-[10px]" style={{ color: "var(--faint)" }}>
              tap to simulate a scan
            </span>
          </button>

          <div className="flex min-w-[180px] flex-1 flex-col gap-3">
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 14px", fontSize: 11, alignSelf: "flex-start" }}
              onClick={() => {
                downloadDemoWgConfig();
                setDownloaded(true);
              }}
            >
              Download WireGuard config
            </button>
            {downloaded && (
              <span className="chip" style={{ ...cssVar("dot", "var(--sev-low)"), alignSelf: "flex-start" }}>
                <span className="dot" aria-hidden="true" />
                genaryx-demo.conf saved
              </span>
            )}
            <div className="flex flex-col gap-1">
              <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
                endpoint (demo only)
              </span>
              <code className="mono text-[11px]" style={{ color: "var(--dim)" }}>
                {DEMO_ENDPOINT_LABEL}
              </code>
            </div>
          </div>
        </div>

        <button
          type="button"
          onClick={onEnterConsole}
          className="mt-1 w-full rounded-lg px-3 py-2 text-sm font-semibold"
          style={{ background: "var(--fg)", color: "var(--ink)" }}
        >
          Enter console
        </button>
      </div>
    </div>
  );
}
