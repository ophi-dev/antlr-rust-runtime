// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::any::Any;
use std::fmt;
use std::sync::{Arc, OnceLock};

use crate::atn::parser_atn::ParserAtn;
use crate::atn::serialized::SerializedAtn;
use crate::char_stream::{CharStream, InputStream};
use crate::errors::AntlrError;
use crate::int_stream::IntStream;
use crate::parser::{BaseParser, SemanticHooks, grow_generated_rule_stack};
use crate::recognizer::{RecognizerData, RecognizerMetadata};
use crate::token::{Token as _, TokenSource, TokenStore, TokenView};
use crate::token_stream::CommonTokenStream;
use crate::tree::{
    ErrorNodeView, Node, NodeId, NodeKind, ParseTreeStorage, ParserRuleContext, RuleNodeView,
    TerminalNodeView,
};
use crate::validated::ValidationError;
use crate::vocabulary::Vocabulary;

/// Defines the grammar-independent storage, conversion, and accessor mechanics
/// for one generated typed parser context.
///
/// The optional `validated_downcast: branded,` field selects the revision-10
/// validated-downcast impl against the runtime-owned branded
/// [`crate::validated`] surface; without it the expansion targets the
/// module-local validated types that generated-code API revisions 9 and
/// earlier declare themselves.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_context {
    (
        pub struct $context:ident {
            rule_index: $rule_index:expr,
            context_kind: $kind_mode:ident $(($kind:expr))?,
            $(validated_downcast: $validated_downcast:ident,)?
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

        $crate::__antlr4_rust_context!(
            @from_validated $($validated_downcast)?,
            $context,
            $rule_index,
            $kind_mode $(($kind))?
        );

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
    // Legacy validated-downcast impl (generated-code API revisions 9 and
    // earlier): `FromValidatedRuleNode` and `ValidatedRuleNode` resolve to the
    // invoking module's own definitions.
    (@from_validated , $context:ident, $rule_index:expr, $kind_mode:ident $(($kind:expr))?) => {
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
    };
    // Branded validated-downcast impl (revision 10 and later): the runtime
    // owns the trait and node type, and the module-local
    // `ValidatedTreeContext` marker brands them for this grammar so
    // `downcast_ref` cannot resolve a node against another grammar's
    // contexts.
    (@from_validated branded, $context:ident, $rule_index:expr, $kind_mode:ident $(($kind:expr))?) => {
        impl<'a> $crate::FromValidatedRuleNode<'a> for $context<'a, ValidatedTreeContext> {
            type Grammar = ValidatedTreeContext;

            fn from_validated_rule_node(
                node: $crate::ValidatedRuleNode<'a, ValidatedTreeContext>,
            ) -> Option<Self> {
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

/// Expands one declarative accessor list into both state-variant impls of a
/// generated typed parser context: the recovery-oriented impl (generic over
/// the module's `__RecoveryContextState`, returning `Result<_,
/// MissingChildError>` for required children) and the validated impl
/// (`ValidatedTreeContext`, returning required children directly).
///
/// Accessor declarations, one per generated method:
///
/// ```text
/// rule <method>: required(<ChildContext>[<child rule index>], "<child name>"),
/// rule <method>: optional(<ChildContext>[<child rule index>]),
/// rule <method>: many(<ChildContext>[<child rule index>]),
/// token <method>: required(<token type>, "<token name>"),
/// token <method>: optional(<token type>),
/// token <method>: many(<token type>),
/// label_rule <method>: required(<selector>, <ChildContext>[<child rule index>], "<label>"),
/// label_rule <method>: optional(<selector>, <ChildContext>[<child rule index>]),
/// label_rule <method>: many(skip(<n>), <ChildContext>[<child rule index>]),
/// label_token <method>: required(<selector>, [<token types>], "<label>"),
/// label_token <method>: optional(<selector>, [<token types>]),
/// label_token <method>: many(skip(<n>), [<token types>]),
/// ```
///
/// where `<selector>` is `nth(<n>)` or `last_after(<n>)`. All names, indices,
/// and token types are grammar data supplied by the generated invocation. The
/// support items (`__rule_children`, `__token_children`,
/// `__labeled_token_children`, `__labeled_token_children_matching`,
/// `TerminalNode`, `__RecoveryContextState`) are runtime-owned
/// [`crate::generated`] items the generated module imports by name, and
/// `ValidatedRuleNode`/`FromValidatedRuleNode` resolve in the generated
/// module's scope (module-local definitions through generated-code API
/// revision 9; from revision 10, a module-local alias of
/// [`crate::validated::ValidatedRuleNode`] branded with the module's
/// `ValidatedTreeContext` marker, plus a re-export of
/// [`crate::validated::FromValidatedRuleNode`]); `ValidatedTreeContext` plus the
/// `__from_child_node`/`__from_validated_child_node` constructors and the
/// `__node`/`__invocation_states` context fields stay emitted per generated
/// module (the validated marker must remain crate-local for the two impl
/// blocks below to be coherent), exactly as `__antlr4_rust_context!` relies
/// on.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_context_accessors {
    (
        $context:ident {
            $( $kind:tt $method:ident: $card:tt $payload:tt ),* $(,)?
        }
    ) => {
        #[allow(dead_code, private_bounds, clippy::all)]
        impl<'a, State: __RecoveryContextState> $context<'a, State> {
            $(
                $crate::__antlr4_rust_context_accessors!(
                    @recovered $context, $kind $card $method $payload
                );
            )*
        }

        #[allow(dead_code, clippy::all)]
        impl<'a> $context<'a, ValidatedTreeContext> {
            $(
                $crate::__antlr4_rust_context_accessors!(
                    @validated $context, $kind $card $method $payload
                );
            )*
        }
    };

    // Recovery-oriented rule children.
    (@recovered $context:ident, rule required $method:ident ($child:ident[$index:expr], $name:literal)) => {
        pub fn $method(&self) -> Result<$child<'a>, $crate::MissingChildError> {
            __rule_children(self.__node, $index)
                .next()
                .map(|node| $child::__from_child_node(node, self.__invocation_states.as_deref()))
                .ok_or_else(|| $crate::MissingChildError::new(stringify!($context), $name))
        }
    };
    (@recovered $context:ident, rule optional $method:ident ($child:ident[$index:expr])) => {
        pub fn $method(&self) -> Option<$child<'a>> {
            __rule_children(self.__node, $index)
                .next()
                .map(|node| $child::__from_child_node(node, self.__invocation_states.as_deref()))
        }
    };
    (@recovered $context:ident, rule many $method:ident ($child:ident[$index:expr])) => {
        pub fn $method(&self) -> impl Iterator<Item = $child<'a>> + '_ {
            __rule_children(self.__node, $index)
                .map(move |node| $child::__from_child_node(node, self.__invocation_states.as_deref()))
        }
    };

    // Recovery-oriented token children.
    (@recovered $context:ident, token required $method:ident ($token_type:expr, $name:literal)) => {
        pub fn $method(&self) -> Result<TerminalNode<'a>, $crate::MissingChildError> {
            __token_children(self.__node, $token_type)
                .next()
                .map(TerminalNode::new)
                .ok_or_else(|| $crate::MissingChildError::new(stringify!($context), $name))
        }
    };
    (@recovered $context:ident, token optional $method:ident ($token_type:expr)) => {
        pub fn $method(&self) -> Option<TerminalNode<'a>> {
            __token_children(self.__node, $token_type)
                .next()
                .map(TerminalNode::new)
        }
    };
    (@recovered $context:ident, token many $method:ident ($token_type:expr)) => {
        pub fn $method(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {
            __token_children(self.__node, $token_type).map(TerminalNode::new)
        }
    };

    // Recovery-oriented labeled rule children.
    (@recovered $context:ident, label_rule required $method:ident ($sel:ident($selarg:expr), $child:ident[$index:expr], $name:literal)) => {
        pub fn $method(&self) -> Result<$child<'a>, $crate::MissingChildError> {
            $crate::__antlr4_rust_context_accessors!(
                @selected(__rule_children(self.__node, $index)) $sel($selarg)
            )
            .map(|node| $child::__from_child_node(node, self.__invocation_states.as_deref()))
            .ok_or_else(|| $crate::MissingChildError::new(stringify!($context), $name))
        }
    };
    (@recovered $context:ident, label_rule optional $method:ident ($sel:ident($selarg:expr), $child:ident[$index:expr])) => {
        pub fn $method(&self) -> Option<$child<'a>> {
            $crate::__antlr4_rust_context_accessors!(
                @selected(__rule_children(self.__node, $index)) $sel($selarg)
            )
            .map(|node| $child::__from_child_node(node, self.__invocation_states.as_deref()))
        }
    };
    (@recovered $context:ident, label_rule many $method:ident (skip($skip:expr), $child:ident[$index:expr])) => {
        pub fn $method(&self) -> impl Iterator<Item = $child<'a>> + '_ {
            __rule_children(self.__node, $index)
                .skip($skip)
                .map(move |node| $child::__from_child_node(node, self.__invocation_states.as_deref()))
        }
    };

    // Recovery-oriented labeled token children.
    (@recovered $context:ident, label_token required $method:ident ($sel:ident($selarg:expr), $tokens:tt, $name:literal)) => {
        pub fn $method(&self) -> Result<TerminalNode<'a>, $crate::MissingChildError> {
            $crate::__antlr4_rust_context_accessors!(
                @selected($crate::__antlr4_rust_context_accessors!(
                    @labeled_token_children(self.__node) $tokens
                )) $sel($selarg)
            )
            .map(TerminalNode::new)
            .ok_or_else(|| $crate::MissingChildError::new(stringify!($context), $name))
        }
    };
    (@recovered $context:ident, label_token optional $method:ident ($sel:ident($selarg:expr), $tokens:tt)) => {
        pub fn $method(&self) -> Option<TerminalNode<'a>> {
            $crate::__antlr4_rust_context_accessors!(
                @selected($crate::__antlr4_rust_context_accessors!(
                    @labeled_token_children(self.__node) $tokens
                )) $sel($selarg)
            )
            .map(TerminalNode::new)
        }
    };
    (@recovered $context:ident, label_token many $method:ident (skip($skip:expr), $tokens:tt)) => {
        pub fn $method(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {
            $crate::__antlr4_rust_context_accessors!(
                @labeled_token_children(self.__node) $tokens
            )
            .skip($skip)
            .map(TerminalNode::new)
        }
    };

    // Validated rule children.
    (@validated $context:ident, rule required $method:ident ($child:ident[$index:expr], $name:literal)) => {
        pub fn $method(&self) -> $child<'a, ValidatedTreeContext> {
            let Some(node) = __rule_children(self.__node, $index).next() else {
                unreachable!(concat!(
                    "validated ",
                    stringify!($context),
                    " is missing required child ",
                    $name
                ))
            };
            $child::<ValidatedTreeContext>::__from_validated_child_node(
                node,
                self.__invocation_states.as_deref(),
            )
        }
    };
    (@validated $context:ident, rule optional $method:ident ($child:ident[$index:expr])) => {
        pub fn $method(&self) -> Option<$child<'a, ValidatedTreeContext>> {
            __rule_children(self.__node, $index)
                .next()
                .map(|node| {
                    $child::<ValidatedTreeContext>::__from_validated_child_node(
                        node,
                        self.__invocation_states.as_deref(),
                    )
                })
        }
    };
    (@validated $context:ident, rule many $method:ident ($child:ident[$index:expr])) => {
        pub fn $method(&self) -> impl Iterator<Item = $child<'a, ValidatedTreeContext>> + '_ {
            __rule_children(self.__node, $index).map(move |node| {
                $child::<ValidatedTreeContext>::__from_validated_child_node(
                    node,
                    self.__invocation_states.as_deref(),
                )
            })
        }
    };

    // Validated token children.
    (@validated $context:ident, token required $method:ident ($token_type:expr, $name:literal)) => {
        pub fn $method(&self) -> TerminalNode<'a> {
            let Some(node) = __token_children(self.__node, $token_type).next() else {
                unreachable!(concat!(
                    "validated ",
                    stringify!($context),
                    " is missing required child ",
                    $name
                ))
            };
            TerminalNode::new(node)
        }
    };
    (@validated $context:ident, token optional $method:ident ($token_type:expr)) => {
        pub fn $method(&self) -> Option<TerminalNode<'a>> {
            __token_children(self.__node, $token_type)
                .next()
                .map(TerminalNode::new)
        }
    };
    (@validated $context:ident, token many $method:ident ($token_type:expr)) => {
        pub fn $method(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {
            __token_children(self.__node, $token_type).map(TerminalNode::new)
        }
    };

    // Validated labeled rule children.
    (@validated $context:ident, label_rule required $method:ident ($sel:ident($selarg:expr), $child:ident[$index:expr], $name:literal)) => {
        pub fn $method(&self) -> $child<'a, ValidatedTreeContext> {
            let Some(node) = $crate::__antlr4_rust_context_accessors!(
                @selected(__rule_children(self.__node, $index)) $sel($selarg)
            ) else {
                unreachable!(concat!(
                    "validated ",
                    stringify!($context),
                    " is missing required child ",
                    $name
                ))
            };
            $child::<ValidatedTreeContext>::__from_validated_child_node(
                node,
                self.__invocation_states.as_deref(),
            )
        }
    };
    (@validated $context:ident, label_rule optional $method:ident ($sel:ident($selarg:expr), $child:ident[$index:expr])) => {
        pub fn $method(&self) -> Option<$child<'a, ValidatedTreeContext>> {
            $crate::__antlr4_rust_context_accessors!(
                @selected(__rule_children(self.__node, $index)) $sel($selarg)
            )
            .map(|node| {
                $child::<ValidatedTreeContext>::__from_validated_child_node(
                    node,
                    self.__invocation_states.as_deref(),
                )
            })
        }
    };
    (@validated $context:ident, label_rule many $method:ident (skip($skip:expr), $child:ident[$index:expr])) => {
        pub fn $method(&self) -> impl Iterator<Item = $child<'a, ValidatedTreeContext>> + '_ {
            __rule_children(self.__node, $index)
                .skip($skip)
                .map(move |node| {
                    $child::<ValidatedTreeContext>::__from_validated_child_node(
                        node,
                        self.__invocation_states.as_deref(),
                    )
                })
        }
    };

    // Validated labeled token children.
    (@validated $context:ident, label_token required $method:ident ($sel:ident($selarg:expr), $tokens:tt, $name:literal)) => {
        pub fn $method(&self) -> TerminalNode<'a> {
            let Some(node) = $crate::__antlr4_rust_context_accessors!(
                @selected($crate::__antlr4_rust_context_accessors!(
                    @labeled_token_children(self.__node) $tokens
                )) $sel($selarg)
            ) else {
                unreachable!(concat!(
                    "validated ",
                    stringify!($context),
                    " is missing required child ",
                    $name
                ))
            };
            TerminalNode::new(node)
        }
    };
    (@validated $context:ident, label_token optional $method:ident ($sel:ident($selarg:expr), $tokens:tt)) => {
        pub fn $method(&self) -> Option<TerminalNode<'a>> {
            $crate::__antlr4_rust_context_accessors!(
                @selected($crate::__antlr4_rust_context_accessors!(
                    @labeled_token_children(self.__node) $tokens
                )) $sel($selarg)
            )
            .map(TerminalNode::new)
        }
    };
    (@validated $context:ident, label_token many $method:ident (skip($skip:expr), $tokens:tt)) => {
        pub fn $method(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {
            $crate::__antlr4_rust_context_accessors!(
                @labeled_token_children(self.__node) $tokens
            )
            .skip($skip)
            .map(TerminalNode::new)
        }
    };

    // Label occurrence selectors.
    (@selected($children:expr) nth($occurrence:expr)) => {
        $children.nth($occurrence)
    };
    (@selected($children:expr) last_after($skip:expr)) => {
        $children.skip($skip).last()
    };

    // Labeled token child sources: a single token type uses the scalar
    // helper; a set uses the matching helper.
    (@labeled_token_children($node:expr) [$token_type:expr]) => {
        __labeled_token_children($node, $token_type)
    };
    (@labeled_token_children($node:expr) [$($token_type:expr),+ $(,)?]) => {
        __labeled_token_children_matching($node, &[$($token_type),+])
    };

    // Catch-alls that name the offending declaration instead of failing with
    // a bare `no rules expected this token` deep inside a generated module.
    // The structured arms reconstruct the source declaration order; the
    // trailing generic arms catch shapes that do not even parse as one.
    (@recovered $context:ident, $kind:tt $card:tt $method:ident $payload:tt) => {
        compile_error!(concat!(
            "unsupported generated accessor declaration for ",
            stringify!($context),
            ": ",
            stringify!($kind),
            " ",
            stringify!($method),
            ": ",
            stringify!($card),
            stringify!($payload)
        ));
    };
    (@validated $context:ident, $kind:tt $card:tt $method:ident $payload:tt) => {
        compile_error!(concat!(
            "unsupported generated accessor declaration for ",
            stringify!($context),
            ": ",
            stringify!($kind),
            " ",
            stringify!($method),
            ": ",
            stringify!($card),
            stringify!($payload)
        ));
    };
    (@recovered $context:ident, $($declaration:tt)*) => {
        compile_error!(concat!(
            "unsupported generated accessor declaration for ",
            stringify!($context),
            ": ",
            stringify!($($declaration)*)
        ));
    };
    (@validated $context:ident, $($declaration:tt)*) => {
        compile_error!(concat!(
            "unsupported generated accessor declaration for ",
            stringify!($context),
            ": ",
            stringify!($($declaration)*)
        ));
    };
    (@selected($children:expr) $($selector:tt)*) => {
        compile_error!(concat!(
            "unsupported generated accessor selector: ",
            stringify!($($selector)*)
        ))
    };
    (@labeled_token_children($node:expr) $($tokens:tt)*) => {
        compile_error!(concat!(
            "unsupported generated accessor token set: ",
            stringify!($($tokens)*)
        ))
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

        impl<$source, $hooks> $crate::generated::GeneratedRuleParser for $parser<$source, $hooks>
        where
            $source: $crate::token::TokenSource,
            $hooks: $crate::parser::SemanticHooks,
        {
            type Source = $source;
            type Hooks = $hooks;

            fn generated_rule_base(
                &mut self,
            ) -> &mut $crate::parser::BaseParser<Self::Source, Self::Hooks> {
                &mut self.$base
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

/// Defines the grammar-independent parse-driver core for one generated
/// parser: entry/reset bookkeeping, generated-vs-interpreted engine routing,
/// sticky-abort and fail-loud semantic-error draining, and the interpreted
/// fallback with uniform action dispatch through the module's `run_action`.
///
/// This is an implementation detail of `antlr4-rust-gen`, not a stable
/// hand-written parser API. The `fallback` binder block supplied by generated
/// code composes the grammar's `ParserRuntimeOptions` (semantics table,
/// action indices, policy) and must evaluate to
/// `Result<(ParseTree, Vec<ParserAction>), AntlrError>`; this macro owns
/// everything else.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_parser_driver {
    (
        type: $parser:ident<$source:ident, $hooks:ident>,
        fields: {
            base: $base:ident,
            simulator: $simulator:ident $(,)?
        },
        atn: $atn:path,
        adaptive_direct: $adaptive_direct:expr,
        fallback($this:ident, $rule_index:ident, $precedence:ident) $fallback:block
        $(,)?
    ) => {
        impl<$source, $hooks> $parser<$source, $hooks>
        where
            $source: $crate::token::TokenSource,
            $hooks: $crate::parser::SemanticHooks,
        {
            #[allow(dead_code)]
            fn parse_rule(
                &mut self,
                rule_index: usize,
            ) -> ::core::result::Result<$crate::tree::ParseTree, $crate::errors::AntlrError> {
                self.parse_rule_precedence(rule_index, 0)
            }

            #[allow(dead_code)]
            fn parse_rule_precedence(
                &mut self,
                rule_index: usize,
                precedence: i32,
            ) -> ::core::result::Result<$crate::tree::ParseTree, $crate::errors::AntlrError> {
                self.parse_rule_precedence_inner(rule_index, precedence, true)
            }

            #[allow(dead_code)]
            fn parse_rule_precedence_from_generated(
                &mut self,
                rule_index: usize,
                precedence: i32,
            ) -> ::core::result::Result<$crate::tree::ParseTree, $crate::errors::AntlrError> {
                self.parse_rule_precedence_inner(rule_index, precedence, false)
            }

            #[allow(dead_code)]
            fn parse_rule_precedence_inner(
                &mut self,
                rule_index: usize,
                precedence: i32,
                allow_generated_fallback: bool,
            ) -> ::core::result::Result<$crate::tree::ParseTree, $crate::errors::AntlrError> {
                if allow_generated_fallback {
                    // True top-level entry: drop any fail-loud coordinates left by a
                    // previous parse so a reused parser starts clean. Mid-parse the hits
                    // are preserved so a generated parent can surface a recovered child's
                    // fail-loud coordinate at this boundary.
                    self.$base.reset_unknown_semantic_hits();
                    // Likewise drop stale sticky aborts (depth-cap violation,
                    // parse-listener abort): entry rules share one parser instance,
                    // and the flags must not poison the next parse when the previous
                    // one exited through an error path.
                    let _ = self.$base.take_parse_abort();
                }
                let __rule_start = $crate::int_stream::IntStream::index(self.$base.input());
                let __generated_only = self.generated_only();
                let __tree = if let ::core::option::Option::Some(result) =
                    self.parse_generated_rule(rule_index, precedence, allow_generated_fallback)
                {
                    match result {
                        ::core::result::Result::Ok(tree) => tree,
                        ::core::result::Result::Err(error) => {
                            $crate::int_stream::IntStream::seek(self.$base.input(), __rule_start);
                            let __report_error = ::core::matches!(
                                &error,
                                $crate::generated::GeneratedRuleError::Fatal(_)
                            );
                            // A fatal unwind retains recovery diagnostics committed
                            // earlier in this entry. Dispatch them before a semantic
                            // or parser-abort override can return, or they would leak
                            // into the next entry on a reused parser.
                            if allow_generated_fallback && __report_error {
                                self.$base.report_generated_parser_diagnostics();
                            }
                            if allow_generated_fallback {
                                // A sticky abort (depth cap, listener) wins over an
                                // error or semantic miss derived after recovery absorbed
                                // the aborted rule. Drain any masked semantic miss too,
                                // so neither condition poisons the next entry.
                                if let ::core::option::Option::Some(abort) =
                                    self.$base.take_parse_abort()
                                {
                                    let _ = self.$base.take_unknown_semantic_error();
                                    return ::core::result::Result::Err(abort);
                                }
                                // A generated predicate that consulted an unimplemented
                                // hook fails the alternative and surfaces here as a generic
                                // failed-predicate/rule error. Prefer the recorded fail-loud
                                // semantic error when no parser abort occurred.
                                if let ::core::option::Option::Some(semantic_error) =
                                    self.$base.take_unknown_semantic_error()
                                {
                                    return ::core::result::Result::Err(semantic_error);
                                }
                            }
                            let error = error.into_error();
                            if allow_generated_fallback && __report_error {
                                self.$base.report_unrecovered_parser_error(&error);
                            }
                            return ::core::result::Result::Err(error);
                        }
                    }
                } else if __generated_only {
                    return ::core::result::Result::Err($crate::errors::AntlrError::Unsupported(
                        ::std::format!("generated parser did not emit rule {}", rule_index),
                    ));
                } else {
                    self.parse_interpreted_rule_precedence(rule_index, precedence)?
                };
                if allow_generated_fallback {
                    self.$base.report_generated_parser_diagnostics();
                    // A sticky abort (depth-cap violation, listener abort) is not a
                    // syntax error: rule-level recovery may have produced a tree
                    // and semantic miss anyway, but the abort is the root cause. Drain
                    // both sticky conditions before returning so parser reuse is clean.
                    if let ::core::option::Option::Some(error) = self.$base.take_parse_abort() {
                        let _ = self.$base.take_unknown_semantic_error();
                        return ::core::result::Result::Err(error);
                    }
                    // Surface unknown predicate/action coordinates recorded under the
                    // Error policy only after parser aborts have been ruled out.
                    if let ::core::option::Option::Some(error) =
                        self.$base.take_unknown_semantic_error()
                    {
                        return ::core::result::Result::Err(error);
                    }
                }
                ::core::result::Result::Ok(__tree)
            }

            #[allow(dead_code)]
            fn parse_interpreted_rule(
                &mut self,
                rule_index: usize,
            ) -> ::core::result::Result<$crate::tree::ParseTree, $crate::errors::AntlrError> {
                self.parse_interpreted_rule_precedence(rule_index, 0)
            }

            #[allow(dead_code)]
            fn parse_interpreted_rule_precedence(
                &mut self,
                rule_index: usize,
                precedence: i32,
            ) -> ::core::result::Result<$crate::tree::ParseTree, $crate::errors::AntlrError> {
                if precedence == 0
                    && $adaptive_direct
                    && ::std::env::var_os("ANTLR4_RUST_ADAPTIVE_DIRECT").is_some()
                {
                    let simulator = self.$simulator.get_or_insert_with(|| {
                        $crate::atn::parser::ParserAtnSimulator::new_shared($atn())
                    });
                    self.$base
                        .parse_atn_rule_adaptive_or_fallback($atn(), simulator, rule_index)
                } else {
                    let (__tree, __actions) = {
                        let $this = &mut *self;
                        let $rule_index = rule_index;
                        let $precedence = precedence;
                        $fallback
                    }?;
                    // Uniform dispatch: grammars without action states define an
                    // empty `run_action` and collect no deferred actions, so this
                    // loop is a no-op for them.
                    for __action in __actions {
                        self.run_action(__action, __tree);
                    }
                    ::core::result::Result::Ok(__tree)
                }
            }
        }
    };
}

/// Defines the grammar-independent parse entry points for one generated
/// parser module: the `<Grammar>ParserParseOutput` alias of
/// [`GeneratedParseOutput`], the validation bridge behind
/// [`GeneratedParseOutput::validate`], and the `parse*` / `parse_stream*`
/// convenience functions.
///
/// This is an implementation detail of `antlr4-rust-gen`, not a stable
/// hand-written parser API.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_parser_entry_points {
    (
        parser: $parser:ident,
        output: $output:ident,
        validated_tree: $validated:ident,
        validation_error: $validation_error:ident,
        validate_tree: $validate_tree:path
        $(,)?
    ) => {
        #[doc = ::core::concat!(
                    "Result from [`parse_with_parser`] or [`parse_stream_with_parser`].\n\n",
                    "Keeps the generated parser available after the entry rule runs so callers\n",
                    "can inspect diagnostics or recover the parser-owned token stream. Alias of\n",
                    "the runtime's `GeneratedParseOutput` with [`",
                    ::core::stringify!($parser),
                    "`] substituted for its parser type parameter.",
                )]
        pub type $output<R, L> = $crate::generated::GeneratedParseOutput<R, $parser<L>>;

        impl<L, H> $crate::generated::__GeneratedParserValidate for $parser<L, H>
        where
            L: $crate::token::TokenSource,
            H: $crate::parser::SemanticHooks,
        {
            type Validated = $validated;

            fn __validate(
                self,
                root: $crate::tree::NodeId,
            ) -> ::core::result::Result<$validated, $crate::validated::ValidationError> {
                let lexer = self.token_stream().number_of_source_errors();
                let parser = $crate::parser::Parser::number_of_syntax_errors(&self);
                if lexer != 0 || parser != 0 {
                    return ::core::result::Result::Err($validation_error::SyntaxErrors {
                        lexer,
                        parser,
                    });
                }
                let parsed = self.into_parsed_file(root);
                $validate_tree(&parsed)?;
                ::core::result::Result::Ok(<$validated>::__new(parsed))
            }
        }

        /// Parses UTF-8 text by constructing the lexer, token stream, parser, and
        /// caller-selected entry rule in one call.
        ///
        #[doc = ::core::concat!(
                    "Pass the generated lexer constructor and a parser entry rule, for example\n",
                    "`parse(src, MyGrammarLexer::new, ",
                    ::core::stringify!($parser),
                    "::file)`.",
                )]
        ///
        /// The returned [`antlr4_runtime::ParsedFile`] owns the canonical token store,
        /// flat CST storage, and entry-rule root.
        /// Use [`parse_with_parser`] instead when the caller also needs parser
        /// diagnostics after the entry rule runs.
        pub fn parse<L: $crate::token::TokenSource>(
            input: impl ::core::convert::AsRef<str>,
            lexer: impl ::core::ops::FnOnce($crate::char_stream::InputStream) -> L,
            entry: impl ::core::ops::FnOnce(
                &mut $parser<L>,
            ) -> ::core::result::Result<
                $crate::tree::NodeId,
                $crate::errors::AntlrError,
            >,
        ) -> ::core::result::Result<$crate::tree::ParsedFile, $crate::errors::AntlrError> {
            parse_stream(
                $crate::char_stream::InputStream::new(input.as_ref()),
                lexer,
                entry,
            )
        }

        /// Parses UTF-8 text and returns a typed tree whose required generated child
        /// accessors are infallible.
        pub fn parse_validated<L: $crate::token::TokenSource>(
            input: impl ::core::convert::AsRef<str>,
            lexer: impl ::core::ops::FnOnce($crate::char_stream::InputStream) -> L,
            entry: impl ::core::ops::FnOnce(
                &mut $parser<L>,
            ) -> ::core::result::Result<
                $crate::tree::NodeId,
                $crate::errors::AntlrError,
            >,
        ) -> ::core::result::Result<$validated, $validation_error> {
            parse_stream_validated(
                $crate::char_stream::InputStream::new(input.as_ref()),
                lexer,
                entry,
            )
        }

        /// Parses UTF-8 text like [`parse`] while returning the parser after the entry
        /// rule has run.
        ///
        #[doc = ::core::concat!(
                    "This keeps the compact generated setup path available for callers that also\n",
                    "need `Parser::number_of_syntax_errors()` or `",
                    ::core::stringify!($parser),
                    "::into_token_stream()`.",
                )]
        pub fn parse_with_parser<L: $crate::token::TokenSource, R>(
            input: impl ::core::convert::AsRef<str>,
            lexer: impl ::core::ops::FnOnce($crate::char_stream::InputStream) -> L,
            entry: impl ::core::ops::FnOnce(
                &mut $parser<L>,
            )
                -> ::core::result::Result<R, $crate::errors::AntlrError>,
        ) -> ::core::result::Result<$output<R, L>, $crate::errors::AntlrError> {
            parse_stream_with_parser(
                $crate::char_stream::InputStream::new(input.as_ref()),
                lexer,
                entry,
            )
        }

        /// Parses a caller-provided character stream by constructing the lexer, token
        /// stream, parser, and caller-selected entry rule in one call.
        ///
        /// Unlike [`parse`], this accepts any [`antlr4_runtime::CharStream`], including
        /// a named [`antlr4_runtime::InputStream`] or a byte-oriented
        /// [`antlr4_runtime::ByteStream`].
        pub fn parse_stream<I: $crate::char_stream::CharStream, L: $crate::token::TokenSource>(
            input: I,
            lexer: impl ::core::ops::FnOnce(I) -> L,
            entry: impl ::core::ops::FnOnce(
                &mut $parser<L>,
            ) -> ::core::result::Result<
                $crate::tree::NodeId,
                $crate::errors::AntlrError,
            >,
        ) -> ::core::result::Result<$crate::tree::ParsedFile, $crate::errors::AntlrError> {
            let $crate::generated::GeneratedParseOutput { result, parser } =
                parse_stream_with_parser(input, lexer, entry)?;
            ::core::result::Result::Ok(parser.into_parsed_file(result))
        }

        /// Parses a caller-provided character stream and validates the completed tree.
        pub fn parse_stream_validated<
            I: $crate::char_stream::CharStream,
            L: $crate::token::TokenSource,
        >(
            input: I,
            lexer: impl ::core::ops::FnOnce(I) -> L,
            entry: impl ::core::ops::FnOnce(
                &mut $parser<L>,
            ) -> ::core::result::Result<
                $crate::tree::NodeId,
                $crate::errors::AntlrError,
            >,
        ) -> ::core::result::Result<$validated, $validation_error> {
            let output = parse_stream_with_parser(input, lexer, entry)
                .map_err($validation_error::Recognition)?;
            output.validate()
        }

        /// Parses a caller-provided character stream like [`parse_stream`] while
        /// returning the parser after the entry rule has run.
        pub fn parse_stream_with_parser<
            I: $crate::char_stream::CharStream,
            L: $crate::token::TokenSource,
            R,
        >(
            input: I,
            lexer: impl ::core::ops::FnOnce(I) -> L,
            entry: impl ::core::ops::FnOnce(
                &mut $parser<L>,
            )
                -> ::core::result::Result<R, $crate::errors::AntlrError>,
        ) -> ::core::result::Result<$output<R, L>, $crate::errors::AntlrError> {
            let lexer = lexer(input);
            let tokens = $crate::token_stream::CommonTokenStream::new(lexer);
            let mut parser = $parser::new(tokens);
            let result = entry(&mut parser)?;
            ::core::result::Result::Ok($crate::generated::GeneratedParseOutput { result, parser })
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

/// Exposes the parser base needed by the generated-rule dispatch lifecycle.
///
/// This is implemented by [`crate::__antlr4_rust_parser_facade`] for generated
/// parsers so [`dispatch_generated_rule`] can remain grammar-agnostic.
#[doc(hidden)]
pub trait GeneratedRuleParser {
    type Source: TokenSource;
    type Hooks: SemanticHooks;

    fn generated_rule_base(&mut self) -> &mut BaseParser<Self::Source, Self::Hooks>;
}

// ---------------------------------------------------------------------------
// Parse-driver and entry-point support shared by every generated parser and
// lexer module. The `__antlr4_rust_parser_driver!` and
// `__antlr4_rust_parser_entry_points!` expansions, and the generated
// `lex`/`lex_stream` re-exports, resolve against these items.
// ---------------------------------------------------------------------------

/// Error routing for generated rule bodies.
///
/// This is an implementation detail of `antlr4-rust-gen`, not a stable
/// hand-written parser API. Generated dispatch code distinguishes fatal
/// unwinds (which report recovery diagnostics) from errors that request the
/// interpreted fallback, plus the internal adaptive-ATN retry signal.
#[doc(hidden)]
#[derive(Debug)]
pub enum GeneratedRuleError {
    /// Unrecoverable failure of the generated engine for this entry.
    Fatal(AntlrError),
    /// Failure that requests the interpreted fallback for this entry.
    Interpreted(AntlrError),
    /// Internal adaptive-ATN retry unwind; never escapes the routing boundary.
    AdaptiveRetry,
}

impl GeneratedRuleError {
    /// Unwraps the underlying recognition error.
    #[must_use]
    pub fn into_error(self) -> AntlrError {
        match self {
            Self::Fatal(error) | Self::Interpreted(error) => error,
            Self::AdaptiveRetry => AntlrError::Unsupported(
                "internal adaptive ATN retry escaped its routing boundary".to_owned(),
            ),
        }
    }
}

/// Uniform function-pointer shape for one generated parser rule body.
#[doc(hidden)]
pub type GeneratedRuleBody<P> =
    fn(&mut P, i32, bool) -> Result<crate::tree::ParseTree, GeneratedRuleError>;

/// Applies the grammar-independent generated-rule guard around one table-selected body.
///
/// The function stays out of line so every generated parser monomorphization
/// carries one lifecycle shell instead of repeating it for every rule.
#[doc(hidden)]
#[inline(never)]
pub fn dispatch_generated_rule<P>(
    parser: &mut P,
    rule_index: usize,
    precedence: i32,
    allow_fallback: bool,
    body: GeneratedRuleBody<P>,
) -> Result<crate::tree::ParseTree, GeneratedRuleError>
where
    P: GeneratedRuleParser,
{
    if let Some(error) = parser.generated_rule_base().rule_depth_cap_violation() {
        return Err(GeneratedRuleError::Fatal(error));
    }
    if let Some(error) = parser
        .generated_rule_base()
        .parse_listener_enter_rule(rule_index)
    {
        return Err(GeneratedRuleError::Fatal(error));
    }
    let result = if parser
        .generated_rule_base()
        .generated_rule_stack_check_due()
    {
        grow_generated_rule_stack(|| body(parser, precedence, allow_fallback))
    } else {
        body(parser, precedence, allow_fallback)
    };
    parser
        .generated_rule_base()
        .parse_listener_exit_rule(rule_index);
    result
}

/// Adaptive-ATN preference state for the generated rules a grammar routes
/// through warmed-retry dispatch.
///
/// `RULES` is the number of adaptive-ATN-preferred rules the generator
/// assigned retry slots to; grammars without residual adaptive routing
/// instantiate `AdaptiveAtnRetryState<0>`, whose [`Self::retry_pending`]
/// check constant-folds to `false`.
#[doc(hidden)]
#[derive(Debug)]
pub struct AdaptiveAtnRetryState<const RULES: usize> {
    /// Rules whose warmed adaptive-prediction work marked them expensive.
    pub preferred_rules: [bool; RULES],
    /// Per-slot recursion depth of in-flight adaptive dispatches.
    pub preference_depths: [usize; RULES],
    /// Adaptive-prediction work counters captured at outermost entry.
    pub preference_starts: [(usize, usize); RULES],
    /// Syntax-error counts captured at outermost entry.
    pub syntax_error_starts: [usize; RULES],
    /// Slot currently unwinding through an adaptive retry, if any.
    pub retry_slot: Option<usize>,
}

impl<const RULES: usize> AdaptiveAtnRetryState<RULES> {
    /// Creates cleared preference state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            preferred_rules: [false; RULES],
            preference_depths: [0; RULES],
            preference_starts: [(0, 0); RULES],
            syntax_error_starts: [0; RULES],
            retry_slot: None,
        }
    }

    /// Clears every learned preference and any in-flight retry.
    pub const fn reset(&mut self) {
        *self = Self::new();
    }

    /// Reports whether an adaptive retry is currently unwinding.
    ///
    /// Constant-folds to `false` when the grammar has no retry slots, so the
    /// uniform generated retry clause costs nothing for such grammars.
    #[must_use]
    pub const fn retry_pending(&self) -> bool {
        RULES > 0 && self.retry_slot.is_some()
    }
}

impl<const RULES: usize> Default for AdaptiveAtnRetryState<RULES> {
    fn default() -> Self {
        Self::new()
    }
}

/// Result from a generated `parse_with_parser` or `parse_stream_with_parser`
/// call.
///
/// Keeps the generated parser available after the entry rule runs so callers
/// can inspect diagnostics or recover the parser-owned token stream. Generated
/// modules alias this type as `<Grammar>ParserParseOutput<R, L>` with their
/// parser type substituted for `P`; [`Self::validate`] is available for those
/// generated parser types, which wire in their module's validated surface.
#[derive(Debug)]
pub struct GeneratedParseOutput<R, P> {
    /// Value returned by the caller-selected entry rule.
    pub result: R,
    /// The generated parser after the entry rule has run.
    pub parser: P,
}

/// Grammar-specific validation step behind [`GeneratedParseOutput::validate`].
///
/// This is an implementation detail of `antlr4-rust-gen`, not a stable
/// hand-written parser API: each generated module implements it for its parser
/// type via `__antlr4_rust_parser_entry_points!`.
#[doc(hidden)]
pub trait __GeneratedParserValidate: Sized {
    /// The module-branded validated-tree type.
    type Validated;

    /// Validates a completed parse rooted at `root`.
    fn __validate(self, root: NodeId) -> Result<Self::Validated, ValidationError>;
}

impl<P: __GeneratedParserValidate> GeneratedParseOutput<NodeId, P> {
    /// Validates a completed parse and changes its generated context surface.
    ///
    /// Validation rejects lexer diagnostics, parser recovery, recovered error
    /// nodes, and missing generated required children before constructing the
    /// validated-tree type boundary.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the parse recorded lexer or parser
    /// syntax errors or the completed tree fails structural validation.
    pub fn validate(self) -> Result<P::Validated, ValidationError> {
        self.parser.__validate(self.result)
    }
}

/// Lexes UTF-8 text into an eagerly filled token stream without constructing
/// a parser.
///
/// Pass the generated lexer constructor, for example
/// `lex(src, MyGrammarLexer::new)`.
///
/// The stream retains every emitted token, including EOF and tokens on hidden
/// or custom channels. Lexer rules using `skip` do not emit tokens.
///
/// With `use antlr4_runtime::Token as _;` and the generated module's
/// `metadata()`, print each token's vocabulary name, numeric channel, and
/// text:
/// `let vocabulary = metadata().vocabulary(); for token in lex(src, MyGrammarLexer::new).tokens() { println!("type={} channel={} text={:?}", vocabulary.display_name(token.token_type()), token.channel(), token.text()); }`
///
/// `number_of_source_errors()` reports buffered lexer diagnostics. After
/// iterating `tokens()`, `drain_source_errors()` retrieves those diagnostics.
///
/// # Panics
///
/// Panics if buffering returns an [`crate::TokenStoreError`]. Construct the
/// lexer and call [`CommonTokenStream::try_new`] to handle that error instead.
pub fn lex<L: TokenSource>(
    input: impl AsRef<str>,
    lexer: impl FnOnce(InputStream) -> L,
) -> CommonTokenStream<L> {
    lex_stream(InputStream::new(input.as_ref()), lexer)
}

/// Lexes a caller-provided character stream without constructing a parser.
///
/// Unlike [`lex`], this accepts any [`CharStream`], including a named
/// [`InputStream`] or a byte-oriented [`crate::ByteStream`].
///
/// `number_of_source_errors()` reports buffered lexer diagnostics. After
/// iterating `tokens()`, `drain_source_errors()` retrieves those diagnostics.
///
/// # Panics
///
/// Panics if buffering returns an [`crate::TokenStoreError`]. Call
/// [`CommonTokenStream::try_new`] with the constructed lexer to handle that
/// error instead.
pub fn lex_stream<I: CharStream, L: TokenSource>(
    input: I,
    lexer: impl FnOnce(I) -> L,
) -> CommonTokenStream<L> {
    CommonTokenStream::new(lexer(input))
}

// ---------------------------------------------------------------------------
// Grammar-independent support surface imported by every generated parser
// module. These items back the typed context views, listener/visitor bridges,
// and embedded-action facades emitted by `antlr4-rust-gen`; the generated
// module brings them into scope by name so the `__antlr4_rust_context!` /
// `__antlr4_rust_context_accessors!` expansions resolve against them.
// ---------------------------------------------------------------------------

/// Token-stream facade backing embedded-action `$input` translation
/// (`self.input().text()` / `.la(i)` / `.lt(i).text()`).
#[doc(hidden)]
pub struct __GeneratedInput<'a, L: TokenSource>(#[doc(hidden)] pub &'a mut CommonTokenStream<L>);

impl<L: TokenSource> __GeneratedInput<'_, L> {
    #[must_use]
    #[inline]
    pub fn text(&self) -> String {
        self.0.text_all()
    }

    #[inline]
    pub fn la(&mut self, offset: isize) -> i32 {
        IntStream::la(self.0, offset)
    }

    #[must_use]
    #[inline]
    pub fn lt(&self, offset: isize) -> __GeneratedTokenView {
        __GeneratedTokenView {
            text: self
                .0
                .lt(offset)
                .map(|token| token.text_or_empty().to_owned())
                .unwrap_or_default(),
        }
    }
}

impl<L: TokenSource> fmt::Debug for __GeneratedInput<'_, L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("__GeneratedInput").finish_non_exhaustive()
    }
}

/// Owned token view returned by [`__GeneratedInput::lt`] and the generated
/// contexts' `start()` accessors.
#[doc(hidden)]
#[derive(Debug)]
pub struct __GeneratedTokenView {
    #[doc(hidden)]
    pub text: String,
}

impl __GeneratedTokenView {
    #[must_use]
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Typed terminal wrapper exposed by generated listener, visitor, and context
/// surfaces.
///
/// Recovery-inserted error nodes travel through the same surface; use
/// [`TerminalNode::is_error`] and [`TerminalNode::is_missing`] to identify
/// them.
#[derive(Clone, Debug)]
pub struct TerminalNode<'a> {
    __node: TerminalNodeView<'a>,
}

impl<'a> TerminalNode<'a> {
    #[doc(hidden)]
    #[must_use]
    #[inline]
    pub const fn new(node: TerminalNodeView<'a>) -> Self {
        Self { __node: node }
    }

    #[must_use]
    #[inline]
    pub fn symbol(&self) -> TokenView<'a> {
        self.__node.symbol()
    }

    #[must_use]
    #[inline]
    pub fn is_error(&self) -> bool {
        matches!(self.__node.node().kind(), NodeKind::Error)
    }

    #[must_use]
    #[inline]
    pub fn is_missing(&self) -> bool {
        self.symbol().is_synthetic()
    }

    /// The underlying parse-tree node.
    #[doc(hidden)]
    #[must_use]
    #[inline]
    pub const fn node(&self) -> Node<'a> {
        self.__node.node()
    }
}

impl fmt::Display for TerminalNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.__node.text())
    }
}

/// Typed error-node wrapper exposed by generated listener and visitor
/// surfaces.
#[derive(Clone, Debug)]
pub struct ErrorNode<'a> {
    __node: ErrorNodeView<'a>,
}

impl<'a> ErrorNode<'a> {
    #[doc(hidden)]
    #[must_use]
    #[inline]
    pub const fn new(node: ErrorNodeView<'a>) -> Self {
        Self { __node: node }
    }

    #[must_use]
    #[inline]
    pub fn symbol(&self) -> TokenView<'a> {
        self.__node.symbol()
    }

    #[must_use]
    #[inline]
    pub fn is_missing(&self) -> bool {
        self.symbol().is_synthetic()
    }

    /// The underlying parse-tree node.
    #[doc(hidden)]
    #[must_use]
    #[inline]
    pub const fn node(&self) -> Node<'a> {
        self.__node.node()
    }
}

impl fmt::Display for ErrorNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.__node.text())
    }
}

/// Child source of one generated context view: either a stored parse-tree
/// node or the live parser context observed from an in-flight rule.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum __GeneratedRuleContext<'a> {
    Stored(RuleNodeView<'a>),
    Active {
        context: &'a ParserRuleContext,
        storage: &'a ParseTreeStorage,
        tokens: &'a TokenStore,
    },
}

/// Marker state for generated contexts borrowed from a stored parse tree.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct StoredTreeContext;

/// Marker state for generated contexts observed from an in-flight parse.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct __ActiveParserContext;

#[doc(hidden)]
#[inline]
pub fn __context_children(
    source: __GeneratedRuleContext<'_>,
) -> impl Iterator<Item = Node<'_>> + '_ {
    let mut stored = match source {
        __GeneratedRuleContext::Stored(node) => Some(node.children()),
        __GeneratedRuleContext::Active { .. } => None,
    };
    let mut active = match source {
        __GeneratedRuleContext::Stored(_) => None,
        __GeneratedRuleContext::Active {
            context,
            storage,
            tokens,
        } => Some(context.child_nodes(storage, tokens)),
    };
    std::iter::from_fn(move || {
        stored
            .as_mut()
            .and_then(Iterator::next)
            .or_else(|| active.as_mut().and_then(Iterator::next))
    })
}

#[doc(hidden)]
#[inline]
pub fn __rule_children(
    source: __GeneratedRuleContext<'_>,
    rule_index: usize,
) -> impl Iterator<Item = RuleNodeView<'_>> + '_ {
    __context_children(source).filter_map(move |child| {
        let rule = child.as_rule()?;
        (rule.rule_index() == rule_index).then_some(rule)
    })
}

#[doc(hidden)]
#[inline]
pub fn __terminal_children(
    source: __GeneratedRuleContext<'_>,
) -> impl Iterator<Item = TerminalNodeView<'_>> + '_ {
    __context_children(source).filter_map(Node::terminal_view)
}

#[doc(hidden)]
#[inline]
pub fn __token_children(
    source: __GeneratedRuleContext<'_>,
    token_type: i32,
) -> impl Iterator<Item = TerminalNodeView<'_>> + '_ {
    __terminal_children(source).filter(move |terminal| terminal.symbol().token_type() == token_type)
}

#[doc(hidden)]
#[inline]
pub fn __token_children_matching<'a>(
    source: __GeneratedRuleContext<'a>,
    token_types: &'static [i32],
) -> impl Iterator<Item = TerminalNodeView<'a>> + 'a {
    __terminal_children(source)
        .filter(move |terminal| token_types.contains(&terminal.symbol().token_type()))
}

#[doc(hidden)]
#[inline]
pub fn __labeled_token_children(
    source: __GeneratedRuleContext<'_>,
    token_type: i32,
) -> impl Iterator<Item = TerminalNodeView<'_>> + '_ {
    __context_children(source).filter_map(move |child| {
        let terminal = child.labeled_terminal_view()?;
        (terminal.symbol().token_type() == token_type).then_some(terminal)
    })
}

#[doc(hidden)]
#[inline]
pub fn __labeled_token_children_matching<'a>(
    source: __GeneratedRuleContext<'a>,
    token_types: &'static [i32],
) -> impl Iterator<Item = TerminalNodeView<'a>> + 'a {
    __context_children(source).filter_map(move |child| {
        let terminal = child.labeled_terminal_view()?;
        token_types
            .contains(&terminal.symbol().token_type())
            .then_some(terminal)
    })
}

/// Constructs a generated context view over a live parser context.
/// Implemented by `__antlr4_rust_context!` for every generated context.
#[doc(hidden)]
pub trait __FromActiveRuleContext<'a>: Sized {
    fn __from_active(
        context: &'a ParserRuleContext,
        live_attrs: Option<&dyn Any>,
        invocation_states: Vec<isize>,
        storage: &'a ParseTreeStorage,
        tokens: &'a TokenStore,
    ) -> Option<Self>;
}

#[doc(hidden)]
#[inline]
pub fn __active_context_view<'a, T: __FromActiveRuleContext<'a>>(
    context: &'a ParserRuleContext,
    invocation_states: Vec<isize>,
    storage: &'a ParseTreeStorage,
    tokens: &'a TokenStore,
) -> Option<T> {
    T::__from_active(context, None, invocation_states, storage, tokens)
}

#[doc(hidden)]
#[inline]
pub fn __active_context_view_with_attrs<'a, T: __FromActiveRuleContext<'a>>(
    context: &'a ParserRuleContext,
    live_attrs: &dyn Any,
    invocation_states: Vec<isize>,
    storage: &'a ParseTreeStorage,
    tokens: &'a TokenStore,
) -> Option<T> {
    T::__from_active(
        context,
        Some(live_attrs),
        invocation_states,
        storage,
        tokens,
    )
}

/// Formats an invoking-state chain the way Java's `RuleContext.toString`
/// renders it: `[13 6]`.
#[doc(hidden)]
pub fn __write_invocation_states(
    f: &mut fmt::Formatter<'_>,
    states: impl Iterator<Item = isize>,
) -> fmt::Result {
    f.write_str("[")?;
    let mut separator = "";
    for state in states {
        write!(f, "{separator}{state}")?;
        separator = " ";
    }
    f.write_str("]")
}

/// Bound on the recovery-oriented state markers accepted by generated
/// context accessors.
///
/// The validated marker (`ValidatedTreeContext`) intentionally stays emitted
/// per generated module: the accessors macro relies on rustc proving that the
/// validated marker never implements this trait, and coherence only permits
/// that negative reasoning while the marker type is local to the generated
/// crate.
#[doc(hidden)]
pub trait __RecoveryContextState {}

impl __RecoveryContextState for StoredTreeContext {}
impl __RecoveryContextState for __ActiveParserContext {}

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

#[cfg(test)]
mod parser_driver_tests {
    // Anchors into the `__antlr4_rust_parser_driver!` macro body. Each search
    // string is declared once so a rename inside the macro is a one-line
    // update here, and the assertions below stay readable.
    const DRIVER_MACRO: &str = "macro_rules! __antlr4_rust_parser_driver";
    const ENTRY_POINTS_MACRO: &str = "macro_rules! __antlr4_rust_parser_entry_points";
    /// Unique to the top-level surfacing check: the Err-arm check binds
    /// `semantic_error` and the sticky-abort drains bind `_`, so anchoring on
    /// the `Some(error)` binder cannot accidentally match a drain.
    const TOP_LEVEL_SEMANTIC_SURFACE: &str =
        "Some(error) = self.$base.take_unknown_semantic_error()";
    const ERR_ARM_SEMANTIC_SURFACE: &str =
        "Some(semantic_error) = self.$base.take_unknown_semantic_error()";
    const ERR_ARM_START: &str = "::core::result::Result::Err(error) => {";
    const ERR_CONVERSION: &str = "let error = error.into_error();";
    const ERR_RETURN: &str = "return ::core::result::Result::Err(error);";
    const DIAGNOSTICS_DISPATCH: &str = "self.$base.report_generated_parser_diagnostics();";
    const ABORT_DRAIN: &str = "Some(abort) = self.$base.take_parse_abort()";
    const UNRECOVERED_REPORT: &str = "self.$base.report_unrecovered_parser_error(&error);";
    const OK_TREE: &str = "::core::result::Result::Ok(__tree)";
    const INTERPRETED_FALLBACK: &str =
        "self.parse_interpreted_rule_precedence(rule_index, precedence)?";
    const ACTION_DISPATCH: &str = "self.run_action(__action, __tree);";

    /// The driver macro body with all whitespace collapsed, so anchors that
    /// rustfmt wraps across lines (`if let ::core::option::Option::Some(error)
    /// = self.$base...`) match as single strings.
    fn driver_macro_body() -> String {
        let source = include_str!("generated.rs");
        let driver_at = source
            .find(DRIVER_MACRO)
            .expect("driver macro is defined in this module");
        let entry_points_at = source
            .find(ENTRY_POINTS_MACRO)
            .expect("entry-points macro is defined in this module");
        source[driver_at..entry_points_at]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Pins ordering invariants of the `__antlr4_rust_parser_driver!` entry
    /// body that generated-parser behavior depends on. These checks lived in
    /// the generator's per-grammar rendered-text tests while the driver was
    /// emitted per module; the macro is the single copy now, so the source
    /// text is asserted here once. The runtime behavior itself is covered by
    /// `public_entry_surfaces_recorded_semantic_miss_after_clean_parse` in
    /// the generator's CLI suite, which parses through a generated entry and
    /// asserts the fail-loud error is returned.
    #[test]
    fn parser_driver_entry_ordering_invariants() {
        let driver = driver_macro_body();

        // The fail-loud surfacing check runs before the entry can return Ok,
        // or a parse that consulted an unimplemented hook would return a
        // recovered Ok tree instead of AntlrError::Unsupported. The binder
        // anchor is unique, so this cannot be satisfied by one of the
        // `let _ =` drains.
        assert_eq!(
            driver.matches(TOP_LEVEL_SEMANTIC_SURFACE).count(),
            1,
            "exactly one top-level surfacing check binds `error`"
        );
        let surface_at = driver
            .find(TOP_LEVEL_SEMANTIC_SURFACE)
            .expect("entry surfaces recorded unknown-semantic coordinates");
        let ok_at = driver[surface_at..]
            .find(OK_TREE)
            .map(|offset| surface_at + offset)
            .expect("entry returns the tree after semantic checks");
        assert!(surface_at < ok_at);

        // Inside the generated-rule Err arm, retained diagnostics dispatch
        // first, then parser aborts, then recorded semantic misses; otherwise
        // the documented fail-loud error would be shadowed or diagnostics
        // would leak into the next entry on a reused parser.
        let arm_start = driver
            .find(ERR_ARM_START)
            .expect("the generated-rule match has an Err arm");
        let conversion_at = driver[arm_start..]
            .find(ERR_CONVERSION)
            .map(|offset| arm_start + offset)
            .expect("Err arm converts the generic rule error");
        let arm = &driver[arm_start..conversion_at];
        let diagnostics_at = arm
            .find(DIAGNOSTICS_DISPATCH)
            .expect("the fatal Err arm drains retained diagnostics");
        let abort_at = arm
            .find(ABORT_DRAIN)
            .expect("the Err arm drains a recorded parser abort");
        let semantic_at = arm
            .find(ERR_ARM_SEMANTIC_SURFACE)
            .expect("the Err arm drains a recorded semantic error");
        assert!(
            diagnostics_at < abort_at && abort_at < semantic_at,
            "retained diagnostics dispatch first, then parser aborts precede semantic misses"
        );

        // A fatal unwind reports the converted error through the listener
        // boundary before returning it.
        let err_return_at = driver[conversion_at..]
            .find(ERR_RETURN)
            .map(|offset| conversion_at + offset)
            .expect("the Err arm returns the converted error");
        assert!(
            driver[conversion_at..err_return_at].contains(UNRECOVERED_REPORT),
            "the Err arm reports the unrecovered error before returning it"
        );

        // The interpreted fallback runs the uniform action-dispatch loop
        // (every generated parser defines `run_action`; grammars without
        // action states get the empty stub); the top-level boundary then
        // dispatches recovery diagnostics and never returns Ok between the
        // fallback and the surfacing check, so an action-hook miss recorded
        // by the fallback's immediate `run_action` loop cannot escape as Ok.
        let fallback_at = driver
            .find(INTERPRETED_FALLBACK)
            .expect("entry runs the interpreted fallback when a rule is not generated");
        assert!(
            fallback_at < surface_at,
            "the surfacing check follows the interpreted fallback"
        );
        assert!(
            driver[fallback_at..surface_at].contains(DIAGNOSTICS_DISPATCH),
            "the entry dispatches boundary diagnostics between the fallback and the surfacing check"
        );
        assert!(
            !driver[fallback_at..surface_at].contains(OK_TREE),
            "the entry must not return Ok between the interpreted fallback and the surfacing check"
        );
        assert!(driver.contains(ACTION_DISPATCH));
    }
}
