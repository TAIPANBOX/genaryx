import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { EvidenceArtifactsTable } from "./EvidenceArtifactsTable";
import type { ManifestArtifact } from "../evidenceTypes";

const HEX = "0123456789abcdef".repeat(4);

const art = (o: Partial<ManifestArtifact>): ManifestArtifact => ({
  name: "Agent-BOM",
  filename: "bom.json",
  content_type: "application/json",
  source: "genaryx",
  tool_version: null,
  verify_status: null,
  sha256: `sha256:${HEX}`,
  size_bytes: 2048,
  ...o,
});

const render = (artifacts: ManifestArtifact[]) =>
  renderToStaticMarkup(createElement(EvidenceArtifactsTable, { artifacts }));

describe("the evidence pack's artifact table", () => {
  it("an_empty_pack_says_so_and_draws_no_table_head", () => {
    const out = render([]);
    expect(out).toContain("no artifacts in this pack.");
    expect(out).not.toContain("verify status");
  });

  it("a_row_shows_name_file_source_size_and_a_shortened_hash_with_the_full_one_on_hover", () => {
    const out = render([art({})]);
    expect(out).toContain("Agent-BOM");
    expect(out).toContain("bom.json");
    expect(out).toContain("application/json");
    expect(out).toContain("2.00 KB");
    expect(out).toContain("01234567…89abcd".replace("89abcd", HEX.slice(-6)));
    expect(out).toContain(`title="sha256:${HEX}"`);
  });

  it("a_short_hash_is_shown_whole_with_or_without_the_prefix", () => {
    expect(render([art({ sha256: "sha256:abcd" })])).toContain(">abcd<");
    expect(render([art({ sha256: "ffff" })])).toContain(">ffff<");
  });

  it("an_artifact_with_no_self_verification_gets_a_dash_never_an_unverified_claim", () => {
    const out = render([art({ verify_status: null })]);
    expect(out).toMatch(/color:var\(--faint\)">-</);
    expect(out.toLowerCase()).not.toContain("unverified");
  });

  it.each([
    ["VERIFIED", "var(--sev-low)"],
    ["self-verifying", "var(--sev-low)"],
    ["BROKEN chain", "var(--sev-high)"],
    ["NOT verified", "var(--sev-high)"],
    ["pending", "var(--faint)"],
  ])("the_status_%s_is_coloured_by_its_own_wording", (status, tone) => {
    const out = render([art({ verify_status: status })]);
    expect(out).toContain(`style="color:${tone}"`);
    expect(out).toContain(`title="${status}"`);
  });
});
