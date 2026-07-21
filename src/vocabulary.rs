#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Vocabulary {
    literal: Vec<Option<String>>,
    symbolic: Vec<Option<String>>,
    display: Vec<Option<String>>,
}

impl Vocabulary {
    pub fn new(
        literal_names: impl IntoIterator<Item = Option<impl Into<String>>>,
        symbolic_names: impl IntoIterator<Item = Option<impl Into<String>>>,
        display_names: impl IntoIterator<Item = Option<impl Into<String>>>,
    ) -> Self {
        Self {
            literal: literal_names
                .into_iter()
                .map(|value| value.map(Into::into))
                .collect(),
            symbolic: symbolic_names
                .into_iter()
                .map(|value| value.map(Into::into))
                .collect(),
            display: display_names
                .into_iter()
                .map(|value| value.map(Into::into))
                .collect(),
        }
    }

    pub fn from_token_names(
        token_names: impl IntoIterator<Item = Option<impl Into<String>>>,
    ) -> Self {
        let display = token_names
            .into_iter()
            .map(|value| value.map(Into::into))
            .collect::<Vec<Option<String>>>();
        let literal = display
            .iter()
            .map(|name| name.as_ref().filter(|name| name.starts_with('\'')).cloned())
            .collect();
        let symbolic = display
            .iter()
            .map(|name| {
                name.as_ref()
                    .filter(|name| name.starts_with(char::is_uppercase))
                    .cloned()
            })
            .collect();
        Self {
            literal,
            symbolic,
            display,
        }
    }

    pub fn literal_name(&self, token_type: i32) -> Option<&str> {
        Self::get(&self.literal, token_type)
    }

    pub fn symbolic_name(&self, token_type: i32) -> Option<&str> {
        if token_type == crate::token::TOKEN_EOF {
            return Some("EOF");
        }
        Self::get(&self.symbolic, token_type)
    }

    pub fn display_name(&self, token_type: i32) -> String {
        Self::get(&self.display, token_type)
            .or_else(|| self.literal_name(token_type))
            .or_else(|| self.symbolic_name(token_type))
            .map_or_else(|| token_type.to_string(), ToOwned::to_owned)
    }

    fn get(values: &[Option<String>], token_type: i32) -> Option<&str> {
        usize::try_from(token_type)
            .ok()
            .and_then(|index| values.get(index))
            .and_then(Option::as_deref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_falls_back_in_antlr_order() {
        let vocabulary = Vocabulary::new(
            [None, Some("'let'")],
            [None, Some("LET"), Some("ID")],
            [None::<&str>, None, Some("identifier")],
        );
        assert_eq!(vocabulary.display_name(1), "'let'");
        assert_eq!(vocabulary.display_name(2), "identifier");
        assert_eq!(vocabulary.display_name(99), "99");
        assert_eq!(vocabulary.symbolic_name(-1), Some("EOF"));
    }

    mod upstream_vocabulary {
        use super::*;

        #[test]
        fn empty_vocabulary_matches_java() {
            let vocabulary = Vocabulary::empty();

            assert_eq!(
                vocabulary.symbolic_name(crate::token::TOKEN_EOF),
                Some("EOF")
            );
            assert_eq!(vocabulary.display_name(0), "0");
        }

        #[test]
        fn vocabulary_from_token_names_matches_java() {
            let token_names = [
                "<INVALID>",
                "TOKEN_REF",
                "RULE_REF",
                "'//'",
                "'/'",
                "'*'",
                "'!'",
                "ID",
                "STRING",
            ];
            let vocabulary =
                Vocabulary::from_token_names(token_names.map(|name| Some(name.to_owned())));

            assert_eq!(
                vocabulary.symbolic_name(crate::token::TOKEN_EOF),
                Some("EOF")
            );
            for (token_type, token_name) in token_names.into_iter().enumerate() {
                let token_type = i32::try_from(token_type).expect("test token type fits i32");
                assert_eq!(vocabulary.display_name(token_type), token_name);

                if token_name.starts_with('\'') {
                    assert_eq!(vocabulary.literal_name(token_type), Some(token_name));
                    assert_eq!(vocabulary.symbolic_name(token_type), None);
                } else if token_name.starts_with(char::is_uppercase) {
                    assert_eq!(vocabulary.literal_name(token_type), None);
                    assert_eq!(vocabulary.symbolic_name(token_type), Some(token_name));
                } else {
                    assert_eq!(vocabulary.literal_name(token_type), None);
                    assert_eq!(vocabulary.symbolic_name(token_type), None);
                }
            }
        }
    }
}
