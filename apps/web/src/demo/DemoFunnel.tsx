import { useState, type ReactNode } from "react";
import { ConnectStep } from "./ConnectStep";
import { DemoControls } from "./DemoControls";
import { SignInStep } from "./SignInStep";

type FunnelStep = "signin" | "connect" | "console";

/**
 * The Live Demo funnel (it-rat.com "Live demo"): three steps, no dead ends,
 * wrapping the real console (`children`, rendered unchanged) as its last
 * step. Only ever mounted when `MOCK` is true; `App.tsx` is the one call
 * site, and it owns that gate, not this component.
 *
 * State is plain in-memory `useState` on purpose, per the brief: a hard
 * sandbox guarantee is "refresh = clean slate", so nothing here persists
 * across a reload, matching `scenario.ts`'s own module-level state right
 * next to it. "Reset demo" (`DemoControls`) leans on exactly that: a full
 * `location.reload()` is the reset, rather than this component tracking its
 * own teardown.
 */
export function DemoFunnel({ children }: { children: ReactNode }) {
  const [step, setStep] = useState<FunnelStep>("signin");

  if (step === "signin") {
    return <SignInStep onSignIn={() => setStep("connect")} />;
  }

  return (
    <>
      {step === "connect" && <ConnectStep onEnterConsole={() => setStep("console")} />}
      {step === "console" && children}
      <DemoControls />
    </>
  );
}
