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

    pub const fn empty() -> Self {
        Self {
            literal: Vec::new(),
            symbolic: Vec::new(),
            display: Vec::new(),
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

    /// Resolves a literal or symbolic token name to its token type.
    ///
    /// This follows ANTLR's recognizer lookup behavior: display-only names are
    /// ignored, duplicate names resolve to the highest token type, and `EOF`
    /// resolves to [`crate::TOKEN_EOF`].
    #[must_use]
    pub fn token_type(&self, name: &str) -> Option<i32> {
        if name == "EOF" {
            return Some(crate::token::TOKEN_EOF);
        }

        let len = self.literal.len().max(self.symbolic.len());
        (0..len).rev().find_map(|index| {
            let matches = self
                .literal
                .get(index)
                .and_then(Option::as_deref)
                .is_some_and(|literal| literal == name)
                || self
                    .symbolic
                    .get(index)
                    .and_then(Option::as_deref)
                    .is_some_and(|symbolic| symbolic == name);
            matches.then(|| i32::try_from(index).ok()).flatten()
        })
    }

    fn get(values: &[Option<String>], token_type: i32) -> Option<&str> {
        usize::try_from(token_type)
            .ok()
            .and_then(|index| values.get(index))
            .and_then(Option::as_deref)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
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
        assert_eq!(vocabulary.token_type("'let'"), Some(1));
        assert_eq!(vocabulary.token_type("ID"), Some(2));
        assert_eq!(vocabulary.token_type("identifier"), None);
        assert_eq!(vocabulary.token_type("EOF"), Some(crate::token::TOKEN_EOF));
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

            // The observed (display, literal, symbolic) classification for every token type is
            // more legible as one snapshot than as a branchy per-token invariant recomputed in a
            // loop: it pins the literal-vs-symbolic split for all names at once.
            let classification = (0..token_names.len())
                .map(|token_type| {
                    let token_type = i32::try_from(token_type).expect("test token type fits i32");
                    (
                        token_type,
                        vocabulary.display_name(token_type),
                        vocabulary.literal_name(token_type).map(str::to_owned),
                        vocabulary.symbolic_name(token_type).map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            insta::assert_debug_snapshot!(
                "vocabulary_from_token_names_matches_java",
                classification
            );
        }
    }
}
