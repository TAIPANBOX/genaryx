import { describe, expect, it } from "vitest";
import { applyOnboardPreset, PRESETS } from "./onboardPresets";
import type { OnboardPreset, OnboardPresetFields } from "./onboardPresets";
import { ATTESTATION_METHODS } from "../onboardTypes";

// ---------------------------------------------------------------------------
// Shared fixtures / helpers.
// ---------------------------------------------------------------------------

const ALLOWED_FIELD_KEYS = ["runtime", "attestation_method", "models", "filesystem"].sort();

/** The exact operator-identity fields a preset must never carry, on either
 * `OnboardPreset` itself or its `fields` - see `onboardPresets.ts`'s own doc
 * comment for why these four are excluded. */
const OPERATOR_IDENTITY_KEYS = ["trust_domain", "path", "owner", "unit"];

function findPreset(id: string): OnboardPreset {
  const preset = PRESETS.find((p) => p.id === id);
  if (!preset) throw new Error(`no fixture preset with id ${id}`);
  return preset;
}

// ---------------------------------------------------------------------------
// PRESETS - shape
// ---------------------------------------------------------------------------

describe("PRESETS", () => {
  it("has exactly the three known frameworks, each with a distinct id", () => {
    expect(PRESETS.map((p) => p.id)).toEqual(["langgraph", "crewai", "autogen"]);
    expect(new Set(PRESETS.map((p) => p.id)).size).toBe(PRESETS.length);
  });

  it("gives every preset a non-blank label and hint", () => {
    for (const preset of PRESETS) {
      expect(preset.label.trim().length).toBeGreaterThan(0);
      expect(preset.hint.trim().length).toBeGreaterThan(0);
    }
  });

  it("gives every preset a non-blank runtime that matches its own id", () => {
    for (const preset of PRESETS) {
      expect(preset.fields.runtime).toBe(preset.id);
    }
  });

  it("only ever proposes an attestation method from the form's own closed list", () => {
    for (const preset of PRESETS) {
      expect(ATTESTATION_METHODS).toContain(preset.fields.attestation_method);
    }
  });

  it('defaults every preset to "none" attestation - no framework here has a clear reason to require one', () => {
    for (const preset of PRESETS) {
      expect(preset.fields.attestation_method).toBe("none");
    }
  });

  it("gives every preset exactly two example model bindings, each with a non-blank provider", () => {
    for (const preset of PRESETS) {
      expect(preset.fields.models).toHaveLength(2);
      for (const m of preset.fields.models) {
        expect(m.provider.trim().length).toBeGreaterThan(0);
      }
    }
  });

  it("never declares the same (provider, model, endpoint) triple twice within one preset", () => {
    for (const preset of PRESETS) {
      const keys = preset.fields.models.map((m) => JSON.stringify([m.provider, m.model ?? null, m.endpoint ?? null]));
      expect(new Set(keys).size).toBe(keys.length);
    }
  });

  it("keeps every declared filesystem path non-blank with a legal read/write mode", () => {
    for (const preset of PRESETS) {
      for (const scope of preset.fields.filesystem) {
        expect(scope.path.trim().length).toBeGreaterThan(0);
        expect(["read", "write"]).toContain(scope.mode);
      }
    }
  });

  it("never declares the same filesystem path twice within one preset", () => {
    for (const preset of PRESETS) {
      const paths = preset.fields.filesystem.map((s) => s.path);
      expect(new Set(paths).size).toBe(paths.length);
    }
  });

  it("only proposes a filesystem scope where the framework has a genuinely conventional workdir", () => {
    // LangGraph (checkpoint store) and AutoGen (code-executor work_dir) each
    // get one; CrewAI has no single such convention and proposes none -
    // asserted by name so a future edit that quietly adds/removes one is a
    // visible, deliberate diff here, not a silent behavior change.
    expect(findPreset("langgraph").fields.filesystem).toHaveLength(1);
    expect(findPreset("crewai").fields.filesystem).toHaveLength(0);
    expect(findPreset("autogen").fields.filesystem).toHaveLength(1);
  });

  it("never lets a preset itself carry one of the operator's own identity fields", () => {
    for (const preset of PRESETS) {
      const presetKeys = Object.keys(preset);
      const fieldKeys = Object.keys(preset.fields);
      for (const forbidden of OPERATOR_IDENTITY_KEYS) {
        expect(presetKeys).not.toContain(forbidden);
        expect(fieldKeys).not.toContain(forbidden);
      }
    }
  });

  it("gives every preset's `fields` exactly the four allowed keys, nothing more", () => {
    for (const preset of PRESETS) {
      expect(Object.keys(preset.fields).sort()).toEqual(ALLOWED_FIELD_KEYS);
    }
  });
});

// ---------------------------------------------------------------------------
// applyOnboardPreset - behavior
// ---------------------------------------------------------------------------

describe("applyOnboardPreset", () => {
  it("returns exactly the langgraph preset's own field values", () => {
    const expected: OnboardPresetFields = {
      runtime: "langgraph",
      attestation_method: "none",
      models: [
        { provider: "anthropic", model: "claude-sonnet-4-5" },
        { provider: "openai", model: "gpt-4o" },
      ],
      filesystem: [{ path: "./langgraph_checkpoints", mode: "write" }],
    };
    expect(applyOnboardPreset(findPreset("langgraph"))).toEqual(expected);
  });

  it("returns exactly the crewai preset's own field values, with an empty filesystem list", () => {
    const applied = applyOnboardPreset(findPreset("crewai"));
    expect(applied.runtime).toBe("crewai");
    expect(applied.attestation_method).toBe("none");
    expect(applied.models).toEqual([
      { provider: "anthropic", model: "claude-sonnet-4-5" },
      { provider: "openai", model: "gpt-4o-mini" },
    ]);
    expect(applied.filesystem).toEqual([]);
  });

  it("returns exactly the autogen preset's own field values", () => {
    const applied = applyOnboardPreset(findPreset("autogen"));
    expect(applied.runtime).toBe("autogen");
    expect(applied.attestation_method).toBe("none");
    expect(applied.models).toEqual([
      { provider: "anthropic", model: "claude-sonnet-4-5" },
      { provider: "openai", model: "gpt-4o" },
    ]);
    expect(applied.filesystem).toEqual([{ path: "./coding", mode: "write" }]);
  });

  it("returns a defensive copy - mutating the result never mutates the shared PRESETS entry", () => {
    const preset = findPreset("langgraph");
    const beforeModelsLen = preset.fields.models.length;
    const beforeFsLen = preset.fields.filesystem.length;

    const applied = applyOnboardPreset(preset);
    applied.models.push({ provider: "mutated" });
    applied.filesystem.push({ path: "/mutated", mode: "write" });
    applied.models[0]!.provider = "mutated-in-place";

    expect(preset.fields.models).toHaveLength(beforeModelsLen);
    expect(preset.fields.filesystem).toHaveLength(beforeFsLen);
    expect(preset.fields.models[0]!.provider).toBe("anthropic");
  });

  it("never returns one of the operator's own identity fields, for any preset", () => {
    for (const preset of PRESETS) {
      const applied = applyOnboardPreset(preset);
      const appliedKeys = Object.keys(applied);
      expect(appliedKeys.sort()).toEqual(ALLOWED_FIELD_KEYS);
      for (const forbidden of OPERATOR_IDENTITY_KEYS) {
        expect(appliedKeys).not.toContain(forbidden);
      }
    }
  });
});
