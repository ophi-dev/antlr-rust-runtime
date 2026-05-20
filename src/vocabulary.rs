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
}
