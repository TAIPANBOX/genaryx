//! RFC 8785 JSON Canonicalization Scheme, for the JSON a case pack holds.
//!
//! A digest is only worth sealing if another reader, in another language, can
//! compute it from the same document. `serde_json` writes a struct's fields in
//! declaration order and a map's keys in whatever order they were inserted, so
//! the bytes a pack was written with are not a thing a second implementation can
//! reproduce. RFC 8785 fixes the bytes: no whitespace, object members sorted by
//! the UTF-16 code units of their names, strings escaped the way `JSON.stringify`
//! escapes them, and numbers written the way ECMAScript's `Number::toString`
//! writes them.
//!
//! One thing is refused rather than guessed: an integer past the range a double
//! holds exactly (`+-(2^53 - 1)`). RFC 8785 writes every number as a double, so
//! such an integer would be rounded, and a digest that depends on a rounding is
//! a digest two honest readers can disagree about.

use serde_json::{Number, Value};

/// The largest integer a double holds exactly, and so the largest this writes.
pub const MAX_EXACT_INT: u64 = 9_007_199_254_740_991;

/// Why a value has no canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JcsError {
    /// An integer past `+-(2^53 - 1)`, which RFC 8785 would round.
    IntegerNotExact,
}

/// The canonical text of `value`.
pub fn canonical(value: &Value) -> Result<String, JcsError> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

fn write_value(value: &Value, out: &mut String) -> Result<(), JcsError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&number(n)?),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(&map[key.as_str()], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// A string as `JSON.stringify` writes it: the quote, the backslash and the
/// control characters are escaped, everything else is written as it is.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number(n: &Number) -> Result<String, JcsError> {
    if let Some(i) = n.as_i64() {
        return if i.unsigned_abs() <= MAX_EXACT_INT {
            Ok(i.to_string())
        } else {
            Err(JcsError::IntegerNotExact)
        };
    }
    if let Some(u) = n.as_u64() {
        return if u <= MAX_EXACT_INT {
            Ok(u.to_string())
        } else {
            Err(JcsError::IntegerNotExact)
        };
    }
    // A float: serde_json never holds a non-finite one.
    Ok(es6_number(n.as_f64().unwrap_or(0.0)))
}

/// ECMAScript's `Number::toString` for a finite double.
///
/// The shortest digits that read back as the same double come from Rust's `{:e}`
/// (`1.2345e-7`); the rest is where ECMAScript puts the point and when it falls
/// back to an exponent. `n` below is the exponent of the first digit plus one,
/// `k` how many digits there are.
pub fn es6_number(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let scientific = format!("{:e}", x.abs());
    let (mantissa, exponent) = scientific
        .split_once('e')
        .unwrap_or((scientific.as_str(), "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let n: i32 = exponent.parse::<i32>().unwrap_or(0) + 1;
    let k = digits.len() as i32;

    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign_e = if e < 0 { "-" } else { "+" };
        if k == 1 {
            format!("{digits}e{sign_e}{}", e.abs())
        } else {
            format!("{}.{}e{sign_e}{}", &digits[..1], &digits[1..], e.abs())
        }
    };
    format!("{sign}{body}")
}

#[cfg(test)]
mod tests;
