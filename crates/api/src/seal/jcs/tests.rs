use super::*;
use serde_json::json;

fn text(v: &Value) -> String {
    canonical(v).expect("a canonical form")
}

#[test]
fn members_are_sorted_by_utf16_code_units_not_by_bytes() {
    // RFC 8785 section 3.2.3: the order a UTF-8 byte sort would get wrong is
    // U+1F600 (a surrogate pair, D83D DE00) before U+FB33 (FB33).
    let mut object = serde_json::Map::new();
    for key in [
        "\u{20ac}",
        "\r",
        "\u{fb33}",
        "1",
        "\u{1f600}",
        "\u{80}",
        "\u{f6}",
    ] {
        object.insert(key.to_string(), Value::String("x".to_string()));
    }
    let canonical = text(&Value::Object(object));
    let keys: Vec<&str> = [
        "\"\\r\"",
        "\"1\"",
        "\"\u{80}\"",
        "\"\u{f6}\"",
        "\"\u{20ac}\"",
        "\"\u{1f600}\"",
        "\"\u{fb33}\"",
    ]
    .to_vec();
    let mut at = 0;
    for key in keys {
        let found = canonical[at..]
            .find(key)
            .unwrap_or_else(|| panic!("{key:?} is missing or out of order in {canonical:?}"));
        at += found + key.len();
    }
}

#[test]
fn the_form_has_no_whitespace_and_the_members_are_in_order() {
    let value = json!({ "b": [1, 2, { "z": null, "a": true }], "a": "x", "c": false });
    assert_eq!(
        text(&value),
        r#"{"a":"x","b":[1,2,{"a":true,"z":null}],"c":false}"#
    );
    assert_eq!(text(&json!({})), "{}");
    assert_eq!(text(&json!([])), "[]");
}

#[test]
fn strings_are_escaped_the_way_json_stringify_escapes_them() {
    let value = json!("\u{0}\u{1f}\u{8}\t\n\u{c}\r\"\\/\u{e9}\u{7f}\u{2028}\u{1f600}");
    assert_eq!(
        text(&value),
        "\"\\u0000\\u001f\\b\\t\\n\\f\\r\\\"\\\\/\u{e9}\u{7f}\u{2028}\u{1f600}\""
    );
    // A key is a string and is escaped the same way.
    assert_eq!(text(&json!({ "a\"b": 1 })), "{\"a\\\"b\":1}");
}

#[test]
fn numbers_are_written_the_way_ecmascript_writes_them() {
    for (x, expected) in [
        (0.0, "0"),
        (-0.0, "0"),
        (1.0, "1"),
        (-1.0, "-1"),
        (4.5, "4.5"),
        (0.002, "0.002"),
        (0.000001, "0.000001"),
        (1e-7, "1e-7"),
        (1.5e-9, "1.5e-9"),
        (-1.5e-9, "-1.5e-9"),
        (100.0, "100"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
        (1.5e300, "1.5e+300"),
        (123456789012345680000.0, "123456789012345680000"),
        (333333333.33333329, "333333333.3333333"),
        (1e30, "1e+30"),
        (1e-27, "1e-27"),
        (9007199254740992.0, "9007199254740992"),
        (1.7976931348623157e308, "1.7976931348623157e+308"),
        (5e-324, "5e-324"),
        (0.1, "0.1"),
        (12.345, "12.345"),
    ] {
        assert_eq!(es6_number(x), expected, "{x:e}");
    }
    let float = Value::Number(Number::from_f64(4.5).expect("finite"));
    assert_eq!(text(&float), "4.5");
}

#[test]
fn an_integer_a_double_cannot_hold_is_refused_and_one_it_can_is_written_whole() {
    assert_eq!(text(&json!(9_007_199_254_740_991u64)), "9007199254740991");
    assert_eq!(text(&json!(-9_007_199_254_740_991i64)), "-9007199254740991");
    assert_eq!(text(&json!(0)), "0");
    for refused in [
        json!(9_007_199_254_740_992u64),
        json!(-9_007_199_254_740_992i64),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(i64::MAX),
    ] {
        assert_eq!(
            canonical(&refused),
            Err(JcsError::IntegerNotExact),
            "{refused}"
        );
    }
    // The refusal is found inside a document, not only at its top.
    assert!(canonical(&json!({ "a": [1, { "b": u64::MAX }] })).is_err());
}

fn pseudo(x: &mut u64) -> u64 {
    *x = x
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *x >> 33
}

fn random_value(x: &mut u64, depth: u32) -> Value {
    let keys = [
        "a",
        "b",
        "\u{e9}",
        "\u{1f600}",
        "\u{fb33}",
        "z",
        "\"",
        "\n",
        "",
        "10",
        "9",
    ];
    match pseudo(x) % if depth == 0 { 5 } else { 7 } {
        0 => Value::Null,
        1 => Value::Bool(pseudo(x) % 2 == 0),
        2 => json!(pseudo(x) as i64 - (1 << 30)),
        3 => Value::String(
            (0..pseudo(x) % 5)
                .map(|_| {
                    ['a', '"', '\\', '\n', '\u{1}', '\u{e9}', '\u{1f600}'][pseudo(x) as usize % 7]
                })
                .collect(),
        ),
        4 => Value::Number(Number::from_f64((pseudo(x) % 100000) as f64 / 8.0).expect("finite")),
        5 => Value::Array(
            (0..pseudo(x) % 4)
                .map(|_| random_value(x, depth - 1))
                .collect(),
        ),
        _ => {
            let mut map = serde_json::Map::new();
            for _ in 0..pseudo(x) % 5 {
                let key = keys[pseudo(x) as usize % keys.len()].to_string();
                map.insert(key, random_value(x, depth - 1));
            }
            Value::Object(map)
        }
    }
}

#[test]
fn the_canonical_text_reads_back_to_a_value_with_the_same_canonical_text() {
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(13);
        let value = random_value(&mut x, 3);
        let once = text(&value);
        let again: Value = serde_json::from_str(&once).expect("canonical text is JSON");
        assert_eq!(text(&again), once, "seed {seed}");
        // And it is the same text whichever order the members were inserted in.
        if let Value::Object(map) = &value {
            let mut reversed = serde_json::Map::new();
            for (k, v) in map.iter().rev() {
                reversed.insert(k.clone(), v.clone());
            }
            assert_eq!(text(&Value::Object(reversed)), once, "seed {seed}");
        }
    }
}
