use super::{property_ranges, simple_lowercase, simple_uppercase};
use std::collections::BTreeMap;
use std::fmt::Write;

use sha2::{Digest, Sha256};

const JAVA_PROPERTY_ORACLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/codegen-direct/fixtures/lexer-unicode/oracle/java-unicode-properties.tsv"
));

fn contains(property: &str, code_point: i32) -> bool {
    property_ranges(property)
        .expect("known Unicode property")
        .chunks_exact(2)
        .any(|range| (range[0]..=range[1]).contains(&code_point))
}

#[test]
fn unicode_17_script_data_is_available() {
    assert!(contains("Script=Sidetic", 0x10940));
    assert!(contains("Sidetic", 0x10940));
}

#[test]
fn unicode_17_blocks_are_available() {
    assert_eq!(intl::unicode::UNICODE_VERSION, (17, 0, 0));
    assert_eq!(
        property_ranges("Block=Sidetic"),
        Some(&[0x10940, 0x1095f][..])
    );
    assert_eq!(
        property_ranges("InCJK_Unified_Ideographs_Extension_J"),
        Some(&[0x323b0, 0x3347f][..])
    );
}

#[test]
fn surrogate_category_and_blocks_are_not_limited_to_rust_chars() {
    assert!(contains("gc=Cs", 0xd800));
    assert_eq!(
        property_ranges("InHigh_Surrogates"),
        Some(&[0xd800, 0xdb7f][..])
    );
}

#[test]
fn invalid_scalars_are_unchanged_by_simple_case_mapping() {
    for code_point in [-1, 0xd800, 0x11_0000] {
        assert_eq!(simple_lowercase(code_point), code_point);
        assert_eq!(simple_uppercase(code_point), code_point);
    }
}

#[test]
fn direct_property_precedes_a_colliding_alias() {
    let historical = property_ranges("extended_pictographic").expect("ANTLR historical property");
    let current = property_ranges("extpict").expect("current ICU property");

    assert_eq!(historical.len() / 2, 102);
    assert_eq!(current.len() / 2, 156);
    assert_ne!(historical, current);
    assert_eq!(property_ranges("EP"), Some(historical));
}

#[test]
fn every_unicode_property_and_alias_matches_java() {
    let mut properties = BTreeMap::new();
    let mut aliases = Vec::new();
    for line in JAVA_PROPERTY_ORACLE.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        match fields.as_slice() {
            ["property", name, interval_count, digest] => {
                properties.insert(
                    *name,
                    (
                        interval_count
                            .parse::<usize>()
                            .expect("valid Java interval count"),
                        *digest,
                    ),
                );
            }
            ["alias", alias, canonical] => aliases.push((*alias, *canonical)),
            _ => panic!("invalid Unicode oracle line: {line}"),
        }
    }

    let mut failures = String::new();
    for (&name, &(expected_count, expected_digest)) in &properties {
        let Some(ranges) = property_ranges(name) else {
            writeln!(failures, "{name}: missing property").expect("write to string");
            continue;
        };
        let actual_count = ranges.len() / 2;
        let actual_digest = interval_digest(ranges);
        if actual_count != expected_count || actual_digest != expected_digest {
            writeln!(
                failures,
                "{name}: expected {expected_count}/{expected_digest}, \
                 found {actual_count}/{actual_digest}"
            )
            .expect("write to string");
        }
    }

    for (alias, canonical) in aliases {
        // UnicodeData checks direct properties before consulting this map.
        if properties.contains_key(alias) {
            continue;
        }
        let Some(expected) = property_ranges(canonical) else {
            writeln!(
                failures,
                "{alias}: canonical property {canonical} is missing"
            )
            .expect("write to string");
            continue;
        };
        match property_ranges(alias) {
            Some(actual) if actual == expected => {}
            Some(_) => {
                writeln!(failures, "{alias}: differs from {canonical}").expect("write to string");
            }
            None => {
                writeln!(failures, "{alias}: missing alias for {canonical}")
                    .expect("write to string");
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Rust Unicode data differs from the Java oracle:\n{failures}"
    );
}

fn interval_digest(ranges: &[i32]) -> String {
    let mut digest = Sha256::new();
    for range in ranges.chunks_exact(2) {
        digest.update(range[0].to_be_bytes());
        digest.update(range[1].to_be_bytes());
    }
    format!("{:x}", digest.finalize())
}
