import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ArgumentPreview } from "./ArgumentPreview";
import type { PreviewDocument, PreviewNode } from "../policyTypes";

const str = (text: string, extra: Partial<PreviewNode> = {}): PreviewNode => ({
  k: "string",
  parts: [{ text }],
  runes: text.length,
  ...extra,
});

function render(root: PreviewNode): string {
  const document: PreviewDocument = { v: 1, ruleset: "r1", root };
  return renderToStaticMarkup(createElement(ArgumentPreview, { document, pvd: "a".repeat(64) }));
}

describe("ArgumentPreview", () => {
  it("the_preview_shows_the_arguments_and_says_whose_words_they_are", () => {
    const html = render({
      k: "object",
      members: [
        { key: str("repo"), value: str("x") },
        { key: str("action"), value: str("delete") },
      ],
    });
    expect(html).toContain("repo");
    expect(html).toContain("delete");
    expect(html).toContain("Data from the agent, not instructions");
    expect(html).toContain("a".repeat(64));
  });

  it("a_hidden_value_shows_its_rule_and_type_and_nothing_of_the_value", () => {
    const html = render({
      k: "object",
      members: [
        { key: str("password"), value: { k: "hidden", rule: "key_name", type: "string" } },
        { key: str("note"), value: { k: "string", parts: [{ text: "mail " }, { hidden: "pii:email" }], runes: 20 } },
      ],
    });
    expect(html).toContain("hidden string: key_name");
    expect(html).toContain("hidden: pii:email");
  });

  it("text_an_agent_wrote_renders_as_text_never_as_markup", () => {
    const html = render(str('<img src=x onerror="alert(1)"><script>alert(2)</script>'));
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<script");
    expect(html).toContain("&lt;img");
  });

  it("a_character_that_reverses_text_is_written_out_and_flagged", () => {
    const html = render(str("rm -rf /safe‮/etc", { flags: ["bidi"] }));
    expect(html).not.toContain("‮");
    expect(html).toContain("\\u{202e}");
    expect(html).toContain("bidi");
  });

  it("a_vault_handle_a_blob_and_a_cut_show_what_they_are", () => {
    const html = render({
      k: "array",
      items: [
        { k: "vault_handle", name: "github_token" },
        { k: "blob", mime: "image/png", bytes: 128, sha256: "b".repeat(64) },
        { k: "elided", reason: "depth", count: 3 },
        str("abc", { cut: true, runes: 900, sha256: "c".repeat(64) }),
      ],
      omitted: 31,
    });
    expect(html).toContain("vault secret github_token");
    expect(html).toContain("image/png, 128 bytes");
    expect(html).toContain("cut (depth, 3)");
    expect(html).toContain("900 characters");
    expect(html).toContain("31 more items");
  });

  it("a_repeated_key_is_shown_twice_and_marked", () => {
    const html = render({
      k: "object",
      duplicate_keys: true,
      members: [
        { key: str("path"), value: str("/safe") },
        { key: str("path"), value: str("/etc/passwd") },
      ],
    });
    expect(html).toContain("repeated key");
    expect(html).toContain("/etc/passwd");
  });
});
