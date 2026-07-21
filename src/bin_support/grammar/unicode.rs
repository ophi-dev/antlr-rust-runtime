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
}
