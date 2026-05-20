use crate::atn::serialized::SerializedAtn;
use crate::vocabulary::Vocabulary;

#[derive(Clone, Debug)]
pub struct GrammarMetadata {
    grammar_file_name: &'static str,
    rule_names: &'static [&'static str],
    literal_names: &'static [Option<&'static str>],
    symbolic_names: &'static [Option<&'static str>],
    display_names: &'static [Option<&'static str>],
    channel_names: &'static [&'static str],
    mode_names: &'static [&'static str],
    serialized_atn: &'static [i32],
}

impl GrammarMetadata {
    /// Creates static grammar metadata emitted by the Rust target generator.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        grammar_file_name: &'static str,
        rule_names: &'static [&'static str],
        literal_names: &'static [Option<&'static str>],
        symbolic_names: &'static [Option<&'static str>],
        display_names: &'static [Option<&'static str>],
        channel_names: &'static [&'static str],
        mode_names: &'static [&'static str],
        serialized_atn: &'static [i32],
    ) -> Self {
        Self {
            grammar_file_name,
            rule_names,
            literal_names,
            symbolic_names,
            display_names,
            channel_names,
            mode_names,
            serialized_atn,
        }
    }

    pub const fn grammar_file_name(&self) -> &'static str {
        self.grammar_file_name
    }

    pub const fn rule_names(&self) -> &'static [&'static str] {
        self.rule_names
    }

    pub const fn channel_names(&self) -> &'static [&'static str] {
        self.channel_names
    }

    pub const fn mode_names(&self) -> &'static [&'static str] {
        self.mode_names
    }

    pub fn vocabulary(&self) -> Vocabulary {
        Vocabulary::new(
            self.literal_names.iter().copied(),
            self.symbolic_names.iter().copied(),
            self.display_names.iter().copied(),
        )
    }

    /// Borrows the serialized ATN values for deserialization by the runtime
    /// simulators without copying generated static data.
    pub const fn serialized_atn(&self) -> SerializedAtn<'_> {
        SerializedAtn::from_i32(self.serialized_atn)
    }
}

pub trait GeneratedLexer {
    fn metadata() -> &'static GrammarMetadata;
}

pub trait GeneratedParser {
    fn metadata() -> &'static GrammarMetadata;
}

#[cfg(test)]
mod tests {
    use super::*;

    static META: GrammarMetadata = GrammarMetadata::new(
        "Mini.g4",
        &["file"],
        &[None, Some("'x'")],
        &[None, Some("X")],
        &[None, None],
        &["DEFAULT_TOKEN_CHANNEL", "HIDDEN"],
        &["DEFAULT_MODE"],
        &[4, 1, 1, 0, 0, 0],
    );

    #[test]
    fn metadata_builds_vocabulary() {
        assert_eq!(META.grammar_file_name(), "Mini.g4");
        assert_eq!(META.vocabulary().display_name(1), "'x'");
    }
}
