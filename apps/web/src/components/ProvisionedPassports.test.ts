/**
 * What the "Provisioned passports" table SHOWS.
 *
 * A `.ts` test for a `.tsx` component: `createElement` +
 * `renderToStaticMarkup` need no JSX here and no DOM at all, so this runs in
 * the repo's existing node-environment vitest config with no new dependency
 * and no config change. See `MemoryProvenance.test.ts`'s header for the same
 * reasoning at more length.
 *
 * The gap this file was written for: `onboard_status` reads every passport
 * off disk and deserializes each declared `filesystem` scope and `models`
 * entry, then surfaces only the LENGTH of each array. The parse already ran;
 * the console just threw the result away. Nothing here invents a value: every
 * assertion is about a field the backend already had in hand.
 */
import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ProvisionedPassports } from "./ProvisionedPassports";
import type { OnboardStatus, Provisioned } from "../onboardTypes";

function statusWith(passports: Provisioned[], skipped: OnboardStatus["skipped"] = []): OnboardStatus {
  return {
    map_path: "/home/op/.taipan/identity.json",
    map_loaded: true,
    map_error: null,
    units: [],
    passports_dir: "/home/op/.taipan/passports",
    passports,
    skipped,
  };
}

function render(status: OnboardStatus): string {
  return renderToStaticMarkup(
    createElement(ProvisionedPassports, { status, onVerify: () => {} }),
  );
}

/** Visible text with every tag stripped and runs of whitespace collapsed -
 * what an operator would read off the row, minus the layout. */
function text(html: string): string {
  return html.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();
}

const RECON: Provisioned = {
  agent_id: "agent://bank.example/treasury/recon-batch",
  owner: "olena",
  file: "/home/op/.taipan/passports/treasury-recon-batch.json",
  filesystem_count: 2,
  filesystem: [
    { path: "/data/reports", mode: "read" },
    { path: "/data/out", mode: "write" },
  ],
  models_count: 2,
  models: [
    { provider: "anthropic", model: "claude-sonnet-4-5", endpoint: "api.anthropic.com" },
    { provider: "openai", model: null, endpoint: null },
  ],
  in_map: true,
};

const BARE: Provisioned = {
  agent_id: "agent://bank.example/fraud/triage",
  owner: "petro",
  file: "/home/op/.taipan/passports/fraud-triage.json",
  filesystem_count: 0,
  filesystem: [],
  models_count: 0,
  models: [],
  in_map: false,
};

describe("the table itself", () => {
  it("names the directory it found nothing in", () => {
    const html = render(statusWith([]));
    expect(text(html)).toContain("no provisioned passports found in /home/op/.taipan/passports");
  });

  it("shows a row's id, owner, file and map binding", () => {
    const html = render(statusWith([RECON]));
    const shown = text(html);
    expect(shown).toContain("agent://bank.example/treasury/recon-batch");
    expect(shown).toContain("olena");
    expect(shown).toContain("/home/op/.taipan/passports/treasury-recon-batch.json");
    expect(shown).toContain("in map");
  });

  it("keeps the quiet count columns", () => {
    const shown = text(render(statusWith([RECON])));
    expect(shown).toContain("2 folders");
    expect(shown).toContain("2 models");
  });

  it("lists a file it could not parse, with the reason", () => {
    const html = render(
      statusWith([], [{ file: "/home/op/.taipan/passports/broken.json", reason: "could not parse: expected value at line 1 column 1" }]),
    );
    const shown = text(html);
    expect(shown).toContain("1 file skipped");
    expect(shown).toContain("broken.json");
    expect(shown).toContain("expected value at line 1 column 1");
  });
});

describe("the declarations the passport already carries", () => {
  it("names each declared model, not just how many there are", () => {
    const shown = text(render(statusWith([RECON])));
    expect(shown).toContain("anthropic");
    expect(shown).toContain("claude-sonnet-4-5");
    expect(shown).toContain("api.anthropic.com");
    expect(shown).toContain("openai");
  });

  it("names each declared folder with the mode it was declared in", () => {
    const shown = text(render(statusWith([RECON])));
    expect(shown).toContain("/data/reports");
    expect(shown).toContain("read");
    expect(shown).toContain("/data/out");
    expect(shown).toContain("write");
  });

  it("says a model entry declared no model or endpoint, rather than showing a blank", () => {
    // The second entry is provider-only, which the passport schema allows
    // (SPEC.md section 4.5). An empty cell would read as a rendering bug; the
    // honest statement is that the passport does not declare it.
    const only = { ...RECON, models_count: 1, models: [{ provider: "openai", model: null, endpoint: null }] };
    const shown = text(render(statusWith([only])));
    expect(shown).toContain("openai");
    expect(shown).toContain("model not declared");
    expect(shown).toContain("endpoint not declared");
  });

  it("offers nothing to open when a passport declares neither", () => {
    const shown = text(render(statusWith([BARE])));
    expect(shown).not.toContain("declared");
    expect(shown).not.toContain("not reported");
  });

  it("separates a source that reported no declarations from one that reported none declared", () => {
    // The mock preview and any genaryx-api older than this change answer with
    // the counts alone. That is NOT the same statement as "this passport
    // declares nothing", and the table must not let the two read alike.
    const countsOnly = {
      agent_id: RECON.agent_id,
      owner: RECON.owner,
      file: RECON.file,
      filesystem_count: 2,
      models_count: 1,
      in_map: true,
    } as Provisioned;
    const shown = text(render(statusWith([countsOnly])));
    expect(shown).toContain("2 folders");
    expect(shown).toContain("1 model");
    expect(shown).toContain("this source reported the count only, not the declarations");
    expect(shown).not.toContain("not declared");
  });
});
