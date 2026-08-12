// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// Disposition policy for semantic predicate/action coordinates that the
/// generator cannot translate into runtime metadata.
///
/// Mirrors the runtime's `UnknownSemanticPolicy`; the generator additionally
/// uses [`Self::Error`] to fail code generation before emitting a module whose
/// semantics would be unreliable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SemUnknownPolicy {
    /// Unknown predicates pass unconditionally and unknown actions are
    /// no-ops. Matches the historical metadata-only behavior; deprecated as a
    /// default and slated to change to [`Self::Error`] in a future minor
    /// release.
    #[default]
    AssumeTrue,
    /// Unknown predicates fail unconditionally; unknown actions remain
    /// no-ops.
    AssumeFalse,
    /// Unknown coordinates are intentionally delegated to runtime hooks.
    Hook,
    /// Fail code generation when any semantic coordinate has no Rust
    /// implementation.
    Error,
}

impl SemUnknownPolicy {
    const fn manifest_name(self) -> &'static str {
        match self {
            Self::AssumeTrue => "assume-true",
            Self::AssumeFalse => "assume-false",
            Self::Hook => "hook",
            Self::Error => "error",
        }
    }

    /// Manifest disposition recorded for a predicate coordinate that has no
    /// translated template. [`Self::Error`] aborts generation before a
    /// manifest is written, so its mapping is only ever read by the
    /// fail-loud report.
    const fn unknown_predicate_disposition(self) -> SemanticsDisposition {
        match self {
            Self::AssumeTrue | Self::Error => SemanticsDisposition::AssumeTrue,
            Self::AssumeFalse => SemanticsDisposition::AssumeFalse,
            Self::Hook => SemanticsDisposition::Hooked,
        }
    }

    const fn unknown_action_disposition(self) -> SemanticsDisposition {
        match self {
            Self::Hook => SemanticsDisposition::Hooked,
            Self::AssumeTrue | Self::AssumeFalse | Self::Error => SemanticsDisposition::Ignored,
        }
    }
}

/// Coordinate kinds tracked by the `semantics.json` manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SemanticsKind {
    LexerAction,
    LexerPredicate,
    ParserPredicate,
    ParserAction,
}

impl SemanticsKind {
    const fn manifest_name(self) -> &'static str {
        match self {
            Self::LexerAction => "lexer-action",
            Self::LexerPredicate => "lexer-predicate",
            Self::ParserPredicate => "parser-predicate",
            Self::ParserAction => "parser-action",
        }
    }

    const fn error_label(self) -> &'static str {
        match self {
            Self::LexerAction => "unsupported grammar action",
            Self::LexerPredicate | Self::ParserPredicate => "unsupported semantic predicate",
            Self::ParserAction => "unsupported grammar action",
        }
    }
}

/// How generation disposed of one semantic coordinate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SemanticsDisposition {
    /// A supported template translated the coordinate into runtime metadata
    /// or generated Rust code.
    Translated,
    /// No implementation exists; recognition treats the predicate as passing.
    AssumeTrue,
    /// No implementation exists; recognition treats the predicate as failing.
    AssumeFalse,
    /// No generated implementation exists; runtime hooks own the coordinate.
    Hooked,
    /// The coordinate is intentionally rejected by policy.
    Error,
    /// No implementation exists; the action is a no-op at recognition time.
    Ignored,
    /// An action ANTLR synthesized (e.g. during left-recursion elimination),
    /// not written by the grammar author. It is a no-op at recognition time
    /// like [`Self::Ignored`], but carries no author intent, so it is exempt
    /// from the `--sem-unknown=error` gate.
    Synthetic,
}

impl SemanticsDisposition {
    const fn manifest_name(self) -> &'static str {
        match self {
            Self::Translated => "translated",
            Self::AssumeTrue => "assume-true",
            Self::AssumeFalse => "assume-false",
            Self::Hooked => "hooked",
            Self::Error => "error",
            Self::Ignored => "ignored",
            Self::Synthetic => "synthetic",
        }
    }
}

/// How the Rust backend treats one top-level ANTLR grammar option.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GrammarOptionDisposition {
    /// The direct compiler represented the option in the compiled artifacts.
    Metadata,
    /// The option affects the upstream tool invocation, not Rust runtime
    /// behavior.
    ToolHandled,
    /// Caller-owned Rust hooks explicitly provide the target behavior.
    Hooked,
    /// The option is legal ANTLR syntax but its target behavior is not
    /// automatically implemented by the Rust backend.
    Unsupported,
}

impl GrammarOptionDisposition {
    pub(crate) const fn manifest_name(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::ToolHandled => "tool-handled",
            Self::Hooked => "hooked",
            Self::Unsupported => "unsupported",
        }
    }
}

/// One source-level grammar option inventoried from the compilation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrammarOptionEntry {
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) disposition: GrammarOptionDisposition,
}

impl GrammarOptionEntry {
    fn assignment(&self) -> String {
        format!("{}={}", self.key, self.value)
    }

    fn describe_unsupported(&self) -> String {
        format!(
            "unsupported grammar option: {} at {}:{} is accepted by ANTLR but is not \
             automatically implemented by antlr4-rust-gen; target-specific behavior may be \
             missing (see #98)",
            self.assignment(),
            self.line,
            self.column
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinateDispose {
    Hook,
    AssumeTrue,
    AssumeFalse,
    Error,
}

impl CoordinateDispose {
    fn parse(value: &str) -> io::Result<Self> {
        match value {
            "hook" => Ok(Self::Hook),
            "assume-true" => Ok(Self::AssumeTrue),
            "assume-false" => Ok(Self::AssumeFalse),
            "error" => Ok(Self::Error),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown coordinate dispose {other}"),
            )),
        }
    }

    pub(crate) const fn disposition(self) -> SemanticsDisposition {
        match self {
            Self::Hook => SemanticsDisposition::Hooked,
            Self::AssumeTrue => SemanticsDisposition::AssumeTrue,
            Self::AssumeFalse => SemanticsDisposition::AssumeFalse,
            Self::Error => SemanticsDisposition::Error,
        }
    }

    pub(crate) const fn predicate_template(self) -> Option<PredicateTemplate> {
        match self {
            Self::Hook => Some(PredicateTemplate::Hook),
            Self::AssumeTrue => Some(PredicateTemplate::True),
            Self::AssumeFalse => Some(PredicateTemplate::False),
            Self::Error => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SemPatternFile {
    patterns: Vec<SemPatternRule>,
    helpers: Vec<SemHelperRule>,
    coordinates: Vec<SemCoordinateOverride>,
    /// `[[member]]` slot inventory backing the `stack_member` lowerings
    /// (issue #206). Declaration order fixes slot numbering.
    members: Vec<stack_member::MemberDeclaration>,
}

impl SemPatternFile {
    /// Slot numbers for the members visible to one recognizer, or an error
    /// naming a duplicate.
    ///
    /// Lexer and parser inventories are separate: a combined grammar may
    /// declare independent `@lexer::members` and `@parser::members`, including
    /// same-named ones with different kinds or initial values, and the two
    /// recognizers hold separate member environments at runtime.
    fn member_slots_for(
        &self,
        recognizer: stack_member::MemberScope,
    ) -> io::Result<stack_member::MemberSlots> {
        stack_member::MemberSlots::assign_scoped(&self.members, recognizer)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemPatternRule {
    id: String,
    match_body: String,
    lower: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemHelperRule {
    /// Explicit helper kind. A missing kind preserves the pre-v1 wildcard
    /// behavior for lexer and parser predicates.
    kind: Option<SemanticsKind>,
    /// Additional receiver spelling accepted for this helper. Bare calls and
    /// the established `this.` / `self.` forms remain available by default.
    receiver: Option<String>,
    name: String,
    arguments: Vec<SemanticLiteralKind>,
    lower: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SemanticLiteralKind {
    String,
    Bool,
    Integer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SemanticLiteral {
    String(String),
    Bool(bool),
    Integer(i64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SemanticHelperCall {
    pub(crate) name: String,
    pub(crate) arguments: Vec<SemanticLiteral>,
    pub(crate) negated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SemCoordinateOverride {
    pub(crate) kind: SemanticsKind,
    pub(crate) rule: Option<String>,
    pub(crate) index: Option<usize>,
    pub(crate) atn_state: Option<usize>,
    pub(crate) dispose: CoordinateDispose,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ActionTemplate {
    LexerPopMode,
    Hook(SemanticHelperCall),
    /// Mutation of grammar-declared member state, lowered from a
    /// `stack_member` pattern (issue #206). Emitted as a `LexerSemantics`
    /// table entry rather than inline Rust, so it needs no hook.
    MemberStmt(stack_member::MemberStmt),
    UnsupportedLexerAction {
        rule_name: String,
        body: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PredicateTemplate {
    Hook,
    /// An untranslated parser predicate whose `<fail=...>` metadata must
    /// survive. Evaluation still follows hook -> unknown-policy fallback.
    UnknownWithFailMessage {
        message: String,
    },
    /// An untranslated parser predicate with no `<fail=...>` metadata.
    /// Lowering it (instead of leaving the coordinate uncovered) keeps its
    /// rule — and every caller of that rule — on the generated fast path
    /// rather than cascading to the 5-6x slower interpreter (issue #209).
    /// Evaluation follows the same hook -> unknown-policy fallback as
    /// [`Self::UnknownWithFailMessage`], so typed/closure hooks stay
    /// consulted and `--sem-unknown` dispositions apply unchanged.
    Unknown,
    True,
    False,
    FalseWithMessage {
        message: String,
    },
    /// A non-constant-false predicate carrying an ANTLR `<fail=...>` message.
    /// Transparent to evaluation — `inner` provides the truth value and codegen;
    /// the message is surfaced only when `inner` returns false at runtime.
    WithFailMessage {
        inner: Box<Self>,
        message: String,
    },
    Invoke {
        value: bool,
    },
    LocalIntEquals {
        value: i64,
    },
    LocalIntLessOrEqual {
        value: i64,
    },
    LookaheadTextEquals {
        offset: isize,
        text: String,
    },
    TextEquals(String),
    TokenStartColumnEquals(usize),
    ColumnLessThan(usize),
    ColumnGreaterOrEqual(usize),
    LookaheadNotEquals {
        offset: isize,
        token_name: String,
    },
    TokenPairAdjacent,
    ContextChildRuleTextNotEquals {
        rule_name: String,
        text: String,
    },
    /// A predicate over grammar-declared member state, lowered from a
    /// `stack_member` pattern (issue #206).
    MemberExpr(stack_member::MemberExpr),
}

pub(crate) fn can_generate_parser_predicate(predicate: &PredicateTemplate) -> bool {
    // A `<fail=...>` wrapper is transparent: generatability follows the inner.
    matches!(
        predicate_effective_template(predicate),
        PredicateTemplate::Hook
            | PredicateTemplate::UnknownWithFailMessage { .. }
            | PredicateTemplate::Unknown
            | PredicateTemplate::True
            | PredicateTemplate::False
            | PredicateTemplate::FalseWithMessage { .. }
            | PredicateTemplate::Invoke { .. }
            | PredicateTemplate::LocalIntEquals { .. }
            | PredicateTemplate::LocalIntLessOrEqual { .. }
            | PredicateTemplate::LookaheadTextEquals { .. }
            | PredicateTemplate::LookaheadNotEquals { .. }
            | PredicateTemplate::TokenPairAdjacent
            | PredicateTemplate::ContextChildRuleTextNotEquals { .. }
            // Member-state predicates lower into `parser_semantics()`, and the
            // generated predicate step reads member state off the parser
            // (`parser_semantic_ir_predicate_matches_with_context_and_local`),
            // so they are generatable. Omitting them forced every rule holding
            // one onto the 5-6x slower interpreter (the issue #209 cascade).
            | PredicateTemplate::MemberExpr(_)
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuleArgTemplate {
    Literal(i64),
    InheritLocal,
}
