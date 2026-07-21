#[path = "unicode_data.rs"]
mod data;

pub(crate) fn property_ranges(name: &str) -> Option<&'static [i32]> {
    let normalized = normalize_property_name(name);
    let entry = data::PROPERTY_ENTRIES
        .binary_search_by(|entry| entry.name.cmp(normalized.as_str()))
        .ok()
        .map(|index| data::PROPERTY_ENTRIES[index])?;
    let start = usize::try_from(entry.offset).expect("Unicode table offset fits usize");
    let length = usize::try_from(entry.length).expect("Unicode table length fits usize");
    data::PROPERTY_RANGES.get(start..start + length)
}

pub(crate) fn simple_lowercase(code_point: i32) -> i32 {
    simple_case_mapping(code_point, data::SIMPLE_LOWERCASE)
}

pub(crate) fn simple_uppercase(code_point: i32) -> i32 {
    simple_case_mapping(code_point, data::SIMPLE_UPPERCASE)
}

fn simple_case_mapping(code_point: i32, mappings: &[(i32, i32)]) -> i32 {
    mappings
        .binary_search_by_key(&code_point, |(source, _)| *source)
        .map_or(code_point, |index| mappings[index].1)
}

fn normalize_property_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            '-' => normalized.push('_'),
            'A'..='Z' => {
                let value = u32::from(character) + 0x20;
                normalized.push(char::from_u32(value).expect("ASCII lowercase is valid"));
            }
            _ => normalized.push(character),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(property: &str, code_point: i32) -> bool {
        property_ranges(property)
            .expect("known Unicode property")
            .chunks_exact(2)
            .any(|range| (range[0]..=range[1]).contains(&code_point))
    }

    #[test]
    fn resolves_properties_aliases_and_normalization() {
        assert_eq!(property_ranges("Gothic"), Some(&[66352, 66378][..]));
        assert_eq!(property_ranges("Deseret"), Some(&[66560, 66639][..]));
        assert_eq!(
            property_ranges("InLatin-Extended-B"),
            property_ranges("block=latin_extended_b"),
        );
        assert!(property_ranges("not_a_unicode_property").is_none());
    }

    #[test]
    fn uses_pinned_java_simple_case_mappings() {
        assert_eq!(simple_lowercase(i32::from(b'A')), i32::from(b'a'));
        assert_eq!(simple_uppercase(i32::from(b'a')), i32::from(b'A'));
        assert_eq!(simple_uppercase(0x00df), 0x00df);
        assert_eq!(simple_lowercase(0x0130), i32::from(b'i'));
    }

    #[test]
    fn unicode_general_categories_latin_matches_java() {
        assert!(contains("Lu", i32::from(b'X')));
        assert!(!contains("Lu", i32::from(b'x')));
        assert!(contains("Ll", i32::from(b'x')));
        assert!(!contains("Ll", i32::from(b'X')));
        assert!(contains("L", i32::from(b'X')));
        assert!(contains("L", i32::from(b'x')));
        assert!(contains("N", i32::from(b'0')));
        assert!(contains("Z", i32::from(b' ')));
    }

    #[test]
    fn unicode_general_categories_bmp_matches_java() {
        assert!(contains("Lu", 0x1e3a));
        assert!(!contains("Lu", 0x1e3b));
        assert!(contains("Ll", 0x1e3b));
        assert!(!contains("Ll", 0x1e3a));
        assert!(contains("L", 0x1e3a));
        assert!(contains("L", 0x1e3b));
        assert!(contains("N", 0x1bb0));
        assert!(!contains("N", 0x1e3a));
        assert!(contains("Z", 0x2028));
        assert!(!contains("Z", 0x1e3a));
    }

    #[test]
    fn unicode_general_categories_smp_matches_java() {
        assert!(contains("Lu", 0x1d5d4));
        assert!(!contains("Lu", 0x1d770));
        assert!(contains("Ll", 0x1d770));
        assert!(!contains("Ll", 0x1d5d4));
        assert!(contains("L", 0x1d5d4));
        assert!(contains("L", 0x1d770));
        assert!(contains("N", 0x11c50));
        assert!(!contains("N", 0x1d5d4));
    }

    #[test]
    fn unicode_category_aliases_match_java() {
        assert!(contains("Lowercase_Letter", i32::from(b'x')));
        assert!(!contains("Lowercase_Letter", i32::from(b'X')));
        assert!(contains("Letter", i32::from(b'x')));
        assert!(!contains("Letter", i32::from(b'0')));
        assert!(contains("Enclosing_Mark", 0x20e2));
        assert!(!contains("Enclosing_Mark", i32::from(b'x')));
    }

    #[test]
    fn unicode_binary_properties_match_java() {
        assert!(contains("Emoji", 0x1f4a9));
        assert!(!contains("Emoji", i32::from(b'X')));
        assert!(contains("alnum", i32::from(b'9')));
        assert!(!contains("alnum", 0x1f4a9));
        assert!(contains("Dash", i32::from(b'-')));
        assert!(contains("Hex", i32::from(b'D')));
        assert!(!contains("Hex", i32::from(b'Q')));
    }

    #[test]
    fn unicode_binary_property_aliases_match_java() {
        assert!(contains("Ideo", 0x611b));
        assert!(!contains("Ideo", i32::from(b'X')));
        assert!(contains("Soft_Dotted", 0x0456));
        assert!(!contains("Soft_Dotted", i32::from(b'X')));
        assert!(contains("Noncharacter_Code_Point", 0xffff));
        assert!(!contains("Noncharacter_Code_Point", i32::from(b'X')));
    }

    #[test]
    fn unicode_scripts_match_java() {
        assert!(contains("Zyyy", i32::from(b'0')));
        assert!(contains("Latn", i32::from(b'X')));
        assert!(contains("Hani", 0x4e04));
        assert!(contains("Cyrl", 0x0404));
    }

    #[test]
    fn unicode_script_equals_matches_java() {
        assert!(contains("Script=Zyyy", i32::from(b'0')));
        assert!(contains("Script=Latn", i32::from(b'X')));
        assert!(contains("Script=Hani", 0x4e04));
        assert!(contains("Script=Cyrl", 0x0404));
    }

    #[test]
    fn unicode_script_aliases_match_java() {
        assert!(contains("Common", i32::from(b'0')));
        assert!(contains("Latin", i32::from(b'X')));
        assert!(contains("Han", 0x4e04));
        assert!(contains("Cyrillic", 0x0404));
    }

    #[test]
    fn unicode_blocks_match_java() {
        assert!(contains("InASCII", i32::from(b'0')));
        assert!(contains("InCJK", 0x4e04));
        assert!(contains("InCyrillic", 0x0404));
        assert!(contains("InMisc_Pictographs", 0x1f4a9));
    }

    #[test]
    fn unicode_block_equals_matches_java() {
        assert!(contains("Block=ASCII", i32::from(b'0')));
        assert!(contains("Block=CJK", 0x4e04));
        assert!(contains("Block=Cyrillic", 0x0404));
        assert!(contains("Block=Misc_Pictographs", 0x1f4a9));
    }

    #[test]
    fn unicode_block_aliases_match_java() {
        assert!(contains("InBasic_Latin", i32::from(b'0')));
        assert!(contains("InMiscellaneous_Mathematical_Symbols_B", 0x29be));
    }

    #[test]
    fn enumerated_property_equals_matches_java() {
        assert!(!contains("Grapheme_Cluster_Break=E_Base", 0x1f47e));
        assert!(!contains("Grapheme_Cluster_Break=E_Base", 0x1038));
        assert!(contains("East_Asian_Width=Ambiguous", 0x00a1));
        assert!(!contains("East_Asian_Width=Ambiguous", 0x00a2));
    }

    #[test]
    fn extended_pictographic_matches_java() {
        assert!(contains("Extended_Pictographic", 0x1f588));
        assert!(!contains("Extended_Pictographic", i32::from(b'0')));
    }

    #[test]
    fn emoji_presentation_matches_java() {
        assert!(contains("EmojiPresentation=EmojiDefault", 0x1f4a9));
        assert!(!contains("EmojiPresentation=EmojiDefault", i32::from(b'0')));
        assert!(!contains("EmojiPresentation=EmojiDefault", i32::from(b'A')));
        assert!(!contains("EmojiPresentation=TextDefault", 0x1f4a9));
        assert!(contains("EmojiPresentation=TextDefault", i32::from(b'0')));
        assert!(!contains("EmojiPresentation=TextDefault", i32::from(b'A')));
    }

    #[test]
    fn property_case_insensitivity_matches_java() {
        assert!(contains("l", i32::from(b'x')));
        assert!(!contains("l", i32::from(b'0')));
        assert!(contains("common", i32::from(b'0')));
        assert!(contains("Alnum", i32::from(b'0')));
    }

    #[test]
    fn property_dash_same_as_underscore_matches_java() {
        assert!(contains("InLatin-1", 0x00f0));
    }

    #[test]
    fn modifying_unicode_data_should_throw_matches_java() {
        fn requires_read_only_static_slice(_: &'static [i32]) {}

        requires_read_only_static_slice(property_ranges("L").expect("known Unicode property"));
    }
}
