// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { EvidenceReport } from "../cryptoTypes";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("../lib/crypto", async (orig) => ({
  ...(await orig<typeof import("../lib/crypto")>()),
  scanEvidence: vi.fn(),
  verifyEvidence: vi.fn(),
}));

import { scanEvidence, verifyEvidence } from "../lib/crypto";
import { CryptoEvidence } from "./CryptoEvidence";

const scan = vi.mocked(scanEvidence);
const verify = vi.mocked(verifyEvidence);

const report = (o: Partial<EvidenceReport> = {}): EvidenceReport => ({
  tool: "qryx",
  version: "0.9.2",
  standard: "CNSA 2.0",
  generatedAt: "2026-11-27T10:00:00Z",
  root: "/srv/app",
  summary: { compliant: 98, nonCompliant: 29, issues: 12, total: 127, scorePct: 77, bySeverity: { high: 8 } },
  assets: [],
  digest: "sha256:abc",
  signature: null,
  ...o,
});

let root: Root;
let host: HTMLDivElement;

async function flush() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

function button(name: string): HTMLButtonElement {
  const b = Array.from(host.querySelectorAll("button")).find((x) => x.textContent === name);
  if (!b) throw new Error(`no button ${name}`);
  return b;
}

async function click(el: Element) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await flush();
}

async function typeInto(input: HTMLInputElement, value: string) {
  await act(async () => {
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    set.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

beforeEach(async () => {
  vi.resetAllMocks();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root.render(createElement(CryptoEvidence, { defaultPath: "/srv/app", environment: "prod" }));
  });
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

describe("building an evidence bundle", () => {
  it("a_build_shows_the_score_with_its_denominator_the_digest_and_that_the_bundle_is_unsigned", async () => {
    scan.mockResolvedValue(report());
    await click(button("Build evidence"));
    expect(scan).toHaveBeenCalledWith("/srv/app");
    const text = host.textContent ?? "";
    expect(text).toContain("77%");
    expect(text).toContain("of 127 graded");
    expect(text).toContain("digest sha256:abc");
    expect(text).toContain("signature none");
    expect(text).toContain("as of build");
  });

  it("a_signed_bundle_names_the_algorithm_and_the_public_key", async () => {
    scan.mockResolvedValue(report({ signature: { alg: "ml-dsa-65", value: "v", publicKey: "PK123" } }));
    await click(button("Build evidence"));
    expect(host.textContent).toContain("signature ml-dsa-65 · public key PK123");
    expect(host.textContent).not.toContain("signature none");
  });

  it("a_failed_build_prints_the_refusal_and_shows_no_report", async () => {
    scan.mockRejectedValue({ kind: "no_environment" });
    await click(button("Build evidence"));
    expect(host.textContent).toContain("No qryx binary found");
    expect(host.textContent).not.toContain("as of build");
    expect(button("Build evidence").disabled).toBe(false);
  });

  it("an_empty_path_builds_nothing", async () => {
    await typeInto(host.querySelector("input")!, "   ");
    await click(button("Build evidence"));
    expect(scan).not.toHaveBeenCalled();
  });

  it("a_bundle_with_graded_assets_lists_them_and_a_missing_cell_stays_blank", async () => {
    scan.mockResolvedValue(
      report({
        assets: [
          { algorithm: "RSA-2048", type: "key", status: "non-compliant", deadline: "2030", occurrences: 3, action: "migrate", locations: ["a.go", "b.go"] },
          { algorithm: "AES-256" },
        ],
      }),
    );
    await click(button("Build evidence"));
    const text = host.textContent ?? "";
    expect(text).toContain("RSA-2048");
    expect(text).toContain("migrate");
    expect(text).toContain("AES-256");
    expect(host.querySelector('[title="migrate | a.go, b.go"]')).not.toBeNull();
  });
});

describe("verifying a saved evidence file", () => {
  async function verifyWith(path: string) {
    const inputs = host.querySelectorAll("input");
    await typeInto(inputs[1] as HTMLInputElement, path);
    await click(button("Verify"));
  }

  it("a_verified_file_says_verified_with_the_message", async () => {
    verify.mockResolvedValue({ verified: true, message: "digest matches" });
    await verifyWith("/tmp/e.json");
    expect(verify).toHaveBeenCalledWith("/tmp/e.json");
    expect(host.textContent).toContain("VERIFIED - digest matches");
    expect(host.textContent).not.toContain("NOT VERIFIED");
  });

  it("a_file_that_does_not_verify_says_not_verified_not_an_error", async () => {
    verify.mockResolvedValue({ verified: false, message: "digest differs" });
    await verifyWith("/tmp/e.json");
    expect(host.textContent).toContain("NOT VERIFIED - digest differs");
  });

  it("a_failed_check_prints_the_error_and_clears_an_earlier_outcome", async () => {
    verify.mockResolvedValueOnce({ verified: true, message: "ok" });
    await verifyWith("/tmp/e.json");
    verify.mockRejectedValueOnce({ kind: "qryx", message: "unreadable file" });
    await click(button("Verify"));
    expect(host.textContent).toContain("unreadable file");
    expect(host.textContent).not.toContain("VERIFIED - ok");
  });

  it("an_empty_path_verifies_nothing", async () => {
    await click(button("Verify"));
    expect(verify).not.toHaveBeenCalled();
  });
});
