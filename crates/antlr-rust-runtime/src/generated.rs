use std::sync::{Arc, OnceLock};

use crate::atn::parser_atn::ParserAtn;
use crate::atn::serialized::SerializedAtn;
use crate::recognizer::{RecognizerData, RecognizerMetadata};
use crate::vocabulary::Vocabulary;

/// Defines the grammar-independent storage, conversion, and accessor mechanics
/// for one generated typed parser context.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_context {
    (
        pub struct $context:ident {
            rule_index: $rule_index:expr,
            context_kind: $kind_mode:ident $(($kind:expr))?,
            attributes: {
                $(
                    $attrs:ident {
                        $($field:ident: $field_ty:ty),+ $(,)?
                    }
                )?
            },
            methods: {
                rule_node: $rule_node_method:ident,
                child_count: $child_count_method:ident,
                direct_terminals: $direct_terminals_method:ident,
                start: $start_method:ident,
                text: $text_method:ident $(,)?
            }
        }
    ) => {
        #[allow(non_camel_case_types, dead_code)]
        #[derive(Clone)]
        pub struct $context<'a, State = StoredTreeContext> {
            __node: __GeneratedRuleContext<'a>,
            __invocation_states: Option<Vec<isize>>,
            __state: std::marker::PhantomData<State>,
            $(
                $(pub $field: $field_ty,)+
            )?
        }

        impl<'a> $crate::FromRuleNode<'a> for $context<'a> {
            fn from_rule_node(node: $crate::RuleNodeView<'a>) -> Option<Self> {
                if node.rule_index() != $rule_index
                    || $crate::__antlr4_rust_context!(
                        @stored_kind_mismatch $kind_mode $(($kind))?, node
                    )
                {
                    return None;
                }
                Some(Self::__from_node(node))
            }
        }

        impl<'a> $crate::AsRuleNode<'a> for $context<'a> {
            fn as_rule_node(&self) -> $crate::RuleNodeView<'a> {
                self.$rule_node_method()
            }
        }

        impl<'a> $context<'a> {
            pub fn $rule_node_method(&self) -> $crate::RuleNodeView<'a> {
                match self.__node {
                    __GeneratedRuleContext::Stored(node) => node,
                    __GeneratedRuleContext::Active { .. } => {
                        unreachable!("stored context type contains an active parser context")
                    }
                }
            }
        }

        impl<'a> __FromActiveRuleContext<'a> for $context<'a, __ActiveParserContext> {
            fn __from_active(
                context: &'a $crate::ParserRuleContext,
                live_attrs: Option<&dyn std::any::Any>,
                invocation_states: Vec<isize>,
                storage: &'a $crate::ParseTreeStorage,
                tokens: &'a $crate::TokenStore,
            ) -> Option<Self> {
                if context.rule_index() != $rule_index
                    || $crate::__antlr4_rust_context!(
                        @active_kind_mismatch
                        $kind_mode $(($kind))?,
                        context,
                        storage,
                        tokens
                    )
                {
                    return None;
                }
                $(
                    let __default = <$attrs>::default();
                    let __attrs = match live_attrs {
                        Some(live_attrs) => live_attrs
                            .downcast_ref::<$attrs>()
                            .expect("active context attributes match the parser rule"),
                        None => context
                            .generated_attrs::<$attrs>()
                            .unwrap_or(&__default),
                    };
                )?
                Some(Self {
                    __node: __GeneratedRuleContext::Active {
                        context,
                        storage,
                        tokens,
                    },
                    __invocation_states: Some(invocation_states),
                    __state: std::marker::PhantomData,
                    $(
                        $($field: __attrs.$field.clone(),)+
                    )?
                })
            }
        }

        impl<'a> FromValidatedRuleNode<'a> for $context<'a, ValidatedTreeContext> {
            fn from_validated_rule_node(node: ValidatedRuleNode<'a>) -> Option<Self> {
                let node = node.rule_node();
                if node.rule_index() != $rule_index
                    || $crate::__antlr4_rust_context!(
                        @stored_kind_mismatch $kind_mode $(($kind))?, node
                    )
                {
                    return None;
                }
                Some(Self::__from_validated_node(node))
            }
        }

        impl<'a> $crate::AsRuleNode<'a> for $context<'a, ValidatedTreeContext> {
            fn as_rule_node(&self) -> $crate::RuleNodeView<'a> {
                self.$rule_node_method()
            }
        }

        #[allow(dead_code, clippy::all)]
        impl<'a> $context<'a> {
            fn __from_node(node: $crate::RuleNodeView<'a>) -> Self {
                Self::__from_node_with_invocation_states(node, None)
            }

            fn __from_child_node(
                node: $crate::RuleNodeView<'a>,
                parent_invocation_states: Option<&[isize]>,
            ) -> Self {
                let invocation_states = parent_invocation_states.map(|states| {
                    let mut invocation_states = Vec::with_capacity(states.len() + 1);
                    invocation_states.push(node.invoking_state());
                    invocation_states.extend_from_slice(states);
                    invocation_states
                });
                Self::__from_node_with_invocation_states(node, invocation_states)
            }

            fn __from_listener_node(
                node: $crate::RuleNodeView<'a>,
                invocation_states: Option<&[isize]>,
            ) -> Self {
                Self::__from_node_with_invocation_states(
                    node,
                    invocation_states.map(<[isize]>::to_vec),
                )
            }

            fn __from_node_with_invocation_states(
                node: $crate::RuleNodeView<'a>,
                invocation_states: Option<Vec<isize>>,
            ) -> Self {
                $(
                    let __default = <$attrs>::default();
                    let __attrs = node.generated_attrs::<$attrs>().unwrap_or(&__default);
                )?
                Self {
                    __node: __GeneratedRuleContext::Stored(node),
                    __invocation_states: invocation_states,
                    __state: std::marker::PhantomData,
                    $(
                        $($field: __attrs.$field.clone(),)+
                    )?
                }
            }
        }

        #[allow(dead_code, clippy::all)]
        impl<'a, State> $context<'a, State> {
            pub fn $child_count_method(&self) -> usize {
                match &self.__node {
                    __GeneratedRuleContext::Stored(node) => node.child_count(),
                    __GeneratedRuleContext::Active { context, .. } => context.child_count(),
                }
            }

            /// Iterates terminals owned directly by this context without
            /// descending into nested rule contexts.
            ///
            /// Recovered trees expose inserted and deleted recovery tokens as
            /// error nodes through the same `TerminalNode` surface. Use
            /// `TerminalNode::is_error()` to identify recovery nodes and
            /// `TerminalNode::is_missing()` to identify inserted synthetic
            /// tokens.
            pub fn $direct_terminals_method(
                &self,
            ) -> impl Iterator<Item = TerminalNode<'a>> + 'a + use<'a, State> {
                __terminal_children(self.__node).map(TerminalNode::new)
            }

            pub fn $start_method(&self) -> __GeneratedTokenView {
                let token = match &self.__node {
                    __GeneratedRuleContext::Stored(node) => node.start(),
                    __GeneratedRuleContext::Active {
                        context, tokens, ..
                    } => context.start(tokens),
                };
                __GeneratedTokenView {
                    text: token
                        .map(|token| token.text_or_empty().to_owned())
                        .unwrap_or_default(),
                }
            }

            pub fn $text_method(&self) -> String {
                match &self.__node {
                    __GeneratedRuleContext::Stored(node) => node.text(),
                    __GeneratedRuleContext::Active {
                        context,
                        storage,
                        tokens,
                    } => context.text(storage, tokens),
                }
            }
        }

        #[allow(dead_code, clippy::all)]
        impl<'a> $context<'a, ValidatedTreeContext> {
            fn __from_validated_node(node: $crate::RuleNodeView<'a>) -> Self {
                Self::__from_validated_node_with_invocation_states(node, None)
            }

            fn __from_validated_child_node(
                node: $crate::RuleNodeView<'a>,
                parent_invocation_states: Option<&[isize]>,
            ) -> Self {
                let invocation_states = parent_invocation_states.map(|states| {
                    let mut invocation_states = Vec::with_capacity(states.len() + 1);
                    invocation_states.push(node.invoking_state());
                    invocation_states.extend_from_slice(states);
                    invocation_states
                });
                Self::__from_validated_node_with_invocation_states(node, invocation_states)
            }

            fn __from_validated_listener_node(
                node: $crate::RuleNodeView<'a>,
                invocation_states: Option<&[isize]>,
            ) -> Self {
                Self::__from_validated_node_with_invocation_states(
                    node,
                    invocation_states.map(<[isize]>::to_vec),
                )
            }

            fn __from_validated_node_with_invocation_states(
                node: $crate::RuleNodeView<'a>,
                invocation_states: Option<Vec<isize>>,
            ) -> Self {
                $(
                    let __default = <$attrs>::default();
                    let __attrs = node.generated_attrs::<$attrs>().unwrap_or(&__default);
                )?
                Self {
                    __node: __GeneratedRuleContext::Stored(node),
                    __invocation_states: invocation_states,
                    __state: std::marker::PhantomData,
                    $(
                        $($field: __attrs.$field.clone(),)+
                    )?
                }
            }

            pub fn $rule_node_method(&self) -> $crate::RuleNodeView<'a> {
                match self.__node {
                    __GeneratedRuleContext::Stored(node) => node,
                    __GeneratedRuleContext::Active { .. } => {
                        unreachable!("validated context contains an active parser context")
                    }
                }
            }
        }

        impl<State> std::fmt::Display for $context<'_, State> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match &self.__invocation_states {
                    Some(states) => __write_invocation_states(f, states.iter().copied()),
                    None => match self.__node {
                        __GeneratedRuleContext::Stored(node) => {
                            __write_invocation_states(f, node.invocation_states())
                        }
                        __GeneratedRuleContext::Active { .. } => {
                            unreachable!("active context is missing invocation states")
                        }
                    },
                }
            }
        }
    };
    (@stored_kind_mismatch any, $node:expr) => {
        false
    };
    (@stored_kind_mismatch exact($kind:expr), $node:expr) => {
        __context_kind($node) != $kind
    };
    (@active_kind_mismatch any, $context:expr, $storage:expr, $tokens:expr) => {
        false
    };
    (@active_kind_mismatch exact($kind:expr), $context:expr, $storage:expr, $tokens:expr) => {
        __active_context_kind($context, $storage, $tokens) != $kind
    };
}

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
