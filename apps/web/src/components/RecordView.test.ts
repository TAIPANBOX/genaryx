// @vitest-environment jsdom
/**
 * Invariant 54: the Record tab shows what Trailryx sealed and whether the
 * current root carries a Keyryx signature, and its one control runs a
 * verification. It never shows "nothing sealed" for a store it could not read,
 * and never calls a check it could not make a verdict.
 */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RecordPanel } from "../recordTypes";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const box = vi.hoisted(() => ({
  answer: null as unknown,
  verifyAnswer: null as unknown,
  refusal: null as unknown,
  calls: [] as string[],
}));

vi.mock("../lib/transport", () => ({
  hasBackend: () => true,
  isWebShell: () => true,
  webApiBase: () => "",
  subscribeBackend: () => () => {},
  requiredRoleFromCommandError: () => null,
  invokeBackend: async (command: string) => {
    box.calls.push(command);
    if (box.refusal) throw box.refusal;
    if (command === "record_verify") return box.verifyAnswer;
    return box.answer;
  },
}));

import { RecordView } from "./RecordView";
import { NAV_SECTIONS } from "../lib/views";

const ROOT = "fb1dda22152e30ba3dc5ef2c01684b580fce0708a0c13cdf26193c2b0f16cfb7410f85010b284aaa0fbec98617a402b2";

function sealed(over: Partial<Extract<RecordPanel["seal"], { state: "sealed" }>> = {}): RecordPanel["seal"] {
  return {
    state: "sealed",
    path: "/var/lib/stack/trailryx/data",
    segments: 412,
    records: 98213,
    last: { segment: 412, records: 240, first_recorded_at: 1764000000, last_recorded_at: 1764000420 },
    root: ROOT,
    signature: { state: "signed", key: "3f9a1c0b7e2d4a65", algorithm: "es384", at: 1764000480 },
    other_shards: 0,
    ...over,
  } as RecordPanel["seal"];
}

function panel(over: Partial<RecordPanel> = {}): RecordPanel {
  return { seal: sealed(), verify: { state: "not_run" }, verify_stale: false, ...over };
}

let host: HTMLDivElement | undefined;
let root: Root | undefined;

async function flush() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

async function render() {
  const h = document.createElement("div");
  document.body.appendChild(h);
  host = h;
  const r = createRoot(h);
  root = r;
  await act(async () => {
    r.render(createElement(RecordView));
  });
  await flush();
}

function byTestId(id: string): HTMLElement | null {
  return host?.querySelector(`[data-testid="${id}"]`) ?? null;
}

beforeEach(() => {
  box.answer = null;
  box.verifyAnswer = null;
  box.refusal = null;
  box.calls = [];
});

afterEach(() => {
  const r = root;
  if (r) act(() => r.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
});

describe("the Record tab", () => {
  it("the_record_tab_is_in_the_assure_section", () => {
    const assure = NAV_SECTIONS.find((s) => s.label === "Assure");
    expect(assure?.items.map((i) => i.id)).toContain("record");
  });

  it("the_last_seal_shows_segments_records_and_the_root", async () => {
    box.answer = panel();
    await render();
    expect(box.calls).toEqual(["record_status"]);
    expect(byTestId("record-sealed")?.textContent).toContain("412 segment(s), 98213 record(s)");
    expect(byTestId("record-sealed")?.textContent).toContain("last segment 412, 240 record(s)");
    expect(byTestId("record-root")?.textContent).toBe(ROOT);
  });

  it("a_signature_on_file_says_so_with_its_key_and_that_verify_proves_it", async () => {
    box.answer = panel();
    await render();
    const sig = byTestId("record-signature")?.textContent ?? "";
    expect(sig).toContain("Keyryx signature on file: es384, key 3f9a1c0b7e2d4a65");
    expect(sig).toContain("Verify proves it.");
  });

  it("a_root_waiting_for_its_signature_says_so", async () => {
    box.answer = panel({ seal: sealed({ signature: { state: "queued", since: 1764000480 } }) });
    await render();
    expect(byTestId("record-signature")?.textContent).toContain("Waiting for a Keyryx signature since");
  });

  it("a_data_dir_not_named_says_so_and_names_the_setting", async () => {
    box.answer = { seal: { state: "not_configured", variable: "GENARYX_TRAILRYX_DATA_DIR" }, verify: { state: "not_run" }, verify_stale: false } satisfies RecordPanel;
    await render();
    expect(byTestId("record-not-configured")?.textContent).toContain("GENARYX_TRAILRYX_DATA_DIR");
    expect(byTestId("record-nothing-sealed")).toBeNull();
    expect(byTestId("record-verify-button")).toBeNull();
  });

  it("an_unreadable_store_is_not_shown_as_nothing_sealed", async () => {
    box.answer = {
      seal: { state: "unreadable", path: "/var/lib/stack/trailryx/data/s0-000007.mf", reason: "not a regular file" },
      verify: { state: "not_run" },
      verify_stale: false,
    } satisfies RecordPanel;
    await render();
    expect(byTestId("record-unreadable")?.textContent).toMatch(/s0-000007\.mf.*not a regular file/);
    expect(byTestId("record-nothing-sealed")).toBeNull();
  });

  it("not_run_says_not_verified_since_this_console_started", async () => {
    box.answer = panel();
    await render();
    expect(byTestId("record-verify-state")?.textContent).toBe("Not verified since this console started.");
  });

  it("verify_now_calls_record_verify_and_shows_the_verdict", async () => {
    box.answer = panel();
    box.verifyAnswer = panel({
      verify: {
        state: "verified",
        root: ROOT,
        at: 1764000600,
        millis: 2140,
        findings: ['[note] root-signature: "es384" by key 3f9a1c0b7e2d4a65'],
      },
    });
    await render();
    const button = byTestId("record-verify-button") as HTMLButtonElement;
    expect(button.textContent).toBe("Verify now");
    await act(async () => {
      button.click();
    });
    await flush();
    expect(box.calls).toContain("record_verify");
    expect(byTestId("record-verify-state")?.textContent).toContain("Verified");
    expect(byTestId("record-findings")?.textContent).toContain("root-signature");
  });

  it("a_check_that_could_not_be_made_says_it_is_not_a_verdict", async () => {
    box.answer = panel({ verify: { state: "not_checked", at: 1764000600, reason: "trailryx-verify is not on this box" } });
    await render();
    const text = byTestId("record-verify-state")?.textContent ?? "";
    expect(text).toContain("Could not be checked at");
    expect(text).toContain("trailryx-verify is not on this box");
    expect(text).toContain("This is not a verdict.");
    expect(text).not.toContain("NOT verified");
  });

  it("a_verdict_for_an_earlier_root_says_so", async () => {
    box.answer = panel({
      verify: { state: "verified", root: "00", at: 1764000600, millis: 900, findings: [] },
      verify_stale: true,
    });
    await render();
    expect(byTestId("record-verify-stale")?.textContent).toContain("That result was for an earlier root");
  });

  it("the_only_control_is_verify", async () => {
    box.answer = panel();
    await render();
    const controls = host?.querySelectorAll("button, input, select, textarea, form") ?? [];
    expect(controls.length).toBe(1);
    expect(controls[0]?.getAttribute("data-testid")).toBe("record-verify-button");
  });

  it("wire_text_renders_as_text_with_hidden_characters_written_out", async () => {
    box.answer = {
      seal: { state: "unreadable", path: "<img src=x onerror=alert(1)>\u202e/store", reason: "bad" },
      verify: { state: "not_verified", root: ROOT, at: 1764000600, millis: 10, findings: ["<b>bad</b>\u0000"] },
      verify_stale: false,
    } satisfies RecordPanel;
    await render();
    expect(host?.querySelector("img")).toBeNull();
    expect(host?.querySelector("b")).toBeNull();
    expect(byTestId("record-findings")?.textContent).toContain("<b>bad</b>");
    expect(byTestId("record-findings")?.textContent).toContain("\\u{0}");
    expect(byTestId("record-unreadable")?.textContent).toContain("\\u{202e}");
  });
});
