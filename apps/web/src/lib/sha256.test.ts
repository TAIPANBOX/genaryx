import { describe, expect, it } from "vitest";

import { sha256Hex } from "./sha256";

/** FIPS 180-4 and RFC 6234 published vectors, not values computed by this
 * file: the demo's case digests are only worth printing if they are real. */
describe("sha256Hex", () => {
  it("sha256_matches_the_published_vectors", () => {
    expect(sha256Hex("")).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    expect(sha256Hex("abc")).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    expect(sha256Hex("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")).toBe(
      "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    );
  });

  it("sha256_agrees_with_the_rust_digest_of_the_same_bytes", () => {
    // The Rust case test holds this same digest for these bytes
    // (`a_digest_is_the_sha256_of_the_row_and_changes_with_it`).
    expect(sha256Hex('{"a":1}')).toBe("015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862");
  });

  it("sha256_pads_across_the_block_boundary_and_reads_utf8", () => {
    // 55, 56 and 64 bytes straddle the padding boundary; the one-million-a
    // vector is the long-message check.
    expect(sha256Hex("a".repeat(55))).toBe("9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318");
    expect(sha256Hex("a".repeat(56))).toBe("b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
    expect(sha256Hex("a".repeat(64))).toBe("ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb");
    expect(sha256Hex("a".repeat(1_000_000))).toBe(
      "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
    );
  });
});
