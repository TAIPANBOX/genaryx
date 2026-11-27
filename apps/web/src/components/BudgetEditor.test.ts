// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BudgetEditor } from "./BudgetEditor";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;
const onSubmit = vi.fn(async () => {});

const button = (name: string) =>
  Array.from(host.querySelectorAll("button")).find((b) => b.textContent === name) as HTMLButtonElement | undefined;

async function click(el: Element) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

async function type(value: string) {
  const input = host.querySelector("input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function mount(props: { currentUsd: number | null; subject?: "run" | "unit" }) {
  await act(async () => {
    root.render(createElement(BudgetEditor, { runId: "r1", onSubmit, ...props }));
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

describe("the per-row cap editor", () => {
  it("it_starts_as_one_button_and_opens_with_the_current_cap_filled_in", async () => {
    await mount({ currentUsd: 12.5 });
    expect(host.querySelector("input")).toBeNull();
    await click(button("Cap")!);
    expect(host.querySelector("input")!.value).toBe("12.5");
  });

  it("with_no_cap_yet_the_field_opens_empty_and_set_stays_off", async () => {
    await mount({ currentUsd: null });
    await click(button("Cap")!);
    expect(host.querySelector("input")!.value).toBe("");
    expect(button("Set")!.disabled).toBe(true);
  });

  it.each([["-1"], ["abc"], ["  "]])("the_amount_%j_cannot_be_set", async (v) => {
    await mount({ currentUsd: 1 });
    await click(button("Cap")!);
    await type(v);
    expect(button("Set")!.disabled).toBe(true);
  });

  it("zero_is_a_valid_cap_and_set_asks_for_a_reason_before_anything_is_sent", async () => {
    await mount({ currentUsd: 1 });
    await click(button("Cap")!);
    await type("0");
    expect(button("Set")!.disabled).toBe(false);
    await click(button("Set")!);
    expect(document.body.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("a_cancelled_confirm_sends_nothing_and_close_returns_to_the_single_button", async () => {
    await mount({ currentUsd: 1, subject: "unit" });
    await click(button("Cap")!);
    await click(button("Set")!);
    expect(document.body.textContent).toContain("unit r1");
    expect(document.body.textContent).toContain("a month");
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(document.body.querySelector('[role="alertdialog"]')).toBeNull();
    await click(button("Close")!);
    expect(button("Cap")).toBeDefined();
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
