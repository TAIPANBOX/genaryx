//! `path_segment`: encode one customer-controlled id as exactly ONE URL path
//! segment, so the path that gets SIGNED is byte-for-byte the path that gets
//! SENT.
//!
//! ## Why this is not cosmetic
//!
//! [`crate::CloudClient`]'s three admin mutations are ES256-signed over a
//! canonical string whose second line is the request PATH, and the Cloud
//! verifies that signature over `uri.path()` - the raw, still-percent-encoded
//! path it actually received (`tokenfuse`'s `cloud/src/http.rs`). The client
//! must therefore sign and send the same bytes. Interpolating an id RAW into
//! the path and letting `reqwest`/`url` encode it afterwards breaks precisely
//! that invariant: the signature covers `/v1/runs/a b/kill` while the wire
//! carries `/v1/runs/a%20b/kill`, and the Cloud answers
//! `403 signature_invalid`. For [`crate::CloudClient::kill_run`] that is a
//! kill that does not kill, which is the one failure mode this whole product
//! exists to prevent. Ids are server-assigned but customer-derived (agent and
//! run names reach them), so a space, a slash or a non-ASCII letter is
//! reachable, not theoretical.
//!
//! This is the desktop twin of the tokenfuse-mobile fix (`String.asPathSegment`
//! with `percentEncodedPath`, commits `eac44ed`/`ed9c9de`, task #15). The
//! encoding set below is deliberately the SAME set Foundation's
//! `CharacterSet.urlPathAllowed` (minus `/` and `%`) applies, so the phone and
//! the console produce byte-identical paths for the same id, and the pinned
//! cross-language canonical vectors keep matching.
//!
//! ## The set, and why the `url` crate then leaves it alone
//!
//! Everything outside RFC 3986 `pchar` (`unreserved` + `sub-delims` + `:` +
//! `@`) is written as `%XX` over the id's UTF-8 bytes; `/` and `%` are
//! encoded too, so an id can never open a new path segment nor forge a
//! pct-encoded triplet.
//!
//! What survives is exactly what `url`'s PATH percent-encode set (C0 controls,
//! space, `"`, `#`, `<`, `>`, `?`, `` ` ``, `{`, `}`, and non-ASCII) does not
//! touch, and `url` never re-encodes an existing `%`. So parsing
//! `format!("{base}{path}")` returns the path unchanged - which is what
//! [`path_segment_survives_url_parsing`] asserts against the real parser
//! rather than against a claim in a comment.
//!
//! ## Why `.` and `..` are rejected instead of encoded
//!
//! Encoding them would not help: the URL Standard treats `%2e`, `.%2e`,
//! `%2e%2e` and friends as dot segments too, so a `..` id would still be
//! normalized away - changing both the path that is sent and, with it, the
//! resource addressed. There is no id worth spending that on, so an empty,
//! `.` or `..` id fails closed here, before anything is signed or sent
//! (06 §0.5). This also closes the LOW left accepted in the mobile review.

use std::fmt::Write as _;

/// An id that cannot be expressed as a single URL path segment. Surfaced by
/// the callers' own error types (`ConnectorError::InvalidPathSegment`,
/// `WardryxError::InvalidPathSegment`) and returned *before* any request is
/// built, signed or sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathSegmentError {
    /// An empty id addresses no resource; sending it would silently target
    /// the collection route instead (`/v1/runs//kill`).
    #[error("an empty id cannot address a resource")]
    Empty,

    /// `.` or `..`: a relative path segment, not an id. See the module docs
    /// for why this is rejected rather than encoded.
    #[error("`{0}` is a relative path segment, not an id")]
    DotSegment(String),
}

/// True for the bytes that may appear literally in one path segment: RFC 3986
/// `pchar` minus the `%` that introduces a pct-encoded triplet, minus the `/`
/// that separates segments.
const fn is_literal_pchar(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        // unreserved
        || matches!(b, b'-' | b'.' | b'_' | b'~')
        // sub-delims
        || matches!(b, b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=')
        // the two extras `pchar` allows in a segment
        || matches!(b, b':' | b'@')
}

/// Percent-encode `id` as exactly one URL path segment, or fail closed if it
/// cannot be one at all.
///
/// Ordinary ids (`r1`, `reconciliation-batch-eod-002-s128`) encode to
/// themselves, so this is a no-op for every id in practice and a correctness
/// fix for the ones that are not.
pub(crate) fn path_segment(id: &str) -> Result<String, PathSegmentError> {
    if id.is_empty() {
        return Err(PathSegmentError::Empty);
    }
    if id == "." || id == ".." {
        return Err(PathSegmentError::DotSegment(id.to_string()));
    }
    let mut out = String::with_capacity(id.len());
    for &b in id.as_bytes() {
        if is_literal_pchar(b) {
            out.push(char::from(b));
        } else {
            // Uppercase hex: RFC 3986 §6.2.2.1's preferred form, and the form
            // Foundation emits, so the phone and the console agree byte for
            // byte.
            let _ = write!(out, "%{b:02X}");
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same vectors the mobile `PathEncodingTests` pins, so the two
    /// clients cannot silently drift apart.
    #[test]
    fn reserved_characters_become_data() {
        assert_eq!(
            path_segment("a/b").unwrap(),
            "a%2Fb",
            "a slash must not open a new path segment"
        );
        assert_eq!(path_segment("a b").unwrap(), "a%20b");
        assert_eq!(path_segment("a#b").unwrap(), "a%23b");
        assert_eq!(path_segment("a?b").unwrap(), "a%3Fb");
        assert_eq!(
            path_segment("a%b").unwrap(),
            "a%25b",
            "a literal percent must survive as data"
        );
    }

    #[test]
    fn ordinary_ids_are_unchanged() {
        // The overwhelmingly common case: encoding is a no-op, so live ids and
        // the pinned cross-language canonical vectors are unaffected.
        assert_eq!(
            path_segment("reconciliation-batch-eod-002-s128").unwrap(),
            "reconciliation-batch-eod-002-s128"
        );
        assert_eq!(path_segment("r1").unwrap(), "r1");
        assert_eq!(path_segment("inc-42").unwrap(), "inc-42");
    }

    #[test]
    fn an_already_encoded_id_is_not_decoded() {
        // An id that literally contains "%2F" is preserved verbatim, never
        // silently turned back into a slash.
        assert_eq!(path_segment("weird%2Fid").unwrap(), "weird%252Fid");
    }

    #[test]
    fn non_ascii_is_encoded_as_utf8_bytes() {
        assert_eq!(path_segment("é").unwrap(), "%C3%A9");
        assert_eq!(path_segment("run-й").unwrap(), "run-%D0%B9");
    }

    #[test]
    fn unreachable_ids_fail_closed() {
        assert_eq!(path_segment(""), Err(PathSegmentError::Empty));
        assert_eq!(
            path_segment("."),
            Err(PathSegmentError::DotSegment(".".to_string()))
        );
        assert_eq!(
            path_segment(".."),
            Err(PathSegmentError::DotSegment("..".to_string()))
        );
        // Only an *exact* dot segment is refused - an id that merely starts
        // with dots is a perfectly good id.
        assert_eq!(path_segment("..x").unwrap(), "..x");
    }

    /// The invariant the whole fix rests on, asserted against the real parser
    /// `reqwest` uses (`reqwest::Url` IS `url::Url`): the path we sign is
    /// byte-for-byte the path the request carries, which is what the Cloud
    /// verifies over. If `url` ever started re-encoding one of the characters
    /// this module leaves literal, this test fails instead of every signed
    /// mutation failing in production.
    #[test]
    fn path_segment_survives_url_parsing() {
        let base = "http://127.0.0.1:8080";
        for id in [
            "r1",
            "a/b#c d",
            "інцидент-42",
            "weird%2Fid",
            "a?b",
            "..x",
            "n:o@p",
            "back\\slash",
            "{brace}",
            "quote\"",
            "tab\there",
        ] {
            let path = format!("/v1/runs/{}/kill", path_segment(id).unwrap());
            let url = reqwest::Url::parse(&format!("{base}{path}")).expect("parses");
            assert_eq!(
                url.path(),
                path,
                "the signed path and the sent path must match for id {id:?}"
            );
        }
    }
}
