import { beforeEach, describe, expect, it, vi } from "vitest";

// `recentEvents.ts` reaches the backend through these two; mocking the module
// (hoisted above every import here) is what lets a backend FAILURE be driven
// without a backend, which is the whole subject of this suite.
vi.mock("./transport", () => ({
  hasBackend: vi.fn(),
  invokeBackend: vi.fn(),
}));

import { hasBackend, invokeBackend } from "./transport";
import { fetchRecentEvents } from "./recentEvents";
import { MOCK_EVENTS } from "../mockData";

const mockHasBackend = vi.mocked(hasBackend);
const mockInvoke = vi.mocked(invokeBackend);

beforeEach(() => {
  vi.resetAllMocks();
  vi.spyOn(console, "error").mockImplementation(() => {});
});

describe("fetchRecentEvents", () => {
  it("returns what the backend answered, labelled as the backend", async () => {
    mockHasBackend.mockReturnValue(true);
    const real = [{ id: 1, source: "tokenfuse" }];
    mockInvoke.mockResolvedValue(real as never);

    const res = await fetchRecentEvents(10);

    expect(res.source).toBe("backend");
    expect(res.events).toEqual(real);
  });

  // CLAUDE.md invariant 4: the console shows the operator's real records,
  // never a mock, and inventing a plausible number to fill a panel is named
  // there as the single worst thing this product can do. A backend that
  // throws is precisely when a panel is empty and a fixture is tempting.
  it("never answers a backend failure with fixture data", async () => {
    mockHasBackend.mockReturnValue(true);
    mockInvoke.mockRejectedValue(new Error("no answer from the box"));

    const res = await fetchRecentEvents(10);

    expect(res.events).toEqual([]);
    expect(res.source).toBe("error");
    // The strongest form of the assertion: not one fixture row got through.
    for (const fixture of MOCK_EVENTS) {
      expect(res.events).not.toContainEqual(fixture);
    }
  });

  it("carries the reason the read failed, so a panel can say what happened", async () => {
    mockHasBackend.mockReturnValue(true);
    mockInvoke.mockRejectedValue(new Error("no answer from the box"));

    const res = await fetchRecentEvents(10);

    expect(res.error).toContain("no answer from the box");
  });

  // The one case fixtures are honest: there is no backend to be wrong about.
  // A plain `vite build` preview has to render something, and it is labelled
  // as mock everywhere it surfaces.
  it("still shows fixtures when there is no backend at all", async () => {
    mockHasBackend.mockReturnValue(false);

    const res = await fetchRecentEvents(3);

    expect(res.source).toBe("mock");
    expect(res.events).toEqual(MOCK_EVENTS.slice(0, 3));
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("does not treat an empty backend answer as a failure", async () => {
    mockHasBackend.mockReturnValue(true);
    mockInvoke.mockResolvedValue([] as never);

    const res = await fetchRecentEvents(10);

    // An empty real bus is information (see `bus/feed.rs`'s own header), and
    // it must not be papered over with fixtures either.
    expect(res.source).toBe("backend");
    expect(res.events).toEqual([]);
  });
});
