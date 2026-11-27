// @vitest-environment jsdom
/**
 * The console's floating windows (`lib/popover.tsx`): every agent card,
 * incident and detail view opens in one. What they promise: a window opens
 * beside the thing clicked (or centred when nothing was), flips to the other
 * side rather than run off the screen, never leaves the viewport, comes to the
 * front when touched, can be dragged, resized (not below a floor),
 * maximised and restored, and closed one at a time or all at once.
 *
 * jsdom lays nothing out, so every element is 0 by 0 in a 1024 by 768 window;
 * the positions below follow from that.
 */

import { act, createElement, useRef, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PopoverHeader, PopoverProvider, useDraggableCard, usePopover, useWindowControls } from "./popover";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class InertObserver {
  observe() {}
  disconnect() {}
}

let host: HTMLDivElement;
let root: Root;
let api: ReturnType<typeof usePopover>;

function Grab() {
  api = usePopover();
  return null;
}

beforeEach(async () => {
  vi.stubGlobal("ResizeObserver", InertObserver);
  Object.assign(window, { innerWidth: 1024, innerHeight: 768 });
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => root.render(createElement(PopoverProvider, null, createElement(Grab))));
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});

const dialogs = () => Array.from(document.querySelectorAll<HTMLDivElement>('[role="dialog"]'));
const anchor = (left: number, right: number, top: number) => ({ left, right, top, bottom: top + 20 }) as DOMRect;

function pointer(target: EventTarget, type: string, x: number, y: number) {
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y }));
}

async function open(node: ReactNode, opts?: Parameters<typeof api.open>[1]) {
  let id = 0;
  await act(async () => {
    id = api.open(node, opts);
  });
  return id;
}

describe("placing a window", () => {
  it("centres a window opened from nowhere", async () => {
    await open(createElement("p", null, "hello"));
    const [d] = dialogs();
    expect(d.textContent).toBe("hello");
    expect(d.style.left).toBe("512px");
    expect(d.style.top).toBe("384px");
    expect(d.style.width).toBe("380px");
  });

  it("opens beside its anchor, at the requested width", async () => {
    await open("card", { anchor: anchor(100, 200, 50), width: 420 });
    const [d] = dialogs();
    expect(d.style.left).toBe("210px");
    expect(d.style.top).toBe("50px");
    expect(d.style.width).toBe("420px");
  });

  it("flips to the anchor's other side rather than run off the right edge", async () => {
    await open("card", { anchor: anchor(900, 1020, 50) });
    expect(dialogs()[0].style.left).toBe("890px");
  });

  it("never leaves the viewport", async () => {
    await open("card", { anchor: anchor(-500, -400, -300) });
    const [d] = dialogs();
    expect(d.style.left).toBe("12px");
    expect(d.style.top).toBe("12px");
  });
});

describe("the window's own controls", () => {
  function Card() {
    return createElement(PopoverHeader, { kicker: "AGENT", title: "agent://acme/bot" });
  }

  it("shows its kicker, title and a drag grip, and closes from its own button", async () => {
    await open(createElement(Card));
    const d = dialogs()[0];
    expect(d.textContent).toContain("AGENT");
    expect(d.textContent).toContain("agent://acme/bot");
    expect(d.textContent).toContain("⋮⋮");
    await act(async () => d.querySelector<HTMLButtonElement>('[aria-label="Close"]')!.click());
    expect(dialogs()).toHaveLength(0);
  });

  it("maximises to the whole screen, hides the resize edges, and restores", async () => {
    await open(createElement(Card));
    const d = () => dialogs()[0];
    const edges = () => Array.from(d().children).filter((c) => (c as HTMLElement).style.cursor.includes("resize"));
    expect(edges()).toHaveLength(3);

    await act(async () => d().querySelector<HTMLButtonElement>('[aria-label="Maximize"]')!.click());
    expect(d().style.left).toBe("12px");
    expect(d().style.width).toBe("calc(100vw - 24px)");
    expect(edges()).toHaveLength(0);

    await act(async () => d().querySelector<HTMLButtonElement>('[aria-label="Restore"]')!.click());
    expect(d().style.width).toBe("380px");
    expect(edges()).toHaveLength(3);
  });

  it("drags by its header, stops on release, and does not drag while maximised", async () => {
    await open(createElement(Card));
    const header = () => dialogs()[0].querySelector<HTMLDivElement>(".flex.items-start")!;
    await act(async () => pointer(header(), "pointerdown", 600, 400));
    await act(async () => pointer(window, "pointermove", 500, 300));
    expect(dialogs()[0].style.left).toBe("412px");
    expect(dialogs()[0].style.top).toBe("284px");
    await act(async () => pointer(window, "pointerup", 500, 300));
    await act(async () => pointer(window, "pointermove", 0, 0));
    expect(dialogs()[0].style.left).toBe("412px");

    await act(async () => dialogs()[0].querySelector<HTMLButtonElement>('[aria-label="Maximize"]')!.click());
    await act(async () => pointer(header(), "pointerdown", 600, 400));
    await act(async () => pointer(window, "pointermove", 0, 0));
    expect(dialogs()[0].style.left).toBe("12px");
  });

  it("keeps a dragged window inside the screen", async () => {
    await open(createElement(Card));
    const header = dialogs()[0].querySelector<HTMLDivElement>(".flex.items-start")!;
    await act(async () => pointer(header, "pointerdown", 0, 0));
    await act(async () => pointer(window, "pointermove", 5000, 5000));
    expect(dialogs()[0].style.left).toBe("1012px");
    expect(dialogs()[0].style.top).toBe("756px");
    await act(async () => pointer(window, "pointerup", 0, 0));
  });

  it("resizes from its edges, never below the floor", async () => {
    await open(createElement(Card));
    const edge = (cursor: string) =>
      Array.from(dialogs()[0].children).find((c) => (c as HTMLElement).style.cursor === cursor) as HTMLElement;

    await act(async () => pointer(edge("ew-resize"), "pointerdown", 100, 100));
    await act(async () => pointer(window, "pointermove", 200, 100));
    expect(dialogs()[0].style.width).toBe("480px");
    await act(async () => pointer(window, "pointermove", -1000, 100));
    expect(dialogs()[0].style.width).toBe("280px");
    await act(async () => pointer(window, "pointerup", 0, 0));

    await act(async () => pointer(edge("ns-resize"), "pointerdown", 100, 100));
    await act(async () => pointer(window, "pointermove", 100, 400));
    expect(dialogs()[0].style.height).toBe("300px");
    await act(async () => pointer(window, "pointermove", 100, -1000));
    expect(dialogs()[0].style.height).toBe("160px");
    await act(async () => pointer(window, "pointerup", 0, 0));

    await act(async () => pointer(edge("nwse-resize"), "pointerdown", 0, 0));
    await act(async () => pointer(window, "pointermove", 50, 50));
    expect(dialogs()[0].style.width).toBe("330px");
    expect(dialogs()[0].style.height).toBe("210px");
    await act(async () => pointer(window, "pointerup", 0, 0));
  });
});

describe("several windows", () => {
  it("brings the touched one to the front", async () => {
    await open("first");
    await open("second");
    const [a, b] = dialogs();
    expect(Number(b.style.zIndex)).toBeGreaterThan(Number(a.style.zIndex));
    await act(async () => pointer(a, "pointerdown", 1, 1));
    const [a2, b2] = dialogs();
    expect(Number(a2.style.zIndex)).toBeGreaterThan(Number(b2.style.zIndex));
  });

  it("closes one by id, or all of them", async () => {
    const first = await open("first");
    await open("second");
    await open("third");
    await act(async () => api.close(first));
    expect(dialogs().map((d) => d.textContent)).toEqual(["second", "third"]);
    await act(async () => api.close());
    expect(dialogs()).toHaveLength(0);
  });
});

describe("outside a window", () => {
  it("usePopover refuses to run without the provider", () => {
    expect(() => renderToStaticMarkup(createElement(Grab))).toThrow(/inside <PopoverProvider>/);
  });

  it("a header outside a window uses its own close and offers no grip or maximise", async () => {
    const onClose = vi.fn();
    let controls: unknown = "unset";
    function Probe() {
      controls = useWindowControls();
      return createElement(PopoverHeader, { title: "inline", onClose });
    }
    const h = document.createElement("div");
    const r = createRoot(h);
    await act(async () => r.render(createElement(Probe)));
    expect(controls).toBeNull();
    expect(h.textContent).not.toContain("⋮");
    expect(h.querySelector('[aria-label="Maximize"]')).toBeNull();
    await act(async () => h.querySelector<HTMLButtonElement>('[aria-label="Close"]')!.click());
    expect(onClose).toHaveBeenCalledTimes(1);
    await act(async () => r.unmount());
  });
});

describe("useDraggableCard", () => {
  let place: (cw: number, ch: number) => { left: number; top: number } | null;
  let state: { pos: { left: number; top: number } | null };

  function Card() {
    const ref = useRef<HTMLDivElement>(null);
    const { pos, dragHandleProps } = useDraggableCard(ref, place);
    state = { pos };
    return createElement("div", { ref, onPointerDown: dragHandleProps.onPointerDown }, "card");
  }

  async function mount() {
    const h = document.createElement("div");
    document.body.appendChild(h);
    const r = createRoot(h);
    await act(async () => r.render(createElement(Card)));
    return { h, r };
  }

  it("places the card where asked, clamped to the screen", async () => {
    place = () => ({ left: -40, top: 5000 });
    const { h, r } = await mount();
    expect(state.pos).toEqual({ left: 12, top: 756 });
    await act(async () => r.unmount());
    h.remove();
  });

  it("leaves the card unplaced when there is nowhere to put it, and then does not drag", async () => {
    place = () => null;
    const { h, r } = await mount();
    expect(state.pos).toBeNull();
    await act(async () => pointer(h.firstChild!, "pointerdown", 0, 0));
    await act(async () => pointer(window, "pointermove", 50, 50));
    expect(state.pos).toBeNull();
    await act(async () => r.unmount());
    h.remove();
  });

  it("drags from where it was placed, and keeps the dragged spot when the window resizes", async () => {
    place = () => ({ left: 100, top: 100 });
    const { h, r } = await mount();
    await act(async () => pointer(h.firstChild!, "pointerdown", 10, 10));
    await act(async () => pointer(window, "pointermove", 60, 30));
    expect(state.pos).toEqual({ left: 150, top: 120 });
    await act(async () => pointer(window, "pointerup", 60, 30));

    place = () => ({ left: 300, top: 300 });
    await act(async () => r.render(createElement(Card)));
    await act(async () => window.dispatchEvent(new Event("resize")));
    expect(state.pos).toEqual({ left: 150, top: 120 });
    await act(async () => r.unmount());
    h.remove();
  });

  it("re-places an undragged card when the window resizes", async () => {
    place = () => ({ left: 100, top: 100 });
    const { h, r } = await mount();
    place = () => ({ left: 200, top: 220 });
    await act(async () => r.render(createElement(Card)));
    await act(async () => window.dispatchEvent(new Event("resize")));
    expect(state.pos).toEqual({ left: 200, top: 220 });
    await act(async () => r.unmount());
    h.remove();
  });
});
