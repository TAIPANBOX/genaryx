import { describe, expect, it } from "vitest";
import {
  addModelDeclRow,
  blankModelDeclRow,
  duplicateModelKeys,
  hasEmptyProvider,
  isEmptyProvider,
  modelDeclKey,
  modelDeclsAreValid,
  removeModelDeclRow,
  setModelDeclEndpoint,
  setModelDeclModel,
  setModelDeclProvider,
  toModelDecls,
} from "./modelDecls";
import type { ModelDeclRow } from "./modelDecls";

// ---------------------------------------------------------------------------
// Test fixture - minimal, valid instance, overridable.
// ---------------------------------------------------------------------------

function row(overrides: Partial<ModelDeclRow> & { id: string }): ModelDeclRow {
  return {
    id: overrides.id,
    provider: overrides.provider ?? "anthropic",
    model: overrides.model ?? "",
    endpoint: overrides.endpoint ?? "",
  };
}

// ---------------------------------------------------------------------------
// blankModelDeclRow / addModelDeclRow / removeModelDeclRow
// ---------------------------------------------------------------------------

describe("blankModelDeclRow", () => {
  it("is empty in every field and carries the given id", () => {
    expect(blankModelDeclRow("m-1")).toEqual({ id: "m-1", provider: "", model: "", endpoint: "" });
  });
});

describe("addModelDeclRow", () => {
  it("appends one blank row after the existing rows, unchanged", () => {
    const rows = [row({ id: "a" })];
    const next = addModelDeclRow(rows, "b");
    expect(next).toEqual([row({ id: "a" }), blankModelDeclRow("b")]);
    // The input array itself is not mutated - a pure function.
    expect(rows).toEqual([row({ id: "a" })]);
  });

  it("appends to an empty list - zero rows is the common starting case", () => {
    expect(addModelDeclRow([], "first")).toEqual([blankModelDeclRow("first")]);
  });
});

describe("removeModelDeclRow", () => {
  it("drops only the row with the matching id, preserving order", () => {
    const rows = [row({ id: "a" }), row({ id: "b", provider: "openai" }), row({ id: "c" })];
    expect(removeModelDeclRow(rows, "b")).toEqual([row({ id: "a" }), row({ id: "c" })]);
  });

  it("is a no-op when the id is not present", () => {
    const rows = [row({ id: "a" })];
    expect(removeModelDeclRow(rows, "nope")).toEqual(rows);
  });
});

// ---------------------------------------------------------------------------
// setModelDeclProvider / setModelDeclModel / setModelDeclEndpoint
// ---------------------------------------------------------------------------

describe("setModelDeclProvider", () => {
  it("replaces only the matching row's provider, leaving model/endpoint and other rows alone", () => {
    const rows = [row({ id: "a", model: "claude-sonnet-4-5" }), row({ id: "b" })];
    const next = setModelDeclProvider(rows, "a", "bedrock");
    expect(next).toEqual([
      row({ id: "a", provider: "bedrock", model: "claude-sonnet-4-5" }),
      row({ id: "b" }),
    ]);
  });
});

describe("setModelDeclModel", () => {
  it("replaces only the matching row's model, leaving provider/endpoint and other rows alone", () => {
    const rows = [row({ id: "a" }), row({ id: "b" })];
    const next = setModelDeclModel(rows, "b", "claude-opus-4-1");
    expect(next).toEqual([row({ id: "a" }), row({ id: "b", model: "claude-opus-4-1" })]);
  });
});

describe("setModelDeclEndpoint", () => {
  it("replaces only the matching row's endpoint, leaving provider/model and other rows alone", () => {
    const rows = [row({ id: "a" }), row({ id: "b" })];
    const next = setModelDeclEndpoint(rows, "a", "api.anthropic.com");
    expect(next).toEqual([row({ id: "a", endpoint: "api.anthropic.com" }), row({ id: "b" })]);
  });
});

// ---------------------------------------------------------------------------
// isEmptyProvider / hasEmptyProvider
// ---------------------------------------------------------------------------

describe("isEmptyProvider / hasEmptyProvider", () => {
  it("treats a blank or whitespace-only provider as empty", () => {
    expect(isEmptyProvider(row({ id: "a", provider: "" }))).toBe(true);
    expect(isEmptyProvider(row({ id: "a", provider: "   " }))).toBe(true);
    expect(isEmptyProvider(row({ id: "a", provider: "openai" }))).toBe(false);
  });

  it("hasEmptyProvider is true when any row is empty, false when every row has a provider", () => {
    expect(hasEmptyProvider([row({ id: "a" }), row({ id: "b", provider: "" })])).toBe(true);
    expect(hasEmptyProvider([row({ id: "a" }), row({ id: "b", provider: "openai" })])).toBe(false);
    expect(hasEmptyProvider([])).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// modelDeclKey
// ---------------------------------------------------------------------------

describe("modelDeclKey", () => {
  it("trims every field before keying", () => {
    const a = row({ id: "a", provider: "anthropic", model: "claude-sonnet-4-5", endpoint: "api.anthropic.com" });
    const b = row({ id: "b", provider: " anthropic ", model: " claude-sonnet-4-5 ", endpoint: " api.anthropic.com " });
    expect(modelDeclKey(a)).toBe(modelDeclKey(b));
  });

  it("treats a blank model/endpoint the same as an omitted one", () => {
    const a = row({ id: "a", provider: "openai", model: "", endpoint: "" });
    const b = row({ id: "b", provider: "openai", model: "   ", endpoint: "   " });
    expect(modelDeclKey(a)).toBe(modelDeclKey(b));
  });

  it("distinguishes an absent model from the same provider with a set model", () => {
    const bare = row({ id: "a", provider: "anthropic" });
    const pinned = row({ id: "b", provider: "anthropic", model: "claude-sonnet-4-5" });
    expect(modelDeclKey(bare)).not.toBe(modelDeclKey(pinned));
  });
});

// ---------------------------------------------------------------------------
// duplicateModelKeys / modelDeclsAreValid
// ---------------------------------------------------------------------------

describe("duplicateModelKeys", () => {
  it("finds a (provider, model, endpoint) triple declared on more than one row", () => {
    const rows = [
      row({ id: "a", provider: "anthropic", model: "claude-sonnet-4-5" }),
      row({ id: "b", provider: " anthropic ", model: " claude-sonnet-4-5 " }),
      row({ id: "c", provider: "openai" }),
    ];
    expect(duplicateModelKeys(rows)).toEqual(new Set([modelDeclKey(rows[0])]));
  });

  it("does not flag two blank-provider rows as duplicates of each other", () => {
    const rows = [row({ id: "a", provider: "" }), row({ id: "b", provider: "  " })];
    expect(duplicateModelKeys(rows).size).toBe(0);
  });

  it("does not flag the same provider with a different model as a duplicate", () => {
    const rows = [
      row({ id: "a", provider: "anthropic", model: "claude-sonnet-4-5" }),
      row({ id: "b", provider: "anthropic", model: "claude-opus-4-1" }),
    ];
    expect(duplicateModelKeys(rows).size).toBe(0);
  });

  it("is empty when every triple is unique", () => {
    const rows = [row({ id: "a", provider: "anthropic" }), row({ id: "b", provider: "openai" })];
    expect(duplicateModelKeys(rows).size).toBe(0);
  });
});

describe("modelDeclsAreValid", () => {
  it("is true for zero rows - the common, no-models case", () => {
    expect(modelDeclsAreValid([])).toBe(true);
  });

  it("is true when every row has a non-empty provider and a distinct triple", () => {
    const rows = [row({ id: "a", provider: "anthropic" }), row({ id: "b", provider: "openai" })];
    expect(modelDeclsAreValid(rows)).toBe(true);
  });

  it("is false when any row has an empty provider", () => {
    const rows = [row({ id: "a", provider: "anthropic" }), row({ id: "b", provider: "   " })];
    expect(modelDeclsAreValid(rows)).toBe(false);
  });

  it("is false when two rows share the same trimmed (provider, model, endpoint) triple", () => {
    const rows = [
      row({ id: "a", provider: "anthropic", model: "claude-sonnet-4-5" }),
      row({ id: "b", provider: "anthropic", model: "claude-sonnet-4-5" }),
    ];
    expect(modelDeclsAreValid(rows)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// toModelDecls
// ---------------------------------------------------------------------------

describe("toModelDecls", () => {
  it("maps rows to the wire shape, trimmed, dropping the UI-only id, in order", () => {
    const rows = [
      row({ id: "a", provider: "  anthropic  ", model: "  claude-sonnet-4-5  ", endpoint: "  api.anthropic.com  " }),
      row({ id: "b", provider: "openai", model: "", endpoint: "" }),
    ];
    expect(toModelDecls(rows)).toEqual([
      { provider: "anthropic", model: "claude-sonnet-4-5", endpoint: "api.anthropic.com" },
      { provider: "openai" },
    ]);
  });

  it("omits model/endpoint entirely rather than sending an empty string", () => {
    const [decl] = toModelDecls([row({ id: "a", provider: "openai", model: "  ", endpoint: "  " })]);
    expect("model" in decl).toBe(false);
    expect("endpoint" in decl).toBe(false);
  });

  it("maps an empty row list to an empty array", () => {
    expect(toModelDecls([])).toEqual([]);
  });
});
