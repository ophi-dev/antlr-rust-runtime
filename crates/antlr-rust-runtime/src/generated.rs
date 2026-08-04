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

/// Defines the grammar-independent facade and trait delegation for one
/// generated lexer.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_lexer_facade {
    (
        type: $lexer:ident<$input:ident, $hooks:ident>,
        fields: {
            base: $base:ident,
            hooks: $hooks_field:ident $(,)?
        },
        metadata: $metadata:path,
        next_token($this:ident, $sink:ident) $next_token:block
        $(,)?
    ) => {
        impl<$input, $hooks> $lexer<$input, $hooks>
        where
            $input: $crate::char_stream::CharStream,
            $hooks: $crate::parser::SemanticHooks,
        {
            pub fn metadata() -> &'static $crate::generated::GrammarMetadata {
                $metadata()
            }

            /// Adds a listener for lexer diagnostics.
            pub fn add_error_listener<T>(&mut self, listener: T)
            where
                T: for<'a> $crate::errors::ErrorListener<dyn $crate::recognizer::Recognizer + 'a>
                    + ::core::marker::Send
                    + 'static,
            {
                $crate::recognizer::Recognizer::add_error_listener(&mut self.$base, listener);
            }

            /// Removes every lexer error listener, including the default console listener.
            pub fn remove_error_listeners(&mut self) {
                $crate::recognizer::Recognizer::remove_error_listeners(&mut self.$base);
            }

            /// Routes every token through ATN interpretation instead of the compiled
            /// lexer DFA, so the learned-DFA trace (`lexer_dfa_string`) observes each
            /// match.
            pub fn set_force_interpreted(&mut self, force_interpreted: bool) {
                self.$base.set_force_interpreted(force_interpreted);
            }

            /// Resets this lexer and any caller-owned lifecycle state for reuse.
            pub fn reset(&mut self) {
                if <$hooks as $crate::parser::SemanticHooks>::ENABLES_LEXER_LIFECYCLE {
                    $crate::atn::lexer::reset_with_semantic_hooks(
                        &mut self.$base,
                        &mut self.$hooks_field,
                    );
                } else {
                    self.$base.reset();
                }
            }

            /// Replaces the input stream and resets runtime and lifecycle state.
            pub fn set_input_stream(&mut self, input: $input) {
                if <$hooks as $crate::parser::SemanticHooks>::ENABLES_LEXER_LIFECYCLE {
                    $crate::atn::lexer::set_input_stream_with_semantic_hooks(
                        &mut self.$base,
                        &mut self.$hooks_field,
                        input,
                    );
                } else {
                    self.$base.set_input_stream(input);
                }
            }

            /// Clears the learned lexer DFA shared by this grammar.
            pub fn clear_dfa(&self) {
                self.$base.clear_dfa();
            }
        }

        impl<$input, $hooks> $crate::generated::GeneratedLexer for $lexer<$input, $hooks>
        where
            $input: $crate::char_stream::CharStream,
            $hooks: $crate::parser::SemanticHooks,
        {
            fn metadata() -> &'static $crate::generated::GrammarMetadata {
                $metadata()
            }
        }

        impl<$input, $hooks> $crate::recognizer::Recognizer for $lexer<$input, $hooks>
        where
            $input: $crate::char_stream::CharStream,
            $hooks: $crate::parser::SemanticHooks,
        {
            fn data(&self) -> &$crate::recognizer::RecognizerData {
                $crate::recognizer::Recognizer::data(&self.$base)
            }

            fn data_mut(&mut self) -> &mut $crate::recognizer::RecognizerData {
                $crate::recognizer::Recognizer::data_mut(&mut self.$base)
            }
        }

        impl<$input, $hooks> $crate::lexer::Lexer for $lexer<$input, $hooks>
        where
            $input: $crate::char_stream::CharStream,
            $hooks: $crate::parser::SemanticHooks,
        {
            fn mode(&self) -> i32 {
                $crate::lexer::Lexer::mode(&self.$base)
            }

            fn set_mode(&mut self, mode: i32) {
                $crate::lexer::Lexer::set_mode(&mut self.$base, mode);
            }

            fn push_mode(&mut self, mode: i32) {
                $crate::lexer::Lexer::push_mode(&mut self.$base, mode);
            }

            fn pop_mode(&mut self) -> ::core::option::Option<i32> {
                $crate::lexer::Lexer::pop_mode(&mut self.$base)
            }
        }

        impl<$input, $hooks> $crate::token::TokenSource for $lexer<$input, $hooks>
        where
            $input: $crate::char_stream::CharStream,
            $hooks: $crate::parser::SemanticHooks,
        {
            fn next_token(
                &mut self,
                $sink: &mut $crate::token::TokenSink<'_>,
            ) -> ::core::result::Result<$crate::token::TokenId, $crate::token::TokenStoreError>
            {
                let $this = self;
                $next_token
            }

            fn line(&self) -> usize {
                self.$base.line()
            }

            fn column(&self) -> usize {
                self.$base.column()
            }

            fn source_name(&self) -> &str {
                self.$base.source_name()
            }

            fn source_text(&self) -> ::core::option::Option<::std::rc::Rc<str>> {
                self.$base.source_text()
            }

            fn drain_errors(&mut self) -> ::std::vec::Vec<$crate::token::TokenSourceError> {
                self.$base.drain_errors()
            }

            fn report_error(&self, source_error: &$crate::token::TokenSourceError) -> bool {
                $crate::recognizer::Recognizer::notify_error_listeners(self, source_error.into());
                true
            }

            fn lexer_dfa_string(&self) -> ::std::string::String {
                self.$base.lexer_dfa_string()
            }
        }
    };
}

/// Defines the grammar-independent facade and trait delegation for one
/// generated parser.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_parser_facade {
    (
        type: $parser:ident<$source:ident, $hooks:ident>,
        fields: {
            base: $base:ident,
            simulator: $simulator:ident,
            generated_only: $generated_only:ident $(,)?
        },
        metadata: $metadata:path,
        parser_atn: $parser_atn:path,
        reset($this:ident) $reset:block
        $(,)?
    ) => {
        impl<$source, $hooks> $parser<$source, $hooks>
        where
            $source: $crate::token::TokenSource,
            $hooks: $crate::parser::SemanticHooks,
        {
            pub fn metadata() -> &'static $crate::generated::GrammarMetadata {
                $metadata()
            }

            /// Adds a listener for parser diagnostics.
            pub fn add_error_listener<T>(&mut self, listener: T)
            where
                T: for<'a> $crate::errors::ErrorListener<dyn $crate::recognizer::Recognizer + 'a>
                    + ::core::marker::Send
                    + 'static,
            {
                $crate::recognizer::Recognizer::add_error_listener(&mut self.$base, listener);
            }

            /// Removes every parser error listener, including the default console listener.
            pub fn remove_error_listeners(&mut self) {
                $crate::recognizer::Recognizer::remove_error_listeners(&mut self.$base);
            }

            /// Registers a listener for committed rule enter/exit events during
            /// recognition (ANTLR's `addParseListener`). See
            /// [`antlr4_runtime::ParseListener`] for the delivery contract.
            pub fn add_parse_listener<T>(&mut self, listener: T)
            where
                T: $crate::parser::ParseListener + 'static,
            {
                self.$base.add_parse_listener(listener);
            }

            /// Removes every registered parse listener and returns them, dropping
            /// any sticky abort a removed listener had requested.
            pub fn remove_parse_listeners(
                &mut self,
            ) -> ::std::vec::Vec<::std::boxed::Box<dyn $crate::parser::ParseListener>> {
                self.$base.remove_parse_listeners()
            }

            /// Fully resets parser-owned state and rewinds the current token stream.
            pub fn reset(&mut self) {
                self.$base.reset();
                if let ::core::option::Option::Some(simulator) = self.$simulator.as_mut() {
                    simulator.reset();
                }
                let $this = &mut *self;
                $reset
            }

            /// Replaces the token stream and fully resets parser-owned state.
            pub fn set_token_stream(
                &mut self,
                input: $crate::token_stream::CommonTokenStream<$source>,
            ) {
                self.$base.set_token_stream(input);
                if let ::core::option::Option::Some(simulator) = self.$simulator.as_mut() {
                    simulator.reset();
                }
                let $this = &mut *self;
                $reset
            }

            #[must_use]
            pub const fn token_stream(&self) -> &$crate::token_stream::CommonTokenStream<$source> {
                self.$base.token_stream()
            }

            #[must_use]
            pub const fn token_stream_mut(
                &mut self,
            ) -> &mut $crate::token_stream::CommonTokenStream<$source> {
                self.$base.token_stream_mut()
            }

            #[must_use]
            pub const fn token_store(&self) -> &$crate::token::TokenStore {
                self.$base.token_store()
            }

            #[must_use]
            pub const fn parse_tree_storage(&self) -> &$crate::tree::ParseTreeStorage {
                self.$base.parse_tree_storage()
            }

            #[must_use]
            pub fn prediction_context_stats(&self) -> $crate::prediction::PredictionContextStats {
                self.$simulator.as_ref().map_or_else(
                    $crate::prediction::PredictionContextStats::default,
                    $crate::atn::parser::ParserAtnSimulator::prediction_context_stats,
                )
            }

            #[must_use]
            pub fn parser_dfa_stats(&self) -> $crate::dfa::ParserDfaStats {
                self.$simulator.as_ref().map_or_else(
                    $crate::dfa::ParserDfaStats::default,
                    $crate::atn::parser::ParserAtnSimulator::parser_dfa_stats,
                )
            }

            /// Clears this grammar's learned parser decision DFAs.
            pub fn clear_dfa(&mut self) {
                if let ::core::option::Option::Some(simulator) = self.$simulator.as_mut() {
                    simulator.clear_dfa();
                } else {
                    $crate::atn::parser::ParserAtnSimulator::clear_shared_dfa($parser_atn());
                }
                let $this = &mut *self;
                $reset
            }

            #[must_use]
            pub fn node(&self, id: $crate::tree::NodeId) -> $crate::tree::Node<'_> {
                self.$base.node(id)
            }

            #[must_use]
            pub fn into_token_stream(self) -> $crate::token_stream::CommonTokenStream<$source> {
                self.$base.into_token_stream()
            }

            #[must_use]
            pub fn into_token_store(self) -> $crate::token::TokenStore {
                self.$base.into_token_store()
            }

            #[must_use]
            pub fn into_parsed_file(self, root: $crate::tree::NodeId) -> $crate::tree::ParsedFile {
                self.$base.into_parsed_file(root)
            }

            /// Compiles a tree pattern rooted at parser rule `rule_index`.
            ///
            /// Mirrors ANTLR's `Parser.compileParseTreePattern`. Literal chunks of
            /// `pattern` are lexed with a fresh lexer built by `make_lexer`.
            pub fn compile_parse_tree_pattern<PL>(
                &self,
                pattern: &str,
                rule_index: usize,
                mut make_lexer: impl ::core::ops::FnMut($crate::char_stream::InputStream) -> PL,
            ) -> ::core::result::Result<
                $crate::tree_pattern::ParseTreePattern,
                $crate::tree_pattern::ParseTreePatternError,
            >
            where
                PL: $crate::token::TokenSource,
            {
                static PATTERN_DATA: ::std::sync::OnceLock<$crate::recognizer::RecognizerData> =
                    ::std::sync::OnceLock::new();
                static PATTERN_MATCHER: ::std::sync::OnceLock<
                    $crate::tree_pattern::ParseTreePatternMatcher<'static>,
                > = ::std::sync::OnceLock::new();
                let matcher = match PATTERN_MATCHER.get() {
                    ::core::option::Option::Some(matcher) => matcher,
                    ::core::option::Option::None => {
                        let data = PATTERN_DATA.get_or_init(|| $metadata().recognizer_data());
                        let matcher = $crate::tree_pattern::ParseTreePatternMatcher::new(
                            $parser_atn(),
                            data,
                        )?;
                        PATTERN_MATCHER.get_or_init(|| matcher)
                    }
                };
                matcher.compile(pattern, rule_index, move |text: &str| {
                    $crate::tree_pattern::lex_pattern_chunk(text, &mut make_lexer)
                })
            }

            #[allow(dead_code)]
            fn simulator(&mut self) -> &mut $crate::atn::parser::ParserAtnSimulator<'static> {
                self.$simulator.get_or_insert_with(|| {
                    $crate::atn::parser::ParserAtnSimulator::new_shared($parser_atn())
                })
            }

            #[allow(dead_code)]
            fn generated_only(&self) -> bool {
                self.$generated_only
            }
        }

        impl<$source, $hooks> $crate::generated::GeneratedParser for $parser<$source, $hooks>
        where
            $source: $crate::token::TokenSource,
            $hooks: $crate::parser::SemanticHooks,
        {
            fn metadata() -> &'static $crate::generated::GrammarMetadata {
                $metadata()
            }

            fn parser_atn() -> &'static $crate::atn::parser_atn::ParserAtn {
                $parser_atn()
            }
        }

        impl<$source, $hooks> $crate::recognizer::Recognizer for $parser<$source, $hooks>
        where
            $source: $crate::token::TokenSource,
            $hooks: $crate::parser::SemanticHooks,
        {
            fn data(&self) -> &$crate::recognizer::RecognizerData {
                $crate::recognizer::Recognizer::data(&self.$base)
            }

            fn data_mut(&mut self) -> &mut $crate::recognizer::RecognizerData {
                $crate::recognizer::Recognizer::data_mut(&mut self.$base)
            }
        }

        impl<$source, $hooks> $crate::parser::Parser for $parser<$source, $hooks>
        where
            $source: $crate::token::TokenSource,
            $hooks: $crate::parser::SemanticHooks,
        {
            fn build_parse_trees(&self) -> bool {
                $crate::parser::Parser::build_parse_trees(&self.$base)
            }

            fn set_build_parse_trees(&mut self, build: bool) {
                $crate::parser::Parser::set_build_parse_trees(&mut self.$base, build);
            }

            fn number_of_syntax_errors(&self) -> usize {
                $crate::parser::Parser::number_of_syntax_errors(&self.$base)
            }

            fn report_diagnostic_errors(&self) -> bool {
                $crate::parser::Parser::report_diagnostic_errors(&self.$base)
            }

            fn set_report_diagnostic_errors(&mut self, report: bool) {
                $crate::parser::Parser::set_report_diagnostic_errors(&mut self.$base, report);
            }

            fn prediction_mode(&self) -> $crate::parser::PredictionMode {
                $crate::parser::Parser::prediction_mode(&self.$base)
            }

            fn set_prediction_mode(&mut self, mode: $crate::parser::PredictionMode) {
                $crate::parser::Parser::set_prediction_mode(&mut self.$base, mode);
            }

            fn max_rule_depth(&self) -> ::core::option::Option<usize> {
                $crate::parser::Parser::max_rule_depth(&self.$base)
            }

            fn set_max_rule_depth(&mut self, depth: ::core::option::Option<usize>) {
                $crate::parser::Parser::set_max_rule_depth(&mut self.$base, depth);
            }

            fn add_parse_listener(
                &mut self,
                listener: ::std::boxed::Box<dyn $crate::parser::ParseListener>,
            ) {
                $crate::parser::Parser::add_parse_listener(&mut self.$base, listener);
            }

            fn remove_parse_listeners(
                &mut self,
            ) -> ::std::vec::Vec<::std::boxed::Box<dyn $crate::parser::ParseListener>> {
                $crate::parser::Parser::remove_parse_listeners(&mut self.$base)
            }
        }
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

    // Compile-only fixture: successful expansion of both facade macros with
    // invocation-site prelude names shadowed is the assertion.
    #[allow(dead_code, unreachable_pub)]
    mod facade_hygiene {
        struct Box;
        struct FnMut;
        struct None;
        struct Option;
        struct Rc;
        struct Result;
        struct Send;
        struct Some;
        struct String;
        struct Vec;

        struct HygieneLexer<I, H> {
            base: crate::lexer::BaseLexer<I>,
            hooks: H,
        }

        struct HygieneParser<S, H> {
            base: crate::parser::BaseParser<S, H>,
            simulator: ::core::option::Option<crate::atn::parser::ParserAtnSimulator<'static>>,
            generated_only: bool,
        }

        fn metadata() -> &'static crate::generated::GrammarMetadata {
            &super::META
        }

        fn parser_atn() -> &'static crate::atn::parser_atn::ParserAtn {
            panic!("compile-only facade hygiene fixture")
        }

        crate::__antlr4_rust_lexer_facade! {
            type: HygieneLexer<I, H>,
            fields: {
                base: base,
                hooks: hooks,
            },
            metadata: metadata,
            next_token(_lexer, _sink) {
                panic!("compile-only facade hygiene fixture")
            }
        }

        crate::__antlr4_rust_parser_facade! {
            type: HygieneParser<S, H>,
            fields: {
                base: base,
                simulator: simulator,
                generated_only: generated_only,
            },
            metadata: metadata,
            parser_atn: parser_atn,
            reset(_parser) {}
        }
    }

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
