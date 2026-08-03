use std::sync::{Arc, OnceLock};

use crate::atn::parser_atn::ParserAtn;
use crate::atn::serialized::SerializedAtn;
use crate::recognizer::{RecognizerData, RecognizerMetadata};
use crate::vocabulary::Vocabulary;

#[derive(Debug)]
pub struct GrammarMetadata {
    grammar_file_name: &'static str,
    rule_names: &'static [&'static str],
    literal_names: &'static [Option<&'static str>],
    symbolic_names: &'static [Option<&'static str>],
    display_names: &'static [Option<&'static str>],
    channel_names: &'static [&'static str],
    mode_names: &'static [&'static str],
    serialized_atn: &'static [i32],
    recognizer_metadata: OnceLock<Arc<RecognizerMetadata>>,
}

impl Clone for GrammarMetadata {
    fn clone(&self) -> Self {
        Self {
            grammar_file_name: self.grammar_file_name,
            rule_names: self.rule_names,
            literal_names: self.literal_names,
            symbolic_names: self.symbolic_names,
            display_names: self.display_names,
            channel_names: self.channel_names,
            mode_names: self.mode_names,
            serialized_atn: self.serialized_atn,
            recognizer_metadata: OnceLock::from(Arc::clone(self.cached_recognizer_metadata())),
        }
    }
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
            recognizer_metadata: OnceLock::new(),
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

    /// Creates per-instance recognizer state backed by this grammar's cached
    /// immutable metadata.
    pub fn recognizer_data(&self) -> RecognizerData {
        RecognizerData::from_shared(Arc::clone(self.cached_recognizer_metadata()))
    }

    fn cached_recognizer_metadata(&self) -> &Arc<RecognizerMetadata> {
        self.recognizer_metadata.get_or_init(|| {
            Arc::new(RecognizerMetadata::from_static(
                self.grammar_file_name,
                self.rule_names,
                self.channel_names,
                self.mode_names,
                self.vocabulary(),
            ))
        })
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

    /// Borrows the validated packed ATN embedded by the matching generator.
    fn parser_atn() -> &'static ParserAtn;
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

    #[test]
    fn cloned_metadata_shares_the_cache_before_explicit_initialization() {
        let original = GrammarMetadata::new(
            "Clone.g4",
            &["start"],
            &[None, Some("'x'")],
            &[None, Some("X")],
            &[None, None],
            &["DEFAULT_TOKEN_CHANNEL"],
            &["DEFAULT_MODE"],
            &[],
        );
        let cloned = original.clone();
        let first = original.recognizer_data();
        let second = cloned.recognizer_data();

        assert!(std::ptr::eq(first.rule_names(), second.rule_names()));
        assert!(std::ptr::eq(first.vocabulary(), second.vocabulary()));
    }
}
