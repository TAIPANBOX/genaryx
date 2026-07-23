import { describe, expect, it } from "vitest";
import {
  addFsScopeRow,
  blankFsScopeRow,
  duplicatePaths,
  fsScopesAreValid,
  hasEmptyPath,
  isEmptyPath,
  removeFsScopeRow,
  setFsScopeMode,
  setFsScopePath,
  toFsScopes,
} from "./fsScopes";
import type { FsScopeRow } from "./fsScopes";

// ---------------------------------------------------------------------------
// Test fixture - minimal, valid instance, overridable.
// ---------------------------------------------------------------------------

function row(overrides: Partial<FsScopeRow> & { id: string }): FsScopeRow {
  return {
    id: overrides.id,
    path: overrides.path ?? "/data/reports",
    mode: overrides.mode ?? "read",
  };
}

// ---------------------------------------------------------------------------
// blankFsScopeRow / addFsScopeRow / removeFsScopeRow
// ---------------------------------------------------------------------------

describe("blankFsScopeRow", () => {
  it("is empty-path, mode read, and carries the given id", () => {
    expect(blankFsScopeRow("fs-1")).toEqual({ id: "fs-1", path: "", mode: "read" });
  });
});

describe("addFsScopeRow", () => {
  it("appends one blank row after the existing rows, unchanged", () => {
    const rows = [row({ id: "a" })];
    const next = addFsScopeRow(rows, "b");
    expect(next).toEqual([row({ id: "a" }), blankFsScopeRow("b")]);
    // The input array itself is not mutated - a pure function.
    expect(rows).toEqual([row({ id: "a" })]);
  });

  it("appends to an empty list - zero rows is the common starting case", () => {
    expect(addFsScopeRow([], "first")).toEqual([blankFsScopeRow("first")]);
  });
});

describe("removeFsScopeRow", () => {
  it("drops only the row with the matching id, preserving order", () => {
    const rows = [row({ id: "a" }), row({ id: "b", path: "/data/out" }), row({ id: "c" })];
    expect(removeFsScopeRow(rows, "b")).toEqual([row({ id: "a" }), row({ id: "c" })]);
  });

  it("is a no-op when the id is not present", () => {
    const rows = [row({ id: "a" })];
    expect(removeFsScopeRow(rows, "nope")).toEqual(rows);
  });
});

// ---------------------------------------------------------------------------
// setFsScopePath / setFsScopeMode
// ---------------------------------------------------------------------------

describe("setFsScopePath", () => {
  it("replaces only the matching row's path, leaving its mode and other rows alone", () => {
    const rows = [row({ id: "a", mode: "write" }), row({ id: "b" })];
    const next = setFsScopePath(rows, "a", "/data/new");
    expect(next).toEqual([row({ id: "a", path: "/data/new", mode: "write" }), row({ id: "b" })]);
  });
});

describe("setFsScopeMode", () => {
  it("replaces only the matching row's mode, leaving its path and other rows alone", () => {
    const rows = [row({ id: "a" }), row({ id: "b" })];
    const next = setFsScopeMode(rows, "b", "write");
    expect(next).toEqual([row({ id: "a" }), row({ id: "b", mode: "write" })]);
  });
});

// ---------------------------------------------------------------------------
// isEmptyPath / hasEmptyPath
// ---------------------------------------------------------------------------

describe("isEmptyPath / hasEmptyPath", () => {
  it("treats a blank or whitespace-only path as empty", () => {
    expect(isEmptyPath(row({ id: "a", path: "" }))).toBe(true);
    expect(isEmptyPath(row({ id: "a", path: "   " }))).toBe(true);
    expect(isEmptyPath(row({ id: "a", path: "/data/x" }))).toBe(false);
  });

  it("hasEmptyPath is true when any row is empty, false when every row has a path", () => {
    expect(hasEmptyPath([row({ id: "a" }), row({ id: "b", path: "" })])).toBe(true);
    expect(hasEmptyPath([row({ id: "a" }), row({ id: "b", path: "/data/y" })])).toBe(false);
    expect(hasEmptyPath([])).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// duplicatePaths / fsScopesAreValid
// ---------------------------------------------------------------------------

describe("duplicatePaths", () => {
  it("finds a path declared on more than one row, after trimming", () => {
    const rows = [
      row({ id: "a", path: "/data/reports" }),
      row({ id: "b", path: " /data/reports " }),
      row({ id: "c", path: "/data/out" }),
    ];
    expect(duplicatePaths(rows)).toEqual(new Set(["/data/reports"]));
  });

  it("does not flag two blank rows as duplicates of each other", () => {
    const rows = [row({ id: "a", path: "" }), row({ id: "b", path: "  " })];
    expect(duplicatePaths(rows).size).toBe(0);
  });

  it("is empty when every path is unique", () => {
    const rows = [row({ id: "a", path: "/data/a" }), row({ id: "b", path: "/data/b" })];
    expect(duplicatePaths(rows).size).toBe(0);
  });
});

describe("fsScopesAreValid", () => {
  it("is true for zero rows - the common, no-scopes case", () => {
    expect(fsScopesAreValid([])).toBe(true);
  });

  it("is true when every row has a distinct, non-empty path", () => {
    const rows = [row({ id: "a", path: "/data/a" }), row({ id: "b", path: "/data/b", mode: "write" })];
    expect(fsScopesAreValid(rows)).toBe(true);
  });

  it("is false when any row has an empty path", () => {
    const rows = [row({ id: "a", path: "/data/a" }), row({ id: "b", path: "   " })];
    expect(fsScopesAreValid(rows)).toBe(false);
  });

  it("is false when two rows share the same trimmed path", () => {
    const rows = [row({ id: "a", path: "/data/a" }), row({ id: "b", path: "/data/a", mode: "write" })];
    expect(fsScopesAreValid(rows)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// toFsScopes
// ---------------------------------------------------------------------------

describe("toFsScopes", () => {
  it("maps rows to the wire shape, trimmed, dropping the UI-only id, in order", () => {
    const rows = [row({ id: "a", path: "  /data/reports  ", mode: "read" }), row({ id: "b", path: "/data/out", mode: "write" })];
    expect(toFsScopes(rows)).toEqual([
      { path: "/data/reports", mode: "read" },
      { path: "/data/out", mode: "write" },
    ]);
  });

  it("maps an empty row list to an empty array", () => {
    expect(toFsScopes([])).toEqual([]);
  });
});
