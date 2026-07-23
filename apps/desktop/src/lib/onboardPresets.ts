/**
 * Framework starting points for the onboard generate form (I14c,
 * docs/ONBOARD.md "Framework presets"). A prospect onboarding their first
 * agent has to fill in trust domain, path, unit, owner, runtime,
 * attestation, filesystem access, and declared models from a blank form.
 * Clicking a preset pre-fills the framework-shaped fields with sensible,
 * clearly-editable defaults for a known agent framework (LangGraph, CrewAI,
 * AutoGen) - "a catalog of one agent" inside onboarding, without building an
 * actual registry.
 *
 * Purely a client-side convenience: applying a preset only changes what
 * `OnboardView.tsx`'s own `useState` fields hold before Generate is clicked.
 * It calls no backend, adds no command, and changes no validation - the
 * operator still reviews every field and clicks Generate themselves
 * (docs/ONBOARD.md's "propose, never mutate" ethos, one layer earlier: this
 * proposes FORM values, not even a bundle).
 *
 * Deliberately excludes `trust_domain`, `path`, `owner`, and `unit`: those
 * are the operator's OWN identifiers (which domain, which folder, which
 * person, which business unit) - a preset has no legitimate guess for any of
 * them, and silently filling one in would misattribute the agent.
 * {@link OnboardPresetFields} is typed to make that omission structural: the
 * type simply has no room for those four fields, not just a convention this
 * module happens to follow.
 */
import type { FsScope, ModelDecl } from "../onboardTypes";

/** The generate-form fields one preset proposes - each one is exactly an
 * `OnboardGenerateRequest` field (`onboardTypes.ts`), so a preset only ever
 * pre-fills a value the form already knows how to send; it never grows the
 * request shape. `attestation_method` stays `"none"` for every preset below
 * (see {@link PRESETS}) - none of the three frameworks has a clear,
 * framework-specific reason to require attestation, so guessing anything
 * else would be inventing a security posture the framework itself does not
 * impose. */
export interface OnboardPresetFields {
  runtime: string;
  attestation_method: string;
  /** Example provider/model bindings, in the order shown once applied -
   * always exactly the framework's own two-model illustration (see
   * {@link PRESETS}), never fewer or more. */
  models: ModelDecl[];
  /** Example filesystem scopes, in the order shown once applied. Often
   * empty: only a framework with a genuinely conventional workdir (AutoGen's
   * code-executor `work_dir`, LangGraph's checkpoint store) gets an entry -
   * CrewAI has no single such convention, so it proposes none rather than
   * inventing one. */
  filesystem: FsScope[];
}

/** One framework starting point. `id` is also the React key the button row
 * uses; `label` is the button's own text; `hint` is a one-line tooltip
 * naming what the preset actually seeds, so an operator can tell the three
 * apart before clicking. */
export interface OnboardPreset {
  id: string;
  label: string;
  hint: string;
  fields: OnboardPresetFields;
}

/**
 * The three presets, in the order the button row renders them. Each is
 * small and honest on purpose (docs/ONBOARD.md): a runtime label, the
 * default "none" attestation, two example model bindings illustrating that
 * multi-model use is normal for these frameworks, and - only where a
 * framework genuinely has one - an example workdir. Every value here is
 * exactly as editable as if the operator had typed it themselves; nothing
 * about a preset is locked in.
 */
export const PRESETS: readonly OnboardPreset[] = [
  {
    id: "langgraph",
    label: "LangGraph",
    hint: "Graph-orchestrated agent runtime: two example model bindings and a checkpoint directory.",
    fields: {
      runtime: "langgraph",
      attestation_method: "none",
      models: [
        { provider: "anthropic", model: "claude-sonnet-4-5" },
        { provider: "openai", model: "gpt-4o" },
      ],
      filesystem: [{ path: "./langgraph_checkpoints", mode: "write" }],
    },
  },
  {
    id: "crewai",
    label: "CrewAI",
    hint: "Role-based multi-agent crew: two example model bindings, no conventional workdir assumed.",
    fields: {
      runtime: "crewai",
      attestation_method: "none",
      models: [
        { provider: "anthropic", model: "claude-sonnet-4-5" },
        { provider: "openai", model: "gpt-4o-mini" },
      ],
      filesystem: [],
    },
  },
  {
    id: "autogen",
    label: "AutoGen",
    hint: "Conversable multi-agent runtime: two example model bindings and its default code-execution workdir.",
    fields: {
      runtime: "autogen",
      attestation_method: "none",
      models: [
        { provider: "anthropic", model: "claude-sonnet-4-5" },
        { provider: "openai", model: "gpt-4o" },
      ],
      filesystem: [{ path: "./coding", mode: "write" }],
    },
  },
];

/**
 * The field values a preset proposes when its button is clicked - a fresh,
 * defensive copy of `preset.fields` (new top-level object, new `models`/
 * `filesystem` arrays) so nothing downstream can mutate a shared `PRESETS`
 * entry through the value it returns.
 *
 * `OnboardView.tsx`'s click handler takes it from here: `runtime` and
 * `attestation_method` replace the form's current value outright (that is
 * the point of clicking a DIFFERENT preset - "no, use this one instead"),
 * while `models` and `filesystem` are APPENDED to whatever rows the operator
 * already declared, each minted a fresh row id by the view's own counters -
 * id generation stays the view's job for every row mutation in this form,
 * preset-applied or not (mirrors `lib/fsScopes.ts` / `lib/modelDecls.ts`'s
 * own "id generation is the caller's job" rule). This function itself only
 * ever reads `preset` - `trust_domain`/`path`/`owner`/`unit` are not part of
 * {@link OnboardPresetFields} at all, so there is no code path here that
 * could touch them.
 */
export function applyOnboardPreset(preset: OnboardPreset): OnboardPresetFields {
  return {
    runtime: preset.fields.runtime,
    attestation_method: preset.fields.attestation_method,
    models: preset.fields.models.map((m) => ({ ...m })),
    filesystem: preset.fields.filesystem.map((s) => ({ ...s })),
  };
}
