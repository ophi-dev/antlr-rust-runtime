// `HashMap`/`HashSet` here are used as parser-internal caches keyed on
// stable ATN coordinates (state numbers, token indices). They're never
// iterated externally, so the project's `disallowed_types` lint (which
// guards against non-deterministic iteration order leaking out) does not
// apply to these uses.
use std::cell::RefCell;
#[allow(clippy::disallowed_types)]
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::rc::Rc;

/// Rotate constant copied from rustc-hash / `FxHash`. The default
/// `RandomState` hasher seeds itself from the OS RNG and runs `SipHash` on
/// every key, which dominates `recognize_state_fast`'s memo lookups;
/// `FxHasher` is a streaming integer hasher with near-zero per-call overhead
/// and matches the access pattern of small integer keys that the parser memo
/// uses.
#[derive(Clone, Copy, Default)]
struct FxHasher {
    hash: u64,
}

const FX_ROT: u32 = 5;
const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl Hasher for FxHasher {
    /// Folds bytes 8 at a time so a `write(&[u8; 8])` call hashes to the same
    /// state as a `write_u64` of the same little-endian bits. The `Hash` impls
    /// for `String`, `[u8; N]`, and slice-like types reach the hasher through
    /// `write`; matching the typed-method behaviour avoids the silent
    /// divergence flagged in PR #5 review (Greptile P2). Tail bytes that do
    /// not form a full word are mixed one at a time with the same constants,
    /// keeping behaviour deterministic regardless of the slice length.
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let (head, rest) = bytes.split_at(8);
            let word = u64::from_le_bytes(head.try_into().expect("8-byte chunk"));
            self.hash = (self.hash.rotate_left(FX_ROT) ^ word).wrapping_mul(FX_SEED);
            bytes = rest;
        }
        for byte in bytes {
            self.hash = (self.hash.rotate_left(FX_ROT) ^ u64::from(*byte)).wrapping_mul(FX_SEED);
        }
    }
    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.hash = (self.hash.rotate_left(FX_ROT) ^ value).wrapping_mul(FX_SEED);
    }
    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
    #[inline]
    fn write_i32(&mut self, value: i32) {
        self.write_u64(u64::from(i32::cast_unsigned(value)));
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxBuildHasher = BuildHasherDefault<FxHasher>;
#[allow(clippy::disallowed_types)]
type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;
#[allow(clippy::disallowed_types)]
type FxHashSet<K> = HashSet<K, FxBuildHasher>;

use crate::atn::parser::{
    ParserAtnPrediction, ParserAtnPredictionDiagnosticKind, ParserAtnSimulator,
};
use crate::atn::{Atn, AtnState, AtnStateKind, Transition};
use crate::char_stream::CharStream;
use crate::errors::AntlrError;
use crate::int_stream::IntStream;
use crate::lexer::{LexerCustomAction, LexerSemCtx};
use crate::prediction::{EMPTY_RETURN_STATE, PredictionContext};
use crate::recognizer::{Recognizer, RecognizerData};
use crate::semir::{self, AStmt, ArithOp, CmpOp, ExprId, HookId, PExpr, SemIr, StmtId};
use crate::token::{
    CommonToken, TOKEN_EOF, Token, TokenFactory, TokenRef, TokenSource, TokenSourceError,
};
use crate::token_stream::CommonTokenStream;
use crate::tree::{ErrorNode, ParseTree, ParserRuleContext, RuleNode, TerminalNode};
use crate::vocabulary::Vocabulary;

/// Upper bound for the recursive metadata recognizer before it treats a path as
/// non-viable. Long expression-regression descriptors legitimately walk tens
/// of thousands of ATN edges.
const RECOGNITION_DEPTH_LIMIT: usize = 32_768;
/// Whole-rule direct adaptive execution is allowed to give up and fall back to
/// the existing recognizer. Keep the guard at the same order of magnitude as
/// speculative recognition so malformed cyclic ATNs cannot spin forever.
const ADAPTIVE_DIRECT_STEP_LIMIT: usize = RECOGNITION_DEPTH_LIMIT;
/// Probe window for deciding whether clean-pass one-outcome memo entries are
/// reusable enough to keep caching. Large C# parses mostly produce one-shot
/// entries; small ambiguous Kotlin loops repeatedly hit the same keys.
const CLEAN_SINGLE_OUTCOME_MEMO_PROBE_LIMIT: usize = 4096;
const CLEAN_SINGLE_OUTCOME_MEMO_REPEAT_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SingleOutcomeMemoMode {
    Probe,
    Promote,
    Sparse,
}

fn interval_set_contains(intervals: &[(i32, i32)], symbol: i32) -> bool {
    intervals
        .iter()
        .any(|(start, stop)| (*start..=*stop).contains(&symbol))
}

fn interval_symbols(intervals: &[(i32, i32)]) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    for (start, stop) in intervals {
        symbols.extend(*start..=*stop);
    }
    symbols
}

fn interval_complement_symbols(
    intervals: &[(i32, i32)],
    min_vocabulary: i32,
    max_vocabulary: i32,
) -> BTreeSet<i32> {
    (min_vocabulary..=max_vocabulary)
        .filter(|symbol| !interval_set_contains(intervals, *symbol))
        .collect()
}

#[cfg(feature = "perf-counters")]
mod perf_counters {
    use std::cell::Cell;
    thread_local! {
        pub(super) static RFS_CALLS: Cell<u64> = const { Cell::new(0) };
        pub(super) static RFS_MEMO_HITS: Cell<u64> = const { Cell::new(0) };
        pub(super) static RFS_MEMO_MISSES: Cell<u64> = const { Cell::new(0) };
        pub(super) static RFS_VISITING_CYCLE: Cell<u64> = const { Cell::new(0) };
        pub(super) static MEMO_INSERTED: Cell<u64> = const { Cell::new(0) };
        pub(super) static OUTCOMES_PUSHED: Cell<u64> = const { Cell::new(0) };
        pub(super) static OUTCOMES_CLONED: Cell<u64> = const { Cell::new(0) };
    }
    pub(super) fn inc(c: &'static std::thread::LocalKey<Cell<u64>>, n: u64) {
        c.with(|v| v.set(v.get() + n));
    }
    thread_local! {
        pub(super) static EPSILON_TRANSITIONS: Cell<u64> = const { Cell::new(0) };
        pub(super) static RULE_TRANSITIONS: Cell<u64> = const { Cell::new(0) };
        pub(super) static ATOM_RANGE_TRANSITIONS: Cell<u64> = const { Cell::new(0) };
        pub(super) static SINGLE_TRANS_BODY: Cell<u64> = const { Cell::new(0) };
        pub(super) static MULTI_TRANS_BODY: Cell<u64> = const { Cell::new(0) };
        pub(super) static SINGLE_TRANS_RULE: Cell<u64> = const { Cell::new(0) };
        pub(super) static SINGLE_TRANS_ATOM: Cell<u64> = const { Cell::new(0) };
        pub(super) static SINGLE_TRANS_OTHER: Cell<u64> = const { Cell::new(0) };
        pub(super) static OUTCOMES_RETURN_0: Cell<u64> = const { Cell::new(0) };
        pub(super) static OUTCOMES_RETURN_1: Cell<u64> = const { Cell::new(0) };
        pub(super) static OUTCOMES_RETURN_N: Cell<u64> = const { Cell::new(0) };
    }
    pub(super) fn snapshot() -> [(&'static str, u64); 18] {
        [
            ("rfs_calls", RFS_CALLS.with(Cell::get)),
            ("rfs_memo_hits", RFS_MEMO_HITS.with(Cell::get)),
            ("rfs_memo_misses", RFS_MEMO_MISSES.with(Cell::get)),
            ("rfs_visiting_cycle", RFS_VISITING_CYCLE.with(Cell::get)),
            ("memo_inserted", MEMO_INSERTED.with(Cell::get)),
            ("outcomes_pushed", OUTCOMES_PUSHED.with(Cell::get)),
            ("outcomes_cloned", OUTCOMES_CLONED.with(Cell::get)),
            ("epsilon_transitions", EPSILON_TRANSITIONS.with(Cell::get)),
            ("rule_transitions", RULE_TRANSITIONS.with(Cell::get)),
            (
                "atom_range_transitions",
                ATOM_RANGE_TRANSITIONS.with(Cell::get),
            ),
            ("single_trans_body", SINGLE_TRANS_BODY.with(Cell::get)),
            ("multi_trans_body", MULTI_TRANS_BODY.with(Cell::get)),
            ("single_trans_rule", SINGLE_TRANS_RULE.with(Cell::get)),
            ("single_trans_atom", SINGLE_TRANS_ATOM.with(Cell::get)),
            ("single_trans_other", SINGLE_TRANS_OTHER.with(Cell::get)),
            ("outcomes_return_0", OUTCOMES_RETURN_0.with(Cell::get)),
            ("outcomes_return_1", OUTCOMES_RETURN_1.with(Cell::get)),
            ("outcomes_return_n", OUTCOMES_RETURN_N.with(Cell::get)),
        ]
    }
    pub fn reset() {
        RFS_CALLS.with(|c| c.set(0));
        RFS_MEMO_HITS.with(|c| c.set(0));
        RFS_MEMO_MISSES.with(|c| c.set(0));
        RFS_VISITING_CYCLE.with(|c| c.set(0));
        MEMO_INSERTED.with(|c| c.set(0));
        OUTCOMES_PUSHED.with(|c| c.set(0));
        OUTCOMES_CLONED.with(|c| c.set(0));
        EPSILON_TRANSITIONS.with(|c| c.set(0));
        RULE_TRANSITIONS.with(|c| c.set(0));
        ATOM_RANGE_TRANSITIONS.with(|c| c.set(0));
        SINGLE_TRANS_BODY.with(|c| c.set(0));
        MULTI_TRANS_BODY.with(|c| c.set(0));
        SINGLE_TRANS_RULE.with(|c| c.set(0));
        SINGLE_TRANS_ATOM.with(|c| c.set(0));
        SINGLE_TRANS_OTHER.with(|c| c.set(0));
        OUTCOMES_RETURN_0.with(|c| c.set(0));
        OUTCOMES_RETURN_1.with(|c| c.set(0));
        OUTCOMES_RETURN_N.with(|c| c.set(0));
    }
    pub fn dump() {
        for (name, value) in snapshot() {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("perf {name}={value}");
            }
        }
    }
}

#[cfg(feature = "perf-counters")]
pub use perf_counters::{dump as dump_perf_counters, reset as reset_perf_counters};
/// Preserve lazy lexing for short or failing inputs, but eagerly fill once the
/// fast recognizer has probed far enough that per-token stream sync dominates.
/// Sixty-four tokens is a small rule-sized window: it keeps startup lazy while
/// switching long inputs to the cheaper filled-stream path before large fanout.
const FAST_RECOGNIZER_DEFERRED_FILL_AT: usize = 64;
/// Parser semantic action reached while recognizing one ATN path.
///
/// Generated parsers use `source_state` to dispatch back to the grammar action
/// rendered for that ATN action transition. The token interval is the current
/// rule's input span at the action site, which covers common target templates
/// such as `$text`. Rule-init actions do not have an ATN action source state,
/// so they are marked separately and may carry an ATN state for expected-token
/// rendering.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserAction {
    source_state: usize,
    rule_index: usize,
    start_index: usize,
    stop_index: Option<usize>,
    rule_init: bool,
    expected_state: Option<usize>,
}

impl ParserAction {
    /// Creates an action event for a recognized parser path.
    pub const fn new(
        source_state: usize,
        rule_index: usize,
        start_index: usize,
        stop_index: Option<usize>,
    ) -> Self {
        Self {
            source_state,
            rule_index,
            start_index,
            stop_index,
            rule_init: false,
            expected_state: None,
        }
    }

    /// Creates an action event for a rule-level `@init` action.
    pub const fn new_rule_init(
        rule_index: usize,
        start_index: usize,
        expected_state: Option<usize>,
    ) -> Self {
        Self {
            source_state: usize::MAX,
            rule_index,
            start_index,
            stop_index: None,
            rule_init: true,
            expected_state,
        }
    }

    /// ATN state that owns the semantic-action transition.
    pub const fn source_state(&self) -> usize {
        self.source_state
    }

    /// Grammar rule index recorded by the serialized ATN action transition.
    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    /// Token-stream index where the active rule began.
    pub const fn start_index(&self) -> usize {
        self.start_index
    }

    /// Last token-stream index consumed before the action was reached.
    pub const fn stop_index(&self) -> Option<usize> {
        self.stop_index
    }

    /// Reports whether this event represents a rule-level `@init` action.
    pub const fn is_rule_init(&self) -> bool {
        self.rule_init
    }

    /// ATN state used to compute expected-token display for this action.
    pub const fn expected_state(&self) -> Option<usize> {
        self.expected_state
    }
}

/// Runtime view passed to parser semantic hooks.
///
/// The context is intentionally read-only with respect to parser structure:
/// predicates may run speculatively during prediction, and hooks can be called
/// more than once for paths that are later abandoned. Lookahead methods may
/// buffer tokens from the underlying token source, matching normal parser
/// prediction behavior.
pub struct ParserSemCtx<'a, S>
where
    S: TokenSource,
{
    input: &'a mut CommonTokenStream<S>,
    rule_index: usize,
    coordinate_index: usize,
    rule_name: Option<String>,
    context: Option<&'a ParserRuleContext>,
    tree: Option<&'a ParseTree>,
    local_int_arg: Option<(usize, i64)>,
    member_values: &'a BTreeMap<usize, i64>,
    action: Option<ParserAction>,
}

impl<S> std::fmt::Debug for ParserSemCtx<'_, S>
where
    S: TokenSource,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserSemCtx")
            .field("rule_index", &self.rule_index)
            .field("coordinate_index", &self.coordinate_index)
            .field("rule_name", &self.rule_name)
            .field("context", &self.context)
            .field("tree", &self.tree)
            .field("local_int_arg", &self.local_int_arg)
            .field("member_values", &self.member_values)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

impl<'a, S> ParserSemCtx<'a, S>
where
    S: TokenSource,
{
    /// Rule index that owns the predicate/action coordinate.
    #[must_use]
    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    /// Rule name that owns the coordinate, when recognizer metadata has it.
    #[must_use]
    pub fn rule_name(&self) -> Option<&str> {
        self.rule_name.as_deref()
    }

    /// Predicate/action index inside the owning rule. Parser actions keyed only
    /// by ATN source state report `usize::MAX` here; use [`Self::action`] for
    /// the stable action event.
    #[must_use]
    pub const fn coordinate_index(&self) -> usize {
        self.coordinate_index
    }

    /// Current token-stream index.
    #[must_use]
    pub fn input_index(&self) -> usize {
        self.input.index()
    }

    /// Token type at one-based lookahead/lookbehind offset.
    pub fn la(&mut self, offset: isize) -> i32 {
        self.input.la(offset)
    }

    /// Token at one-based lookahead/lookbehind offset.
    pub fn lt(&mut self, offset: isize) -> Option<&CommonToken> {
        self.input.lt(offset)
    }

    /// Token text at one-based lookahead/lookbehind offset.
    pub fn token_text(&mut self, offset: isize) -> Option<&str> {
        self.lt(offset).and_then(Token::text)
    }

    /// Current generated rule context, when a generated rule predicate supplied
    /// one.
    #[must_use]
    pub const fn context(&self) -> Option<&'a ParserRuleContext> {
        self.context
    }

    /// Completed parse tree passed to an action hook, if the action is being
    /// replayed after recognition.
    #[must_use]
    pub const fn tree(&self) -> Option<&'a ParseTree> {
        self.tree
    }

    /// Integer local argument visible to this predicate coordinate.
    #[must_use]
    pub fn local_int_arg(&self) -> Option<i64> {
        self.local_int_arg.map(|(_, value)| value)
    }

    /// Integer member value observed on the current speculative path.
    #[must_use]
    pub fn member_int(&self, member: usize) -> Option<i64> {
        self.member_values.get(&member).copied()
    }

    /// Parser action event being replayed, when this context belongs to an
    /// action hook.
    #[must_use]
    pub const fn action(&self) -> Option<ParserAction> {
        self.action
    }

    /// Text covered by a parser action event.
    pub fn action_text(&mut self) -> String {
        let Some(action) = self.action else {
            return String::new();
        };
        let Some(stop) = action.stop_index() else {
            return String::new();
        };
        let stop = if self
            .input
            .get(stop)
            .is_some_and(|token| token.token_type() == TOKEN_EOF)
        {
            let Some(previous) = stop.checked_sub(1) else {
                return String::new();
            };
            previous
        } else {
            stop
        };
        self.input.text(action.start_index(), stop)
    }
}

/// User extension point for parser semantic predicates and actions that the
/// metadata generator did not translate into built-in runtime metadata.
///
/// Returning `None`/`false` says "not handled", so the runtime falls through
/// to the configured [`UnknownSemanticPolicy`]. Predicate hooks may run during
/// speculative prediction and must be replay-safe.
pub trait SemanticHooks {
    fn sempred<S>(
        &mut self,
        ctx: &mut ParserSemCtx<'_, S>,
        rule_index: usize,
        pred_index: usize,
    ) -> Option<bool>
    where
        S: TokenSource,
    {
        let _ = (ctx, rule_index, pred_index);
        None
    }

    fn action<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>, action: ParserAction) -> bool
    where
        S: TokenSource,
    {
        let _ = (ctx, action);
        false
    }

    fn lexer_sempred<I, F>(
        &mut self,
        ctx: &mut LexerSemCtx<'_, I, F>,
        rule_index: usize,
        pred_index: usize,
    ) -> Option<bool>
    where
        I: CharStream,
        F: TokenFactory,
    {
        let _ = (ctx, rule_index, pred_index);
        None
    }

    fn lexer_action<I, F>(
        &mut self,
        ctx: &mut LexerSemCtx<'_, I, F>,
        action: LexerCustomAction,
    ) -> bool
    where
        I: CharStream,
        F: TokenFactory,
    {
        let _ = (ctx, action);
        false
    }
}

/// Default hook object used by parsers that do not need user-supplied
/// semantics.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSemanticHooks;

impl SemanticHooks for NoSemanticHooks {}

/// Parser semantic predicate rendered from a supported target template.
///
/// The metadata recognizer evaluates these at the token-stream index where the
/// predicate transition is reached. Unsupported or absent predicate templates
/// remain unconditional so existing generated parsers keep their previous
/// behavior unless the generator opts into this table.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ParserPredicate {
    True,
    False,
    /// Predicate that always fails and carries ANTLR's `<fail='...'>` message.
    FalseWithMessage {
        message: &'static str,
    },
    /// Target-template test helper that reports predicate evaluation before
    /// returning the wrapped boolean value.
    Invoke {
        value: bool,
    },
    LookaheadTextEquals {
        offset: isize,
        text: &'static str,
    },
    LookaheadNotEquals {
        offset: isize,
        token_type: i32,
    },
    /// Checks that the last two consumed visible tokens were adjacent in the
    /// token stream. Used by C# parser predicates for split operator tokens.
    TokenPairAdjacent,
    /// Checks a generated parser context child by rule index and text.
    ///
    /// If the child is absent the predicate succeeds, matching target helpers
    /// that treat incomplete or non-matching contexts as non-restrictive.
    ContextChildRuleTextNotEquals {
        rule_index: usize,
        text: &'static str,
    },
    /// Compares the current rule invocation's integer argument with a literal
    /// value from a supported `ValEquals("$i", "...")` target template.
    LocalIntEquals {
        value: i64,
    },
    /// Checks ANTLR-style raw predicates like `5 >= $_p` against the current
    /// rule invocation's integer argument.
    LocalIntLessOrEqual {
        value: i64,
    },
    /// Compares a generated parser integer member modulo a literal value.
    MemberModuloEquals {
        member: usize,
        modulus: i64,
        value: i64,
        equals: bool,
    },
    /// Compares a generated parser integer member with a literal value.
    MemberEquals {
        member: usize,
        value: i64,
        equals: bool,
    },
}

impl ParserPredicate {
    /// Lowers the legacy predicate metadata variant into `SemIR`.
    ///
    /// This is the compatibility adapter for generated parsers produced while
    /// the runtime still emitted closed enum tables. Newer generated parsers
    /// emit `SemIR` directly.
    pub fn lower_into_semir(self, ir: &mut SemIr) -> ExprId {
        match self {
            Self::True => ir.expr(PExpr::Bool(true)),
            Self::False | Self::FalseWithMessage { .. } => ir.expr(PExpr::Bool(false)),
            Self::Invoke { value } => ir.expr(PExpr::EvalTrace(value)),
            Self::LookaheadTextEquals { offset, text } => {
                let token = ir.expr(PExpr::TokenText(offset));
                let text = ir.intern(text);
                let text = ir.expr(PExpr::Str(text));
                ir.expr(PExpr::Cmp(CmpOp::Eq, token, text))
            }
            Self::LookaheadNotEquals { offset, token_type } => {
                let actual = ir.expr(PExpr::La(offset));
                let expected = ir.expr(PExpr::Int(i64::from(token_type)));
                ir.expr(PExpr::Cmp(CmpOp::Ne, actual, expected))
            }
            Self::TokenPairAdjacent => ir.expr(PExpr::TokenIndexAdjacent),
            Self::ContextChildRuleTextNotEquals { rule_index, text } => {
                let actual = ir.expr(PExpr::CtxRuleText(rule_index));
                let expected = ir.intern(text);
                let expected = ir.expr(PExpr::Str(expected));
                ir.expr(PExpr::Cmp(CmpOp::Ne, actual, expected))
            }
            Self::LocalIntEquals { value } => local_arg_comparison(ir, CmpOp::Eq, value),
            Self::LocalIntLessOrEqual { value } => local_arg_comparison(ir, CmpOp::Le, value),
            Self::MemberModuloEquals {
                member,
                modulus,
                value,
                equals,
            } => {
                if modulus == 0 {
                    return ir.expr(PExpr::Bool(false));
                }
                let member = ir.expr(PExpr::Member(member));
                let modulus = ir.expr(PExpr::Int(modulus));
                let actual = ir.expr(PExpr::Arith(ArithOp::Mod, member, modulus));
                let expected = ir.expr(PExpr::Int(value));
                ir.expr(PExpr::Cmp(
                    if equals { CmpOp::Eq } else { CmpOp::Ne },
                    actual,
                    expected,
                ))
            }
            Self::MemberEquals {
                member,
                value,
                equals,
            } => {
                let actual = ir.expr(PExpr::Member(member));
                let expected = ir.expr(PExpr::Int(value));
                ir.expr(PExpr::Cmp(
                    if equals { CmpOp::Eq } else { CmpOp::Ne },
                    actual,
                    expected,
                ))
            }
        }
    }

    #[must_use]
    pub const fn failure_message(self) -> Option<&'static str> {
        match self {
            Self::FalseWithMessage { message } => Some(message),
            Self::True
            | Self::False
            | Self::Invoke { .. }
            | Self::LookaheadTextEquals { .. }
            | Self::LookaheadNotEquals { .. }
            | Self::TokenPairAdjacent
            | Self::ContextChildRuleTextNotEquals { .. }
            | Self::LocalIntEquals { .. }
            | Self::LocalIntLessOrEqual { .. }
            | Self::MemberModuloEquals { .. }
            | Self::MemberEquals { .. } => None,
        }
    }
}

fn local_arg_comparison(ir: &mut SemIr, op: CmpOp, value: i64) -> ExprId {
    let local = ir.expr(PExpr::LocalArg);
    let absent = ir.expr(PExpr::IsNull(local));
    let expected = ir.expr(PExpr::Int(value));
    let comparison = ir.expr(PExpr::Cmp(op, local, expected));
    ir.expr(PExpr::Or([absent, comparison].into()))
}

/// Policy for semantic predicate coordinates that have no runtime
/// implementation.
///
/// ANTLR grammars may embed target-language predicates that the metadata
/// generator could not translate into a [`ParserPredicate`] table entry. When
/// recognition reaches such a coordinate the runtime cannot know the grammar
/// author's intent, so the caller chooses how to proceed.
///
/// The default is [`Self::AssumeTrue`], matching the historical behavior of
/// this runtime. That default is deprecated and will change to [`Self::Error`]
/// in a future minor release; grammars relying on unconditional predicates
/// should opt in explicitly.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnknownSemanticPolicy {
    /// Treat the predicate as passing, as if it were absent from the grammar.
    #[default]
    AssumeTrue,
    /// Treat the predicate as failing, removing the guarded alternative.
    AssumeFalse,
    /// Fail the parse with [`AntlrError::Unsupported`] naming every unknown
    /// coordinate that recognition evaluated.
    Error,
}

/// Prediction strategy requested by generated parser harnesses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PredictionMode {
    /// Prefer the clean full-context outcome when alternatives reach the same
    /// input position.
    Ll,
    /// Preserve SLL's first-viable alternative bias at a decision, even when a
    /// later full-context alternative could avoid recovery.
    Sll,
    /// Full LL prediction with exact ambiguity detection for diagnostic runs.
    LlExactAmbigDetection,
}

/// Integer argument metadata for a generated parser rule invocation.
///
/// ANTLR's serialized ATN does not retain Rust-target rule argument values, so
/// the generator records the rule-transition source state and the value that
/// should be visible to semantic predicates inside the callee.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserRuleArg {
    /// ATN state containing the rule transition that receives this argument.
    pub source_state: usize,
    /// Callee rule index for the transition.
    pub rule_index: usize,
    /// Literal fallback value to expose in the callee.
    pub value: i64,
    /// Whether the callee should inherit the caller's current integer argument.
    pub inherit_local: bool,
}

/// Integer member mutation attached to an ATN action transition.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserMemberAction {
    /// ATN state containing the action transition.
    pub source_state: usize,
    /// Generator-assigned integer member id.
    pub member: usize,
    /// Delta applied when the action is reached on one speculative path.
    pub delta: i64,
}

/// Integer return-value assignment attached to an ATN action transition.
///
/// Generated parsers use this metadata when target actions assign a simple
/// return field such as `$y=1000;`. The interpreter applies it while selecting
/// the recognized path so the finished parse tree can answer later
/// `$label.y` action templates.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserReturnAction {
    /// ATN state containing the action transition.
    pub source_state: usize,
    /// Rule index recorded by the serialized action transition.
    pub rule_index: usize,
    /// Return-field name as it appears in the grammar.
    pub name: &'static str,
    /// Literal integer value assigned by the action.
    pub value: i64,
}

impl ParserMemberAction {
    /// Lowers this speculative member mutation into a `SemIR` action.
    pub fn lower_into_semir(self, ir: &mut SemIr) -> ParserSemanticAction {
        let delta = ir.expr(PExpr::Int(self.delta));
        ParserSemanticAction {
            source_state: self.source_state,
            rule_index: usize::MAX,
            stmt: ir.stmt(AStmt::AddMember(self.member, delta)),
            speculative: true,
        }
    }
}

impl ParserReturnAction {
    /// Lowers this committed return-value assignment into a `SemIR` action.
    pub fn lower_into_semir(self, ir: &mut SemIr) -> ParserSemanticAction {
        let name = ir.intern(self.name);
        let value = ir.expr(PExpr::Int(self.value));
        ParserSemanticAction {
            source_state: self.source_state,
            rule_index: self.rule_index,
            stmt: ir.stmt(AStmt::SetReturn(name, value)),
            speculative: false,
        }
    }
}

/// Parser predicate coordinate lowered into [`SemIr`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParserSemanticPredicate {
    /// Serialized rule index that owns this predicate.
    pub rule_index: usize,
    /// Predicate index inside the owning rule.
    pub pred_index: usize,
    /// Root expression in the associated [`ParserSemantics::ir`] arena.
    pub expr: ExprId,
    /// ANTLR `<fail='...'>` message for predicates that intentionally fail.
    pub failure_message: Option<&'static str>,
}

/// Parser action coordinate lowered into [`SemIr`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParserSemanticAction {
    /// ATN state containing the action transition.
    pub source_state: usize,
    /// Serialized rule index recorded by the action transition.
    pub rule_index: usize,
    /// Root statement in the associated [`ParserSemantics::ir`] arena.
    pub stmt: StmtId,
    /// Whether this action may run on speculative recognition paths.
    pub speculative: bool,
}

/// Data-driven semantic tables emitted by generated parsers.
///
/// This is the runtime representation for issue #9's `SemIR` path. Existing
/// `ParserPredicate`, `ParserMemberAction`, and `ParserReturnAction` tables
/// remain accepted as deprecated adapters for generated code produced before
/// this table existed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParserSemantics {
    pub ir: SemIr,
    pub predicates: Vec<ParserSemanticPredicate>,
    pub actions: Vec<ParserSemanticAction>,
}

/// Optional generated-runtime metadata for metadata-driven parser execution.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParserRuntimeOptions<'a> {
    /// Rule indexes whose `@init` actions should be replayed.
    pub init_action_rules: &'a [usize],
    /// Whether generated parse-tree contexts should retain alternative numbers.
    pub track_alt_numbers: bool,
    /// Semantic predicate table keyed by serialized `(rule_index, pred_index)`.
    pub predicates: &'a [(usize, usize, ParserPredicate)],
    /// `SemIR` predicate/action table emitted by newer generated parsers.
    pub semantics: Option<&'a ParserSemantics>,
    /// Rule-call integer argument table keyed by ATN source state.
    pub rule_args: &'a [ParserRuleArg],
    /// Integer member mutations keyed by ATN action source state.
    pub member_actions: &'a [ParserMemberAction],
    /// Integer return assignments keyed by ATN action source state.
    pub return_actions: &'a [ParserReturnAction],
    /// How to evaluate semantic predicate coordinates absent from
    /// `predicates`.
    pub unknown_predicate_policy: UnknownSemanticPolicy,
}

pub trait Parser: Recognizer {
    /// Reports whether generated parser rules should build parse-tree nodes
    /// while recognizing input.
    fn build_parse_trees(&self) -> bool;

    /// Enables or disables parse-tree construction for subsequent rule calls.
    fn set_build_parse_trees(&mut self, build: bool);

    /// Returns the number of parser syntax errors recorded by committed parse
    /// paths so far.
    fn number_of_syntax_errors(&self) -> usize {
        0
    }

    /// Reports whether prediction diagnostic-listener messages are emitted
    /// during parser ATN recognition.
    fn report_diagnostic_errors(&self) -> bool {
        false
    }

    /// Enables or disables ANTLR-style prediction diagnostics for subsequent
    /// rule calls.
    fn set_report_diagnostic_errors(&mut self, _report: bool) {}

    /// Reports the prediction strategy used when selecting among alternatives.
    fn prediction_mode(&self) -> PredictionMode {
        PredictionMode::Ll
    }

    /// Sets the prediction strategy for subsequent rule calls.
    fn set_prediction_mode(&mut self, _mode: PredictionMode) {}
}

#[derive(Debug)]
struct CachedPredictionContext {
    version: usize,
    atn_key: usize,
    context: Rc<PredictionContext>,
}

#[derive(Debug)]
pub struct BaseParser<S, H = NoSemanticHooks> {
    input: CommonTokenStream<S>,
    data: RecognizerData,
    semantic_hooks: H,
    build_parse_trees: bool,
    syntax_errors: usize,
    report_diagnostic_errors: bool,
    prediction_mode: PredictionMode,
    prediction_diagnostics: Vec<ParserDiagnostic>,
    reported_prediction_diagnostics: BTreeSet<(usize, usize, String)>,
    generated_parser_diagnostics: Vec<ParserDiagnostic>,
    generated_sync_expected: Option<TokenBitSet>,
    int_members: BTreeMap<usize, i64>,
    rule_context_stack: Vec<RuleContextFrame>,
    rule_context_version: usize,
    prediction_context_cache: Option<CachedPredictionContext>,
    pending_invoking_states: Vec<isize>,
    precedence_stack: Vec<i32>,
    /// Predicate side effects are observable in a few target-template tests;
    /// speculative recognition may revisit the same coordinate, so replay it
    /// once per parser instance.
    invoked_predicates: Vec<(usize, usize)>,
    /// How to evaluate predicate coordinates missing from the active
    /// predicate table. Set from [`ParserRuntimeOptions`] at each parse entry.
    unknown_predicate_policy: UnknownSemanticPolicy,
    /// Unknown predicate coordinates evaluated by the current parse, recorded
    /// so [`UnknownSemanticPolicy::Error`] can report them after recognition.
    unknown_predicate_hits: Vec<(usize, usize)>,
    /// Per-parse rule FIRST-set cache keyed by rule start state. This keeps
    /// hot rule-transition checks to a vector lookup after the first visit
    /// while the thread-local shared ATN cache still owns the cross-parse
    /// computed value.
    rule_first_set_cache: Vec<Option<Rc<FirstSet>>>,
    /// Per-state expected-symbol cache. `state_expected_symbols` walks every
    /// epsilon-reachable consuming transition and shows up as a hot loop in
    /// `next_recovery_context` and recovery diagnostics on long inputs.
    /// Keying on `state_number` and sharing the result through `Rc` removes
    /// repeated DFS plus per-call `BTreeSet` allocations.
    state_expected_cache: FxHashMap<usize, Rc<BTreeSet<i32>>>,
    /// Same expected-symbol cache as a bitset for generated parser sync.
    /// Successful parses only need `contains` and union; keeping that path out
    /// of `BTreeSet` avoids tree allocation for every nullable loop/optional
    /// check and defers deterministic formatting to diagnostics.
    state_expected_token_cache: FxHashMap<usize, Rc<TokenBitSet>>,
    /// Per-state cache for whether a return state can finish its owning rule
    /// without consuming more input. Generated-parser sync uses this to walk
    /// parent prediction contexts for nullable exits without paying repeated
    /// epsilon-closure searches on every loop or optional decision.
    rule_stop_reach_cache: Vec<Option<bool>>,
    /// Per-parser interner for `recovery_symbols` sets. Speculative recursion
    /// threads the same epsilon-recovery context through hundreds of follow
    /// states; sharing `Rc<BTreeSet<i32>>` instances lets clones reduce to a
    /// reference bump and lets the memo key hash by pointer.
    recovery_symbols_intern: FxHashMap<Rc<BTreeSet<i32>>, Rc<BTreeSet<i32>>>,
    /// Per-decision-state look-1 cache. Built lazily so grammars that rarely
    /// touch a given decision state still pay no upfront cost; once cached,
    /// the recognizer prunes alternatives whose look-1 cannot accept the
    /// current lookahead, letting common SLL decisions reduce to a single
    /// transition walk instead of a full speculative fan-out.
    decision_lookahead_cache: FxHashMap<usize, Rc<DecisionLookahead>>,
    /// Caches the LL(1) alt selection per `(state, lookahead_token)`.
    /// Each multi-trans visit asks "given this decision state and this
    /// lookahead token, which alt do I commit to?" Hitting this cache
    /// turns the question into a hashmap probe instead of re-scanning
    /// the decision's per-transition FIRST sets every visit.
    ll1_decision_cache: FxHashMap<(usize, i32), Option<usize>>,
    /// Per-parse cache for whether an ATN state can reach itself without
    /// consuming input. Only those states need the recursive recognizer's
    /// `(state, token-index)` cycle guard.
    empty_cycle_cache: Vec<Option<bool>>,
    /// Probe state for deciding whether clean-pass one-outcome memo entries
    /// are worth storing for the current parse.
    single_outcome_memo_mode: SingleOutcomeMemoMode,
    single_outcome_probe_seen: FxHashSet<FastRecognizeKey>,
    single_outcome_probe_samples: usize,
    single_outcome_probe_repeats: usize,
    /// Empty recovery-symbols singleton used as the default at rule entry and
    /// after token consumption.
    empty_recovery_symbols: Rc<BTreeSet<i32>>,
    /// Whether the fast recognizer's FIRST-set prefilter is enabled. The
    /// prefilter trims speculative rule calls whose called rule cannot
    /// match the current lookahead, but it also bypasses single-token
    /// insertion / deletion recovery that ANTLR runs at the rule's first
    /// consuming transition. `parse_atn_rule` flips this off and retries
    /// when the first pass produces no clean outcome so the runtime can
    /// repair inputs the reference parser would have repaired.
    fast_first_set_prefilter: bool,
    /// Whether the fast recognizer should explore parser error-recovery paths.
    /// Public rule parsing starts with this disabled for the common valid-input
    /// path and enables it only for the retry that needs ANTLR-style repairs.
    fast_recovery_enabled: bool,
    /// Whether the fast recognizer should record terminal-token nodes while
    /// speculating. Clean valid-input parsing can reconstruct terminals from
    /// selected rule spans after recognition, avoiding many speculative `Rc`
    /// nodes that are thrown away with losing paths.
    fast_token_nodes_enabled: bool,
}

/// Rollback marker for speculative generated parser paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeneratedDiagnosticsCheckpoint {
    diagnostics_len: usize,
    syntax_errors: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuleContextFrame {
    rule_index: usize,
    invoking_state: isize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeOutcome {
    index: usize,
    consumed_eof: bool,
    alt_number: usize,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
    diagnostics: Vec<ParserDiagnostic>,
    decisions: Vec<usize>,
    actions: Vec<ParserAction>,
    nodes: Vec<RecognizedNode>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RecognizedNode {
    Token {
        index: usize,
    },
    ErrorToken {
        index: usize,
    },
    MissingToken {
        token_type: i32,
        at_index: usize,
        text: String,
    },
    Rule {
        rule_index: usize,
        invoking_state: isize,
        alt_number: usize,
        start_index: usize,
        stop_index: Option<usize>,
        return_values: BTreeMap<String, i64>,
        children: Vec<Self>,
    },
    LeftRecursiveBoundary {
        rule_index: usize,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FastRecognizeOutcome {
    index: usize,
    consumed_eof: bool,
    diagnostics: FastDiagnostics,
    /// Speculative parse-tree fragment built up as the recognizer climbs.
    /// The list is held as a persistent cons-list of `Rc`-wrapped nodes so
    /// prepending while chaining recognition outcomes is `O(1)` and cloning
    /// an outcome (memo lookup, dedup, or when fanning a child's tree out
    /// to every follow outcome) only bumps a reference count rather than
    /// deep-copying. On left-recursive grammars the unfolded list can carry
    /// thousands of nodes per speculative path; without the persistent-list
    /// shape recognition becomes super-linear in path length.
    nodes: NodeList,
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
#[allow(clippy::box_collection)]
struct FastDiagnostics(Option<Box<Vec<ParserDiagnostic>>>);

impl FastDiagnostics {
    const fn new() -> Self {
        Self(None)
    }

    #[cfg(test)]
    fn from_vec(diagnostics: Vec<ParserDiagnostic>) -> Self {
        if diagnostics.is_empty() {
            Self::new()
        } else {
            Self(Some(Box::new(diagnostics)))
        }
    }

    fn is_empty(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|diagnostics| diagnostics.is_empty())
    }

    fn as_slice(&self) -> &[ParserDiagnostic] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }

    fn insert(&mut self, index: usize, diagnostic: ParserDiagnostic) {
        self.0
            .get_or_insert_with(Box::default)
            .insert(index, diagnostic);
    }

    fn append(&mut self, other: &mut Self) {
        if other.is_empty() {
            return;
        }
        self.0
            .get_or_insert_with(Box::default)
            .append(other.0.get_or_insert_with(Box::default));
        if other.is_empty() {
            other.0 = None;
        }
    }
}

impl std::ops::Deref for FastDiagnostics {
    type Target = [ParserDiagnostic];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

/// Persistent cons-list of fast-recognizer nodes. The list keeps nodes in the
/// same head-first order as the original `Vec<FastRecognizedNode>` they
/// replaced. Shared tails across speculative outcomes amortize the cost of
/// chaining a child rule's nodes onto every follow outcome.
///
/// `One` is an inline single-element variant: most outcomes carry only one
/// node (a single token or a single rule wrapper), so storing that node
/// directly avoids allocating an `Rc<NodeList>` tail wrapper.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
enum NodeList {
    #[default]
    Empty,
    One(Rc<FastRecognizedNode>),
    Cons {
        head: Rc<FastRecognizedNode>,
        tail: Rc<Self>,
    },
}

impl NodeList {
    /// Creates an empty list.
    const fn new() -> Self {
        Self::Empty
    }

    /// Prepends `node` and returns the new list. Both shared tails and the
    /// new head are reference-counted so this is `O(1)`.
    fn cons(self, node: Rc<FastRecognizedNode>) -> Self {
        match self {
            Self::Empty => Self::One(node),
            existing @ (Self::One(_) | Self::Cons { .. }) => Self::Cons {
                head: node,
                tail: Rc::new(existing),
            },
        }
    }

    /// In-place prepend that takes ownership of `self` via [`std::mem::take`]
    /// so existing call sites can keep using `&mut` access.
    fn prepend(&mut self, node: Rc<FastRecognizedNode>) {
        let owned = std::mem::take(self);
        *self = owned.cons(node);
    }

    /// Materializes the list into a `Vec` in head-first order. Used at the
    /// boundaries that need random-access traversal (the public rule entry
    /// when building the final parse tree, and
    /// `fold_fast_left_recursive_boundaries`).
    fn to_vec(&self) -> Vec<Rc<FastRecognizedNode>> {
        let mut out = Vec::new();
        let mut cursor = self;
        loop {
            match cursor {
                Self::Empty => break,
                Self::One(node) => {
                    out.push(Rc::clone(node));
                    break;
                }
                Self::Cons { head, tail } => {
                    out.push(Rc::clone(head));
                    cursor = tail.as_ref();
                }
            }
        }
        out
    }

    const fn iter(&self) -> NodeListIter<'_> {
        NodeListIter { cursor: self }
    }

    fn len(&self) -> usize {
        self.iter().count()
    }

    fn has_left_recursive_boundary(&self) -> bool {
        self.iter()
            .any(|node| fast_node_has_left_recursive_boundary(node.as_ref()))
    }

    fn has_explicit_token_node(&self) -> bool {
        self.iter().any(|node| {
            matches!(
                node.as_ref(),
                FastRecognizedNode::Token { .. }
                    | FastRecognizedNode::ErrorToken { .. }
                    | FastRecognizedNode::MissingToken { .. }
            )
        })
    }

    /// Builds a list from an already ordered vector.
    fn from_vec(nodes: Vec<Rc<FastRecognizedNode>>) -> Self {
        let mut list = Self::new();
        for node in nodes.into_iter().rev() {
            list.prepend(node);
        }
        list
    }
}

struct NodeListIter<'a> {
    cursor: &'a NodeList,
}

impl<'a> Iterator for NodeListIter<'a> {
    type Item = &'a Rc<FastRecognizedNode>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.cursor {
            NodeList::Empty => None,
            NodeList::One(node) => {
                self.cursor = &NodeList::Empty;
                Some(node)
            }
            NodeList::Cons { head, tail } => {
                self.cursor = tail.as_ref();
                Some(head)
            }
        }
    }
}

/// Minimal parse-tree fragment retained by the fast recognizer so the public
/// rule entry can build nested rule contexts without paying for
/// action/decision bookkeeping.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum FastRecognizedNode {
    Token {
        index: usize,
    },
    ErrorToken {
        index: usize,
    },
    MissingToken {
        token_type: i32,
        at_index: usize,
        text: String,
    },
    Rule {
        rule_index: usize,
        invoking_state: isize,
        start_index: usize,
        stop_index: Option<usize>,
        children: NodeList,
    },
    /// Marker emitted at a precedence-rule loop entry where ANTLR would call
    /// `pushNewRecursionContext`. Folded into a wrapper rule node before the
    /// public rule entry hands the tree to the caller.
    LeftRecursiveBoundary {
        rule_index: usize,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ParserDiagnostic {
    line: usize,
    column: usize,
    message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ExpectedTokens {
    index: Option<usize>,
    symbols: BTreeSet<i32>,
    no_viable: Option<NoViableAlternative>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NoViableAlternative {
    start_index: usize,
    error_index: usize,
}

impl ExpectedTokens {
    /// Records the expected symbols for the farthest token index reached by any
    /// failed ATN path.
    fn record_transition(&mut self, index: usize, transition: &Transition, max_token_type: i32) {
        let symbols = transition_expected_symbols(transition, max_token_type);
        match self.index {
            Some(current) if index < current => {}
            Some(current) if index == current => self.symbols.extend(symbols),
            _ => {
                self.index = Some(index);
                self.symbols = symbols;
            }
        }
    }

    /// Records an ambiguous decision that failed after consuming a shared
    /// prefix, which ANTLR reports as `no viable alternative`.
    const fn record_no_viable(&mut self, start_index: usize, error_index: usize) {
        match self.no_viable {
            Some(current) if error_index < current.error_index => {}
            _ => {
                self.no_viable = Some(NoViableAlternative {
                    start_index,
                    error_index,
                });
            }
        }
    }
}

/// Compact token-type set for parser-internal FIRST/lookahead caches.
///
/// Public diagnostics still use `BTreeSet<i32>` for deterministic formatting,
/// but the hot recognizer path mostly needs `contains` and set union over
/// small token ids. A bitset avoids tree traversal and per-symbol allocation
/// while keeping conversion to `BTreeSet` at recovery/reporting boundaries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TokenBitSet {
    words: Vec<u64>,
}

impl TokenBitSet {
    fn insert(&mut self, symbol: i32) {
        let Some(slot) = token_bit_slot(symbol) else {
            return;
        };
        let word = slot / u64::BITS as usize;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1_u64 << (slot % u64::BITS as usize);
    }

    fn extend_range(&mut self, start: i32, stop: i32) {
        let (start, stop) = if start <= stop {
            (start, stop)
        } else {
            (stop, start)
        };
        if start <= TOKEN_EOF && stop >= TOKEN_EOF {
            self.insert(TOKEN_EOF);
        }
        let positive_start = start.max(1);
        if positive_start > stop {
            return;
        }
        let Some(start_slot) = token_bit_slot(positive_start) else {
            return;
        };
        let Some(stop_slot) = token_bit_slot(stop) else {
            return;
        };
        self.extend_slot_range(start_slot, stop_slot);
    }

    fn extend_slot_range(&mut self, start_slot: usize, stop_slot: usize) {
        if start_slot > stop_slot {
            return;
        }
        let start_word = start_slot / u64::BITS as usize;
        let stop_word = stop_slot / u64::BITS as usize;
        if stop_word >= self.words.len() {
            self.words.resize(stop_word + 1, 0);
        }
        let start_offset = start_slot % u64::BITS as usize;
        let stop_offset = stop_slot % u64::BITS as usize;
        if start_word == stop_word {
            self.words[start_word] |=
                (!0_u64 << start_offset) & (!0_u64 >> (u64::BITS as usize - 1 - stop_offset));
            return;
        }
        self.words[start_word] |= !0_u64 << start_offset;
        for word in &mut self.words[(start_word + 1)..stop_word] {
            *word = !0_u64;
        }
        self.words[stop_word] |= !0_u64 >> (u64::BITS as usize - 1 - stop_offset);
    }

    fn extend_iter(&mut self, symbols: impl IntoIterator<Item = i32>) {
        for symbol in symbols {
            self.insert(symbol);
        }
    }

    fn extend_from(&mut self, other: &Self) {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (left, right) in self.words.iter_mut().zip(&other.words) {
            *left |= *right;
        }
    }

    fn contains(&self, symbol: i32) -> bool {
        let Some(slot) = token_bit_slot(symbol) else {
            return false;
        };
        let word = slot / u64::BITS as usize;
        self.words
            .get(word)
            .is_some_and(|bits| bits & (1_u64 << (slot % u64::BITS as usize)) != 0)
    }

    fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    fn extend_btree_set(&self, target: &mut BTreeSet<i32>) {
        for (word_index, word) in self.words.iter().copied().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                if let Some(symbol) = token_bit_symbol(word_index * u64::BITS as usize + bit) {
                    target.insert(symbol);
                }
                bits &= bits - 1;
            }
        }
    }

    fn to_btree_set(&self) -> BTreeSet<i32> {
        let mut out = BTreeSet::new();
        self.extend_btree_set(&mut out);
        out
    }
}

fn token_bit_slot(symbol: i32) -> Option<usize> {
    if symbol == TOKEN_EOF {
        Some(0)
    } else if symbol > 0 {
        usize::try_from(symbol).ok()
    } else {
        None
    }
}

fn token_bit_symbol(slot: usize) -> Option<i32> {
    if slot == 0 {
        Some(TOKEN_EOF)
    } else {
        i32::try_from(slot).ok()
    }
}

/// Converts one consuming transition into the token types that would satisfy it
/// for diagnostic reporting.
fn transition_expected_symbols(transition: &Transition, max_token_type: i32) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    match transition {
        Transition::Atom { label, .. } => {
            symbols.insert(*label);
        }
        Transition::Range { start, stop, .. } => {
            symbols.extend(*start..=*stop);
        }
        Transition::Set { set, .. } => {
            for (start, stop) in set.ranges() {
                symbols.extend(*start..=*stop);
            }
        }
        Transition::NotSet { set, .. } => {
            symbols.extend((1..=max_token_type).filter(|symbol| !set.contains(*symbol)));
        }
        Transition::Wildcard { .. } => {
            symbols.extend(1..=max_token_type);
        }
        Transition::Epsilon { .. }
        | Transition::Rule { .. }
        | Transition::Predicate { .. }
        | Transition::Action { .. }
        | Transition::Precedence { .. } => {}
    }
    symbols
}

fn transition_expected_token_set(transition: &Transition, max_token_type: i32) -> TokenBitSet {
    let mut symbols = TokenBitSet::default();
    match transition {
        Transition::Atom { label, .. } => {
            symbols.insert(*label);
        }
        Transition::Range { start, stop, .. } => {
            symbols.extend_range(*start, *stop);
        }
        Transition::Set { set, .. } => {
            for (start, stop) in set.ranges() {
                symbols.extend_range(*start, *stop);
            }
        }
        Transition::NotSet { set, .. } => {
            symbols.extend_iter((1..=max_token_type).filter(|symbol| !set.contains(*symbol)));
        }
        Transition::Wildcard { .. } => {
            symbols.extend_range(1, max_token_type);
        }
        Transition::Epsilon { .. }
        | Transition::Rule { .. }
        | Transition::Predicate { .. }
        | Transition::Action { .. }
        | Transition::Precedence { .. } => {}
    }
    symbols
}

/// Returns the consuming-token expectations reachable from an ATN state through
/// epsilon transitions. Recovery diagnostics need this closure so alternatives
/// and loop exits report the same expectation set ANTLR users see.
fn state_expected_symbols(atn: &Atn, state_number: usize) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    let mut stack = vec![state_number];
    let mut visited = BTreeSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current) {
            continue;
        }
        let Some(state) = atn.state(current) else {
            continue;
        };
        for transition in &state.transitions {
            let transition_symbols = transition_expected_symbols(transition, atn.max_token_type());
            if transition_symbols.is_empty() {
                if transition.is_epsilon() {
                    stack.push(transition.target());
                }
            } else {
                symbols.extend(transition_symbols);
            }
        }
    }
    symbols
}

fn state_expected_token_set(atn: &Atn, state_number: usize) -> TokenBitSet {
    let mut symbols = TokenBitSet::default();
    let mut stack = vec![state_number];
    let mut visited = BTreeSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current) {
            continue;
        }
        let Some(state) = atn.state(current) else {
            continue;
        };
        for transition in &state.transitions {
            let transition_symbols =
                transition_expected_token_set(transition, atn.max_token_type());
            if transition_symbols.is_empty() {
                if transition.is_epsilon() {
                    stack.push(transition.target());
                }
            } else {
                symbols.extend_from(&transition_symbols);
            }
        }
    }
    symbols
}

fn state_can_reach_rule_stop(atn: &Atn, state_number: usize) -> bool {
    let Some(rule_index) = atn.state(state_number).and_then(|state| state.rule_index) else {
        return false;
    };
    let Some(&stop_state) = atn.rule_to_stop_state().get(rule_index) else {
        return false;
    };
    epsilon_reaches_state(atn, state_number, stop_state)
}

fn epsilon_reaches_state(atn: &Atn, start: usize, target: usize) -> bool {
    let mut stack = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(current) = stack.pop() {
        if current == target {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        let Some(state) = atn.state(current) else {
            continue;
        };
        stack.extend(
            state
                .transitions
                .iter()
                .filter(|transition| transition.is_epsilon())
                .map(Transition::target),
        );
    }
    false
}

/// FIRST set for a rule entry plus whether the rule is nullable.
///
/// Walks epsilon, predicate, action, and rule-call transitions until it finds
/// a consuming transition or reaches the rule's stop state. Used by the fast
/// recognizer to skip rule alternatives whose first-consumed token cannot
/// possibly match the current lookahead.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FirstSet {
    symbols: TokenBitSet,
    nullable: bool,
}

/// Per-parser cache of FIRST sets computed during recognition. The fast path
/// consults this on every speculative `Transition::Rule` encounter, so the
/// computation must amortize across all of those calls — the FIRST set is a
/// pure function of the ATN, not of the input position. Cached entries are
/// shared via `Rc` so the recognizer never deep-copies the underlying
/// `BTreeSet<i32>`.
type FirstSetCache = FxHashMap<(usize, usize), Rc<FirstSet>>;

// Thread-local FIRST-set caches keyed by the ATN pointer. The FIRST set
// and decision-lookahead entries are purely functions of the grammar's
// ATN, so caching across parses lets repeated parsing of the same grammar
// (the common case for a CLI tool or language server) avoid redoing the
// closure work. Generated parsers hand us a `&'static Atn` whose address
// is stable, which is what we hash on.
type DecisionLookaheadCache = FxHashMap<usize, Rc<DecisionLookahead>>;

#[derive(Default)]
struct SharedAtnCache {
    first_set: FirstSetCache,
    decision_lookahead: DecisionLookaheadCache,
    state_expected_tokens: FxHashMap<usize, Rc<TokenBitSet>>,
    rule_stop_reach: FxHashMap<usize, bool>,
}

thread_local! {
    static SHARED_ATN_CACHES: RefCell<FxHashMap<SharedAtnCacheKey, SharedAtnCache>> =
        RefCell::new(FxHashMap::default());
}

/// Compound key for `SHARED_ATN_CACHES`.
///
/// Generated parsers feed us a `&'static Atn` from a `OnceLock<Atn>`, so the
/// pointer identifies one grammar for the program's lifetime. For the
/// non-`'static` case (a dropped `Atn` whose allocation is later reused),
/// the secondary fields below catch the pointer collision: a new grammar
/// would need to match all of `(states ptr, states len, max_token_type)` to
/// be mistaken for the dropped one. That combination changing under us
/// without a rebuild is implausible enough to treat as a bug; bundling them
/// into the key is otherwise a few extra bytes per lookup.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SharedAtnCacheKey {
    atn: usize,
    states: usize,
    state_count: usize,
    max_token_type: i32,
}

impl SharedAtnCacheKey {
    fn for_atn(atn: &Atn) -> Self {
        Self {
            atn: std::ptr::from_ref::<Atn>(atn) as usize,
            states: atn.states().as_ptr() as usize,
            state_count: atn.states().len(),
            max_token_type: atn.max_token_type(),
        }
    }
}

fn with_shared_first_set_cache<R>(atn: &Atn, f: impl FnOnce(&mut FirstSetCache) -> R) -> R {
    SHARED_ATN_CACHES.with(|cell| {
        let key = SharedAtnCacheKey::for_atn(atn);
        let mut map = cell.borrow_mut();
        let cache = map.entry(key).or_default();
        f(&mut cache.first_set)
    })
}

fn with_shared_atn_caches<R>(atn: &Atn, f: impl FnOnce(&mut SharedAtnCache) -> R) -> R {
    SHARED_ATN_CACHES.with(|cell| {
        let key = SharedAtnCacheKey::for_atn(atn);
        let mut map = cell.borrow_mut();
        let cache = map.entry(key).or_default();
        f(cache)
    })
}

/// Per-decision-state cached look-1 sets for each outgoing transition.
///
/// At a multi-alternative state, the recognizer would otherwise speculatively
/// walk every alternative even when only one can possibly accept the current
/// lookahead. Caching the look-1 set per transition lets us prune the
/// non-viable transitions before recursing — the same SLL prediction trick
/// the reference ANTLR runtime uses, just expressed as a `(state, lookahead)`
/// filter rather than a full DFA.
#[derive(Debug, Default)]
struct DecisionLookahead {
    transitions: Vec<TransitionLookSet>,
}

/// Look-1 information for one outgoing transition.
///
/// `nullable` mirrors `FirstSet::nullable` and is true when the transition
/// can reach the rule stop without consuming a token (e.g. an empty alt).
/// Nullable transitions cannot be pruned: they may still be the right path
/// when the lookahead consumes nothing further inside the current rule.
#[derive(Clone, Debug, Default)]
struct TransitionLookSet {
    symbols: TokenBitSet,
    nullable: bool,
}

/// Mutable bookkeeping shared across one FIRST-set computation. Bundling the
/// rarely-touched fields keeps the recursive helpers below the function-arity
/// lint and lets every nested call thread the same cache and cycle guards.
struct FirstSetCtx<'a> {
    cache: &'a mut FirstSetCache,
    in_progress: BTreeSet<(usize, usize)>,
    hit_cycle: bool,
}

/// Returns the FIRST set for the (rule entry, rule stop) pair, populating the
/// shared cache and tolerating recursive nullable rule chains. Mutually
/// recursive rules cannot stack-overflow because callers in flight are tracked
/// in `ctx.in_progress`; revisits return without recursing, and the partial
/// result is cached only when no cycle was detected during its computation.
///
/// On a cache hit the returned `Rc` is shared with the recognizer so subsequent
/// rule-call probes only pay a reference bump.
fn rule_first_set(
    atn: &Atn,
    target: usize,
    rule_stop_state: usize,
    cache: &mut FirstSetCache,
) -> Rc<FirstSet> {
    if let Some(cached) = cache.get(&(target, rule_stop_state)) {
        return Rc::clone(cached);
    }
    let mut ctx = FirstSetCtx {
        cache,
        in_progress: BTreeSet::new(),
        hit_cycle: false,
    };
    rule_first_set_cached(atn, target, rule_stop_state, &mut ctx)
}

fn rule_first_set_cached(
    atn: &Atn,
    target: usize,
    rule_stop_state: usize,
    ctx: &mut FirstSetCtx<'_>,
) -> Rc<FirstSet> {
    let key = (target, rule_stop_state);
    if let Some(cached) = ctx.cache.get(&key) {
        return Rc::clone(cached);
    }
    if !ctx.in_progress.insert(key) {
        // Cycle: a caller above is already computing this entry. Return an
        // empty FIRST set; that caller's traversal supplies the contributions
        // from the rule's other alternatives.
        return Rc::new(FirstSet::default());
    }
    let saved_hit_cycle = ctx.hit_cycle;
    ctx.hit_cycle = false;
    let mut first = FirstSet::default();
    let mut visited = BTreeSet::new();
    rule_first_set_inner(atn, target, rule_stop_state, ctx, &mut visited, &mut first);
    ctx.in_progress.remove(&key);
    let entry = Rc::new(first);
    if !ctx.hit_cycle {
        ctx.cache.insert(key, Rc::clone(&entry));
    }
    ctx.hit_cycle = saved_hit_cycle || ctx.hit_cycle;
    entry
}

/// Returns the look-1 set for traversing `transition` while still inside the
/// current `rule_stop_state`. Used by the multi-alternative prefilter, which
/// prunes transitions whose look-1 cannot accept the current lookahead.
fn transition_first_set(
    atn: &Atn,
    transition: &Transition,
    rule_stop_state: usize,
    cache: &mut FirstSetCache,
) -> TransitionLookSet {
    match transition {
        Transition::Atom { label, .. } => {
            let mut symbols = TokenBitSet::default();
            symbols.insert(*label);
            TransitionLookSet {
                symbols,
                nullable: false,
            }
        }
        Transition::Range { start, stop, .. } => {
            let mut symbols = TokenBitSet::default();
            symbols.extend_range(*start, *stop);
            TransitionLookSet {
                symbols,
                nullable: false,
            }
        }
        Transition::Set { set, .. } => {
            let mut symbols = TokenBitSet::default();
            for (start, stop) in set.ranges() {
                symbols.extend_range(*start, *stop);
            }
            TransitionLookSet {
                symbols,
                nullable: false,
            }
        }
        Transition::NotSet { set, .. } => {
            let max = atn.max_token_type();
            let mut symbols = TokenBitSet::default();
            symbols.extend_iter((1..=max).filter(|symbol| !set.contains(*symbol)));
            TransitionLookSet {
                symbols,
                nullable: false,
            }
        }
        Transition::Wildcard { .. } => {
            let mut symbols = TokenBitSet::default();
            symbols.extend_range(1, atn.max_token_type());
            TransitionLookSet {
                symbols,
                nullable: false,
            }
        }
        Transition::Epsilon { target }
        | Transition::Action { target, .. }
        | Transition::Predicate { target, .. }
        | Transition::Precedence { target, .. } => {
            // Walk the closure starting at `target` until a consuming transition
            // is reached or the rule stop state is hit.
            let first = rule_first_set(atn, *target, rule_stop_state, cache);
            TransitionLookSet {
                symbols: first.symbols.clone(),
                nullable: first.nullable,
            }
        }
        Transition::Rule {
            target,
            rule_index,
            follow_state,
            ..
        } => {
            let Some(child_stop) = atn.rule_to_stop_state().get(*rule_index).copied() else {
                return TransitionLookSet::default();
            };
            let child = rule_first_set(atn, *target, child_stop, cache);
            let mut symbols = child.symbols.clone();
            let nullable = if child.nullable {
                let follow = rule_first_set(atn, *follow_state, rule_stop_state, cache);
                symbols.extend_from(&follow.symbols);
                follow.nullable
            } else {
                false
            };
            TransitionLookSet { symbols, nullable }
        }
    }
}

/// Reports whether `transition` can be pruned at a multi-alt state because
/// its cached look-1 cannot accept the current lookahead.
///
/// Pruning runs only for non-consuming transitions (Epsilon/Action/Predicate/
/// Rule/Precedence) so consuming transitions still reach the
/// `matches`+recovery path that surfaces single-token deletion / insertion
/// repairs and ANTLR-compatible expected-token sets. When a non-consuming
/// transition is pruned, its FIRST set is folded into `expected` so failed
/// parses produce the same `mismatched input ... expecting ...` diagnostic
/// the no-prefilter baseline would emit.
/// Returns the unique alt index (0-based) when `symbol` falls into exactly
/// one transition's FIRST set and no transition is nullable. Used as an
/// LL(1) commit point: when prediction is unambiguous from the lookahead
/// alone, the recursive recognizer can skip every other alt without paying
/// for the per-transition filter probe.
///
/// `None` signals the caller to fall back to per-transition lookahead
/// filtering. Returning `Some` for an alt whose transition cannot actually
/// match would prune the only viable parse path; this is why we require
/// strict disjointness *and* no nullable transitions in the decision.
fn ll1_unique_alt(entry: &DecisionLookahead, symbol: i32) -> Option<usize> {
    let mut chosen: Option<usize> = None;
    for (index, transition) in entry.transitions.iter().enumerate() {
        if transition.nullable {
            return None;
        }
        if transition.symbols.contains(symbol) {
            if chosen.is_some() {
                return None;
            }
            chosen = Some(index);
        }
    }
    chosen
}

/// Returns the unique greedy alt index (0-based) selected by the current
/// lookahead.
///
/// The shortcut is intentionally conservative around nullable exits. If the
/// current symbol can start a consuming alternative and an empty alternative is
/// also present, one-token lookahead is not enough to know whether the symbol
/// belongs to the current construct or to its caller's follow set. `None`
/// signals the caller to fall back to adaptive prediction.
fn ll1_greedy_alt(entry: &DecisionLookahead, symbol: i32, non_greedy: bool) -> Option<usize> {
    let mut matching_non_nullable_alt = None;
    let mut nullable_alt = None;
    for (index, transition) in entry.transitions.iter().enumerate() {
        if transition.nullable {
            if nullable_alt.is_some() {
                return None;
            }
            nullable_alt = Some(index);
        }
        if transition.symbols.contains(symbol) {
            if transition.nullable {
                continue;
            }
            if matching_non_nullable_alt.is_some() {
                return None;
            }
            matching_non_nullable_alt = Some(index);
        }
    }
    if matching_non_nullable_alt.is_some() && nullable_alt.is_some() {
        return None;
    }
    if non_greedy {
        nullable_alt.or(matching_non_nullable_alt)
    } else {
        matching_non_nullable_alt.or(nullable_alt)
    }
}

fn should_skip_via_lookahead(
    transition: &Transition,
    transition_index: usize,
    lookahead_filter: Option<&(i32, Rc<DecisionLookahead>)>,
    index: usize,
    record_expected: bool,
    expected: &mut ExpectedTokens,
) -> bool {
    let prune_non_consuming = matches!(
        transition,
        Transition::Epsilon { .. }
            | Transition::Action { .. }
            | Transition::Predicate { .. }
            | Transition::Rule { .. }
            | Transition::Precedence { .. }
    );
    if !prune_non_consuming {
        return false;
    }
    let Some((symbol, entry)) = lookahead_filter else {
        return false;
    };
    let Some(set) = entry.transitions.get(transition_index) else {
        return false;
    };
    if set.symbols.contains(*symbol) || set.nullable {
        return false;
    }
    if record_expected && !set.symbols.is_empty() {
        record_pruned_transition_expected(set, index, expected);
    }
    true
}

fn should_skip_rule_via_first_set(
    first: &FirstSet,
    symbol: i32,
    record_expected: bool,
    index: usize,
    expected: &mut ExpectedTokens,
) -> bool {
    if first.nullable || first.symbols.contains(symbol) {
        return false;
    }
    if record_expected && !first.symbols.is_empty() {
        record_token_bit_expected(&first.symbols, index, expected);
    }
    true
}

fn record_token_bit_expected(symbols: &TokenBitSet, index: usize, expected: &mut ExpectedTokens) {
    match expected.index {
        Some(current) if index < current => {}
        Some(current) if index == current => {
            symbols.extend_btree_set(&mut expected.symbols);
        }
        _ => {
            expected.index = Some(index);
            expected.symbols = symbols.to_btree_set();
        }
    }
}

/// Folds a pruned transition's FIRST set into the farthest-expected accumulator.
fn record_pruned_transition_expected(
    set: &TransitionLookSet,
    index: usize,
    expected: &mut ExpectedTokens,
) {
    match expected.index {
        Some(current) if index < current => {}
        Some(current) if index == current => {
            set.symbols.extend_btree_set(&mut expected.symbols);
        }
        _ => {
            expected.index = Some(index);
            expected.symbols = set.symbols.to_btree_set();
        }
    }
}

fn rule_first_set_inner(
    atn: &Atn,
    state_number: usize,
    rule_stop_state: usize,
    ctx: &mut FirstSetCtx<'_>,
    visited: &mut BTreeSet<usize>,
    first: &mut FirstSet,
) {
    if !visited.insert(state_number) {
        return;
    }
    if state_number == rule_stop_state {
        first.nullable = true;
        return;
    }
    let Some(state) = atn.state(state_number) else {
        return;
    };
    for transition in &state.transitions {
        let transition_symbols = transition_expected_symbols(transition, atn.max_token_type());
        if !transition_symbols.is_empty() {
            first.symbols.extend_iter(transition_symbols);
            continue;
        }
        match transition {
            Transition::Epsilon { target }
            | Transition::Action { target, .. }
            | Transition::Predicate { target, .. }
            | Transition::Precedence { target, .. } => {
                rule_first_set_inner(atn, *target, rule_stop_state, ctx, visited, first);
            }
            Transition::Rule {
                target,
                rule_index,
                follow_state,
                ..
            } => {
                let Some(child_stop) = atn.rule_to_stop_state().get(*rule_index).copied() else {
                    continue;
                };
                let child_key = (*target, child_stop);
                if ctx.in_progress.contains(&child_key) && !ctx.cache.contains_key(&child_key) {
                    ctx.hit_cycle = true;
                }
                let child = rule_first_set_cached(atn, *target, child_stop, ctx);
                first.symbols.extend_from(&child.symbols);
                if child.nullable {
                    rule_first_set_inner(atn, *follow_state, rule_stop_state, ctx, visited, first);
                }
            }
            Transition::Atom { .. }
            | Transition::Range { .. }
            | Transition::Set { .. }
            | Transition::NotSet { .. }
            | Transition::Wildcard { .. } => {}
        }
    }
}

/// Returns token types that can resume parsing from `state_number` after a
/// failed child rule, following rule calls as well as epsilon transitions.
fn state_sync_symbols(atn: &Atn, state_number: usize, stop_state: usize) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    state_sync_symbols_inner(
        atn,
        state_number,
        stop_state,
        &mut BTreeSet::new(),
        &mut symbols,
    );
    symbols
}

/// Walks epsilon-like continuations from a parent follow state until it finds
/// consuming tokens that can anchor recovery, or EOF if the parent rule can end.
fn state_sync_symbols_inner(
    atn: &Atn,
    state_number: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
    symbols: &mut BTreeSet<i32>,
) {
    if !visited.insert(state_number) {
        return;
    }
    if state_number == stop_state {
        symbols.insert(TOKEN_EOF);
        return;
    }
    let Some(state) = atn.state(state_number) else {
        return;
    };
    for transition in &state.transitions {
        let transition_symbols = transition_expected_symbols(transition, atn.max_token_type());
        if transition_symbols.is_empty() {
            match transition {
                Transition::Rule { target, .. }
                | Transition::Epsilon { target }
                | Transition::Action { target, .. }
                | Transition::Predicate { target, .. }
                | Transition::Precedence { target, .. } => {
                    state_sync_symbols_inner(atn, *target, stop_state, visited, symbols);
                }
                Transition::Atom { .. }
                | Transition::Range { .. }
                | Transition::Set { .. }
                | Transition::NotSet { .. }
                | Transition::Wildcard { .. } => {}
            }
        } else {
            symbols.extend(transition_symbols);
        }
    }
}

fn state_can_reach_symbol_with_precedence(
    atn: &Atn,
    state_number: usize,
    symbol: i32,
    precedence: i32,
    visited: &mut BTreeSet<usize>,
) -> bool {
    if !visited.insert(state_number) {
        return false;
    }
    let Some(state) = atn.state(state_number) else {
        return false;
    };
    state.transitions.iter().any(|transition| {
        if transition.matches(symbol, 1, atn.max_token_type()) {
            return true;
        }
        if !transition.is_epsilon() {
            return false;
        }
        if matches!(
            transition,
            Transition::Precedence {
                precedence: transition_precedence,
                ..
            } if *transition_precedence < precedence
        ) {
            return false;
        }
        state_can_reach_symbol_with_precedence(
            atn,
            transition.target(),
            symbol,
            precedence,
            visited,
        )
    })
}

fn context_can_match_symbol_before_state(
    atn: &Atn,
    context: &PredictionContext,
    stop_state_number: usize,
    symbol: i32,
) -> bool {
    (0..context.len()).any(|index| {
        context.return_state(index).is_some_and(|return_state| {
            let parent = context
                .parent(index)
                .unwrap_or_else(PredictionContext::empty);
            state_or_parent_can_match_symbol_before_state(
                atn,
                return_state,
                &parent,
                stop_state_number,
                symbol,
                &mut BTreeSet::new(),
            )
        })
    })
}

fn state_or_parent_can_match_symbol_before_state(
    atn: &Atn,
    state_number: usize,
    parent: &Rc<PredictionContext>,
    stop_state_number: usize,
    symbol: i32,
    visited: &mut BTreeSet<usize>,
) -> bool {
    if state_number == EMPTY_RETURN_STATE {
        return false;
    }
    if state_number == stop_state_number {
        return context_can_match_symbol_before_state(atn, parent, stop_state_number, symbol);
    }
    if !visited.insert(state_number) {
        return false;
    }
    let Some(state) = atn.state(state_number) else {
        return false;
    };
    state.transitions.iter().any(|transition| {
        if transition.matches(symbol, 1, atn.max_token_type()) {
            return true;
        }
        transition.is_epsilon()
            && state_or_parent_can_match_symbol_before_state(
                atn,
                transition.target(),
                parent,
                stop_state_number,
                symbol,
                visited,
            )
    })
}

/// Carries recovery expectations and their restart state through epsilon-only
/// paths. ANTLR can report and repair at the decision state even when the
/// failed consuming transition is nested under block or loop epsilon edges.
fn next_recovery_context(
    atn: &Atn,
    state: &AtnState,
    inherited: &BTreeSet<i32>,
    inherited_state: Option<usize>,
) -> (BTreeSet<i32>, Option<usize>) {
    let state_symbols = state_expected_symbols(atn, state.state_number);
    if state.transitions.len() > 1 && !state_symbols.is_empty() {
        let mut symbols = state_symbols;
        symbols.extend(inherited.iter().copied());
        return (symbols, Some(state.state_number));
    }
    (inherited.clone(), inherited_state)
}

fn recovery_expected_symbols(
    atn: &Atn,
    state_number: usize,
    inherited: &BTreeSet<i32>,
) -> BTreeSet<i32> {
    let mut symbols = state_expected_symbols(atn, state_number);
    symbols.extend(inherited.iter().copied());
    symbols
}

/// Fast-recognizer variant of [`next_recovery_context`] that reuses the
/// parser's cached state-expected-symbols sets and the inherited `Rc`
/// without copying when the state cannot widen recovery.
fn fast_next_recovery_context<S, H>(
    parser: &mut BaseParser<S, H>,
    atn: &Atn,
    state: &AtnState,
    inherited: &Rc<BTreeSet<i32>>,
    inherited_state: Option<usize>,
) -> (Rc<BTreeSet<i32>>, Option<usize>)
where
    S: TokenSource,
    H: SemanticHooks,
{
    if state.transitions.len() <= 1 {
        return (Rc::clone(inherited), inherited_state);
    }
    let state_symbols = parser.cached_state_expected_symbols(atn, state.state_number);
    if state_symbols.is_empty() {
        return (Rc::clone(inherited), inherited_state);
    }
    if inherited.is_empty() {
        return (state_symbols, Some(state.state_number));
    }
    if Rc::ptr_eq(&state_symbols, inherited) {
        return (state_symbols, Some(state.state_number));
    }
    let mut combined = (*state_symbols).clone();
    combined.extend(inherited.iter().copied());
    (
        parser.intern_recovery_symbols(combined),
        Some(state.state_number),
    )
}

/// Fast-recognizer variant of [`recovery_expected_symbols`] that reuses the
/// cached state-expected-symbols and avoids cloning when no widening is
/// needed.
fn fast_recovery_expected_symbols<S, H>(
    parser: &mut BaseParser<S, H>,
    atn: &Atn,
    state_number: usize,
    inherited: &Rc<BTreeSet<i32>>,
) -> Rc<BTreeSet<i32>>
where
    S: TokenSource,
    H: SemanticHooks,
{
    let cached = parser.cached_state_expected_symbols(atn, state_number);
    if inherited.is_empty() {
        return cached;
    }
    if cached.is_empty() {
        return Rc::clone(inherited);
    }
    if Rc::ptr_eq(&cached, inherited) {
        return cached;
    }
    let mut combined = (*cached).clone();
    combined.extend(inherited.iter().copied());
    parser.intern_recovery_symbols(combined)
}

struct ParserTableSemCtx<'a> {
    member_values: &'a mut BTreeMap<usize, i64>,
    return_values: &'a mut BTreeMap<String, i64>,
}

impl semir::PredContext for ParserTableSemCtx<'_> {
    fn la(&mut self, _offset: isize) -> i64 {
        i64::from(TOKEN_EOF)
    }

    fn token_text(&mut self, _offset: isize) -> Option<&str> {
        None
    }

    fn token_index_adjacent(&mut self) -> bool {
        false
    }

    fn ctx_rule_text(&self, _rule_index: usize) -> Option<String> {
        None
    }

    fn member(&self, member: usize) -> Option<i64> {
        Some(self.member_values.get(&member).copied().unwrap_or_default())
    }

    fn local_arg(&self) -> Option<i64> {
        None
    }

    fn column(&self) -> Option<i64> {
        None
    }

    fn token_start_column(&self) -> Option<i64> {
        None
    }

    fn token_text_so_far(&self) -> Option<String> {
        None
    }

    fn hook(&mut self, _hook: HookId) -> bool {
        false
    }
}

impl semir::ActContext for ParserTableSemCtx<'_> {
    fn set_member(&mut self, member: usize, value: i64) {
        self.member_values.insert(member, value);
    }

    fn set_return(&mut self, name: &str, value: i64) {
        self.return_values.insert(name.to_owned(), value);
    }

    fn action_hook(&mut self, _hook: HookId) {}
}

/// Applies generated integer-member side effects to one speculative path.
fn apply_member_actions(
    source_state: usize,
    actions: &[ParserMemberAction],
    semantics: Option<&ParserSemantics>,
    values: &mut BTreeMap<usize, i64>,
) {
    for action in actions
        .iter()
        .filter(|action| action.source_state == source_state)
    {
        *values.entry(action.member).or_default() += action.delta;
    }
    let Some(semantics) = semantics else {
        return;
    };
    let mut return_values = BTreeMap::new();
    let mut ctx = ParserTableSemCtx {
        member_values: values,
        return_values: &mut return_values,
    };
    for action in semantics
        .actions
        .iter()
        .filter(|action| action.source_state == source_state && action.speculative)
    {
        semir::exec_stmt(&semantics.ir, action.stmt, &mut ctx);
    }
}

/// Returns the speculative member state after replaying one ATN action state.
fn member_values_after_action(
    source_state: usize,
    actions: &[ParserMemberAction],
    semantics: Option<&ParserSemantics>,
    values: &BTreeMap<usize, i64>,
) -> BTreeMap<usize, i64> {
    let mut values = values.clone();
    apply_member_actions(source_state, actions, semantics, &mut values);
    values
}

/// Returns the speculative rule-return state after replaying one ATN action.
fn return_values_after_action(
    source_state: usize,
    rule_index: usize,
    actions: &[ParserReturnAction],
    semantics: Option<&ParserSemantics>,
    values: &BTreeMap<String, i64>,
) -> BTreeMap<String, i64> {
    let mut values = values.clone();
    for action in actions
        .iter()
        .filter(|action| action.source_state == source_state && action.rule_index == rule_index)
    {
        values.insert(action.name.to_owned(), action.value);
    }
    if let Some(semantics) = semantics {
        let mut member_values = BTreeMap::new();
        let mut ctx = ParserTableSemCtx {
            member_values: &mut member_values,
            return_values: &mut values,
        };
        for action in semantics.actions.iter().filter(|action| {
            action.source_state == source_state
                && action.rule_index == rule_index
                && !action.speculative
        }) {
            semir::exec_stmt(&semantics.ir, action.stmt, &mut ctx);
        }
    }
    values
}

/// Resolves the integer argument visible to a child rule invocation.
fn rule_local_int_arg(
    rule_args: &[ParserRuleArg],
    source_state: usize,
    rule_index: usize,
    local_int_arg: Option<(usize, i64)>,
) -> Option<(usize, i64)> {
    rule_args
        .iter()
        .find(|arg| arg.source_state == source_state && arg.rule_index == rule_index)
        .map(|arg| {
            let value = if arg.inherit_local {
                local_int_arg.map_or(arg.value, |(_, value)| value)
            } else {
                arg.value
            };
            (rule_index, value)
        })
}

/// Builds the terminal recognition outcome for a path that reached its stop
/// state.
fn stop_outcome(
    index: usize,
    consumed_eof: bool,
    rule_alt_number: usize,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
) -> Vec<RecognizeOutcome> {
    vec![RecognizeOutcome {
        index,
        consumed_eof,
        alt_number: rule_alt_number,
        member_values,
        return_values,
        diagnostics: Vec::new(),
        decisions: Vec::new(),
        actions: Vec::new(),
        nodes: Vec::new(),
    }]
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecognizeRequest<'a> {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    init_action_rules: &'a BTreeSet<usize>,
    predicates: &'a [(usize, usize, ParserPredicate)],
    semantics: Option<&'a ParserSemantics>,
    rule_args: &'a [ParserRuleArg],
    member_actions: &'a [ParserMemberAction],
    return_actions: &'a [ParserReturnAction],
    local_int_arg: Option<(usize, i64)>,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
    rule_alt_number: usize,
    track_alt_numbers: bool,
    consumed_eof: bool,
    /// Current left-recursive precedence threshold, matching ANTLR's
    /// `precpred(_ctx, k)` check for generated precedence rules.
    precedence: i32,
    depth: usize,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    local_int_arg: Option<(usize, i64)>,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
    rule_alt_number: usize,
    track_alt_numbers: bool,
    consumed_eof: bool,
    precedence: i32,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EpsilonActionStep {
    source_state: usize,
    target: usize,
    action_rule_index: Option<usize>,
    left_recursive_boundary: Option<usize>,
    decision: Option<usize>,
    decision_start_index: Option<usize>,
    alt_number: usize,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

struct RecognizeScratch<'a> {
    visiting: &'a mut BTreeSet<RecognizeKey>,
    memo: &'a mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    expected: &'a mut ExpectedTokens,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FastRecognizeRequest {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    precedence: i32,
    depth: usize,
    recovery_symbols: Rc<BTreeSet<i32>>,
    recovery_state: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FastRecognizeTopRequest {
    start_state: usize,
    stop_state: usize,
    start_index: usize,
    precedence: i32,
    caller_follow_state: Option<usize>,
}

/// Memo key for the fast recognizer. `recovery_symbols` must come from
/// `intern_recovery_symbols` or `empty_recovery_symbols` before it reaches this
/// key, so equal sets share one allocation and the key can store that
/// allocation's address instead of cloning an `Rc` and walking the full
/// `BTreeSet`. Bypassing the interner would turn content-equal recovery sets
/// into distinct cache coordinates.
#[derive(Clone, Debug)]
struct FastRecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    precedence: i32,
    recovery_symbols_id: usize,
    recovery_state: Option<usize>,
}

impl PartialEq for FastRecognizeKey {
    fn eq(&self, other: &Self) -> bool {
        if self.state_number != other.state_number
            || self.stop_state != other.stop_state
            || self.index != other.index
            || self.rule_start_index != other.rule_start_index
            || self.decision_start_index != other.decision_start_index
            || self.precedence != other.precedence
            || self.recovery_state != other.recovery_state
            || self.recovery_symbols_id != other.recovery_symbols_id
        {
            return false;
        }
        true
    }
}

impl Eq for FastRecognizeKey {}

impl Hash for FastRecognizeKey {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.state_number.hash(hasher);
        self.stop_state.hash(hasher);
        self.index.hash(hasher);
        self.rule_start_index.hash(hasher);
        self.decision_start_index.hash(hasher);
        self.precedence.hash(hasher);
        self.recovery_state.hash(hasher);
        self.recovery_symbols_id.hash(hasher);
    }
}

struct FastRecoveryRequest<'a, 'b> {
    atn: &'a Atn,
    transition: &'a Transition,
    expected_symbols: Rc<BTreeSet<i32>>,
    target: usize,
    request: FastRecognizeRequest,
    visiting: &'b mut FxHashSet<(usize, usize)>,
    memo: &'b mut FxHashMap<FastRecognizeKey, Rc<[FastRecognizeOutcome]>>,
    expected: &'b mut ExpectedTokens,
}

struct FastCurrentTokenDeletionRequest<'a, 'b> {
    atn: &'a Atn,
    expected_symbols: Rc<BTreeSet<i32>>,
    request: FastRecognizeRequest,
    visiting: &'b mut FxHashSet<(usize, usize)>,
    memo: &'b mut FxHashMap<FastRecognizeKey, Rc<[FastRecognizeOutcome]>>,
    expected: &'b mut ExpectedTokens,
}

#[derive(Clone, Copy)]
struct FastChildRuleFailureRecoveryRequest<'a> {
    atn: &'a Atn,
    rule_index: usize,
    start_index: usize,
    follow_state: usize,
    stop_state: usize,
    expected: &'a ExpectedTokens,
}

struct RecoveryRequest<'a, 'b> {
    atn: &'a Atn,
    transition: &'a Transition,
    expected_symbols: BTreeSet<i32>,
    target: usize,
    request: RecognizeRequest<'a>,
    visiting: &'b mut BTreeSet<RecognizeKey>,
    memo: &'b mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
}

struct CurrentTokenDeletionRequest<'a, 'b> {
    atn: &'a Atn,
    expected_symbols: BTreeSet<i32>,
    request: RecognizeRequest<'a>,
    visiting: &'b mut BTreeSet<RecognizeKey>,
    memo: &'b mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
}

/// Carries the state needed after the normal token-recovery strategies fail
/// for a consuming transition.
struct ConsumingFailureFallback<'a> {
    atn: &'a Atn,
    target: usize,
    request: RecognizeRequest<'a>,
    symbol: i32,
    expected_symbols: BTreeSet<i32>,
    decision_start_index: Option<usize>,
    decision: Option<usize>,
}

/// Captures the parent-rule context needed when a called rule fails before it
/// can produce a normal outcome.
struct ChildRuleFailureRecovery<'a> {
    atn: &'a Atn,
    rule_index: usize,
    start_index: usize,
    follow_state: usize,
    stop_state: usize,
    member_values: BTreeMap<usize, i64>,
    expected: &'a ExpectedTokens,
}

/// Bundles the context needed to evaluate one semantic predicate transition.
#[derive(Clone, Copy, Debug)]
struct PredicateEval<'a> {
    index: usize,
    rule_index: usize,
    pred_index: usize,
    predicates: &'a [(usize, usize, ParserPredicate)],
    semantics: Option<&'a ParserSemantics>,
    context: Option<&'a ParserRuleContext>,
    local_int_arg: Option<(usize, i64)>,
    member_values: &'a BTreeMap<usize, i64>,
}

#[derive(Clone, Copy, Debug)]
struct ParserSemanticHookRequest<'a> {
    index: usize,
    rule_index: usize,
    pred_index: usize,
    context: Option<&'a ParserRuleContext>,
    local_int_arg: Option<(usize, i64)>,
    member_values: &'a BTreeMap<usize, i64>,
}

/// Predicate-evaluation context over the recognizer's speculative state.
///
/// This sits in the prediction hot loop, so everything is borrowed: member
/// state read-only from the current speculative path and the rule name
/// straight from recognizer metadata. Predicates are pure by construction
/// ([`semir::PExpr`] has no mutating node); statement execution uses
/// [`ParserTableSemCtx`] (speculative member/return replay) and
/// [`BaseParser::parser_action_hook`] (committed action hooks) instead.
struct ParserSemIrCtx<'a, S, H>
where
    S: TokenSource,
    H: SemanticHooks,
{
    input: &'a mut CommonTokenStream<S>,
    semantic_hooks: &'a mut H,
    rule_index: usize,
    coordinate_index: usize,
    rule_name: Option<&'a str>,
    context: Option<&'a ParserRuleContext>,
    local_int_arg: Option<(usize, i64)>,
    member_values: &'a BTreeMap<usize, i64>,
    invoked_predicates: &'a mut Vec<(usize, usize)>,
}

impl<S, H> semir::PredContext for ParserSemIrCtx<'_, S, H>
where
    S: TokenSource,
    H: SemanticHooks,
{
    fn la(&mut self, offset: isize) -> i64 {
        i64::from(self.input.la(offset))
    }

    fn token_text(&mut self, offset: isize) -> Option<&str> {
        self.input.lt(offset).and_then(Token::text)
    }

    fn token_index_adjacent(&mut self) -> bool {
        let Some(first) = self.input.lt(-2).map(Token::token_index) else {
            return false;
        };
        let Some(second) = self.input.lt(-1).map(Token::token_index) else {
            return false;
        };
        first + 1 == second
    }

    fn ctx_rule_text(&self, rule_index: usize) -> Option<String> {
        self.context.and_then(|context| {
            context.children().iter().find_map(|child| match child {
                ParseTree::Rule(rule) if rule.context().rule_index() == rule_index => {
                    Some(child.text())
                }
                ParseTree::Rule(_) | ParseTree::Terminal(_) | ParseTree::Error(_) => None,
            })
        })
    }

    fn member(&self, member: usize) -> Option<i64> {
        Some(self.member_values.get(&member).copied().unwrap_or_default())
    }

    fn local_arg(&self) -> Option<i64> {
        self.local_int_arg.map(|(_, value)| value)
    }

    fn column(&self) -> Option<i64> {
        None
    }

    fn token_start_column(&self) -> Option<i64> {
        None
    }

    fn token_text_so_far(&self) -> Option<String> {
        None
    }

    fn hook(&mut self, _hook: HookId) -> bool {
        let mut ctx = ParserSemCtx {
            input: &mut *self.input,
            rule_index: self.rule_index,
            coordinate_index: self.coordinate_index,
            rule_name: self.rule_name.map(str::to_owned),
            context: self.context,
            tree: None,
            local_int_arg: self.local_int_arg,
            member_values: self.member_values,
            action: None,
        };
        self.semantic_hooks
            .sempred(&mut ctx, self.rule_index, self.coordinate_index)
            .unwrap_or(false)
    }

    fn trace_bool(&mut self, value: bool) -> bool {
        let key = (self.rule_index, self.coordinate_index);
        if !self.invoked_predicates.contains(&key) {
            self.invoked_predicates.push(key);
            use std::io::Write as _;
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "eval={value}");
        }
        value
    }
}

/// Captures predicate-failure recovery metadata for fail-option predicates.
struct PredicateFailureRecovery<'a> {
    rule_index: usize,
    index: usize,
    message: &'a str,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
    rule_alt_number: usize,
}

#[derive(Debug)]
enum DirectAdaptiveParseControl {
    Fallback(DirectAdaptiveFallback),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectAdaptiveFallback {
    Action,
    InvalidAlt,
    LeftRecursiveBoundary,
    MissingAtn,
    NoTransition,
    Predicate,
    Prediction,
    Precedence,
    RuleStop,
    SemanticContext,
    StepLimit,
    TokenMismatch,
    UnknownDecision,
}

type DirectAdaptiveParseResult<T> = Result<T, DirectAdaptiveParseControl>;

struct DirectAdaptiveParser<'atn, 'sim, S, H = NoSemanticHooks>
where
    S: TokenSource,
    H: SemanticHooks,
{
    parser: &'sim mut BaseParser<S, H>,
    atn: &'atn Atn,
    simulator: &'sim mut ParserAtnSimulator<'atn>,
    decision_by_state: Vec<Option<usize>>,
    steps: usize,
}

/// Outcome of a generated token / set / not-set match that may recover.
///
/// Generated parsers append `children` to the current rule context. `consumed_eof`
/// reports whether the match actually consumed a real EOF terminal — it is true
/// only on a successful match (or single-token deletion that lands on EOF), and
/// always false on single-token insertion, which synthesizes a missing token and
/// consumes nothing. Generated code feeds this into `finish_rule`'s
/// `consumed_eof`, so the rule stop token is recorded as EOF only when EOF was
/// truly matched, matching ANTLR's `matchedEOF` semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedMatch {
    children: Vec<ParseTree>,
    consumed_eof: bool,
}

impl GeneratedMatch {
    /// Parse-tree children produced by the match (the matched terminal, an
    /// error node plus deleted-then-matched terminal, or a single missing-token
    /// error node).
    #[must_use]
    pub fn children(&self) -> &[ParseTree] {
        &self.children
    }

    /// Consumes the result, returning the children for appending to the rule
    /// context.
    #[must_use]
    pub fn into_children(self) -> Vec<ParseTree> {
        self.children
    }

    /// Whether a real EOF terminal was consumed by this match.
    #[must_use]
    pub const fn consumed_eof(&self) -> bool {
        self.consumed_eof
    }
}

impl<S> BaseParser<S, NoSemanticHooks>
where
    S: TokenSource,
{
    /// Creates a parser base over a buffered token stream and recognizer
    /// metadata.
    pub fn new(input: CommonTokenStream<S>, data: RecognizerData) -> Self {
        Self::with_semantic_hooks(input, data, NoSemanticHooks)
    }
}

impl<S, H> BaseParser<S, H>
where
    S: TokenSource,
    H: SemanticHooks,
{
    /// Creates a parser base with caller-owned semantic hooks.
    pub fn with_semantic_hooks(
        input: CommonTokenStream<S>,
        data: RecognizerData,
        semantic_hooks: H,
    ) -> Self {
        Self {
            input,
            data,
            semantic_hooks,
            build_parse_trees: true,
            syntax_errors: 0,
            report_diagnostic_errors: false,
            prediction_mode: PredictionMode::Ll,
            prediction_diagnostics: Vec::new(),
            reported_prediction_diagnostics: BTreeSet::new(),
            generated_parser_diagnostics: Vec::new(),
            generated_sync_expected: None,
            int_members: BTreeMap::new(),
            rule_context_stack: Vec::new(),
            rule_context_version: 0,
            prediction_context_cache: None,
            pending_invoking_states: Vec::new(),
            precedence_stack: vec![0],
            invoked_predicates: Vec::new(),
            unknown_predicate_policy: UnknownSemanticPolicy::default(),
            unknown_predicate_hits: Vec::new(),
            rule_first_set_cache: Vec::new(),
            state_expected_cache: FxHashMap::default(),
            state_expected_token_cache: FxHashMap::default(),
            rule_stop_reach_cache: Vec::new(),
            recovery_symbols_intern: FxHashMap::default(),
            decision_lookahead_cache: FxHashMap::default(),
            ll1_decision_cache: FxHashMap::default(),
            empty_cycle_cache: Vec::new(),
            single_outcome_memo_mode: SingleOutcomeMemoMode::Probe,
            single_outcome_probe_seen: FxHashSet::default(),
            single_outcome_probe_samples: 0,
            single_outcome_probe_repeats: 0,
            empty_recovery_symbols: Rc::new(BTreeSet::new()),
            fast_first_set_prefilter: true,
            fast_recovery_enabled: true,
            fast_token_nodes_enabled: true,
        }
    }

    pub const fn input(&mut self) -> &mut CommonTokenStream<S> {
        &mut self.input
    }

    /// Returns the token stream owned by this parser.
    #[must_use]
    pub const fn token_stream(&self) -> &CommonTokenStream<S> {
        &self.input
    }

    /// Consumes this parser and returns its token stream.
    #[must_use]
    pub fn into_token_stream(self) -> CommonTokenStream<S> {
        self.input
    }

    /// Returns the number of parser syntax errors recorded by committed parse
    /// paths so far.
    pub const fn number_of_syntax_errors(&self) -> usize {
        self.syntax_errors
    }

    /// Records a syntax error that generated parser code returns as fatal before
    /// it can recover into the current rule context.
    pub const fn record_generated_syntax_error(&mut self) {
        self.record_syntax_errors(1);
    }

    const fn record_syntax_errors(&mut self, count: usize) {
        self.syntax_errors = self.syntax_errors.saturating_add(count);
    }

    /// Emits diagnostics buffered by the token stream while generated parser
    /// code was fetching lexer tokens directly.
    pub fn report_token_source_errors(&mut self) {
        report_token_source_errors(&self.input.drain_source_errors());
    }

    /// Captures generated-parser diagnostics and syntax-error count before a
    /// speculative generated rule path.
    pub const fn generated_diagnostics_checkpoint(&self) -> GeneratedDiagnosticsCheckpoint {
        GeneratedDiagnosticsCheckpoint {
            diagnostics_len: self.generated_parser_diagnostics.len(),
            syntax_errors: self.syntax_errors,
        }
    }

    /// Restores generated-parser diagnostics after a speculative rule path failed.
    pub fn restore_generated_diagnostics(&mut self, marker: GeneratedDiagnosticsCheckpoint) {
        self.generated_parser_diagnostics
            .truncate(marker.diagnostics_len);
        self.syntax_errors = marker.syntax_errors;
        self.generated_sync_expected = None;
    }

    /// Emits diagnostics recorded by committed generated parser recovery.
    pub fn report_generated_parser_diagnostics(&mut self) {
        let parser_diagnostics = std::mem::take(&mut self.generated_parser_diagnostics);
        let token_errors = self.input.drain_source_errors();
        report_generated_diagnostics(&parser_diagnostics, &token_errors);
    }

    /// Buffers ANTLR-style ambiguity diagnostics discovered by generated
    /// decision code.
    pub fn record_generated_ambiguity_diagnostic(
        &mut self,
        atn: &Atn,
        state_number: usize,
        start_index: usize,
        stop_index: usize,
        alts: &[usize],
    ) {
        if !self.report_diagnostic_errors || alts.len() < 2 {
            return;
        }
        let Some(decision) = atn
            .decision_to_state()
            .iter()
            .position(|candidate| *candidate == state_number)
        else {
            return;
        };
        let Some(rule_index) = atn.state(state_number).and_then(|state| state.rule_index) else {
            return;
        };
        let rule_name = self
            .rule_names()
            .get(rule_index)
            .map_or_else(|| "<unknown>".to_owned(), Clone::clone);
        let input = display_input_text(&self.input.text(start_index, stop_index));
        let alts = alts
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let key = (decision, start_index, format!("{alts}:{input}"));
        if !self.reported_prediction_diagnostics.insert(key) {
            return;
        }
        let start_token = self.token_at(start_index);
        let stop_token = self.token_at(stop_index);
        self.generated_parser_diagnostics.push(diagnostic_for_token(
            start_token.as_ref(),
            format!("reportAttemptingFullContext d={decision} ({rule_name}), input='{input}'"),
        ));
        self.generated_parser_diagnostics.push(diagnostic_for_token(
            stop_token.as_ref(),
            format!(
                "reportAmbiguity d={decision} ({rule_name}): ambigAlts={{{alts}}}, input='{input}'"
            ),
        ));
    }

    /// Buffers ANTLR-style diagnostic-listener messages produced by generated
    /// parser calls to the adaptive simulator.
    pub fn record_generated_prediction_diagnostic(
        &mut self,
        atn: &Atn,
        state_number: usize,
        prediction: &ParserAtnPrediction,
    ) {
        let Some(diagnostic) = &prediction.diagnostic else {
            return;
        };
        if !self.report_diagnostic_errors || diagnostic.conflicting_alts.len() < 2 {
            return;
        }
        let Some(decision) = atn
            .decision_to_state()
            .iter()
            .position(|candidate| *candidate == state_number)
        else {
            return;
        };
        let Some(rule_index) = atn.state(state_number).and_then(|state| state.rule_index) else {
            return;
        };
        let rule_name = self
            .rule_names()
            .get(rule_index)
            .map_or_else(|| "<unknown>".to_owned(), Clone::clone);
        let attempt_input = display_input_text(
            &self
                .input
                .text(diagnostic.start_index, diagnostic.sll_stop_index),
        );
        let result_input = display_input_text(
            &self
                .input
                .text(diagnostic.start_index, diagnostic.ll_stop_index),
        );
        let alts = diagnostic
            .conflicting_alts
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let key = (
            decision,
            diagnostic.start_index,
            format!(
                "{:?}:{alts}:{attempt_input}:{result_input}",
                diagnostic.kind
            ),
        );
        if !self.reported_prediction_diagnostics.insert(key) {
            return;
        }
        let attempt_token = self.token_at(diagnostic.sll_stop_index);
        self.generated_parser_diagnostics.push(diagnostic_for_token(
            attempt_token.as_ref(),
            format!(
                "reportAttemptingFullContext d={decision} ({rule_name}), input='{attempt_input}'"
            ),
        ));
        let result_token = self.token_at(diagnostic.ll_stop_index);
        let message = match diagnostic.kind {
            ParserAtnPredictionDiagnosticKind::Ambiguity => {
                format!(
                    "reportAmbiguity d={decision} ({rule_name}): ambigAlts={{{alts}}}, input='{result_input}'"
                )
            }
            ParserAtnPredictionDiagnosticKind::ContextSensitivity => {
                format!(
                    "reportContextSensitivity d={decision} ({rule_name}), input='{result_input}'"
                )
            }
        };
        self.generated_parser_diagnostics
            .push(diagnostic_for_token(result_token.as_ref(), message));
    }

    pub fn la(&mut self, offset: isize) -> i32 {
        self.input.la_token(offset)
    }

    pub fn consume(&mut self) {
        IntStream::consume(&mut self.input);
    }

    /// Sets a generated integer member value used by target-template tests.
    pub fn set_int_member(&mut self, member: usize, value: i64) {
        self.int_members.insert(member, value);
    }

    /// Reads a generated integer member value.
    pub fn int_member(&self, member: usize) -> Option<i64> {
        self.int_members.get(&member).copied()
    }

    /// Captures generated integer members before speculative generated parser
    /// execution.
    pub fn int_members_checkpoint(&self) -> BTreeMap<usize, i64> {
        self.int_members.clone()
    }

    /// Restores generated integer members after generated parser fallback.
    pub fn restore_int_members(&mut self, members: BTreeMap<usize, i64>) {
        self.int_members = members;
    }

    /// Adds `delta` to a generated integer member and returns the new value.
    pub fn add_int_member(&mut self, member: usize, delta: i64) -> i64 {
        let value = self.int_members.entry(member).or_default();
        *value += delta;
        *value
    }

    /// Matches and consumes the current token when it has the expected token
    /// type.
    ///
    /// On success the consumed token is wrapped as a terminal parse-tree node.
    /// On mismatch the error carries vocabulary display names so diagnostics are
    /// stable across literal and symbolic token naming.
    pub fn match_token(&mut self, token_type: i32) -> Result<ParseTree, AntlrError> {
        let current = self
            .input
            .lt_ref(1)
            .ok_or_else(|| AntlrError::ParserError {
                line: 0,
                column: 0,
                message: "missing current token".to_owned(),
            })?;
        if current.token_type() == token_type {
            self.consume();
            Ok(ParseTree::Terminal(TerminalNode::from_ref(current)))
        } else {
            Err(AntlrError::MismatchedInput {
                expected: self.vocabulary().display_name(token_type),
                found: self.vocabulary().display_name(current.token_type()),
            })
        }
    }

    /// Matches a token from generated recursive-descent code, including ANTLR's
    /// single-token insertion recovery when the active rule context can legally
    /// continue at the current input symbol.
    pub fn match_token_recovering(
        &mut self,
        token_type: i32,
        follow_state: usize,
        atn: &Atn,
    ) -> Result<GeneratedMatch, AntlrError> {
        let current = self
            .input
            .lt_ref(1)
            .ok_or_else(|| AntlrError::ParserError {
                line: 0,
                column: 0,
                message: "missing current token".to_owned(),
            })?;
        if current.token_type() == token_type {
            self.generated_sync_expected = None;
            let consumed_eof = current.token_type() == TOKEN_EOF;
            self.consume();
            return Ok(GeneratedMatch {
                children: vec![ParseTree::Terminal(TerminalNode::from_ref(current))],
                consumed_eof,
            });
        }
        let mut expected_symbols = BTreeSet::new();
        expected_symbols.insert(token_type);
        self.recover_generated_match(
            current.as_ref().clone(),
            &expected_symbols,
            follow_state,
            atn,
            |symbol| symbol == token_type,
        )
    }

    pub fn match_set_recovering(
        &mut self,
        intervals: &[(i32, i32)],
        follow_state: usize,
        atn: &Atn,
    ) -> Result<GeneratedMatch, AntlrError> {
        let current = self
            .input
            .lt_ref(1)
            .ok_or_else(|| AntlrError::ParserError {
                line: 0,
                column: 0,
                message: "missing current token".to_owned(),
            })?;
        if interval_set_contains(intervals, current.token_type()) {
            self.generated_sync_expected = None;
            let consumed_eof = current.token_type() == TOKEN_EOF;
            self.consume();
            return Ok(GeneratedMatch {
                children: vec![ParseTree::Terminal(TerminalNode::from_ref(current))],
                consumed_eof,
            });
        }
        let expected_symbols = interval_symbols(intervals);
        self.recover_generated_match(
            current.as_ref().clone(),
            &expected_symbols,
            follow_state,
            atn,
            |symbol| interval_set_contains(intervals, symbol),
        )
    }

    pub fn match_not_set_recovering(
        &mut self,
        intervals: &[(i32, i32)],
        min_vocabulary: i32,
        max_vocabulary: i32,
        follow_state: usize,
        atn: &Atn,
    ) -> Result<GeneratedMatch, AntlrError> {
        let current = self
            .input
            .lt_ref(1)
            .ok_or_else(|| AntlrError::ParserError {
                line: 0,
                column: 0,
                message: "missing current token".to_owned(),
            })?;
        if (min_vocabulary..=max_vocabulary).contains(&current.token_type())
            && !interval_set_contains(intervals, current.token_type())
        {
            self.generated_sync_expected = None;
            let consumed_eof = current.token_type() == TOKEN_EOF;
            self.consume();
            return Ok(GeneratedMatch {
                children: vec![ParseTree::Terminal(TerminalNode::from_ref(current))],
                consumed_eof,
            });
        }
        let expected_symbols =
            interval_complement_symbols(intervals, min_vocabulary, max_vocabulary);
        self.recover_generated_match(
            current.as_ref().clone(),
            &expected_symbols,
            follow_state,
            atn,
            |symbol| {
                (min_vocabulary..=max_vocabulary).contains(&symbol)
                    && !interval_set_contains(intervals, symbol)
            },
        )
    }

    fn recover_generated_match(
        &mut self,
        current: CommonToken,
        expected_symbols: &BTreeSet<i32>,
        follow_state: usize,
        atn: &Atn,
        matches: impl Fn(i32) -> bool,
    ) -> Result<GeneratedMatch, AntlrError> {
        let expected_display = self.expected_symbols_display(expected_symbols);
        if current.token_type() != TOKEN_EOF
            && let Some(next) = self.input.lt(2).cloned()
            && matches(next.token_type())
        {
            let message = format!(
                "extraneous input {} expecting {expected_display}",
                token_input_display(&current)
            );
            self.push_generated_parser_diagnostic(diagnostic_for_token(Some(&current), message));
            self.record_syntax_errors(1);
            self.generated_sync_expected = None;
            // Single-token deletion: skip `current`, then accept `next`. The
            // accepted token can be EOF only if it is a real EOF terminal.
            let consumed_eof = next.token_type() == TOKEN_EOF;
            self.consume();
            self.consume();
            return Ok(GeneratedMatch {
                children: vec![
                    ParseTree::Error(ErrorNode::new(current)),
                    ParseTree::Terminal(TerminalNode::new(next)),
                ],
                consumed_eof,
            });
        }
        let follow_symbols = self.generated_recovery_follow_symbols(atn, follow_state);
        // ANTLR's `singleTokenInsertion` inserts a missing token when the state
        // *after* the current element can consume the current symbol. At EOF that
        // only holds when the follow state EXPLICITLY expects EOF (e.g. an `EOF`
        // terminal follows in the rule, as in `r: . EOF;` or `r: ID EOF;`), not
        // when EOF merely leaks in from the empty enclosing context (as in
        // `start: ID+;` on empty input — antlr#6 `InvalidEmptyInput`, which must
        // stay a `mismatched input` error). `follow_symbols` mixes both sources,
        // so consult the follow state's OWN expected set for the explicit case.
        let follow_explicitly_expects_eof = current.token_type() == TOKEN_EOF
            && self
                .cached_state_expected_symbols(atn, follow_state)
                .contains(&TOKEN_EOF);
        if follow_symbols.contains(&current.token_type())
            && (current.token_type() != TOKEN_EOF
                || self.rule_context_stack.len() > 1
                || expected_symbols.is_empty()
                || follow_explicitly_expects_eof)
        {
            let message = format!(
                "missing {expected_display} at {}",
                token_input_display(&current)
            );
            self.push_generated_parser_diagnostic(diagnostic_for_token(Some(&current), message));
            self.record_syntax_errors(1);
            self.generated_sync_expected = None;
            let token_type = expected_symbols.iter().next().copied().unwrap_or(TOKEN_EOF);
            let mut missing_symbol = BTreeSet::new();
            missing_symbol.insert(token_type);
            let missing_display = self.expected_symbols_display(&missing_symbol);
            let token = CommonToken::new(token_type)
                .with_text(format!("<missing {missing_display}>"))
                .with_span(usize::MAX, usize::MAX)
                .with_position(current.line(), current.column());
            // Single-token insertion synthesizes a missing token and consumes
            // nothing, so no EOF terminal is consumed even when the lookahead is
            // EOF. Reporting consumed_eof=false here is what keeps `finish_rule`
            // from recording EOF as the rule stop on this recovery path.
            return Ok(GeneratedMatch {
                children: vec![ParseTree::Error(ErrorNode::new(token))],
                consumed_eof: false,
            });
        }
        let mismatch_expected = self.generated_sync_expected.take().map_or_else(
            || expected_symbols.clone(),
            |symbols| symbols.to_btree_set(),
        );
        let mismatch_expected_display = self.expected_symbols_display(&mismatch_expected);
        Err(AntlrError::ParserError {
            line: current.line(),
            column: current.column(),
            message: format!(
                "mismatched input {} expecting {mismatch_expected_display}",
                token_input_display(&current)
            ),
        })
    }

    fn generated_recovery_follow_symbols(
        &mut self,
        atn: &Atn,
        follow_state: usize,
    ) -> BTreeSet<i32> {
        let mut follow = self
            .cached_state_expected_symbols(atn, follow_state)
            .as_ref()
            .clone();
        if self.cached_state_can_reach_rule_stop(atn, follow_state) {
            follow.extend(self.context_expected_symbols(atn));
        }
        follow
    }

    pub fn match_eof(&mut self) -> Result<ParseTree, AntlrError> {
        self.match_token(TOKEN_EOF)
    }

    pub fn match_set(&mut self, intervals: &[(i32, i32)]) -> Result<ParseTree, AntlrError> {
        self.match_interval_condition(intervals, |symbol| interval_set_contains(intervals, symbol))
    }

    pub fn match_not_set(
        &mut self,
        intervals: &[(i32, i32)],
        min_vocabulary: i32,
        max_vocabulary: i32,
    ) -> Result<ParseTree, AntlrError> {
        self.match_interval_condition(intervals, |symbol| {
            (min_vocabulary..=max_vocabulary).contains(&symbol)
                && !interval_set_contains(intervals, symbol)
        })
    }

    fn match_interval_condition(
        &mut self,
        intervals: &[(i32, i32)],
        matches: impl FnOnce(i32) -> bool,
    ) -> Result<ParseTree, AntlrError> {
        let current = self
            .input
            .lt_ref(1)
            .ok_or_else(|| AntlrError::ParserError {
                line: 0,
                column: 0,
                message: "missing current token".to_owned(),
            })?;
        if matches(current.token_type()) {
            self.consume();
            Ok(ParseTree::Terminal(TerminalNode::from_ref(current)))
        } else {
            Err(AntlrError::MismatchedInput {
                expected: self.interval_display(intervals),
                found: self.vocabulary().display_name(current.token_type()),
            })
        }
    }

    fn interval_display(&self, intervals: &[(i32, i32)]) -> String {
        let values = intervals
            .iter()
            .map(|(start, stop)| {
                if start == stop {
                    self.vocabulary().display_name(*start)
                } else {
                    format!(
                        "{}..{}",
                        self.vocabulary().display_name(*start),
                        self.vocabulary().display_name(*stop)
                    )
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{{{values}}}")
    }

    pub const fn rule_node(&self, context: ParserRuleContext) -> ParseTree {
        ParseTree::Rule(RuleNode::new(context))
    }

    /// Enters a generated parser rule and returns the context object the
    /// generated method should populate.
    pub fn enter_rule(&mut self, state: isize, rule_index: usize) -> ParserRuleContext {
        self.set_state(state);
        let invoking_state = self.pending_invoking_states.pop().unwrap_or(state);
        self.rule_context_stack.push(RuleContextFrame {
            rule_index,
            invoking_state,
        });
        self.invalidate_prediction_context_cache();
        let start_index = self.current_visible_index();
        let mut context = ParserRuleContext::new(rule_index, invoking_state);
        if let Some(token) = self.token_ref_at(start_index) {
            context.set_start_ref(token);
        }
        context
    }

    /// Records the ATN source state for the next generated rule invocation.
    ///
    /// ANTLR's full-context prediction reconstructs caller follow states from
    /// each active rule context's invoking state. Generated Rust rule methods are
    /// plain functions, so the caller supplies that ATN state just before making a
    /// rule call; `enter_rule` consumes it when the callee starts.
    pub fn push_invoking_state(&mut self, invoking_state: isize) -> usize {
        let marker = self.pending_invoking_states.len();
        self.pending_invoking_states.push(invoking_state);
        marker
    }

    /// Discards an invoking-state marker if the callee did not consume it.
    pub fn discard_invoking_state(&mut self, marker: usize) {
        self.pending_invoking_states.truncate(marker);
    }

    /// Exits the current generated parser rule.
    pub fn exit_rule(&mut self) {
        self.rule_context_stack.pop();
        self.invalidate_prediction_context_cache();
    }

    /// Converts the active generated-parser rule stack into an ANTLR prediction
    /// context for full-context adaptive prediction.
    pub fn prediction_context(&mut self, atn: &Atn) -> Rc<PredictionContext> {
        let atn_ptr: *const Atn = atn;
        let atn_key = atn_ptr as usize;
        if let Some(cached) = &self.prediction_context_cache
            && cached.version == self.rule_context_version
            && cached.atn_key == atn_key
        {
            return Rc::clone(&cached.context);
        }
        let mut context = PredictionContext::empty();
        for frame in self.rule_context_stack.iter().skip(1) {
            let Ok(state_number) = usize::try_from(frame.invoking_state) else {
                continue;
            };
            let Some(Transition::Rule { follow_state, .. }) = atn
                .state(state_number)
                .and_then(|state| state.transitions.first())
            else {
                continue;
            };
            context = PredictionContext::singleton(context, *follow_state);
        }
        self.prediction_context_cache = Some(CachedPredictionContext {
            version: self.rule_context_version,
            atn_key,
            context: Rc::clone(&context),
        });
        context
    }

    fn invalidate_prediction_context_cache(&mut self) {
        self.rule_context_version = self.rule_context_version.wrapping_add(1);
        self.prediction_context_cache = None;
    }

    /// Adds a generated parser child only when parse-tree construction is
    /// enabled. The match is recorded on the context either way (via `add_child`,
    /// or `note_matched_child` when trees are off) so generated recovery can tell
    /// whether the rule has matched anything yet without depending on `children`.
    pub fn add_parse_child(&self, context: &mut ParserRuleContext, child: ParseTree) {
        if self.build_parse_trees {
            context.add_child(child);
        } else {
            context.note_matched_child();
        }
    }

    /// Finishes a generated parser rule and returns its parse-tree node.
    pub fn finish_rule(&mut self, mut context: ParserRuleContext, consumed_eof: bool) -> ParseTree {
        let stop_index = self.rule_stop_token_index(self.input.index(), consumed_eof);
        if let Some(token) = stop_index.and_then(|index| self.token_ref_at(index)) {
            context.set_stop_ref(token);
        }
        self.exit_rule();
        self.rule_node(context)
    }

    /// Recovers a generated rule catch block after a committed mismatch.
    ///
    /// ANTLR's generated parsers catch recognition errors inside each rule,
    /// report the original error, then consume unexpected tokens until the
    /// caller's recovery set can resume. Tokens consumed during recovery become
    /// error nodes in the current rule context.
    pub fn recover_generated_rule(
        &mut self,
        context: &mut ParserRuleContext,
        atn: &Atn,
        error: AntlrError,
    ) {
        let diagnostic = self.generated_rule_error_diagnostic(error);
        self.push_generated_parser_diagnostic(diagnostic);
        self.generated_sync_expected = None;
        let recovery_symbols = self.context_expected_symbols(atn);
        loop {
            let symbol = self.la(1);
            if symbol == TOKEN_EOF || recovery_symbols.contains(&symbol) {
                break;
            }
            let Some(token) = self.input.lt(1).cloned() else {
                break;
            };
            self.consume();
            self.add_parse_child(context, ParseTree::Error(ErrorNode::new(token)));
        }
        self.record_syntax_errors(1);
    }

    fn push_generated_parser_diagnostic(&mut self, diagnostic: ParserDiagnostic) {
        if self
            .generated_parser_diagnostics
            .iter()
            .any(|existing| existing == &diagnostic)
        {
            return;
        }
        self.generated_parser_diagnostics.push(diagnostic);
    }

    fn generated_rule_error_diagnostic(&mut self, error: AntlrError) -> ParserDiagnostic {
        match error {
            AntlrError::ParserError {
                line,
                column,
                message,
            } => ParserDiagnostic {
                line,
                column,
                message,
            },
            AntlrError::MismatchedInput { expected, found } => diagnostic_for_token(
                self.input.lt(1),
                format!("mismatched input {found} expecting {expected}"),
            ),
            AntlrError::NoViableAlternative { input } => diagnostic_for_token(
                self.input.lt(1),
                format!("no viable alternative at input {input}"),
            ),
            AntlrError::LexerError {
                line,
                column,
                message,
            } => ParserDiagnostic {
                line,
                column,
                message,
            },
            AntlrError::Unsupported(message) => diagnostic_for_token(self.input.lt(1), message),
        }
    }

    /// Finishes a generated left-recursive parser rule and returns its parse-tree node.
    pub fn finish_recursion_rule(
        &mut self,
        mut context: ParserRuleContext,
        consumed_eof: bool,
    ) -> ParseTree {
        let stop_index = self.rule_stop_token_index(self.input.index(), consumed_eof);
        if let Some(token) = stop_index.and_then(|index| self.token_ref_at(index)) {
            context.set_stop_ref(token);
        }
        self.unroll_recursion_context();
        self.rule_node(context)
    }

    /// Enters a generated left-recursive rule at `precedence`.
    pub fn enter_recursion_rule(
        &mut self,
        state: isize,
        rule_index: usize,
        precedence: i32,
    ) -> ParserRuleContext {
        self.precedence_stack.push(precedence);
        self.enter_rule(state, rule_index)
    }

    /// Replaces the current context while expanding a left-recursive rule.
    pub fn push_new_recursion_context(
        &mut self,
        state: isize,
        rule_index: usize,
    ) -> ParserRuleContext {
        self.set_state(state);
        ParserRuleContext::new(rule_index, state)
    }

    /// Wraps the previous left-recursive context before parsing the next
    /// recursive operator alternative.
    pub fn push_new_recursion_context_with_previous(
        &mut self,
        state: isize,
        rule_index: usize,
        current: &mut ParserRuleContext,
    ) {
        self.set_state(state);
        if let Some(stop) = self
            .rule_stop_token_index(self.input.index(), false)
            .and_then(|index| self.token_ref_at(index))
        {
            current.set_stop_ref(stop);
        }
        let invoking_state = current.invoking_state();
        let start = current.start_ref();
        let mut replacement = ParserRuleContext::new(rule_index, invoking_state);
        if let Some(start) = start {
            replacement.set_start_ref(start);
        }
        let previous = std::mem::replace(current, replacement);
        if self.build_parse_trees {
            current.add_child(self.rule_node(previous));
        }
    }

    /// Leaves a generated left-recursive rule.
    pub fn unroll_recursion_context(&mut self) {
        if self.precedence_stack.len() > 1 {
            self.precedence_stack.pop();
        }
        self.exit_rule();
    }

    /// Checks whether a generated left-recursive loop has an operator
    /// alternative that can start at the current token under the active
    /// precedence. The operator block still performs adaptive prediction; this
    /// guard only decides whether the loop should enter or exit.
    pub fn left_recursive_loop_enter_matches(
        &mut self,
        atn: &Atn,
        state_number: usize,
        precedence: i32,
    ) -> bool {
        let symbol = self.la(1);
        if symbol == TOKEN_EOF {
            return false;
        }
        let Some(state) = atn.state(state_number) else {
            return false;
        };
        let context = self.prediction_context(atn);
        if context_can_match_symbol_before_state(atn, &context, state_number, symbol) {
            return false;
        }
        state.transitions.iter().any(|transition| {
            let target = transition.target();
            if atn
                .state(target)
                .is_some_and(|state| state.kind == AtnStateKind::LoopEnd)
            {
                return false;
            }
            state_can_reach_symbol_with_precedence(
                atn,
                target,
                symbol,
                precedence,
                &mut BTreeSet::new(),
            )
        })
    }

    /// Implements generated `precpred(_ctx, k)` checks.
    pub fn precpred(&self, precedence: i32) -> bool {
        precedence >= self.precedence_stack.last().copied().unwrap_or_default()
    }

    /// Evaluates a generated parser semantic predicate at the current input
    /// position.
    pub fn parser_semantic_predicate_matches(
        &mut self,
        predicates: &[(usize, usize, ParserPredicate)],
        rule_index: usize,
        pred_index: usize,
    ) -> bool {
        self.parser_semantic_predicate_matches_inner(predicates, rule_index, pred_index, None)
    }

    /// Evaluates a generated parser semantic predicate with the current integer
    /// rule argument exposed as `$_p`/`$i` metadata where applicable.
    pub fn parser_semantic_predicate_matches_with_local(
        &mut self,
        predicates: &[(usize, usize, ParserPredicate)],
        rule_index: usize,
        pred_index: usize,
        local_int_arg: i32,
    ) -> bool {
        self.parser_semantic_predicate_matches_inner(
            predicates,
            rule_index,
            pred_index,
            Some((rule_index, i64::from(local_int_arg))),
        )
    }

    fn parser_semantic_predicate_matches_inner(
        &mut self,
        predicates: &[(usize, usize, ParserPredicate)],
        rule_index: usize,
        pred_index: usize,
        local_int_arg: Option<(usize, i64)>,
    ) -> bool {
        let index = self.input.index();
        let member_values = self.int_members.clone();
        self.parser_predicate_matches(PredicateEval {
            index,
            rule_index,
            pred_index,
            predicates,
            semantics: None,
            context: None,
            local_int_arg,
            member_values: &member_values,
        })
    }

    /// Evaluates a generated parser semantic predicate with access to the
    /// current generated rule context.
    pub fn parser_semantic_predicate_matches_with_context_and_local(
        &mut self,
        predicates: &[(usize, usize, ParserPredicate)],
        rule_index: usize,
        pred_index: usize,
        context: &ParserRuleContext,
        local_int_arg: i32,
    ) -> bool {
        let index = self.input.index();
        let member_values = self.int_members.clone();
        self.parser_predicate_matches(PredicateEval {
            index,
            rule_index,
            pred_index,
            predicates,
            semantics: None,
            context: Some(context),
            local_int_arg: Some((rule_index, i64::from(local_int_arg))),
            member_values: &member_values,
        })
    }

    /// Evaluates a generated `SemIR` parser predicate with access to the current
    /// generated rule context.
    pub fn parser_semantic_ir_predicate_matches_with_context_and_local(
        &mut self,
        semantics: &ParserSemantics,
        rule_index: usize,
        pred_index: usize,
        context: &ParserRuleContext,
        local_int_arg: i32,
    ) -> bool {
        let index = self.input.index();
        let member_values = self.int_members.clone();
        self.parser_predicate_matches(PredicateEval {
            index,
            rule_index,
            pred_index,
            predicates: &[],
            semantics: Some(semantics),
            context: Some(context),
            local_int_arg: Some((rule_index, i64::from(local_int_arg))),
            member_values: &member_values,
        })
    }

    /// Returns a generated fail-option message for a parser semantic
    /// predicate coordinate.
    pub fn parser_semantic_predicate_failure_message(
        &self,
        rule_index: usize,
        pred_index: usize,
        predicates: &[(usize, usize, ParserPredicate)],
    ) -> Option<&'static str> {
        self.parser_predicate_failure_message(rule_index, pred_index, predicates)
    }

    /// Matches any non-EOF token.
    pub fn match_wildcard(&mut self) -> Result<ParseTree, AntlrError> {
        let current = self
            .input
            .lt_ref(1)
            .ok_or_else(|| AntlrError::ParserError {
                line: 0,
                column: 0,
                message: "missing current token".to_owned(),
            })?;
        if current.token_type() == TOKEN_EOF {
            return Err(AntlrError::MismatchedInput {
                expected: "wildcard".to_owned(),
                found: self.vocabulary().display_name(TOKEN_EOF),
            });
        }
        self.consume();
        Ok(ParseTree::Terminal(TerminalNode::from_ref(current)))
    }

    /// Generated parser synchronization hook. The current interpreter owns
    /// recovery; direct generated methods can call this as a no-op until the
    /// generated recovery strategy is expanded.
    #[allow(clippy::unnecessary_wraps)]
    pub fn sync(&mut self, state: isize) -> Result<(), AntlrError> {
        self.set_state(state);
        Ok(())
    }

    /// Synchronizes a generated parser decision against the ATN lookahead set.
    ///
    /// ANTLR generated parsers call the error strategy before optional and loop
    /// decisions. When the current token cannot start any alternative, follow a
    /// nullable exit, or be deleted before a later synchronization token, the
    /// generated Rust method reports that decision-level mismatch instead of
    /// descending into a child rule that cannot start at the current token.
    pub fn sync_decision(
        &mut self,
        atn: &Atn,
        state_number: usize,
        current_context_empty: bool,
        loop_back: bool,
    ) -> Result<Vec<ParseTree>, AntlrError> {
        self.set_state(isize::try_from(state_number).unwrap_or(isize::MAX));
        self.generated_sync_expected = None;
        let Some(state) = atn.state(state_number) else {
            return Ok(Vec::new());
        };
        let Some(rule_index) = state.rule_index else {
            return Ok(Vec::new());
        };
        let Some(rule_stop) = atn.rule_to_stop_state().get(rule_index).copied() else {
            return Ok(Vec::new());
        };
        let entry = self.cached_decision_lookahead(atn, state, rule_stop);
        let symbol = self.la(1);
        let mut has_expected_symbols = false;
        let mut nullable = false;
        // Whether EOF is an EXPLICIT expected token of this decision (a real `EOF`
        // reference in the grammar, e.g. `A* EOF`), as opposed to merely the
        // implicit rule-follow that a nullable exit inherits (e.g. a start rule's
        // end). Only an explicit EOF makes a token-before-EOF genuinely extraneous
        // and worth deleting; an implicit-follow EOF means the loop should simply
        // exit and leave the token for the (absent) caller — matching ANTLR, which
        // exits the loop via prediction rather than consuming up to a synthetic EOF.
        let mut explicit_eof_expected = false;
        for transition in &entry.transitions {
            if transition.symbols.contains(symbol) {
                return Ok(Vec::new());
            }
            has_expected_symbols |= !transition.symbols.is_empty();
            nullable |= transition.nullable;
            explicit_eof_expected |= transition.symbols.contains(TOKEN_EOF);
        }
        // Happy path: a nullable decision exits when the symbol is in the
        // rule-stack follow set. Answer the membership question with an
        // early-exit walk; the full union below is only needed for the
        // mismatch/deletion diagnostics.
        if nullable && self.context_expected_contains(atn, symbol) {
            return Ok(Vec::new());
        }
        let context_expected = nullable.then(|| self.context_expected_token_set(atn));
        if !has_expected_symbols && context_expected.as_ref().is_none_or(TokenBitSet::is_empty) {
            return Ok(Vec::new());
        }
        let mut expected = TokenBitSet::default();
        for transition in &entry.transitions {
            expected.extend_from(&transition.symbols);
        }
        if let Some(context_expected) = context_expected {
            expected.extend_from(&context_expected);
        }
        let can_delete_in_place =
            !(nullable && current_context_empty && self.rule_context_stack.len() > 1);
        // ANTLR's `DefaultErrorStrategy.sync` recovers differently by decision kind:
        // a loop-BACK sync (STAR_LOOP_BACK / PLUS_LOOP_BACK — reached only after at
        // least one iteration) does `consumeUntil` the follow set — multi-token
        // deletion, one error per skipped token across iterations; a loop ENTRY
        // (STAR_LOOP_ENTRY) and a plain optional/block entry (BLOCK_START /
        // *-block / +-block starts) do `singleTokenDeletion` — delete the one
        // unexpected token only when LA(2) is expected, otherwise report a mismatch
        // and leave recovery to the rule.
        //
        // The generated loop always presents the loop-ENTRY state to this method on
        // every pass, so `state.kind` cannot distinguish entry from back; the caller
        // passes `loop_back` (false on a `*` loop's first sync / on a block, true once
        // an iteration has been taken, and true on a `+` loop's first sync since its
        // mandatory first element is iteration 1). Treating a loop entry as a
        // loop-back would over-consume (e.g. `s: A* EOF;` on `c c` would delete both
        // `c`s, which ANTLR rejects with `mismatched input`).
        let loop_sync = loop_back;
        if symbol != TOKEN_EOF && can_delete_in_place {
            let mut cursor = self.input.index();
            let mut skipped = Vec::new();
            loop {
                let current = self.token_type_at(cursor);
                if current == TOKEN_EOF {
                    break;
                }
                skipped.push(cursor);
                let next = self.consume_index(cursor, current);
                if next == cursor {
                    break;
                }
                let next_symbol = self.token_type_at(next);
                // Stop (and delete the skipped tokens as error nodes) when the next
                // token is a real expected continuation. EOF counts only when it is
                // an EXPLICIT grammar token (`A* EOF`): then the deleted tokens are
                // genuinely extraneous and the generated EOF match consumes the real
                // EOF afterwards. An implicit-follow EOF (a nullable exit's inherited
                // rule-follow) does NOT count — the loop must exit and leave the
                // token, as ANTLR does, instead of deleting up to a synthetic EOF.
                let next_is_expected_stop = if next_symbol == TOKEN_EOF {
                    explicit_eof_expected
                } else {
                    expected.contains(next_symbol)
                };
                if next_is_expected_stop {
                    let current_token = self.input.lt(1).cloned();
                    let expected_symbols = expected.to_btree_set();
                    let message = format!(
                        "extraneous input {} expecting {}",
                        current_token
                            .as_ref()
                            .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                        self.expected_symbols_display(&expected_symbols)
                    );
                    self.push_generated_parser_diagnostic(diagnostic_for_token(
                        current_token.as_ref(),
                        message,
                    ));
                    self.record_syntax_errors(1);
                    let mut children = Vec::with_capacity(skipped.len());
                    for index in skipped {
                        if let Some(token) = self.token_at(index) {
                            self.consume();
                            children.push(ParseTree::Error(ErrorNode::new(token)));
                        }
                    }
                    return Ok(children);
                }
                // A non-loop block entry deletes at most one token (single-token
                // deletion): if LA(2) is not expected, stop scanning so the mismatch
                // is reported at the first token instead of skipping ahead.
                if !loop_sync {
                    break;
                }
                cursor = next;
            }
        }
        if nullable {
            self.generated_sync_expected = Some(expected);
            return Ok(Vec::new());
        }
        let current = self.input.lt(1).cloned();
        let expected_symbols = expected.to_btree_set();
        Err(AntlrError::ParserError {
            line: current.as_ref().map(Token::line).unwrap_or_default(),
            column: current.as_ref().map(Token::column).unwrap_or_default(),
            message: format!(
                "mismatched input {} expecting {}",
                current
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(&expected_symbols)
            ),
        })
    }

    /// Returns a generated-parser prediction when one token of lookahead
    /// uniquely selects an alternative for `state_number`.
    ///
    /// This mirrors the interpreter's LL(1) commit point and lets generated
    /// recursive-descent methods avoid invoking the adaptive simulator for
    /// simple optional/block/loop decisions.
    pub fn ll1_decision_prediction(
        &mut self,
        atn: &Atn,
        state_number: usize,
    ) -> Option<ParserAtnPrediction> {
        let state = atn.state(state_number)?;
        if state.precedence_rule_decision {
            return None;
        }
        let rule_stop = state
            .rule_index
            .and_then(|rule_index| atn.rule_to_stop_state().get(rule_index).copied())?;
        let symbol = self.la(1);
        let entry = self.cached_decision_lookahead(atn, state, rule_stop);
        ll1_greedy_alt(&entry, symbol, state.non_greedy).map(|alt| ParserAtnPrediction {
            alt: alt + 1,
            requires_full_context: false,
            has_semantic_context: false,
            diagnostic: None,
        })
    }

    fn context_expected_symbols(&mut self, atn: &Atn) -> BTreeSet<i32> {
        let context = self.prediction_context(atn);
        let mut expected = BTreeSet::new();
        self.collect_context_expected_symbols(atn, &context, &mut expected);
        expected
    }

    fn context_expected_token_set(&mut self, atn: &Atn) -> TokenBitSet {
        let context = self.prediction_context(atn);
        let mut expected = TokenBitSet::default();
        self.collect_context_expected_token_set(atn, &context, &mut expected);
        expected
    }

    /// Reports whether `symbol` is in `context_expected_token_set(atn)`
    /// without materializing the union.
    ///
    /// This walks the rule-invocation stack directly, innermost frame first —
    /// the same frames, in the same order, with the same rule-stop gating as
    /// `collect_context_expected_token_set` over `prediction_context(atn)`
    /// (whose chain head is the innermost frame's follow state). The nullable
    /// exit in `sync_decision` asks only this membership question, and on
    /// valid input the innermost frame answers it, so the early exit replaces
    /// an O(stack-depth) set union per loop/optional exit with one probe.
    fn context_expected_contains(&mut self, atn: &Atn, symbol: i32) -> bool {
        for index in (1..self.rule_context_stack.len()).rev() {
            let invoking_state = self.rule_context_stack[index].invoking_state;
            let Ok(state_number) = usize::try_from(invoking_state) else {
                continue;
            };
            let Some(Transition::Rule { follow_state, .. }) = atn
                .state(state_number)
                .and_then(|state| state.transitions.first())
            else {
                continue;
            };
            let follow_state = *follow_state;
            if self
                .cached_state_expected_token_set(atn, follow_state)
                .contains(symbol)
            {
                return true;
            }
            if !self.cached_state_can_reach_rule_stop(atn, follow_state) {
                return false;
            }
        }
        symbol == TOKEN_EOF
    }

    fn collect_context_expected_symbols(
        &mut self,
        atn: &Atn,
        context: &Rc<PredictionContext>,
        expected: &mut BTreeSet<i32>,
    ) {
        if context.is_empty() {
            expected.insert(TOKEN_EOF);
            return;
        }
        for index in 0..context.len() {
            let Some(return_state) = context.return_state(index) else {
                continue;
            };
            if return_state == EMPTY_RETURN_STATE {
                expected.insert(TOKEN_EOF);
                continue;
            }
            expected.extend(self.cached_state_expected_symbols(atn, return_state).iter());
            if self.cached_state_can_reach_rule_stop(atn, return_state)
                && let Some(parent) = context.parent(index)
            {
                self.collect_context_expected_symbols(atn, &parent, expected);
            }
        }
    }

    fn collect_context_expected_token_set(
        &mut self,
        atn: &Atn,
        context: &Rc<PredictionContext>,
        expected: &mut TokenBitSet,
    ) {
        if context.is_empty() {
            expected.insert(TOKEN_EOF);
            return;
        }
        for index in 0..context.len() {
            let Some(return_state) = context.return_state(index) else {
                continue;
            };
            if return_state == EMPTY_RETURN_STATE {
                expected.insert(TOKEN_EOF);
                continue;
            }
            let state_expected = self.cached_state_expected_token_set(atn, return_state);
            expected.extend_from(&state_expected);
            if self.cached_state_can_reach_rule_stop(atn, return_state)
                && let Some(parent) = context.parent(index)
            {
                self.collect_context_expected_token_set(atn, &parent, expected);
            }
        }
    }

    /// Builds a generated no-viable-alternative parser error.
    pub fn no_viable_alternative_error(&mut self, start_index: usize) -> AntlrError {
        let error_index = self.input.index();
        self.no_viable_alternative_error_at(start_index, error_index)
    }

    /// Builds a generated no-viable-alternative parser error at the simulator's
    /// failing lookahead index. `adaptive_predict` restores the input cursor
    /// before returning, so generated parsers have to pass the recorded index
    /// explicitly to preserve ANTLR's LL(k) diagnostic span.
    pub fn no_viable_alternative_error_at(
        &mut self,
        start_index: usize,
        error_index: usize,
    ) -> AntlrError {
        let diagnostic = self.no_viable_alternative(start_index, error_index);
        AntlrError::ParserError {
            line: diagnostic.line,
            column: diagnostic.column,
            message: diagnostic.message,
        }
    }

    /// Builds a generated failed-predicate parser error.
    pub fn failed_predicate_error(&mut self, message: impl Into<String>) -> AntlrError {
        let current = self.input.lt(1).cloned();
        AntlrError::ParserError {
            line: current.as_ref().map(Token::line).unwrap_or_default(),
            column: current.as_ref().map(Token::column).unwrap_or_default(),
            message: format!("rule failed predicate: {}", message.into()),
        }
    }

    /// Builds a generated parser error for a semantic predicate with ANTLR's
    /// `<fail='...'>` option.
    pub fn failed_predicate_option_error(
        &mut self,
        rule_index: usize,
        message: impl Into<String>,
    ) -> AntlrError {
        let current = self.input.lt(1).cloned();
        let rule_name = self
            .rule_names()
            .get(rule_index)
            .map_or_else(|| rule_index.to_string(), Clone::clone);
        AntlrError::ParserError {
            line: current.as_ref().map(Token::line).unwrap_or_default(),
            column: current.as_ref().map(Token::column).unwrap_or_default(),
            message: format!("rule {rule_name} {}", message.into()),
        }
    }

    /// Builds a generated parser-action event at the current input position.
    pub fn parser_action_at_current(
        &mut self,
        source_state: usize,
        rule_index: usize,
        start_index: usize,
        consumed_eof: bool,
    ) -> ParserAction {
        let stop_index = self.rule_stop_token_index(self.input.index(), consumed_eof);
        ParserAction::new(source_state, rule_index, start_index, stop_index)
    }

    /// Offers a committed parser action event to the user semantic hook.
    ///
    /// Generated parsers call this for action source states that were present
    /// in the ATN but not translated into a built-in Rust action template.
    pub fn parser_action_hook(&mut self, action: ParserAction, tree: &ParseTree) -> bool {
        let rule_index = action.rule_index();
        let rule_name = self.rule_names().get(rule_index).cloned();
        let context = match tree {
            ParseTree::Rule(rule) if rule.context().rule_index() == rule_index => {
                Some(rule.context())
            }
            ParseTree::Rule(_) | ParseTree::Terminal(_) | ParseTree::Error(_) => None,
        };
        let input = &mut self.input;
        let semantic_hooks = &mut self.semantic_hooks;
        let member_values = &self.int_members;
        let mut ctx = ParserSemCtx {
            input,
            rule_index,
            coordinate_index: usize::MAX,
            rule_name,
            context,
            tree: Some(tree),
            local_int_arg: None,
            member_values,
            action: Some(action),
        };
        semantic_hooks.action(&mut ctx, action)
    }

    /// Attempts to execute a whole generated rule by committing simulator
    /// decisions directly. Unsupported constructs or decisions that need
    /// full-context / predicate evaluation restore the input cursor and fall
    /// back to [`Self::parse_atn_rule`].
    pub fn parse_atn_rule_adaptive_or_fallback<'atn>(
        &mut self,
        atn: &'atn Atn,
        simulator: &mut ParserAtnSimulator<'atn>,
        rule_index: usize,
    ) -> Result<ParseTree, AntlrError> {
        let start_index = self.current_visible_index();
        self.clear_prediction_diagnostics();
        self.reset_per_parse_caches();
        let mut decision_by_state = vec![None; atn.states().len()];
        for (decision, &state_number) in atn.decision_to_state().iter().enumerate() {
            if let Some(slot) = decision_by_state.get_mut(state_number) {
                *slot = Some(decision);
            }
        }

        let result = DirectAdaptiveParser {
            parser: self,
            atn,
            simulator,
            decision_by_state,
            steps: 0,
        }
        .parse_rule(rule_index, -1, 0);

        match result {
            Ok(tree) => {
                report_token_source_errors(&self.input.drain_source_errors());
                Ok(tree)
            }
            Err(DirectAdaptiveParseControl::Fallback(reason)) => {
                let _ = reason;
                self.input.seek(start_index);
                self.parse_atn_rule(atn, rule_index)
            }
        }
    }

    /// Parses a generated rule by interpreting the parser ATN from the rule's
    /// start state to its stop state.
    ///
    /// The recognizer backtracks across alternatives and loop exits using token
    /// stream indices instead of committing to input consumption immediately.
    /// Once a viable ATN path is found, the parser commits the accepted token
    /// interval and returns a rule node whose children mirror every grammar
    /// rule invocation reached on that path, matching ANTLR's parse-tree
    /// shape.
    pub fn parse_atn_rule(
        &mut self,
        atn: &Atn,
        rule_index: usize,
    ) -> Result<ParseTree, AntlrError> {
        self.parse_atn_rule_with_precedence(atn, rule_index, 0)
    }

    /// Parses a generated rule by interpreting the parser ATN with an initial
    /// left-recursive precedence threshold.
    pub fn parse_atn_rule_with_precedence(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        precedence: i32,
    ) -> Result<ParseTree, AntlrError> {
        let start_state = atn
            .rule_to_start_state()
            .get(rule_index)
            .copied()
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no start state"))
            })?;
        let stop_state = atn
            .rule_to_stop_state()
            .get(rule_index)
            .copied()
            .filter(|state| *state != usize::MAX)
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no stop state"))
            })?;

        let start_index = self.current_visible_index();
        self.clear_prediction_diagnostics();
        self.reset_per_parse_caches();
        let caller_follow_state = self.pending_invoking_follow_state(atn);
        self.fast_recovery_enabled = false;
        self.fast_token_nodes_enabled = false;
        let top_request = FastRecognizeTopRequest {
            start_state,
            stop_state,
            start_index,
            precedence,
            caller_follow_state,
        };
        let first_pass = self.fast_recognize_top(atn, top_request);
        self.fast_token_nodes_enabled = true;
        self.fast_recovery_enabled = true;
        let needs_tree_retry = matches!(
            &first_pass,
            Ok((outcome, _)) if self.build_parse_trees && outcome.nodes.has_left_recursive_boundary()
        );
        let needs_retry = match &first_pass {
            // The FIRST-set prefilter trims speculative rule calls that can't
            // match the current lookahead — useful for perf on grammars with
            // many epsilon-reachable rules, but the trim also bypasses
            // single-token insertion / deletion recovery that ANTLR's
            // reference parser runs at the child rule's first consuming
            // transition. Retry without the prefilter whenever the first pass
            // either produced no outcome at all or produced a recovered
            // outcome (diagnostics non-empty), since the second pass might
            // surface a child-level recovery with cleaner diagnostics or
            // closer parity to ANTLR's tree shape. Left-recursive tree
            // boundaries also need the token-node pass; otherwise the fold has
            // no concrete left operand to wrap into ANTLR's recursive context.
            Err(_) => true,
            Ok((outcome, _)) => !outcome.diagnostics.is_empty() || needs_tree_retry,
        };
        let (outcome, _expected) = if needs_retry {
            self.fast_first_set_prefilter = false;
            let retry = self.fast_recognize_top(atn, top_request);
            self.fast_first_set_prefilter = true;
            let selected = if needs_tree_retry {
                match retry {
                    ok @ Ok(_) => ok,
                    Err(_) => first_pass,
                }
            } else {
                select_better_top_outcome(first_pass, retry)
            };
            selected.map_err(|expected| {
                let error = self.recognition_error(rule_index, start_index, &expected);
                self.record_syntax_errors(1);
                report_token_source_errors(&self.input.drain_source_errors());
                error
            })?
        } else {
            first_pass.expect("first_pass is Ok in the no-retry branch")
        };
        self.record_syntax_errors(outcome.diagnostics.len());
        report_parser_diagnostics(&self.prediction_diagnostics);
        report_parser_diagnostics(&outcome.diagnostics);
        report_token_source_errors(&self.input.drain_source_errors());
        let mut context = ParserRuleContext::with_child_capacity(
            rule_index,
            self.state(),
            if self.build_parse_trees {
                outcome.nodes.len()
            } else {
                0
            },
        );
        if let Some(token) = self.token_ref_at(start_index) {
            context.set_start_ref(token);
        }
        let stop_index = self.rule_stop_token_index(outcome.index, outcome.consumed_eof);
        if let Some(token) = stop_index.and_then(|token_index| self.token_ref_at(token_index)) {
            context.set_stop_ref(token);
        }
        if self.build_parse_trees {
            if outcome.nodes.has_left_recursive_boundary() {
                let folded = fold_fast_left_recursive_boundaries(outcome.nodes.to_vec());
                if folded.iter().any(|node| {
                    matches!(
                        node.as_ref(),
                        FastRecognizedNode::Token { .. }
                            | FastRecognizedNode::ErrorToken { .. }
                            | FastRecognizedNode::MissingToken { .. }
                    )
                }) {
                    for node in &folded {
                        context.add_child(self.fast_recognized_node_tree(node.as_ref())?);
                    }
                } else {
                    self.add_fast_implicit_token_children(
                        &mut context,
                        start_index,
                        stop_index,
                        &folded,
                    )?;
                }
            } else if outcome.nodes.has_explicit_token_node() {
                for node in outcome.nodes.iter() {
                    context.add_child(self.fast_recognized_node_tree(node.as_ref())?);
                }
            } else {
                self.add_fast_implicit_token_children_iter(
                    &mut context,
                    start_index,
                    stop_index,
                    outcome.nodes.iter(),
                )?;
            }
        }
        self.input.seek(outcome.index);

        Ok(self.rule_node(context))
    }

    fn pending_invoking_follow_state(&self, atn: &Atn) -> Option<usize> {
        let invoking_state = self.pending_invoking_states.last().copied()?;
        let state_number = usize::try_from(invoking_state).ok()?;
        match atn.state(state_number)?.transitions.first()? {
            Transition::Rule { follow_state, .. } => Some(*follow_state),
            _ => None,
        }
    }

    fn caller_follow_token_info(&mut self, index: usize) -> (i32, bool, bool) {
        // Generated callers own statement separators; leave them available when
        // an interpreted child rule can either stop before or consume one.
        let token_type = self.token_type_at(index);
        let visible_channel = self.input.channel();
        let token = self.token_at(index);
        let is_boundary = token
            .as_ref()
            .and_then(Token::text)
            .is_some_and(is_caller_follow_boundary_text);
        let is_boundary_gap = token.as_ref().is_some_and(|token| {
            token.channel() != visible_channel
                || token.text().is_some_and(is_caller_follow_boundary_gap_text)
        });
        (token_type, is_boundary, is_boundary_gap)
    }

    /// Runs the fast recognizer once from the rule's start state and returns
    /// the best outcome or the per-attempt expected-token accumulator. The
    /// caller flips `fast_first_set_prefilter` between calls when a retry is
    /// needed, so the FIRST-set cache is left intact across both passes.
    fn fast_recognize_top(
        &mut self,
        atn: &Atn,
        request: FastRecognizeTopRequest,
    ) -> Result<(FastRecognizeOutcome, ExpectedTokens), ExpectedTokens> {
        let FastRecognizeTopRequest {
            start_state,
            stop_state,
            start_index,
            precedence,
            caller_follow_state,
        } = request;
        // `input.size()` is intentionally only the currently buffered token
        // count here. Do not restore an up-front fill just to size this map:
        // the fixed floor avoids small-input churn, and large inputs grow the
        // cache after the deferred-fill threshold without forcing startup
        // tokenization. The 8x multiplier matches the empirical
        // memo-insert / token ratio on heavy grammars (C# averages ~6× and
        // Kotlin ~12× memo entries per token), so the table avoids one
        // rehash on the typical hot path.
        let memo_capacity = self.input.size().saturating_mul(8).clamp(65_536, 524_288);
        let mut visiting = FxHashSet::with_capacity_and_hasher(256, FxBuildHasher::default());
        let mut memo = FxHashMap::with_capacity_and_hasher(memo_capacity, FxBuildHasher::default());
        let mut expected = ExpectedTokens::default();
        let empty_recovery = self.empty_recovery_symbols();
        let outcomes = self.recognize_state_fast(
            atn,
            FastRecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                rule_start_index: start_index,
                decision_start_index: None,
                precedence,
                depth: 0,
                recovery_symbols: empty_recovery,
                recovery_state: None,
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        #[cfg(feature = "perf-counters")]
        if std::env::var("ANTLR_PERF_DUMP").is_ok() {
            perf_counters::dump();
            perf_counters::reset();
        }
        let caller_follow =
            caller_follow_state.map(|state| self.cached_state_expected_token_set(atn, state));
        match select_best_fast_outcome(
            outcomes.into_iter(),
            self.prediction_mode,
            caller_follow.as_deref(),
            |index| self.caller_follow_token_info(index),
        ) {
            Some(outcome) => Ok((outcome, expected)),
            None => Err(expected),
        }
    }

    /// Converts a recognized fast-recognizer node into a public parse-tree
    /// node, mirroring [`Self::recognized_node_tree`] for the slow path.
    fn fast_recognized_node_tree(
        &mut self,
        node: &FastRecognizedNode,
    ) -> Result<ParseTree, AntlrError> {
        match node {
            FastRecognizedNode::Token { index } => {
                let token = self
                    .input
                    .get_ref(*index)
                    .ok_or_else(|| AntlrError::ParserError {
                        line: 0,
                        column: 0,
                        message: format!("missing token at index {index}"),
                    })?;
                Ok(ParseTree::Terminal(TerminalNode::from_ref(token)))
            }
            FastRecognizedNode::ErrorToken { index } => {
                let token = self
                    .input
                    .get_ref(*index)
                    .ok_or_else(|| AntlrError::ParserError {
                        line: 0,
                        column: 0,
                        message: format!("missing error token at index {index}"),
                    })?;
                Ok(ParseTree::Error(ErrorNode::from_ref(token)))
            }
            FastRecognizedNode::MissingToken {
                token_type,
                at_index,
                text,
            } => {
                let current = self.token_at(*at_index);
                let token = CommonToken::new(*token_type)
                    .with_text(text.as_str())
                    .with_span(usize::MAX, usize::MAX)
                    .with_position(
                        current.as_ref().map(Token::line).unwrap_or_default(),
                        current.as_ref().map(Token::column).unwrap_or_default(),
                    );
                Ok(ParseTree::Error(ErrorNode::new(token)))
            }
            FastRecognizedNode::Rule {
                rule_index,
                invoking_state,
                start_index,
                stop_index,
                children,
            } => {
                let mut context = ParserRuleContext::with_child_capacity(
                    *rule_index,
                    *invoking_state,
                    children.len(),
                );
                if let Some(token) = self.token_ref_at(*start_index) {
                    context.set_start_ref(token);
                }
                if let Some(token) = stop_index.and_then(|index| self.token_ref_at(index)) {
                    context.set_stop_ref(token);
                }
                if children.has_left_recursive_boundary() {
                    let folded = fold_fast_left_recursive_boundaries(children.to_vec());
                    for child in &folded {
                        context.add_child(self.fast_recognized_node_tree(child.as_ref())?);
                    }
                } else {
                    for child in children.iter() {
                        context.add_child(self.fast_recognized_node_tree(child.as_ref())?);
                    }
                }
                Ok(self.rule_node(context))
            }
            FastRecognizedNode::LeftRecursiveBoundary { rule_index } => {
                Err(AntlrError::Unsupported(format!(
                    "unfolded left-recursive boundary for rule {rule_index}"
                )))
            }
        }
    }

    fn fast_recognized_node_tree_with_implicit_tokens(
        &mut self,
        node: &FastRecognizedNode,
    ) -> Result<ParseTree, AntlrError> {
        match node {
            FastRecognizedNode::Rule {
                rule_index,
                invoking_state,
                start_index,
                stop_index,
                children,
            } => {
                let mut context = ParserRuleContext::with_child_capacity(
                    *rule_index,
                    *invoking_state,
                    children.len(),
                );
                if let Some(token) = self.token_ref_at(*start_index) {
                    context.set_start_ref(token);
                }
                if let Some(token) = stop_index.and_then(|index| self.token_ref_at(index)) {
                    context.set_stop_ref(token);
                }
                if children.has_left_recursive_boundary() {
                    let folded = fold_fast_left_recursive_boundaries(children.to_vec());
                    self.add_fast_implicit_token_children(
                        &mut context,
                        *start_index,
                        *stop_index,
                        &folded,
                    )?;
                } else {
                    self.add_fast_implicit_token_children_iter(
                        &mut context,
                        *start_index,
                        *stop_index,
                        children.iter(),
                    )?;
                }
                Ok(self.rule_node(context))
            }
            _ => self.fast_recognized_node_tree(node),
        }
    }

    fn add_fast_implicit_token_children(
        &mut self,
        context: &mut ParserRuleContext,
        start_index: usize,
        stop_index: Option<usize>,
        children: &[Rc<FastRecognizedNode>],
    ) -> Result<(), AntlrError> {
        self.add_fast_implicit_token_children_iter(
            context,
            start_index,
            stop_index,
            children.iter(),
        )
    }

    fn add_fast_implicit_token_children_iter<'a>(
        &mut self,
        context: &mut ParserRuleContext,
        start_index: usize,
        stop_index: Option<usize>,
        children: impl IntoIterator<Item = &'a Rc<FastRecognizedNode>>,
    ) -> Result<(), AntlrError> {
        let mut cursor = Some(start_index);
        for child in children {
            if let Some((child_start, child_stop)) = fast_recognized_node_span(child.as_ref()) {
                self.add_visible_terminals_before(context, &mut cursor, child_start)?;
                context.add_child(
                    self.fast_recognized_node_tree_with_implicit_tokens(child.as_ref())?,
                );
                if let Some(child_stop) = child_stop {
                    cursor = self.next_visible_after_token(child_stop);
                }
            } else {
                context.add_child(
                    self.fast_recognized_node_tree_with_implicit_tokens(child.as_ref())?,
                );
            }
        }
        if let Some(stop) = stop_index {
            self.add_visible_terminals_through(context, cursor, stop)?;
        }
        Ok(())
    }

    fn add_visible_terminals_before(
        &mut self,
        context: &mut ParserRuleContext,
        cursor: &mut Option<usize>,
        before: usize,
    ) -> Result<(), AntlrError> {
        let Some(stop) = before.checked_sub(1) else {
            return Ok(());
        };
        let next = self.add_visible_terminals_through(context, *cursor, stop)?;
        *cursor = next;
        Ok(())
    }

    fn add_visible_terminals_through(
        &mut self,
        context: &mut ParserRuleContext,
        mut cursor: Option<usize>,
        stop: usize,
    ) -> Result<Option<usize>, AntlrError> {
        while let Some(index) = cursor {
            if index > stop {
                return Ok(Some(index));
            }
            let token = self
                .input
                .get_ref(index)
                .ok_or_else(|| AntlrError::ParserError {
                    line: 0,
                    column: 0,
                    message: format!("missing token at index {index}"),
                })?;
            let is_eof = token.token_type() == TOKEN_EOF;
            context.add_child(ParseTree::Terminal(TerminalNode::from_ref(token)));
            if is_eof {
                return Ok(None);
            }
            cursor = self.next_visible_after_token(index);
        }
        Ok(None)
    }

    fn next_visible_after_token(&mut self, index: usize) -> Option<usize> {
        let next = self.input.next_visible_after(index);
        (next != index).then_some(next)
    }

    /// Parses a generated rule and returns semantic actions reached on the
    /// selected ATN path.
    ///
    /// This slower path preserves action ordering and token intervals for
    /// generated code that replays target-specific action templates after the
    /// recognizer has chosen one viable parse path.
    pub fn parse_atn_rule_with_actions(
        &mut self,
        atn: &Atn,
        rule_index: usize,
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        self.parse_atn_rule_with_action_options(atn, rule_index, &[], false)
    }

    /// Parses a generated rule and emits ATN actions plus selected rule-init
    /// actions reached on the chosen path.
    ///
    /// Generated parsers use this when a grammar contains rule-level `@init`
    /// templates that must run for nested rule invocations. The runtime keeps
    /// the action list path-sensitive, so init templates are replayed only for
    /// rules that were actually entered by the selected parse.
    pub fn parse_atn_rule_with_action_inits(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        init_action_rules: &[usize],
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        self.parse_atn_rule_with_action_options(atn, rule_index, init_action_rules, false)
    }

    /// Parses a generated rule with optional semantic-action replay features.
    ///
    /// `track_alt_numbers` is used by grammars that opt into ANTLR's
    /// alt-numbered context behavior. It keeps ordinary parse-tree rendering
    /// unchanged for grammars that do not request that target template.
    pub fn parse_atn_rule_with_action_options(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        init_action_rules: &[usize],
        track_alt_numbers: bool,
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        self.parse_atn_rule_with_runtime_options(
            atn,
            rule_index,
            ParserRuntimeOptions {
                init_action_rules,
                track_alt_numbers,
                ..ParserRuntimeOptions::default()
            },
        )
    }

    /// Parses a generated rule with action replay and parser predicate support.
    ///
    /// `predicates` maps serialized `(rule_index, pred_index)` coordinates to
    /// target-template predicate semantics emitted by the generator. Missing
    /// entries are treated as true so unsupported predicate-free grammars keep
    /// the previous unconditional transition behavior.
    pub fn parse_atn_rule_with_runtime_options(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        options: ParserRuntimeOptions<'_>,
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        self.parse_atn_rule_with_runtime_options_and_precedence(atn, rule_index, 0, options)
    }

    /// Parses a generated rule with action replay, parser predicate support,
    /// and an initial left-recursive precedence threshold.
    pub fn parse_atn_rule_with_runtime_options_and_precedence(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        precedence: i32,
        options: ParserRuntimeOptions<'_>,
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        let ParserRuntimeOptions {
            init_action_rules,
            track_alt_numbers,
            predicates,
            semantics,
            rule_args,
            member_actions,
            return_actions,
            unknown_predicate_policy,
        } = options;
        self.unknown_predicate_policy = unknown_predicate_policy;
        self.unknown_predicate_hits.clear();
        let start_state = atn
            .rule_to_start_state()
            .get(rule_index)
            .copied()
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no start state"))
            })?;
        let stop_state = atn
            .rule_to_stop_state()
            .get(rule_index)
            .copied()
            .filter(|state| *state != usize::MAX)
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no stop state"))
            })?;

        let start_index = self.current_visible_index();
        self.clear_prediction_diagnostics();
        self.reset_per_parse_caches();
        let init_action_rules = init_action_rules.iter().copied().collect::<BTreeSet<_>>();
        let invoking_state = self.pending_invoking_states.pop();
        let local_int_arg = invoking_state
            .and_then(|state| usize::try_from(state).ok())
            .and_then(|state| rule_local_int_arg(rule_args, state, rule_index, None));
        let mut visiting = BTreeSet::new();
        let mut memo = BTreeMap::new();
        let mut expected = ExpectedTokens::default();
        let member_values = self.int_members.clone();
        let return_values = BTreeMap::new();
        let outcomes = self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                rule_start_index: start_index,
                decision_start_index: None,
                init_action_rules: &init_action_rules,
                predicates,
                semantics,
                rule_args,
                member_actions,
                return_actions,
                local_int_arg,
                member_values,
                return_values,
                rule_alt_number: 0,
                track_alt_numbers,
                consumed_eof: false,
                precedence,
                depth: 0,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        if let Some(error) = self.unknown_semantic_error() {
            report_token_source_errors(&self.input.drain_source_errors());
            return Err(error);
        }
        let Some(outcome) = select_best_outcome(outcomes.into_iter(), self.prediction_mode) else {
            let error = self.recognition_error(rule_index, start_index, &expected);
            self.record_syntax_errors(1);
            report_token_source_errors(&self.input.drain_source_errors());
            return Err(error);
        };

        self.record_syntax_errors(outcome.diagnostics.len());
        report_parser_diagnostics(&self.prediction_diagnostics);
        report_parser_diagnostics(&outcome.diagnostics);
        report_token_source_errors(&self.input.drain_source_errors());
        let mut actions = outcome.actions;
        if init_action_rules.contains(&rule_index) {
            actions.insert(
                0,
                ParserAction::new_rule_init(rule_index, start_index, Some(start_state)),
            );
        }
        let mut context =
            ParserRuleContext::new(rule_index, invoking_state.unwrap_or_else(|| self.state()));
        if track_alt_numbers {
            context.set_alt_number(outcome.alt_number);
        }
        for (name, value) in outcome.return_values {
            context.set_int_return(name, value);
        }
        if let Some(token) = self.token_ref_at(start_index) {
            context.set_start_ref(token);
        }
        if let Some(token) = self.rule_stop_token_ref(outcome.index, outcome.consumed_eof) {
            context.set_stop_ref(token);
        }
        if self.build_parse_trees {
            let nodes = fold_left_recursive_boundaries(outcome.nodes);
            for node in &nodes {
                context.add_child(self.recognized_node_tree(node, track_alt_numbers)?);
            }
        }
        self.input.seek(outcome.index);

        Ok((self.rule_node(context), actions))
    }

    /// Temporary parser entry used by generated parser methods while the parser
    /// ATN simulator is being implemented.
    ///
    /// This keeps generated parser crates buildable and gives us a stable method
    /// surface for every grammar rule. It intentionally accepts all remaining
    /// tokens into one rule context; it is not the final parser semantics.
    pub fn parse_interpreted_rule(&mut self, rule_index: usize) -> Result<ParseTree, AntlrError> {
        let mut context = ParserRuleContext::new(rule_index, self.state());
        while self.la(1) != TOKEN_EOF {
            let token_type = self.la(1);
            let child = self.match_token(token_type)?;
            if self.build_parse_trees {
                context.add_child(child);
            }
        }
        if self.build_parse_trees {
            context.add_child(self.match_eof()?);
        }
        Ok(self.rule_node(context))
    }

    /// Builds the parser error reported when no ATN path can reach the active
    /// rule stop state.
    fn recognition_error(
        &mut self,
        rule_index: usize,
        start_index: usize,
        expected: &ExpectedTokens,
    ) -> AntlrError {
        let (index, message) = self.expected_error_message(rule_index, start_index, expected);
        self.input.seek(index);
        let current = self.input.lt(1).cloned();
        let line = current.as_ref().map(Token::line).unwrap_or_default();
        let column = current.as_ref().map(Token::column).unwrap_or_default();
        AntlrError::ParserError {
            line,
            column,
            message,
        }
    }

    /// Builds the token index and ANTLR-compatible message for a failed rule.
    fn expected_error_message(
        &mut self,
        rule_index: usize,
        start_index: usize,
        expected: &ExpectedTokens,
    ) -> (usize, String) {
        let index = expected
            .index
            .or_else(|| expected.no_viable.map(|no_viable| no_viable.error_index))
            .unwrap_or_else(|| self.input.index());
        self.input.seek(index);
        let current = self.input.lt(1).cloned();
        let message = if expected
            .no_viable
            .as_ref()
            .is_some_and(|no_viable| no_viable.error_index == index)
        {
            let start = expected
                .no_viable
                .as_ref()
                .map_or(start_index, |no_viable| no_viable.start_index);
            let text = display_input_text(&self.input.text(start, index));
            format!("no viable alternative at input '{text}'")
        } else if expected.symbols.is_empty() {
            if expected.index.is_some() {
                let found = current
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display);
                if current
                    .as_ref()
                    .is_some_and(|token| token.token_type() == TOKEN_EOF)
                {
                    format!(
                        "missing {} at {found}",
                        self.expected_symbols_display(&expected.symbols)
                    )
                } else {
                    format!("mismatched input {found}")
                }
            } else {
                format!("no viable alternative while parsing rule {rule_index}")
            }
        } else {
            format!(
                "mismatched input {} expecting {}",
                current
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(&expected.symbols)
            )
        };
        (index, message)
    }

    /// Converts a failed child rule into a recovered outcome so the parent can
    /// continue after reporting the child diagnostic.
    fn child_rule_failure_recovery(
        &mut self,
        rule_index: usize,
        start_index: usize,
        sync_symbols: &BTreeSet<i32>,
        member_values: BTreeMap<usize, i64>,
        expected: &ExpectedTokens,
    ) -> Option<RecognizeOutcome> {
        let (error_index, message) = self.expected_error_message(rule_index, start_index, expected);
        let token = self.token_at(error_index);
        let mut next_index = error_index;
        loop {
            let symbol = self.token_type_at(next_index);
            if sync_symbols.contains(&symbol) {
                if next_index == error_index {
                    return None;
                }
                break;
            }
            if symbol == TOKEN_EOF {
                break;
            }
            let after = self.consume_index(next_index, symbol);
            if after == next_index {
                break;
            }
            next_index = after;
        }
        Some(RecognizeOutcome {
            index: next_index,
            consumed_eof: false,
            alt_number: 0,
            member_values,
            return_values: BTreeMap::new(),
            diagnostics: vec![diagnostic_for_token(token.as_ref(), message)],
            decisions: Vec::new(),
            actions: Vec::new(),
            nodes: vec![RecognizedNode::ErrorToken { index: error_index }],
        })
    }

    /// Adapts the optional recovery result to the normal outcome list used by
    /// rule-call transitions.
    fn child_rule_failure_recovery_outcomes(
        &mut self,
        request: ChildRuleFailureRecovery<'_>,
    ) -> Vec<RecognizeOutcome> {
        let sync_symbols =
            state_sync_symbols(request.atn, request.follow_state, request.stop_state);
        self.child_rule_failure_recovery(
            request.rule_index,
            request.start_index,
            &sync_symbols,
            request.member_values,
            request.expected,
        )
        .into_iter()
        .collect()
    }

    /// Formats expected token types using ANTLR's single-token or set syntax.
    fn expected_symbols_display(&self, symbols: &BTreeSet<i32>) -> String {
        expected_symbols_display(symbols, self.vocabulary())
    }

    /// Returns the single-token deletion repair if the token after `index`
    /// satisfies the failed consuming transition.
    fn single_token_deletion(
        &mut self,
        transition: &Transition,
        index: usize,
        max_token_type: i32,
        expected_symbols: &BTreeSet<i32>,
    ) -> Option<(ParserDiagnostic, usize, i32)> {
        let current_symbol = self.token_type_at(index);
        if current_symbol == TOKEN_EOF {
            return None;
        }
        let next_index = self.consume_index(index, current_symbol);
        if next_index == index {
            return None;
        }
        let next_symbol = self.token_type_at(next_index);
        if !transition.matches(next_symbol, 1, max_token_type) {
            return None;
        }
        let transition_expected = transition_expected_symbols(transition, max_token_type);
        let expected_display = self.expected_symbols_display(if expected_symbols.is_empty() {
            &transition_expected
        } else {
            expected_symbols
        });
        let current = self.token_at(index);
        let message = format!(
            "extraneous input {} expecting {expected_display}",
            current
                .as_ref()
                .map_or_else(|| "'<EOF>'".to_owned(), token_input_display)
        );
        Some((
            diagnostic_for_token(current.as_ref(), message),
            next_index,
            next_symbol,
        ))
    }

    /// Returns the repair used when deleting the current token lets a recovery
    /// state continue with the following token.
    fn current_token_deletion(
        &mut self,
        index: usize,
        expected_symbols: &BTreeSet<i32>,
    ) -> Option<(ParserDiagnostic, usize, Vec<usize>)> {
        if expected_symbols.is_empty() {
            return None;
        }
        let current_symbol = self.token_type_at(index);
        if current_symbol == TOKEN_EOF {
            return None;
        }
        let current = self.token_at(index);
        let message = format!(
            "extraneous input {} expecting {}",
            current
                .as_ref()
                .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
            self.expected_symbols_display(expected_symbols)
        );
        let diagnostic = diagnostic_for_token(current.as_ref(), message);
        let mut skipped = Vec::new();
        let mut cursor = index;
        loop {
            let symbol = self.token_type_at(cursor);
            if symbol == TOKEN_EOF {
                return None;
            }
            skipped.push(cursor);
            let next_index = self.consume_index(cursor, symbol);
            if next_index == cursor {
                return None;
            }
            let next_symbol = self.token_type_at(next_index);
            if expected_symbols.contains(&next_symbol) {
                return Some((diagnostic, next_index, skipped));
            }
            cursor = next_index;
        }
    }

    /// Returns the single-token insertion repair for a failed consuming
    /// transition. The caller validates the repair by continuing from the
    /// transition target at the same input index.
    fn single_token_insertion(
        &mut self,
        transition: &Transition,
        index: usize,
        max_token_type: i32,
        expected_symbols: &BTreeSet<i32>,
        follow_symbols: &BTreeSet<i32>,
    ) -> Option<(ParserDiagnostic, i32, String)> {
        let current_symbol = self.token_type_at(index);
        if !follow_symbols.contains(&current_symbol) {
            return None;
        }
        let transition_expected = transition_expected_symbols(transition, max_token_type);
        let token_type = transition_expected.iter().next().copied()?;
        let expected_display = self.expected_symbols_display(if expected_symbols.is_empty() {
            &transition_expected
        } else {
            expected_symbols
        });
        let mut token_symbols = BTreeSet::new();
        token_symbols.insert(token_type);
        let missing_token_display = self.expected_symbols_display(&token_symbols);
        let current = self.token_at(index);
        let message = format!(
            "missing {expected_display} at {}",
            current
                .as_ref()
                .map_or_else(|| "'<EOF>'".to_owned(), token_input_display)
        );
        let text = format!("<missing {missing_token_display}>");
        Some((
            diagnostic_for_token(current.as_ref(), message),
            token_type,
            text,
        ))
    }

    /// Explores ANTLR's single-token deletion recovery for the fast recognizer:
    /// skip the unexpected current token when the following token satisfies the
    /// transition that failed.
    fn fast_single_token_deletion_recovery(
        &mut self,
        recovery: FastRecoveryRequest<'_, '_>,
    ) -> Vec<FastRecognizeOutcome> {
        let FastRecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let FastRecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            precedence,
            depth,
            ..
        } = request;
        let Some((diagnostic, next_index, next_symbol)) =
            self.single_token_deletion(transition, index, atn.max_token_type(), &expected_symbols)
        else {
            return Vec::new();
        };
        let after_next = self.consume_index(next_index, next_symbol);
        let empty_recovery = self.empty_recovery_symbols();
        self.recognize_state_fast(
            atn,
            FastRecognizeRequest {
                state_number: target,
                stop_state,
                index: after_next,
                rule_start_index,
                decision_start_index,
                precedence,
                depth: depth + 1,
                recovery_symbols: empty_recovery,
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.consumed_eof |= next_symbol == TOKEN_EOF;
            outcome.diagnostics.insert(0, diagnostic.clone());
            if self.fast_token_nodes_enabled {
                outcome
                    .nodes
                    .prepend(Rc::new(FastRecognizedNode::Token { index: next_index }));
                outcome
                    .nodes
                    .prepend(Rc::new(FastRecognizedNode::ErrorToken { index }));
            }
            outcome
        })
        .collect()
    }

    /// Explores ANTLR's single-token insertion recovery for the fast recognizer:
    /// pretend the expected transition token was present and continue without
    /// consuming the current token.
    fn fast_single_token_insertion_recovery(
        &mut self,
        recovery: FastRecoveryRequest<'_, '_>,
    ) -> Vec<FastRecognizeOutcome> {
        let FastRecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let FastRecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            precedence,
            depth,
            ..
        } = request;
        let follow_symbols = self.cached_state_expected_symbols(atn, transition.target());
        let Some((diagnostic, token_type, text)) = self.single_token_insertion(
            transition,
            index,
            atn.max_token_type(),
            &expected_symbols,
            &follow_symbols,
        ) else {
            return Vec::new();
        };
        let empty_recovery = self.empty_recovery_symbols();
        self.recognize_state_fast(
            atn,
            FastRecognizeRequest {
                state_number: target,
                stop_state,
                index,
                rule_start_index,
                decision_start_index,
                precedence,
                depth: depth + 1,
                recovery_symbols: empty_recovery,
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome
                .nodes
                .prepend(Rc::new(FastRecognizedNode::MissingToken {
                    token_type,
                    at_index: index,
                    text: text.clone(),
                }));
            outcome
        })
        .collect()
    }

    /// Retries the current fast-recognition state after deleting one
    /// unexpected token that precedes a valid loop or block continuation.
    fn fast_current_token_deletion_recovery(
        &mut self,
        recovery: FastCurrentTokenDeletionRequest<'_, '_>,
    ) -> Vec<FastRecognizeOutcome> {
        let FastCurrentTokenDeletionRequest {
            atn,
            expected_symbols,
            mut request,
            visiting,
            memo,
            expected,
        } = recovery;
        if request.index == request.rule_start_index {
            return Vec::new();
        }
        let Some((diagnostic, next_index, skipped)) =
            self.current_token_deletion(request.index, &expected_symbols)
        else {
            return Vec::new();
        };
        request.state_number = request.recovery_state.unwrap_or(request.state_number);
        request.index = next_index;
        request.depth += 1;
        request.recovery_state = None;
        self.recognize_state_fast(atn, request, visiting, memo, expected)
            .into_iter()
            .map(|mut outcome| {
                outcome.diagnostics.insert(0, diagnostic.clone());
                for index in skipped.iter().rev() {
                    outcome
                        .nodes
                        .prepend(Rc::new(FastRecognizedNode::ErrorToken { index: *index }));
                }
                outcome
            })
            .collect()
    }

    /// Converts a failed child rule into a recovered fast-recognizer outcome so
    /// the parent can keep its child rule context and continue at a sync token.
    fn fast_child_rule_failure_recovery(
        &mut self,
        rule_index: usize,
        start_index: usize,
        sync_symbols: &BTreeSet<i32>,
        expected: &ExpectedTokens,
    ) -> Option<FastRecognizeOutcome> {
        let (error_index, message) = self.expected_error_message(rule_index, start_index, expected);
        let token = self.token_at(error_index);
        let mut next_index = error_index;
        loop {
            let symbol = self.token_type_at(next_index);
            if sync_symbols.contains(&symbol) {
                if next_index == error_index {
                    return None;
                }
                break;
            }
            if symbol == TOKEN_EOF {
                break;
            }
            let after = self.consume_index(next_index, symbol);
            if after == next_index {
                break;
            }
            next_index = after;
        }
        let mut diagnostics = FastDiagnostics::new();
        diagnostics.insert(0, diagnostic_for_token(token.as_ref(), message));
        let mut nodes = NodeList::new();
        if self.fast_token_nodes_enabled {
            nodes.prepend(Rc::new(FastRecognizedNode::ErrorToken {
                index: error_index,
            }));
        }
        Some(FastRecognizeOutcome {
            index: next_index,
            consumed_eof: false,
            diagnostics,
            nodes,
        })
    }

    /// Adapts the optional child-rule recovery result to the fast-recognizer
    /// outcome list used by rule-call transitions.
    fn fast_child_rule_failure_recovery_outcomes(
        &mut self,
        request: FastChildRuleFailureRecoveryRequest<'_>,
    ) -> Vec<FastRecognizeOutcome> {
        let FastChildRuleFailureRecoveryRequest {
            atn,
            rule_index,
            start_index,
            follow_state,
            stop_state,
            expected,
        } = request;
        let sync_symbols = state_sync_symbols(atn, follow_state, stop_state);
        self.fast_child_rule_failure_recovery(rule_index, start_index, &sync_symbols, expected)
            .into_iter()
            .collect()
    }

    /// Attempts to reach `stop_state` from `state_number` without committing
    /// token consumption to the parser's public stream position.
    #[allow(clippy::too_many_lines)]
    fn recognize_state_fast(
        &mut self,
        atn: &Atn,
        request: FastRecognizeRequest,
        visiting: &mut FxHashSet<(usize, usize)>,
        memo: &mut FxHashMap<FastRecognizeKey, Rc<[FastRecognizeOutcome]>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<FastRecognizeOutcome> {
        #[cfg(feature = "perf-counters")]
        perf_counters::inc(&perf_counters::RFS_CALLS, 1);
        let FastRecognizeRequest {
            mut state_number,
            stop_state,
            mut index,
            rule_start_index,
            decision_start_index,
            precedence,
            mut depth,
            recovery_symbols,
            recovery_state,
        } = request;
        // Walk straight-line epsilon chains in a loop instead of recursing
        // into `recognize_state_fast` for each intermediate state. ATN
        // serialization places long sequences of `BasicBlock` epsilon
        // transitions between decisions: turning that chain into a loop
        // collapses many recursive calls (and their memo lookups, vec
        // allocations, and visit-set churn) into a single function frame.
        // The loop exits as soon as we hit the original state's logic
        // (multi-alt, decision, rule call, unmatched atom/range/set, gated
        // precedence) so existing fanout, recovery, and memoization still
        // apply unchanged.
        //
        // The inline case also handles single-atom-match states on the
        // happy-pass path: when the lone consuming transition matches the
        // current lookahead, advance the index and continue without paying
        // for a full `recognize_state_fast` recursion. We track tokens we
        // consumed inline in `inline_consumed_tokens` so they can be
        // prepended onto the eventual outcome list once we hit a state
        // whose handling falls outside this fast loop.
        let mut inline_consumed_tokens: Vec<usize> = Vec::new();
        let mut inline_consumed_eof = false;
        loop {
            if depth > RECOGNITION_DEPTH_LIMIT {
                return Vec::new();
            }
            if state_number == stop_state {
                let mut nodes = NodeList::new();
                if self.fast_token_nodes_enabled {
                    for token_index in inline_consumed_tokens.iter().rev() {
                        nodes.prepend(Rc::new(FastRecognizedNode::Token {
                            index: *token_index,
                        }));
                    }
                }
                return vec![FastRecognizeOutcome {
                    index,
                    consumed_eof: inline_consumed_eof,
                    diagnostics: FastDiagnostics::new(),
                    nodes,
                }];
            }
            let Some(state) = atn.state(state_number) else {
                return Vec::new();
            };
            if state.transitions.len() == 1
                && !starts_prediction_decision(state)
                && !state.precedence_rule_decision
            {
                match &state.transitions[0] {
                    Transition::Epsilon { target }
                    | Transition::Predicate { target, .. }
                    | Transition::Action { target, .. }
                        if left_recursive_boundary(atn, state, *target).is_none() =>
                    {
                        #[cfg(feature = "perf-counters")]
                        perf_counters::inc(&perf_counters::EPSILON_TRANSITIONS, 1);
                        state_number = *target;
                        depth += 1;
                        continue;
                    }
                    Transition::Precedence {
                        target,
                        precedence: transition_precedence,
                    } if *transition_precedence >= precedence
                        && left_recursive_boundary(atn, state, *target).is_none() =>
                    {
                        #[cfg(feature = "perf-counters")]
                        perf_counters::inc(&perf_counters::EPSILON_TRANSITIONS, 1);
                        state_number = *target;
                        depth += 1;
                        continue;
                    }
                    // Single-atom / range / set / wildcard / not-set states
                    // are common (~17K of ~125K calls on C#) and almost
                    // always succeed in pass 1: no fanout, no recovery, no
                    // diagnostics. Inline the token match and continue
                    // walking instead of recursing — the recursive path
                    // would just allocate a Vec, build one outcome, prepend
                    // a Token node, and return. Skip pass 2 (recovery
                    // enabled): there the failure branch matters and the
                    // existing recursive code records expected symbols.
                    Transition::Atom { target, .. }
                    | Transition::Range { target, .. }
                    | Transition::Set { target, .. }
                    | Transition::NotSet { target, .. }
                    | Transition::Wildcard { target, .. }
                        if !self.fast_recovery_enabled =>
                    {
                        let symbol = self.token_type_at(index);
                        let transition = &state.transitions[0];
                        if transition.matches(symbol, 1, atn.max_token_type()) {
                            #[cfg(feature = "perf-counters")]
                            perf_counters::inc(&perf_counters::ATOM_RANGE_TRANSITIONS, 1);
                            if self.fast_token_nodes_enabled {
                                inline_consumed_tokens.push(index);
                            }
                            inline_consumed_eof |= symbol == TOKEN_EOF;
                            index = self.consume_index(index, symbol);
                            state_number = *target;
                            depth += 1;
                            continue;
                        }
                        // Fall through to break and let the regular
                        // body handle the no-match case (returns empty).
                    }
                    _ => {}
                }
            }
            break;
        }
        // If we collected token nodes inline but bail to the recursive
        // body (decision state, rule call, etc.), the outcomes returned
        // below will need those token nodes prepended.
        let inline_pending = !inline_consumed_tokens.is_empty() || inline_consumed_eof;
        let Some(state) = atn.state(state_number) else {
            return Vec::new();
        };
        let transition_count = state.transitions.len();
        let memo_lookup_enabled = self.fast_recovery_enabled || transition_count > 1;
        // In pass 1 (`fast_recovery_enabled == false`) the recovery-related
        // fields and the rule/decision boundary indices are pure plumbing —
        // they only affect the recovery branch and the no-viable diagnostic
        // recording, neither of which fires when recovery is off. Zeroing
        // them in the memo key collapses calls that visit the same
        // `(state, index)` from different rule-call sites onto one cache
        // entry, which is the dominant cost on large grammars (e.g. C#) where
        // many rules eventually delegate into the same `expression` /
        // `primary_expression` / `type` branches.
        let key = if self.fast_recovery_enabled {
            FastRecognizeKey {
                state_number,
                stop_state,
                index,
                rule_start_index,
                decision_start_index,
                precedence,
                recovery_symbols_id: Rc::as_ptr(&recovery_symbols) as usize,
                recovery_state,
            }
        } else {
            FastRecognizeKey {
                state_number,
                stop_state,
                index,
                rule_start_index: 0,
                decision_start_index: None,
                precedence,
                recovery_symbols_id: 0,
                recovery_state: None,
            }
        };
        if memo_lookup_enabled {
            if let Some(outcomes) = memo.get(&key) {
                #[cfg(feature = "perf-counters")]
                {
                    perf_counters::inc(&perf_counters::RFS_MEMO_HITS, 1);
                    perf_counters::inc(&perf_counters::OUTCOMES_CLONED, outcomes.len() as u64);
                }
                // Materialize a fresh `Vec` from the cached slice; the caller
                // mutates per-outcome state (eof flags, prepended nodes) so we
                // can't hand them the shared backing.
                if !inline_consumed_tokens.is_empty() || inline_consumed_eof {
                    let inline_eof = inline_consumed_eof;
                    let inline_tokens = &inline_consumed_tokens;
                    return outcomes
                        .iter()
                        .cloned()
                        .map(|mut outcome| {
                            if inline_eof {
                                outcome.consumed_eof = true;
                            }
                            if self.fast_token_nodes_enabled {
                                for token_index in inline_tokens.iter().rev() {
                                    outcome.nodes.prepend(Rc::new(FastRecognizedNode::Token {
                                        index: *token_index,
                                    }));
                                }
                            }
                            outcome
                        })
                        .collect();
                }
                return outcomes.to_vec();
            }
            #[cfg(feature = "perf-counters")]
            perf_counters::inc(&perf_counters::RFS_MEMO_MISSES, 1);
        }

        // Cycle detection: only insert into the visiting set for states
        // that *could* re-enter without consuming — multi-alternative
        // states. Single-transition states are walked in the loop above and
        // never form cycles (the loop only advances toward the rule stop).
        // Multi-alt states might contain epsilon-only edges that loop back
        // to the same `(state, index)` (e.g. left-recursive precedence
        // loops); we still need the guard there.
        let needs_cycle_guard =
            transition_count > 1 && self.state_can_reenter_without_consuming(atn, state_number);
        #[cfg(feature = "perf-counters")]
        if needs_cycle_guard {
            perf_counters::inc(&perf_counters::MULTI_TRANS_BODY, 1);
        } else {
            perf_counters::inc(&perf_counters::SINGLE_TRANS_BODY, 1);
            match &state.transitions[0] {
                Transition::Rule { .. } => {
                    perf_counters::inc(&perf_counters::SINGLE_TRANS_RULE, 1);
                }
                Transition::Atom { .. }
                | Transition::Range { .. }
                | Transition::Set { .. }
                | Transition::NotSet { .. }
                | Transition::Wildcard { .. } => {
                    perf_counters::inc(&perf_counters::SINGLE_TRANS_ATOM, 1);
                }
                _ => {
                    perf_counters::inc(&perf_counters::SINGLE_TRANS_OTHER, 1);
                }
            }
        }
        let visit_id = (state_number, index);
        if needs_cycle_guard && !visiting.insert(visit_id) {
            #[cfg(feature = "perf-counters")]
            perf_counters::inc(&perf_counters::RFS_VISITING_CYCLE, 1);
            return Vec::new();
        }
        let next_decision_start_index = if starts_prediction_decision(state) {
            Some(index)
        } else {
            decision_start_index
        };
        let (epsilon_recovery_symbols, epsilon_recovery_state) = if self.fast_recovery_enabled {
            fast_next_recovery_context(self, atn, state, &recovery_symbols, recovery_state)
        } else {
            (Rc::clone(&recovery_symbols), recovery_state)
        };

        // Lookahead-based pruning. At a multi-alternative state we cache the
        // look-1 set of every outgoing transition; on visit we keep only the
        // transitions whose look-1 can accept the current lookahead (or that
        // can be reached without consuming and so could legitimately match a
        // shorter input). This is the main speedup vs. blind speculative
        // recursion: it lets each visit fan out only to the alternatives that
        // could possibly contribute a clean parse, mirroring the SLL phase of
        // ANTLR's adaptive prediction.
        //
        // Pruning is skipped at:
        //   * rule-start states (a child rule call may need every internal
        //     transition to surface single-token recovery diagnostics that
        //     ANTLR's reference parser emits at the rule's first consuming
        //     transition; the FIRST-set retry path turns the prefilter off
        //     entirely so let's keep this lightweight too),
        //   * left-recursive precedence loops (the precedence transition's
        //     gating is dynamic),
        //   * states with too few alternatives to benefit.
        let transition_count = state.transitions.len();
        let lookahead_filter = if transition_count > 1
            && self.fast_first_set_prefilter
            && !state.precedence_rule_decision
            && (!self.fast_recovery_enabled || state.kind != AtnStateKind::RuleStart)
        {
            state
                .rule_index
                .and_then(|rule_index| atn.rule_to_stop_state().get(rule_index).copied())
                .map(|rule_stop| {
                    let symbol = self.token_type_at(index);
                    let entry = self.cached_decision_lookahead(atn, state, rule_stop);
                    (symbol, entry)
                })
        } else {
            None
        };
        // LL(1) fast path: when the FIRST sets for the decision are disjoint
        // and none is nullable, the lookahead deterministically selects one
        // alternative. The recursive recognizer can then commit to that single
        // alt without iterating every transition through `should_skip_via_lookahead`
        // — saving (transition_count - 1) filter probes per visit.
        //
        // Result is cached per `(state, lookahead_token)` on the parser
        // instance, so subsequent visits skip the FIRST-set scan entirely.
        let ll1_only_alt: Option<usize> = if transition_count > 1
            && let Some((symbol, entry)) = lookahead_filter.as_ref()
        {
            let key = (state.state_number, *symbol);
            if let Some(&cached) = self.ll1_decision_cache.get(&key) {
                cached
            } else {
                let result = ll1_unique_alt(entry, *symbol);
                self.ll1_decision_cache.insert(key, result);
                result
            }
        } else {
            None
        };
        let lookahead_filter = lookahead_filter.as_ref();
        // Pre-size only when we expect at least one outcome to land — most
        // single-transition fall-throughs (the loop above didn't catch
        // because they're atom/rule/predicate) push at most one entry, so
        // reserving one slot avoids a reallocation while keeping the
        // unused-slot waste at one element.
        let mut outcomes: Vec<FastRecognizeOutcome> = Vec::with_capacity(transition_count.min(2));
        for (transition_index, transition) in state.transitions.iter().enumerate() {
            if let Some(alt) = ll1_only_alt {
                // LL(1) determinism: skip every alt except the chosen one.
                if alt != transition_index {
                    continue;
                }
            } else if should_skip_via_lookahead(
                transition,
                transition_index,
                lookahead_filter,
                index,
                self.fast_recovery_enabled,
                expected,
            ) {
                continue;
            }
            match transition {
                Transition::Epsilon { target }
                | Transition::Predicate { target, .. }
                | Transition::Action { target, .. } => {
                    #[cfg(feature = "perf-counters")]
                    perf_counters::inc(&perf_counters::EPSILON_TRANSITIONS, 1);
                    let boundary = left_recursive_boundary(atn, state, *target);
                    outcomes.extend(
                        self.recognize_state_fast(
                            atn,
                            FastRecognizeRequest {
                                state_number: *target,
                                stop_state,
                                index,
                                rule_start_index,
                                decision_start_index: next_decision_start_index,
                                precedence,
                                depth: depth + 1,
                                recovery_symbols: Rc::clone(&epsilon_recovery_symbols),
                                recovery_state: epsilon_recovery_state,
                            },
                            visiting,
                            memo,
                            expected,
                        )
                        .into_iter()
                        .map(|mut outcome| {
                            if let Some(rule_index) = boundary {
                                outcome.nodes.prepend(Rc::new(
                                    FastRecognizedNode::LeftRecursiveBoundary { rule_index },
                                ));
                            }
                            outcome
                        }),
                    );
                }
                Transition::Precedence {
                    target,
                    precedence: transition_precedence,
                } => {
                    if *transition_precedence >= precedence {
                        let boundary = left_recursive_boundary(atn, state, *target);
                        outcomes.extend(
                            self.recognize_state_fast(
                                atn,
                                FastRecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: Rc::clone(&epsilon_recovery_symbols),
                                    recovery_state: epsilon_recovery_state,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                if let Some(rule_index) = boundary {
                                    outcome.nodes.prepend(Rc::new(
                                        FastRecognizedNode::LeftRecursiveBoundary { rule_index },
                                    ));
                                }
                                outcome
                            }),
                        );
                    }
                }
                Transition::Rule {
                    target,
                    rule_index,
                    follow_state,
                    precedence: rule_precedence,
                    ..
                } => {
                    #[cfg(feature = "perf-counters")]
                    perf_counters::inc(&perf_counters::RULE_TRANSITIONS, 1);
                    let Some(child_stop) = atn.rule_to_stop_state().get(*rule_index).copied()
                    else {
                        continue;
                    };
                    // Lookahead-based pruning. The recognizer would otherwise
                    // explore every speculative rule call, producing exponential
                    // work on grammars with many epsilon-reachable rules. When
                    // the rule is non-nullable and its FIRST set excludes the
                    // current lookahead, recursion can't find a clean path
                    // *through this rule*. Skipping is only safe if some sibling
                    // transition can still consume the lookahead — otherwise the
                    // rule call is the sole continuation and must run so the
                    // single-token insertion / deletion recovery inside the
                    // called rule can fire (mirroring ANTLR's reference behavior
                    // of conjuring a missing token at child-rule entry).
                    let symbol = self.token_type_at(index);
                    if self.fast_first_set_prefilter {
                        // Probe the shared cross-parse cache first; build
                        // the entry on miss and intern it there. The
                        // computation is purely a function of the ATN, so
                        // the cached entry is reused across parses (and
                        // freshly-instantiated parser values that share
                        // the same `&'static Atn`).
                        //
                        // `rule_first_set` returns the computed entry
                        // directly — it intentionally skips inserting into
                        // the cache when the FIRST-set walk hit a cycle, so
                        // we cannot assume the entry is in the cache after
                        // computing it.
                        let first = self.cached_rule_first_set(atn, *target, child_stop);
                        if should_skip_rule_via_first_set(
                            &first,
                            symbol,
                            self.fast_recovery_enabled,
                            index,
                            expected,
                        ) {
                            continue;
                        }
                    }
                    let expected_before_child =
                        self.fast_recovery_enabled.then(|| expected.clone());
                    let mut children = self.recognize_state_fast(
                        atn,
                        FastRecognizeRequest {
                            state_number: *target,
                            stop_state: child_stop,
                            index,
                            rule_start_index: index,
                            decision_start_index: None,
                            precedence: *rule_precedence,
                            depth: depth + 1,
                            recovery_symbols: Rc::clone(&epsilon_recovery_symbols),
                            recovery_state: epsilon_recovery_state,
                        },
                        visiting,
                        memo,
                        expected,
                    );
                    if children.is_empty() && self.fast_recovery_enabled {
                        children = self.fast_child_rule_failure_recovery_outcomes(
                            FastChildRuleFailureRecoveryRequest {
                                atn,
                                rule_index: *rule_index,
                                start_index: index,
                                follow_state: *follow_state,
                                stop_state,
                                expected,
                            },
                        );
                    }
                    if let Some(expected_before_child) = expected_before_child {
                        if children
                            .iter()
                            .any(|child| child.diagnostics.is_empty() && child.index > index)
                        {
                            *expected = expected_before_child;
                        }
                    }
                    for child in children {
                        let child_index = child.index;
                        let child_consumed_eof = child.consumed_eof;
                        let child_diagnostics = child.diagnostics;
                        let empty_recovery = self.empty_recovery_symbols();
                        let follow_outcomes = self.recognize_state_fast(
                            atn,
                            FastRecognizeRequest {
                                state_number: *follow_state,
                                stop_state,
                                index: child_index,
                                rule_start_index,
                                decision_start_index: next_decision_start_index,
                                precedence,
                                depth: depth + 1,
                                recovery_symbols: empty_recovery,
                                recovery_state: None,
                            },
                            visiting,
                            memo,
                            expected,
                        );
                        if follow_outcomes.is_empty() {
                            continue;
                        }
                        let child_node = Rc::new(FastRecognizedNode::Rule {
                            rule_index: *rule_index,
                            invoking_state: invoking_state_number(state_number),
                            start_index: index,
                            stop_index: self.rule_stop_token_index(child_index, child_consumed_eof),
                            children: child.nodes,
                        });
                        let child_diags_empty = child_diagnostics.is_empty();
                        outcomes.extend(follow_outcomes.into_iter().map(|mut outcome| {
                            outcome.consumed_eof |= child_consumed_eof;
                            // Skip the prepend dance when there's nothing to
                            // merge from the child — common case in pass 1.
                            if !child_diags_empty {
                                let mut diagnostics = child_diagnostics.clone();
                                diagnostics.append(&mut outcome.diagnostics);
                                outcome.diagnostics = diagnostics;
                            }
                            outcome.nodes.prepend(Rc::clone(&child_node));
                            outcome
                        }));
                    }
                }
                Transition::Atom { target, .. }
                | Transition::Range { target, .. }
                | Transition::Set { target, .. }
                | Transition::NotSet { target, .. }
                | Transition::Wildcard { target, .. } => {
                    #[cfg(feature = "perf-counters")]
                    perf_counters::inc(&perf_counters::ATOM_RANGE_TRANSITIONS, 1);
                    let symbol = self.token_type_at(index);
                    if transition.matches(symbol, 1, atn.max_token_type()) {
                        let next_index = self.consume_index(index, symbol);
                        let empty_recovery = self.empty_recovery_symbols();
                        outcomes.extend(
                            self.recognize_state_fast(
                                atn,
                                FastRecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index: next_index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: empty_recovery,
                                    recovery_state: None,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= symbol == TOKEN_EOF;
                                if self.fast_token_nodes_enabled {
                                    outcome
                                        .nodes
                                        .prepend(Rc::new(FastRecognizedNode::Token { index }));
                                }
                                outcome
                            }),
                        );
                    } else {
                        if !self.fast_recovery_enabled {
                            // In pass 1 there is no recovery to attempt; the
                            // recovery branch below would never run, and the
                            // `expected_symbols` computation is just there
                            // to gate that branch. Skipping it eliminates
                            // ~1× `state_expected_symbols` lookup per failed
                            // atom transition (≈82K on mono-statement.cs)
                            // for zero observable behavior change.
                            continue;
                        }
                        let expected_symbols = fast_recovery_expected_symbols(
                            self,
                            atn,
                            state.state_number,
                            &recovery_symbols,
                        );
                        if expected_symbols.contains(&symbol) {
                            continue;
                        }
                        {
                            expected.record_transition(index, transition, atn.max_token_type());
                            record_no_viable_if_ambiguous(
                                expected,
                                next_decision_start_index,
                                index,
                            );
                            outcomes.extend(self.fast_single_token_deletion_recovery(
                                FastRecoveryRequest {
                                    atn,
                                    transition,
                                    expected_symbols: Rc::clone(&expected_symbols),
                                    target: *target,
                                    request: FastRecognizeRequest {
                                        state_number,
                                        stop_state,
                                        index,
                                        rule_start_index,
                                        decision_start_index,
                                        precedence,
                                        depth,
                                        recovery_symbols: Rc::clone(&recovery_symbols),
                                        recovery_state,
                                    },
                                    visiting,
                                    memo,
                                    expected,
                                },
                            ));
                            if !state_is_left_recursive_rule(atn, state) {
                                outcomes.extend(self.fast_single_token_insertion_recovery(
                                    FastRecoveryRequest {
                                        atn,
                                        transition,
                                        expected_symbols: Rc::clone(&expected_symbols),
                                        target: *target,
                                        request: FastRecognizeRequest {
                                            state_number,
                                            stop_state,
                                            index,
                                            rule_start_index,
                                            decision_start_index,
                                            precedence,
                                            depth,
                                            recovery_symbols: Rc::clone(&recovery_symbols),
                                            recovery_state,
                                        },
                                        visiting,
                                        memo,
                                        expected,
                                    },
                                ));
                            }
                            outcomes.extend(self.fast_current_token_deletion_recovery(
                                FastCurrentTokenDeletionRequest {
                                    atn,
                                    expected_symbols,
                                    request: FastRecognizeRequest {
                                        state_number,
                                        stop_state,
                                        index,
                                        rule_start_index,
                                        decision_start_index,
                                        precedence,
                                        depth,
                                        recovery_symbols: Rc::clone(&recovery_symbols),
                                        recovery_state,
                                    },
                                    visiting,
                                    memo,
                                    expected,
                                },
                            ));
                        }
                    }
                }
            }
        }

        if needs_cycle_guard {
            visiting.remove(&visit_id);
        }
        if matches!(
            self.prediction_mode,
            PredictionMode::Ll | PredictionMode::LlExactAmbigDetection
        ) && self.fast_recovery_enabled
        {
            // Without recovery enabled every outcome already has empty
            // diagnostics, so the discard pass is a no-op — skipping it
            // saves an iter+retain on each of the ~1M visits.
            discard_recovered_fast_outcomes_if_clean_path_exists(&mut outcomes);
        }
        if self.fast_recovery_enabled {
            dedupe_fast_outcomes(&mut outcomes);
        } else {
            dedupe_clean_fast_outcomes(&mut outcomes);
        }
        // Skip memoization for single-transition states whose outcome is
        // unambiguous: they only get re-entered if the caller revisits the
        // exact same call site, which is rare since the loop above already
        // collapsed straight-line epsilon walks. Multi-alternative states
        // are where backtracking actually revisits the same coordinate, so
        // we still memoize there. With recovery on we keep the existing
        // memoization unconditionally because the recovery branch may
        // record diagnostics that the cache must surface to repeated
        // failed visits.
        let should_memoize = self.fast_recovery_enabled
            || (transition_count > 1
                && (outcomes.is_empty()
                    || outcomes.len() > 1
                    || (outcomes.len() == 1 && self.should_memoize_single_outcome(&key))));
        // Apply inline pending state to each outcome before returning.
        // Tokens consumed inline by the loop-collapse don't appear in the
        // recursive recognizer's output, so we need to prepend them here.
        let apply_inline_pending = |mut outcome: FastRecognizeOutcome| -> FastRecognizeOutcome {
            if inline_consumed_eof {
                outcome.consumed_eof = true;
            }
            if !inline_consumed_tokens.is_empty() {
                for token_index in inline_consumed_tokens.iter().rev() {
                    outcome.nodes.prepend(Rc::new(FastRecognizedNode::Token {
                        index: *token_index,
                    }));
                }
            }
            outcome
        };
        if should_memoize {
            #[cfg(feature = "perf-counters")]
            {
                perf_counters::inc(&perf_counters::MEMO_INSERTED, 1);
                perf_counters::inc(&perf_counters::OUTCOMES_PUSHED, outcomes.len() as u64);
                match outcomes.len() {
                    0 => perf_counters::inc(&perf_counters::OUTCOMES_RETURN_0, 1),
                    1 => perf_counters::inc(&perf_counters::OUTCOMES_RETURN_1, 1),
                    _ => perf_counters::inc(&perf_counters::OUTCOMES_RETURN_N, 1),
                }
            }
            // The memo is keyed by the loop-exit `(state_number, index)` so
            // the inline-consumed tokens belong to *this* call's output, not
            // the cached result. Memoize the bare outcomes (without the
            // inline-pending data), then prepend the inline data on return.
            let stored: Rc<[FastRecognizeOutcome]> = Rc::from(outcomes);
            memo.insert(key, Rc::clone(&stored));
            if inline_pending {
                return stored.iter().cloned().map(apply_inline_pending).collect();
            }
            return stored.to_vec();
        }
        #[cfg(feature = "perf-counters")]
        match outcomes.len() {
            0 => perf_counters::inc(&perf_counters::OUTCOMES_RETURN_0, 1),
            1 => perf_counters::inc(&perf_counters::OUTCOMES_RETURN_1, 1),
            _ => perf_counters::inc(&perf_counters::OUTCOMES_RETURN_N, 1),
        }
        if inline_pending {
            return outcomes.into_iter().map(apply_inline_pending).collect();
        }
        outcomes
    }

    /// Explores single-token deletion recovery while preserving the matched
    /// token and skipped error token in the selected parse tree path.
    fn single_token_deletion_recovery(
        &mut self,
        recovery: RecoveryRequest<'_, '_>,
    ) -> Vec<RecognizeOutcome> {
        let RecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let RecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            init_action_rules,
            predicates,
            semantics,
            rule_args,
            member_actions,
            return_actions,
            local_int_arg,
            member_values,
            return_values,
            rule_alt_number,
            track_alt_numbers,
            consumed_eof,
            precedence,
            depth,
            ..
        } = request;
        let Some((diagnostic, next_index, next_symbol)) =
            self.single_token_deletion(transition, index, atn.max_token_type(), &expected_symbols)
        else {
            return Vec::new();
        };
        let after_next = self.consume_index(next_index, next_symbol);
        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: target,
                stop_state,
                index: after_next,
                rule_start_index,
                decision_start_index,
                init_action_rules,
                predicates,
                semantics,
                rule_args,
                member_actions,
                return_actions,
                local_int_arg,
                member_values,
                return_values,
                rule_alt_number,
                track_alt_numbers,
                consumed_eof: consumed_eof || next_symbol == TOKEN_EOF,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.consumed_eof |= next_symbol == TOKEN_EOF;
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome
                .nodes
                .insert(0, RecognizedNode::Token { index: next_index });
            outcome
                .nodes
                .insert(0, RecognizedNode::ErrorToken { index });
            outcome
        })
        .collect()
    }

    /// Retries the current recognition state after deleting one unexpected
    /// token, preserving the deleted token as an error node in the parse tree.
    fn current_token_deletion_recovery(
        &mut self,
        recovery: CurrentTokenDeletionRequest<'_, '_>,
    ) -> Vec<RecognizeOutcome> {
        let CurrentTokenDeletionRequest {
            atn,
            expected_symbols,
            mut request,
            visiting,
            memo,
            expected,
        } = recovery;
        let error_index = request.index;
        if error_index == request.rule_start_index {
            return Vec::new();
        }
        let Some((diagnostic, next_index, skipped)) =
            self.current_token_deletion(error_index, &expected_symbols)
        else {
            return Vec::new();
        };
        request.state_number = request.recovery_state.unwrap_or(request.state_number);
        request.index = next_index;
        request.depth += 1;
        request.recovery_state = None;
        self.recognize_state(atn, request, visiting, memo, expected)
            .into_iter()
            .map(|mut outcome| {
                outcome.diagnostics.insert(0, diagnostic.clone());
                for index in skipped.iter().rev() {
                    outcome
                        .nodes
                        .insert(0, RecognizedNode::ErrorToken { index: *index });
                }
                outcome
            })
            .collect()
    }

    /// Falls back after deletion/insertion repairs cannot continue from a
    /// failed consuming transition.
    fn consuming_failure_fallback(
        &mut self,
        fallback: ConsumingFailureFallback<'_>,
        visiting: &mut BTreeSet<RecognizeKey>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        if fallback.expected_symbols.is_empty() {
            return Vec::new();
        }
        if fallback.symbol == TOKEN_EOF {
            return self.eof_consuming_failure_fallback(fallback, expected);
        }
        self.non_eof_consuming_failure_fallback(fallback, visiting, memo, expected)
    }

    /// Keeps unexpected non-EOF input visible as an error node when no repair
    /// path can otherwise reach the transition target.
    fn non_eof_consuming_failure_fallback(
        &mut self,
        fallback: ConsumingFailureFallback<'_>,
        visiting: &mut BTreeSet<RecognizeKey>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let ConsumingFailureFallback {
            atn,
            target,
            request,
            symbol,
            expected_symbols,
            decision_start_index,
            decision,
        } = fallback;
        let error_index = request.index;
        let diagnostic =
            self.recovery_failure_diagnostic(error_index, decision_start_index, &expected_symbols);
        let next_index = self.consume_index(error_index, symbol);
        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: target,
                stop_state: request.stop_state,
                index: next_index,
                rule_start_index: request.rule_start_index,
                decision_start_index,
                init_action_rules: request.init_action_rules,
                predicates: request.predicates,
                semantics: request.semantics,
                rule_args: request.rule_args,
                member_actions: request.member_actions,
                return_actions: request.return_actions,
                local_int_arg: request.local_int_arg,
                member_values: request.member_values,
                return_values: request.return_values,
                rule_alt_number: request.rule_alt_number,
                track_alt_numbers: request.track_alt_numbers,
                consumed_eof: request.consumed_eof,
                precedence: request.precedence,
                depth: request.depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            prepend_decision(&mut outcome, decision);
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome
                .nodes
                .insert(0, RecognizedNode::ErrorToken { index: error_index });
            outcome
        })
        .collect()
    }

    /// Stops the current rule at EOF after a nested failure, matching ANTLR's
    /// behavior of unwinding instead of inserting caller tokens at EOF.
    fn eof_consuming_failure_fallback(
        &mut self,
        fallback: ConsumingFailureFallback<'_>,
        expected: &ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let request = fallback.request;
        if request.index == request.rule_start_index {
            return Vec::new();
        }
        let diagnostic =
            self.eof_rule_recovery_diagnostic(request.index, &fallback.expected_symbols, expected);
        vec![RecognizeOutcome {
            index: request.index,
            consumed_eof: request.consumed_eof,
            alt_number: request.rule_alt_number,
            member_values: request.member_values,
            return_values: request.return_values,
            diagnostics: vec![diagnostic],
            decisions: Vec::new(),
            actions: Vec::new(),
            nodes: Vec::new(),
        }]
    }

    /// Explores single-token insertion recovery while adding a conjured
    /// missing-token error node to the selected parse tree path.
    fn single_token_insertion_recovery(
        &mut self,
        recovery: RecoveryRequest<'_, '_>,
    ) -> Vec<RecognizeOutcome> {
        let RecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let RecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            init_action_rules,
            predicates,
            semantics,
            rule_args,
            member_actions,
            return_actions,
            local_int_arg,
            member_values,
            return_values,
            rule_alt_number,
            track_alt_numbers,
            consumed_eof,
            precedence,
            depth,
            ..
        } = request;
        let follow_symbols = state_expected_symbols(atn, transition.target());
        let Some((diagnostic, token_type, text)) = self.single_token_insertion(
            transition,
            index,
            atn.max_token_type(),
            &expected_symbols,
            &follow_symbols,
        ) else {
            return Vec::new();
        };
        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: target,
                stop_state,
                index,
                rule_start_index,
                decision_start_index,
                init_action_rules,
                predicates,
                semantics,
                rule_args,
                member_actions,
                return_actions,
                local_int_arg,
                member_values,
                return_values,
                rule_alt_number,
                track_alt_numbers,
                consumed_eof,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome.nodes.insert(
                0,
                RecognizedNode::MissingToken {
                    token_type,
                    at_index: index,
                    text: text.clone(),
                },
            );
            outcome
        })
        .collect()
    }

    /// Attempts to reach `stop_state` and carries semantic actions for the
    /// selected parser path.
    #[allow(clippy::too_many_lines)]
    fn recognize_state(
        &mut self,
        atn: &Atn,
        request: RecognizeRequest<'_>,
        visiting: &mut BTreeSet<RecognizeKey>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let request_template = request.clone();
        let RecognizeRequest {
            state_number,
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            init_action_rules,
            predicates,
            semantics,
            rule_args,
            member_actions,
            return_actions,
            local_int_arg,
            member_values,
            return_values,
            rule_alt_number,
            track_alt_numbers,
            consumed_eof,
            precedence,
            depth,
            recovery_symbols,
            recovery_state,
        } = request;
        if depth > RECOGNITION_DEPTH_LIMIT {
            return Vec::new();
        }
        if state_number == stop_state {
            return stop_outcome(
                index,
                consumed_eof,
                rule_alt_number,
                member_values,
                return_values,
            );
        }
        let key = RecognizeKey {
            state_number,
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            local_int_arg,
            member_values: member_values.clone(),
            return_values: return_values.clone(),
            rule_alt_number,
            track_alt_numbers,
            consumed_eof,
            precedence,
            recovery_symbols: recovery_symbols.clone(),
            recovery_state,
        };
        if let Some(outcomes) = memo.get(&key) {
            return outcomes.clone();
        }

        let visit_key = key.clone();
        if !visiting.insert(visit_key.clone()) {
            return Vec::new();
        }

        let Some(state) = atn.state(state_number) else {
            visiting.remove(&visit_key);
            return Vec::new();
        };
        let next_decision_start_index = if starts_prediction_decision(state) {
            Some(index)
        } else {
            decision_start_index
        };
        let (epsilon_recovery_symbols, epsilon_recovery_state) =
            next_recovery_context(atn, state, &recovery_symbols, recovery_state);
        let mut outcomes = Vec::new();
        for (transition_index, transition) in state.transitions.iter().enumerate() {
            let decision = transition_decision(atn, state, transition_index, predicates);
            let next_alt_number =
                next_alt_number(state, transition_index, rule_alt_number, track_alt_numbers);
            match transition {
                Transition::Epsilon { target } | Transition::Action { target, .. } => {
                    let action_rule_index = match transition {
                        Transition::Action { rule_index, .. } => Some(*rule_index),
                        _ => None,
                    };
                    outcomes.extend(self.recognize_epsilon_or_action_step(
                        atn,
                        &request_template,
                        EpsilonActionStep {
                            source_state: state_number,
                            target: *target,
                            action_rule_index,
                            left_recursive_boundary: left_recursive_boundary(atn, state, *target),
                            decision,
                            decision_start_index: next_decision_start_index,
                            alt_number: next_alt_number,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                            recovery_state: epsilon_recovery_state,
                        },
                        RecognizeScratch {
                            visiting,
                            memo,
                            expected,
                        },
                    ));
                }
                Transition::Predicate {
                    target,
                    rule_index,
                    pred_index,
                    ..
                } => {
                    let predicate = PredicateEval {
                        index,
                        rule_index: *rule_index,
                        pred_index: *pred_index,
                        predicates,
                        semantics,
                        context: None,
                        local_int_arg,
                        member_values: &member_values,
                    };
                    if self.parser_predicate_matches(predicate) {
                        let left_recursive_boundary = left_recursive_boundary(atn, state, *target);
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    semantics,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number: next_alt_number,
                                    track_alt_numbers,
                                    consumed_eof,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: epsilon_recovery_symbols.clone(),
                                    recovery_state: epsilon_recovery_state,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                if let Some(rule_index) = left_recursive_boundary {
                                    outcome.nodes.insert(
                                        0,
                                        RecognizedNode::LeftRecursiveBoundary { rule_index },
                                    );
                                }
                                outcome
                            }),
                        );
                    } else if let Some(message) = semantics
                        .and_then(|semantics| {
                            self.parser_semantic_ir_predicate_failure_message(
                                *rule_index,
                                *pred_index,
                                semantics,
                            )
                        })
                        .or_else(|| {
                            self.parser_predicate_failure_message(
                                *rule_index,
                                *pred_index,
                                predicates,
                            )
                        })
                    {
                        outcomes.push(self.predicate_failure_recovery(PredicateFailureRecovery {
                            rule_index: *rule_index,
                            index,
                            message,
                            member_values: member_values.clone(),
                            return_values: return_values.clone(),
                            rule_alt_number,
                        }));
                    } else {
                        record_predicate_no_viable(expected, next_decision_start_index, index);
                    }
                }
                Transition::Precedence {
                    target,
                    precedence: transition_precedence,
                } => {
                    if *transition_precedence >= precedence {
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    semantics,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number: next_alt_number,
                                    track_alt_numbers,
                                    consumed_eof,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: epsilon_recovery_symbols.clone(),
                                    recovery_state: epsilon_recovery_state,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                outcome
                            }),
                        );
                    }
                }
                Transition::Rule {
                    target,
                    rule_index,
                    follow_state,
                    precedence: rule_precedence,
                    ..
                } => {
                    let Some(child_stop) = atn.rule_to_stop_state().get(*rule_index).copied()
                    else {
                        continue;
                    };
                    let child_local_int_arg =
                        rule_local_int_arg(rule_args, state_number, *rule_index, local_int_arg);
                    let expected_before_child = expected.clone();
                    let children = self.recognize_state(
                        atn,
                        RecognizeRequest {
                            state_number: *target,
                            stop_state: child_stop,
                            index,
                            rule_start_index: index,
                            decision_start_index: None,
                            init_action_rules,
                            predicates,
                            semantics,
                            rule_args,
                            member_actions,
                            return_actions,
                            local_int_arg: child_local_int_arg,
                            member_values: member_values.clone(),
                            return_values: BTreeMap::new(),
                            rule_alt_number: 0,
                            track_alt_numbers,
                            consumed_eof: false,
                            precedence: *rule_precedence,
                            depth: depth + 1,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                            recovery_state: epsilon_recovery_state,
                        },
                        visiting,
                        memo,
                        expected,
                    );
                    let children = if children.is_empty() {
                        self.child_rule_failure_recovery_outcomes(ChildRuleFailureRecovery {
                            atn,
                            rule_index: *rule_index,
                            start_index: index,
                            follow_state: *follow_state,
                            stop_state,
                            member_values: member_values.clone(),
                            expected,
                        })
                    } else {
                        children
                    };
                    let preserve_child_expected =
                        self.child_expected_reaches_clean_eof(&children, expected);
                    restore_expected(
                        &children,
                        index,
                        expected,
                        expected_before_child,
                        preserve_child_expected,
                    );
                    for child in children {
                        let child_node = RecognizedNode::Rule {
                            rule_index: *rule_index,
                            invoking_state: invoking_state_number(state_number),
                            alt_number: child.alt_number,
                            start_index: index,
                            stop_index: self.rule_stop_token_index(child.index, child.consumed_eof),
                            return_values: child.return_values.clone(),
                            children: fold_left_recursive_boundaries(child.nodes.clone()),
                        };
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *follow_state,
                                    stop_state,
                                    index: child.index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    semantics,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: child.member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number,
                                    track_alt_numbers,
                                    consumed_eof: consumed_eof || child.consumed_eof,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
                                    recovery_state: None,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= child.consumed_eof;
                                let mut diagnostics = child.diagnostics.clone();
                                diagnostics.append(&mut outcome.diagnostics);
                                outcome.diagnostics = diagnostics;
                                let mut decisions = child.decisions.clone();
                                decisions.append(&mut outcome.decisions);
                                outcome.decisions = decisions;
                                prepend_decision(&mut outcome, decision);
                                let mut actions = child.actions.clone();
                                if init_action_rules.contains(rule_index) {
                                    actions.insert(
                                        0,
                                        ParserAction::new_rule_init(
                                            *rule_index,
                                            index,
                                            Some(*follow_state),
                                        ),
                                    );
                                }
                                actions.append(&mut outcome.actions);
                                outcome.actions = actions;
                                outcome.nodes.insert(0, child_node.clone());
                                outcome
                            }),
                        );
                    }
                }
                Transition::Atom { target, .. }
                | Transition::Range { target, .. }
                | Transition::Set { target, .. }
                | Transition::NotSet { target, .. }
                | Transition::Wildcard { target, .. } => {
                    let symbol = self.token_type_at(index);
                    if transition.matches(symbol, 1, atn.max_token_type()) {
                        let next_index = self.consume_index(index, symbol);
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index: next_index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    semantics,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number: next_alt_number,
                                    track_alt_numbers,
                                    consumed_eof: consumed_eof || symbol == TOKEN_EOF,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
                                    recovery_state: None,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                outcome.consumed_eof |= symbol == TOKEN_EOF;
                                outcome.nodes.insert(0, RecognizedNode::Token { index });
                                outcome
                            }),
                        );
                    } else {
                        let expected_symbols =
                            recovery_expected_symbols(atn, state.state_number, &recovery_symbols);
                        if expected_symbols.contains(&symbol) {
                            continue;
                        }
                        expected.record_transition(index, transition, atn.max_token_type());
                        record_no_viable_if_ambiguous(expected, next_decision_start_index, index);
                        let before_recovery = outcomes.len();
                        let recovery_request = request_template.clone();
                        outcomes.extend(
                            self.single_token_deletion_recovery(RecoveryRequest {
                                atn,
                                transition,
                                expected_symbols: expected_symbols.clone(),
                                target: *target,
                                request: recovery_request.clone(),
                                visiting,
                                memo,
                                expected,
                            })
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                outcome
                            }),
                        );
                        if !state_is_left_recursive_rule(atn, state) {
                            outcomes.extend(
                                self.single_token_insertion_recovery(RecoveryRequest {
                                    atn,
                                    transition,
                                    expected_symbols: expected_symbols.clone(),
                                    target: *target,
                                    request: recovery_request.clone(),
                                    visiting,
                                    memo,
                                    expected,
                                })
                                .into_iter()
                                .map(|mut outcome| {
                                    prepend_decision(&mut outcome, decision);
                                    outcome
                                }),
                            );
                        }
                        outcomes.extend(self.current_token_deletion_recovery(
                            CurrentTokenDeletionRequest {
                                atn,
                                expected_symbols: expected_symbols.clone(),
                                request: recovery_request.clone(),
                                visiting,
                                memo,
                                expected,
                            },
                        ));
                        if outcomes.len() == before_recovery {
                            outcomes.extend(self.consuming_failure_fallback(
                                ConsumingFailureFallback {
                                    atn,
                                    target: *target,
                                    request: recovery_request,
                                    symbol,
                                    expected_symbols,
                                    decision_start_index: next_decision_start_index,
                                    decision,
                                },
                                visiting,
                                memo,
                                expected,
                            ));
                        }
                    }
                }
            }
        }

        visiting.remove(&visit_key);
        self.record_prediction_diagnostics(atn, state, index, &outcomes);
        if matches!(
            self.prediction_mode,
            PredictionMode::Ll | PredictionMode::LlExactAmbigDetection
        ) {
            discard_recovered_outcomes_if_clean_path_exists(&mut outcomes);
        }
        dedupe_outcomes(&mut outcomes);
        memo.insert(key, outcomes.clone());
        outcomes
    }

    /// Follows an epsilon or semantic-action transition while preserving the
    /// path-local side effects that may later become generated action output.
    fn recognize_epsilon_or_action_step(
        &mut self,
        atn: &Atn,
        request: &RecognizeRequest<'_>,
        step: EpsilonActionStep,
        scratch: RecognizeScratch<'_>,
    ) -> Vec<RecognizeOutcome> {
        let RecognizeScratch {
            visiting,
            memo,
            expected,
        } = scratch;
        let action = step.action_rule_index.map(|rule_index| {
            ParserAction::new(
                step.source_state,
                rule_index,
                request.rule_start_index,
                self.rule_stop_token_index(request.index, request.consumed_eof),
            )
        });
        let next_member_values = if action.is_some() {
            member_values_after_action(
                step.source_state,
                request.member_actions,
                request.semantics,
                &request.member_values,
            )
        } else {
            request.member_values.clone()
        };
        let next_return_values = action.map_or_else(
            || request.return_values.clone(),
            |action| {
                return_values_after_action(
                    step.source_state,
                    action.rule_index(),
                    request.return_actions,
                    request.semantics,
                    &request.return_values,
                )
            },
        );

        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: step.target,
                stop_state: request.stop_state,
                index: request.index,
                rule_start_index: request.rule_start_index,
                decision_start_index: step.decision_start_index,
                init_action_rules: request.init_action_rules,
                predicates: request.predicates,
                semantics: request.semantics,
                rule_args: request.rule_args,
                member_actions: request.member_actions,
                return_actions: request.return_actions,
                local_int_arg: request.local_int_arg,
                member_values: next_member_values,
                return_values: next_return_values,
                rule_alt_number: step.alt_number,
                track_alt_numbers: request.track_alt_numbers,
                consumed_eof: request.consumed_eof,
                precedence: request.precedence,
                depth: request.depth + 1,
                recovery_symbols: step.recovery_symbols,
                recovery_state: step.recovery_state,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            prepend_decision(&mut outcome, step.decision);
            if let Some(rule_index) = step.left_recursive_boundary {
                outcome
                    .nodes
                    .insert(0, RecognizedNode::LeftRecursiveBoundary { rule_index });
            }
            if let Some(action) = action {
                outcome.actions.insert(0, action);
            }
            outcome
        })
        .collect()
    }

    /// Reads the token type at an absolute token-stream index without moving
    /// the parser's stream cursor. The fast recognizer probes lookahead at
    /// every state visit, so avoiding the seek round-trip is a measurable
    /// hot-path win on long inputs.
    fn token_type_at(&mut self, index: usize) -> i32 {
        if index >= FAST_RECOGNIZER_DEFERRED_FILL_AT && !self.input.is_filled() {
            self.input.fill();
        }
        self.input.token_type_at_index(index)
    }

    /// Returns the cached `state_expected_symbols` set for an ATN state.
    ///
    /// The fast recognizer consults this set on every state visit through
    /// `next_recovery_context`; the underlying DFS is a pure function of the
    /// ATN, so caching the `Rc` lets clones reduce to a reference bump.
    ///
    /// Caching is layered through `intern_recovery_symbols` so two ATN states
    /// with the same expected-symbol set share one `Rc`. That invariant is
    /// what lets `FastRecognizeKey` hash on `recovery_symbols` by pointer
    /// without violating the `Hash`/`Eq` contract — `recovery_symbols` is
    /// always interned before it ends up in a key.
    fn cached_state_expected_symbols(
        &mut self,
        atn: &Atn,
        state_number: usize,
    ) -> Rc<BTreeSet<i32>> {
        if let Some(cached) = self.state_expected_cache.get(&state_number) {
            return Rc::clone(cached);
        }
        let symbols = state_expected_symbols(atn, state_number);
        let entry = self.intern_recovery_symbols(symbols);
        self.state_expected_cache
            .insert(state_number, Rc::clone(&entry));
        entry
    }

    fn cached_state_expected_token_set(
        &mut self,
        atn: &Atn,
        state_number: usize,
    ) -> Rc<TokenBitSet> {
        if let Some(cached) = self.state_expected_token_cache.get(&state_number) {
            return Rc::clone(cached);
        }
        // Purely a function of the ATN, so back the per-parser cache with the
        // thread-shared one — fresh parser instances (one per parse in
        // generated usage) start warm instead of rewalking the ATN.
        let symbols = with_shared_atn_caches(atn, |cache| {
            if let Some(cached) = cache.state_expected_tokens.get(&state_number) {
                return Rc::clone(cached);
            }
            let symbols = Rc::new(state_expected_token_set(atn, state_number));
            cache
                .state_expected_tokens
                .insert(state_number, Rc::clone(&symbols));
            symbols
        });
        self.state_expected_token_cache
            .insert(state_number, Rc::clone(&symbols));
        symbols
    }

    fn cached_state_can_reach_rule_stop(&mut self, atn: &Atn, state_number: usize) -> bool {
        if self.rule_stop_reach_cache.len() <= state_number {
            self.rule_stop_reach_cache
                .resize_with(atn.states().len().max(state_number + 1), || None);
        }
        if let Some(reaches) = self.rule_stop_reach_cache[state_number] {
            return reaches;
        }
        let reaches = with_shared_atn_caches(atn, |cache| {
            *cache
                .rule_stop_reach
                .entry(state_number)
                .or_insert_with(|| state_can_reach_rule_stop(atn, state_number))
        });
        self.rule_stop_reach_cache[state_number] = Some(reaches);
        reaches
    }

    /// Returns the parser's empty `recovery_symbols` singleton so callers can
    /// share an `Rc` instead of allocating new `BTreeSet`s for the common case.
    fn empty_recovery_symbols(&self) -> Rc<BTreeSet<i32>> {
        Rc::clone(&self.empty_recovery_symbols)
    }

    /// Returns the interned `Rc` form of a `recovery_symbols` set so the fast
    /// recognizer can hash and compare keys by pointer.
    ///
    /// Every `Rc<BTreeSet<i32>>` that flows into a `FastRecognizeKey` must
    /// come from this method or the empty singleton; otherwise two
    /// content-equal `Rc`s could end up with different `Rc::as_ptr` values,
    /// and the pointer-keyed hash on `FastRecognizeKey` would split equivalent
    /// recognition coordinates.
    fn intern_recovery_symbols(&mut self, set: BTreeSet<i32>) -> Rc<BTreeSet<i32>> {
        if set.is_empty() {
            return Rc::clone(&self.empty_recovery_symbols);
        }
        let candidate = Rc::new(set);
        match self.recovery_symbols_intern.get(&candidate) {
            Some(existing) => Rc::clone(existing),
            None => {
                self.recovery_symbols_intern
                    .insert(Rc::clone(&candidate), Rc::clone(&candidate));
                candidate
            }
        }
    }

    /// Returns the cached look-1 entry for a decision state, computing it on
    /// first use. Multi-alternative states are visited many times during
    /// recognition; sharing the entry through `Rc` keeps the prefilter to one
    /// hash lookup per visit.
    fn cached_decision_lookahead(
        &mut self,
        atn: &Atn,
        state: &AtnState,
        rule_stop_state: usize,
    ) -> Rc<DecisionLookahead> {
        // Hit the parser-instance cache first. Decision lookahead is purely
        // a function of the ATN/state, so on a warm cache we skip the
        // thread-local + RefCell + HashMap-entry dance through
        // SHARED_ATN_CACHES — which on multi-trans-heavy grammars (C# does
        // ~58K multi-trans visits per parse) shows up as RefCell borrow and
        // hashmap-entry overhead in profiles.
        if let Some(cached) = self.decision_lookahead_cache.get(&state.state_number) {
            return Rc::clone(cached);
        }
        let entry = with_shared_atn_caches(atn, |cache| {
            if let Some(cached) = cache.decision_lookahead.get(&state.state_number) {
                return Rc::clone(cached);
            }
            let mut entry = DecisionLookahead {
                transitions: Vec::with_capacity(state.transitions.len()),
            };
            for transition in &state.transitions {
                entry.transitions.push(transition_first_set(
                    atn,
                    transition,
                    rule_stop_state,
                    &mut cache.first_set,
                ));
            }
            let entry = Rc::new(entry);
            cache
                .decision_lookahead
                .insert(state.state_number, Rc::clone(&entry));
            entry
        });
        self.decision_lookahead_cache
            .insert(state.state_number, Rc::clone(&entry));
        entry
    }

    fn cached_rule_first_set(
        &mut self,
        atn: &Atn,
        target: usize,
        child_stop: usize,
    ) -> Rc<FirstSet> {
        if self.rule_first_set_cache.len() <= target {
            self.rule_first_set_cache
                .resize_with(atn.states().len().max(target + 1), || None);
        }
        if let Some(cached) = self
            .rule_first_set_cache
            .get(target)
            .and_then(Option::as_ref)
        {
            return Rc::clone(cached);
        }
        let first = with_shared_first_set_cache(atn, |cache| {
            rule_first_set(atn, target, child_stop, cache)
        });
        self.rule_first_set_cache[target] = Some(Rc::clone(&first));
        first
    }

    fn state_can_reenter_without_consuming(&mut self, atn: &Atn, state_number: usize) -> bool {
        if self.empty_cycle_cache.len() <= state_number {
            self.empty_cycle_cache
                .resize_with(atn.states().len().max(state_number + 1), || None);
        }
        if let Some(cached) = self.empty_cycle_cache[state_number] {
            return cached;
        }
        let mut visited = FxHashSet::with_capacity_and_hasher(64, FxBuildHasher::default());
        let result = self.empty_path_reaches_state(atn, state_number, state_number, &mut visited);
        self.empty_cycle_cache[state_number] = Some(result);
        result
    }

    fn empty_path_reaches_state(
        &mut self,
        atn: &Atn,
        state_number: usize,
        target_state: usize,
        visited: &mut FxHashSet<usize>,
    ) -> bool {
        if !visited.insert(state_number) {
            return false;
        }
        let Some(state) = atn.state(state_number) else {
            return false;
        };
        for transition in &state.transitions {
            match transition {
                Transition::Atom { .. }
                | Transition::Range { .. }
                | Transition::Set { .. }
                | Transition::NotSet { .. }
                | Transition::Wildcard { .. } => {}
                Transition::Rule {
                    target,
                    rule_index,
                    follow_state,
                    ..
                } => {
                    if *target == target_state
                        || self.empty_path_reaches_state(atn, *target, target_state, visited)
                    {
                        return true;
                    }
                    let Some(child_stop) = atn.rule_to_stop_state().get(*rule_index).copied()
                    else {
                        continue;
                    };
                    if self
                        .cached_rule_first_set(atn, *target, child_stop)
                        .nullable
                        && (*follow_state == target_state
                            || self.empty_path_reaches_state(
                                atn,
                                *follow_state,
                                target_state,
                                visited,
                            ))
                    {
                        return true;
                    }
                }
                Transition::Epsilon { target }
                | Transition::Predicate { target, .. }
                | Transition::Action { target, .. }
                | Transition::Precedence { target, .. } => {
                    if *target == target_state
                        || self.empty_path_reaches_state(atn, *target, target_state, visited)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Decides whether a clean one-outcome entry is worth storing in the full
    /// outcome memo table for this parse.
    fn should_memoize_single_outcome(&mut self, key: &FastRecognizeKey) -> bool {
        match self.single_outcome_memo_mode {
            SingleOutcomeMemoMode::Promote => true,
            SingleOutcomeMemoMode::Sparse => false,
            SingleOutcomeMemoMode::Probe => {
                self.single_outcome_probe_samples += 1;
                if !self.single_outcome_probe_seen.insert(key.clone()) {
                    self.single_outcome_probe_repeats += 1;
                }
                if self.single_outcome_probe_repeats >= CLEAN_SINGLE_OUTCOME_MEMO_REPEAT_LIMIT {
                    self.single_outcome_memo_mode = SingleOutcomeMemoMode::Promote;
                    self.single_outcome_probe_seen.clear();
                    return true;
                }
                if self.single_outcome_probe_samples >= CLEAN_SINGLE_OUTCOME_MEMO_PROBE_LIMIT {
                    self.single_outcome_memo_mode = SingleOutcomeMemoMode::Sparse;
                    self.single_outcome_probe_seen.clear();
                    return false;
                }
                true
            }
        }
    }

    /// Clones the visible token at an absolute token-stream index.
    fn token_at(&mut self, index: usize) -> Option<CommonToken> {
        self.input.get(index).cloned()
    }

    /// Clones the shared token handle at an absolute token-stream index.
    fn token_ref_at(&mut self, index: usize) -> Option<TokenRef> {
        self.input.get_ref(index)
    }

    /// Normalizes the current token-stream cursor to the next parser-visible
    /// token before capturing a rule start boundary.
    fn current_visible_index(&mut self) -> usize {
        let index = self.input.index();
        self.input.seek(index);
        self.input.index()
    }

    /// Reports whether a child rule reached EOF cleanly while also recording
    /// an EOF expectation from a longer path inside that child.
    fn child_expected_reaches_clean_eof(
        &mut self,
        children: &[RecognizeOutcome],
        expected: &ExpectedTokens,
    ) -> bool {
        let Some(index) = expected.index else {
            return false;
        };
        self.token_type_at(index) == TOKEN_EOF
            && children
                .iter()
                .any(|child| child.diagnostics.is_empty() && child.index == index)
    }

    /// Finds the previous token visible to the parser before `index`.
    ///
    /// The token stream cursor skips hidden-channel tokens, so subtracting one
    /// from a visible-token index can point at whitespace. Parser intervals use
    /// this helper to stop at the previous visible token while preserving hidden
    /// text inside the rendered interval.
    fn previous_token_index(&mut self, index: usize) -> Option<usize> {
        self.input.previous_visible_token_index(index)
    }

    /// Returns the token-stream index used as a rule stop boundary.
    ///
    /// EOF transitions keep the cursor on EOF, so a rule that consumed EOF must
    /// stop at `index` rather than at the previous visible token.
    fn rule_stop_token_index(&mut self, index: usize, consumed_eof: bool) -> Option<usize> {
        if consumed_eof && self.token_type_at(index) == TOKEN_EOF {
            Some(index)
        } else {
            self.previous_token_index(index)
        }
    }

    /// Stop-token index for a rule's `@after` action, matching the boundary that
    /// `finish_rule` records on the rule context.
    ///
    /// A rule that matched EOF leaves the cursor parked on the EOF token
    /// (`CommonTokenStream::consume` does not advance past EOF), so the stop is
    /// the current index rather than the previous visible token. Without this,
    /// `$stop`/`$text` in an `@after` action on a rule like `r: a* EOF;` would
    /// report the token before EOF (or `None` for empty input), diverging from
    /// the rule context that `finish_rule` builds.
    ///
    /// NOTE: this infers `consumed_eof` from the cursor, which is wrong when a
    /// rule ends right before EOF without matching it (the cursor is parked on
    /// EOF, but the rule did not consume it). Prefer
    /// [`Self::after_action_stop_index_for_tree`], which reuses the stop token the
    /// rule context already recorded with the real flag. Kept for callers without
    /// the rule tree in hand.
    #[must_use]
    pub fn after_action_stop_index(&mut self, current_index: usize) -> Option<usize> {
        let consumed_eof = self.token_type_at(current_index) == TOKEN_EOF;
        self.rule_stop_token_index(current_index, consumed_eof)
    }

    /// Stop-token index for a rule's `@after` action, taken from the stop token
    /// the rule context already recorded.
    ///
    /// `finish_rule` computes the rule stop with the real `consumed_eof` flag, so
    /// reading it back keeps `$stop`/`$text` in an `@after` action aligned with
    /// the rule context — even when the rule ends immediately before EOF without
    /// matching it (cursor parked on EOF, but `consumed_eof` is false). Falls back
    /// to the cursor-based inference only when the tree carries no rule stop.
    #[must_use]
    pub fn after_action_stop_index_for_tree(
        &mut self,
        tree: &ParseTree,
        current_index: usize,
    ) -> Option<usize> {
        if let ParseTree::Rule(rule) = tree {
            if let Some(stop) = rule.context().stop() {
                let token_index = stop.token_index();
                if token_index >= 0 {
                    return Some(token_index.unsigned_abs());
                }
            }
        }
        self.after_action_stop_index(current_index)
    }

    /// Start-token index for a rule's `@after` action, taken from the start token
    /// the rule context already recorded.
    ///
    /// `enter_rule` sets the rule context start to the first visible token (it
    /// skips leading hidden-channel tokens), so reading it back keeps `$start` /
    /// `$text` in an `@after` action aligned with the rule context — even when the
    /// rule begins after a hidden prefix (e.g. leading whitespace) that the raw
    /// pre-rule cursor still points at. Falls back to `fallback_index` only when
    /// the tree carries no rule start.
    #[must_use]
    pub fn after_action_start_index_for_tree(
        &self,
        tree: &ParseTree,
        fallback_index: usize,
    ) -> usize {
        if let ParseTree::Rule(rule) = tree {
            if let Some(start) = rule.context().start() {
                let token_index = start.token_index();
                if token_index >= 0 {
                    return token_index.unsigned_abs();
                }
            }
        }
        fallback_index
    }

    /// Returns the rule stop token for a selected parse path.
    ///
    /// EOF transitions do not advance the token-stream cursor, so an EOF match
    /// must use the current token rather than the previous visible token.
    fn rule_stop_token_ref(&mut self, index: usize, consumed_eof: bool) -> Option<TokenRef> {
        self.rule_stop_token_index(index, consumed_eof)
            .and_then(|token_index| self.token_ref_at(token_index))
    }

    /// Recovers from a semantic predicate with an ANTLR `<fail='...'>` option.
    ///
    /// Generated Java reports the failed-predicate message at the current
    /// lookahead, then consumes until rule recovery can resume. The metadata
    /// runtime models the same visible tree shape by keeping skipped tokens as
    /// error nodes and returning from the active rule at EOF.
    fn predicate_failure_recovery(
        &mut self,
        request: PredicateFailureRecovery<'_>,
    ) -> RecognizeOutcome {
        let PredicateFailureRecovery {
            rule_index,
            index,
            message,
            member_values,
            return_values,
            rule_alt_number,
        } = request;
        let rule_name = self
            .rule_names()
            .get(rule_index)
            .map_or_else(|| rule_index.to_string(), Clone::clone);
        let diagnostic = diagnostic_for_token(
            self.token_at(index).as_ref(),
            format!("rule {rule_name} {message}"),
        );
        let mut nodes = Vec::new();
        let mut next_index = index;
        loop {
            let symbol = self.token_type_at(next_index);
            if symbol == TOKEN_EOF {
                break;
            }
            nodes.push(RecognizedNode::ErrorToken { index: next_index });
            let after = self.consume_index(next_index, symbol);
            if after == next_index {
                break;
            }
            next_index = after;
        }
        RecognizeOutcome {
            index: next_index,
            consumed_eof: false,
            alt_number: rule_alt_number,
            member_values,
            return_values,
            diagnostics: vec![diagnostic],
            decisions: Vec::new(),
            actions: Vec::new(),
            nodes,
        }
    }

    /// Evaluates a user hook for a predicate coordinate that has no generated
    /// runtime table entry.
    fn parser_semantic_hook_result(
        &mut self,
        request: ParserSemanticHookRequest<'_>,
    ) -> Option<bool> {
        let ParserSemanticHookRequest {
            index,
            rule_index,
            pred_index,
            context,
            local_int_arg,
            member_values,
        } = request;
        let rule_name = self.rule_names().get(rule_index).cloned();
        self.input.seek(index);
        let input = &mut self.input;
        let semantic_hooks = &mut self.semantic_hooks;
        let mut ctx = ParserSemCtx {
            input,
            rule_index,
            coordinate_index: pred_index,
            rule_name,
            context,
            tree: None,
            local_int_arg,
            member_values,
            action: None,
        };
        semantic_hooks.sempred(&mut ctx, rule_index, pred_index)
    }

    /// Applies the active [`UnknownSemanticPolicy`] to a predicate coordinate
    /// that has no entry in the generated predicate table.
    ///
    /// Under [`UnknownSemanticPolicy::Error`] the coordinate is recorded and
    /// the guarded path is abandoned; the parse entry surfaces the recorded
    /// coordinates as [`AntlrError::Unsupported`] once recognition finishes,
    /// because a parse that consulted an unknown predicate is unreliable no
    /// matter which paths were ultimately selected.
    fn unknown_predicate_result(&mut self, rule_index: usize, pred_index: usize) -> bool {
        match self.unknown_predicate_policy {
            UnknownSemanticPolicy::AssumeTrue => true,
            UnknownSemanticPolicy::AssumeFalse => false,
            UnknownSemanticPolicy::Error => {
                let coordinate = (rule_index, pred_index);
                if !self.unknown_predicate_hits.contains(&coordinate) {
                    self.unknown_predicate_hits.push(coordinate);
                }
                false
            }
        }
    }

    /// Builds the fail-loud error for unknown predicate coordinates recorded
    /// by the current parse, if any.
    fn unknown_semantic_error(&self) -> Option<AntlrError> {
        use std::fmt::Write as _;
        let mut hits = self.unknown_predicate_hits.iter();
        let first = hits.next()?;
        let mut message = String::new();
        for (rule_index, pred_index) in std::iter::once(first).chain(hits) {
            if !message.is_empty() {
                message.push_str("; ");
            }
            let _ = match self.rule_names().get(*rule_index) {
                Some(rule_name) => write!(
                    message,
                    "unsupported semantic predicate: rule={rule_name}({rule_index}) pred_index={pred_index}"
                ),
                None => write!(
                    message,
                    "unsupported semantic predicate: rule_index={rule_index} pred_index={pred_index}"
                ),
            };
        }
        Some(AntlrError::Unsupported(message))
    }

    /// Evaluates one lowered predicate expression at the requested input
    /// position.
    ///
    /// This sits in the prediction hot loop, so the context borrows the
    /// speculative member state read-only and the rule name by reference —
    /// no per-evaluation allocation. Only the hook escape path materializes
    /// owned copies, and only when a hook is actually consulted.
    fn parser_semir_predicate_matches(
        &mut self,
        semantics: &ParserSemantics,
        predicate: &ParserSemanticPredicate,
        request: ParserSemanticHookRequest<'_>,
    ) -> bool {
        self.input.seek(request.index);
        let rule_name = self
            .data
            .rule_names()
            .get(request.rule_index)
            .map(String::as_str);
        let mut ctx = ParserSemIrCtx {
            input: &mut self.input,
            semantic_hooks: &mut self.semantic_hooks,
            rule_index: request.rule_index,
            coordinate_index: request.pred_index,
            rule_name,
            context: request.context,
            local_int_arg: request.local_int_arg,
            member_values: request.member_values,
            invoked_predicates: &mut self.invoked_predicates,
        };
        semir::eval_pred(&semantics.ir, predicate.expr, &mut ctx)
    }

    fn parser_predicate_matches(&mut self, eval: PredicateEval<'_>) -> bool {
        let PredicateEval {
            index,
            rule_index,
            pred_index,
            predicates,
            semantics,
            context,
            local_int_arg,
            member_values,
        } = eval;
        if let Some((semantics, predicate)) = semantics.and_then(|semantics| {
            semantics
                .predicates
                .iter()
                .find(|predicate| {
                    predicate.rule_index == rule_index && predicate.pred_index == pred_index
                })
                .map(|predicate| (semantics, predicate))
        }) {
            return self.parser_semir_predicate_matches(
                semantics,
                predicate,
                ParserSemanticHookRequest {
                    index,
                    rule_index,
                    pred_index,
                    context,
                    local_int_arg,
                    member_values,
                },
            );
        }
        let Some((_, _, predicate)) = predicates
            .iter()
            .find(|(rule, pred, _)| *rule == rule_index && *pred == pred_index)
        else {
            if let Some(result) = self.parser_semantic_hook_result(ParserSemanticHookRequest {
                index,
                rule_index,
                pred_index,
                context,
                local_int_arg,
                member_values,
            }) {
                return result;
            }
            return self.unknown_predicate_result(rule_index, pred_index);
        };
        self.input.seek(index);
        match predicate {
            ParserPredicate::True => true,
            ParserPredicate::False => false,
            ParserPredicate::FalseWithMessage { .. } => false,
            ParserPredicate::Invoke { value } => {
                let key = (rule_index, pred_index);
                if !self.invoked_predicates.contains(&key) {
                    self.invoked_predicates.push(key);
                    use std::io::Write as _;
                    let mut stdout = std::io::stdout().lock();
                    let _ = writeln!(stdout, "eval={value}");
                }
                *value
            }
            ParserPredicate::LookaheadTextEquals { offset, text } => {
                self.input.lt(*offset).and_then(Token::text) == Some(*text)
            }
            ParserPredicate::LookaheadNotEquals { offset, token_type } => {
                self.la(*offset) != *token_type
            }
            ParserPredicate::TokenPairAdjacent => {
                let Some(first) = self.input.lt(-2).map(Token::token_index) else {
                    return false;
                };
                let Some(second) = self.input.lt(-1).map(Token::token_index) else {
                    return false;
                };
                first + 1 == second
            }
            ParserPredicate::ContextChildRuleTextNotEquals { rule_index, text } => context
                .and_then(|context| {
                    context.children().iter().find_map(|child| match child {
                        ParseTree::Rule(rule) if rule.context().rule_index() == *rule_index => {
                            Some(child.text())
                        }
                        ParseTree::Rule(_) | ParseTree::Terminal(_) | ParseTree::Error(_) => None,
                    })
                })
                .is_none_or(|actual| actual != *text),
            ParserPredicate::LocalIntEquals { value } => {
                local_int_arg.is_none_or(|(_, actual)| actual == *value)
            }
            ParserPredicate::LocalIntLessOrEqual { value } => {
                local_int_arg.is_none_or(|(_, actual)| actual <= *value)
            }
            ParserPredicate::MemberModuloEquals {
                member,
                modulus,
                value,
                equals,
            } => {
                if *modulus == 0 {
                    return false;
                }
                let actual = member_values.get(member).copied().unwrap_or_default() % *modulus;
                (actual == *value) == *equals
            }
            ParserPredicate::MemberEquals {
                member,
                value,
                equals,
            } => {
                let actual = member_values.get(member).copied().unwrap_or_default();
                (actual == *value) == *equals
            }
        }
    }

    /// Returns a generated fail-option message for a predicate coordinate.
    fn parser_predicate_failure_message(
        &self,
        rule_index: usize,
        pred_index: usize,
        predicates: &[(usize, usize, ParserPredicate)],
    ) -> Option<&'static str> {
        predicates
            .iter()
            .find_map(|(rule, pred, predicate)| match predicate {
                ParserPredicate::FalseWithMessage { message }
                    if *rule == rule_index && *pred == pred_index =>
                {
                    Some(*message)
                }
                _ => None,
            })
    }

    /// Returns a generated fail-option message for a `SemIR` predicate
    /// coordinate.
    pub fn parser_semantic_ir_predicate_failure_message(
        &self,
        rule_index: usize,
        pred_index: usize,
        semantics: &ParserSemantics,
    ) -> Option<&'static str> {
        semantics
            .predicates
            .iter()
            .find(|predicate| {
                predicate.rule_index == rule_index && predicate.pred_index == pred_index
            })
            .and_then(|predicate| predicate.failure_message)
    }

    /// Returns the token-stream index after consuming `symbol` at `index`.
    ///
    /// EOF is not advanced by ANTLR token streams, so EOF transitions keep the
    /// index stable and rely on `consumed_eof` to record that EOF was matched.
    /// The parser's stream cursor is left untouched: speculative recognition
    /// reads ahead by absolute index, so paying for `seek` on every visited
    /// state would dominate the hot path. Real consumption is committed by
    /// `parse_atn_rule` via `seek` once a viable outcome is selected.
    fn consume_index(&mut self, index: usize, symbol: i32) -> usize {
        if symbol == TOKEN_EOF {
            return index;
        }
        self.input.next_visible_after(index)
    }

    /// Builds ANTLR's no-viable-alternative diagnostic for an ambiguous
    /// decision that failed after consuming a shared prefix.
    fn no_viable_alternative(
        &mut self,
        start_index: usize,
        error_index: usize,
    ) -> ParserDiagnostic {
        let text = display_input_text(&self.input.text(start_index, error_index));
        diagnostic_for_token(
            self.token_at(error_index).as_ref(),
            format!("no viable alternative at input '{text}'"),
        )
    }

    /// Selects the diagnostic for a failed consuming transition after all
    /// recovery repairs have been ruled out.
    fn recovery_failure_diagnostic(
        &mut self,
        index: usize,
        decision_start_index: Option<usize>,
        expected_symbols: &BTreeSet<i32>,
    ) -> ParserDiagnostic {
        if expected_symbols.len() > 1 {
            if let Some(decision_start) = no_viable_decision_start(decision_start_index, index) {
                return self.no_viable_alternative(decision_start, index);
            }
        }
        diagnostic_for_token(
            self.token_at(index).as_ref(),
            format!(
                "mismatched input {} expecting {}",
                self.token_at(index)
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(expected_symbols)
            ),
        )
    }

    /// Builds the EOF diagnostic used when ANTLR unwinds a failed nested rule
    /// instead of inserting missing tokens in the caller.
    fn eof_rule_recovery_diagnostic(
        &mut self,
        index: usize,
        expected_symbols: &BTreeSet<i32>,
        expected: &ExpectedTokens,
    ) -> ParserDiagnostic {
        let symbols = if expected.index == Some(index) && !expected.symbols.is_empty() {
            &expected.symbols
        } else {
            expected_symbols
        };
        diagnostic_for_token(
            self.token_at(index).as_ref(),
            format!(
                "mismatched input {} expecting {}",
                self.token_at(index)
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(symbols)
            ),
        )
    }

    /// Returns token text for a buffered token interval used by generated
    /// `$text` actions.
    ///
    /// ANTLR treats EOF as a range boundary rather than printable input text,
    /// even when an action interval explicitly stops at the EOF token.
    pub fn text_interval(&mut self, start: usize, stop: Option<usize>) -> String {
        let Some(stop) = stop else {
            return String::new();
        };
        let stop = if self
            .token_at(stop)
            .is_some_and(|token| token.token_type() == TOKEN_EOF)
        {
            let Some(previous) = self.previous_token_index(stop) else {
                return String::new();
            };
            previous
        } else {
            stop
        };
        self.input.text(start, stop)
    }

    /// Resets per-parse prediction diagnostics while keeping the parser-level
    /// reporting flag configured by generated harness code.
    fn clear_prediction_diagnostics(&mut self) {
        self.prediction_diagnostics.clear();
        self.reported_prediction_diagnostics.clear();
    }

    /// Drops every per-parse cache that depends on ATN identity or pins
    /// recovery-symbol allocations.
    ///
    /// `BaseParser::parse_atn_rule` takes `&Atn` on each invocation, so the
    /// same parser instance can legally be driven against different grammars
    /// in sequence. The four caches reset here are keyed by raw ATN
    /// coordinates (state numbers, rule indexes) and would silently hand back
    /// entries from a previous ATN if reused — pruning lookahead against the
    /// wrong transitions or pinning recovery `Rc<BTreeSet<i32>>` allocations
    /// for the rest of the process. Clearing them on every parse entry keeps
    /// the perf wins (caches still amortize within one parse) without making
    /// long-lived parsers leak memory or surface stale ATN data:
    ///
    /// * `rule_first_set_cache` and `decision_lookahead_cache` are pure
    ///   functions of the ATN's state graph.
    /// * `state_expected_cache`, `state_expected_token_cache`,
    ///   `rule_stop_reach_cache`, and
    ///   `recovery_symbols_intern` together form
    ///   the identity invariant that lets `FastRecognizeKey` hash
    ///   `recovery_symbols` by pointer; they have to be cleared in lockstep
    ///   so a stale interned `Rc` cannot outlive its map entry.
    fn reset_per_parse_caches(&mut self) {
        self.rule_first_set_cache.clear();
        self.decision_lookahead_cache.clear();
        self.ll1_decision_cache.clear();
        self.empty_cycle_cache.clear();
        self.rule_stop_reach_cache.clear();
        self.single_outcome_memo_mode = SingleOutcomeMemoMode::Probe;
        self.single_outcome_probe_seen.clear();
        self.single_outcome_probe_samples = 0;
        self.single_outcome_probe_repeats = 0;
        self.recovery_symbols_intern.clear();
        self.state_expected_cache.clear();
        self.state_expected_token_cache.clear();
    }

    /// Buffers ANTLR-style diagnostic-listener messages for decision states
    /// where multiple clean alternatives survive full-context recognition.
    fn record_prediction_diagnostics(
        &mut self,
        atn: &Atn,
        state: &AtnState,
        start_index: usize,
        outcomes: &[RecognizeOutcome],
    ) {
        if !self.report_diagnostic_errors || state.transitions.len() < 2 {
            return;
        }
        let Some(decision) = atn
            .decision_to_state()
            .iter()
            .position(|state_number| *state_number == state.state_number)
        else {
            return;
        };
        let Some(rule_index) = state.rule_index else {
            return;
        };
        let mut alts_by_end = BTreeMap::<usize, BTreeSet<usize>>::new();
        for outcome in outcomes
            .iter()
            .filter(|outcome| outcome.diagnostics.is_empty())
        {
            let Some(alt) = outcome.decisions.first() else {
                continue;
            };
            alts_by_end
                .entry(outcome.index)
                .or_default()
                .insert(alt + 1);
        }
        let Some((&end_index, ambig_alts)) = alts_by_end
            .iter()
            .filter(|(_, alts)| alts.len() > 1)
            .max_by_key(|(end, _)| *end)
        else {
            return;
        };
        let rule_name = self
            .rule_names()
            .get(rule_index)
            .map_or_else(|| "<unknown>".to_owned(), Clone::clone);
        let stop_index = self.previous_token_index(end_index).unwrap_or(start_index);
        let input = display_input_text(&self.input.text(start_index, stop_index));
        let alts = ambig_alts
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let key = (decision, start_index, format!("{alts}:{input}"));
        if !self.reported_prediction_diagnostics.insert(key) {
            return;
        }
        let start_token = self.token_at(start_index);
        let stop_token = self.token_at(stop_index);
        self.prediction_diagnostics.push(diagnostic_for_token(
            start_token.as_ref(),
            format!("reportAttemptingFullContext d={decision} ({rule_name}), input='{input}'"),
        ));
        self.prediction_diagnostics.push(diagnostic_for_token(
            stop_token.as_ref(),
            format!(
                "reportAmbiguity d={decision} ({rule_name}): ambigAlts={{{alts}}}, input='{input}'"
            ),
        ));
    }

    /// Formats the tokens expected from an ATN state using ANTLR display names.
    pub fn expected_tokens_at_state(&self, atn: &Atn, state_number: usize) -> String {
        expected_symbols_display(
            &state_expected_symbols(atn, state_number),
            self.vocabulary(),
        )
    }

    /// Formats a buffered token in ANTLR's diagnostic token display form.
    pub fn token_display_at(&mut self, index: usize) -> Option<String> {
        self.token_at(index).map(|token| format!("{token}"))
    }

    /// Converts a recognized internal node into a public parse-tree node.
    fn recognized_node_tree(
        &mut self,
        node: &RecognizedNode,
        track_alt_numbers: bool,
    ) -> Result<ParseTree, AntlrError> {
        match node {
            RecognizedNode::Token { index } => {
                let token = self
                    .input
                    .get_ref(*index)
                    .ok_or_else(|| AntlrError::ParserError {
                        line: 0,
                        column: 0,
                        message: format!("missing token at index {index}"),
                    })?;
                Ok(ParseTree::Terminal(TerminalNode::from_ref(token)))
            }
            RecognizedNode::ErrorToken { index } => {
                let token = self
                    .input
                    .get_ref(*index)
                    .ok_or_else(|| AntlrError::ParserError {
                        line: 0,
                        column: 0,
                        message: format!("missing error token at index {index}"),
                    })?;
                Ok(ParseTree::Error(ErrorNode::from_ref(token)))
            }
            RecognizedNode::MissingToken {
                token_type,
                at_index,
                text,
            } => {
                let current = self.token_at(*at_index);
                let token = CommonToken::new(*token_type)
                    .with_text(text.as_str())
                    .with_span(usize::MAX, usize::MAX)
                    .with_position(
                        current.as_ref().map(Token::line).unwrap_or_default(),
                        current.as_ref().map(Token::column).unwrap_or_default(),
                    );
                Ok(ParseTree::Error(ErrorNode::new(token)))
            }
            RecognizedNode::Rule {
                rule_index,
                invoking_state,
                alt_number,
                start_index,
                stop_index,
                return_values,
                children,
            } => {
                let mut context = ParserRuleContext::new(*rule_index, *invoking_state);
                if track_alt_numbers {
                    context.set_alt_number(*alt_number);
                }
                for (name, value) in return_values {
                    context.set_int_return(name.clone(), *value);
                }
                if let Some(token) = self.token_ref_at(*start_index) {
                    context.set_start_ref(token);
                }
                if let Some(token) = stop_index.and_then(|index| self.token_ref_at(index)) {
                    context.set_stop_ref(token);
                }
                for child in children {
                    context.add_child(self.recognized_node_tree(child, track_alt_numbers)?);
                }
                Ok(self.rule_node(context))
            }
            RecognizedNode::LeftRecursiveBoundary { rule_index } => Err(AntlrError::Unsupported(
                format!("unfolded left-recursive boundary for rule {rule_index}"),
            )),
        }
    }
}

impl<S, H> DirectAdaptiveParser<'_, '_, S, H>
where
    S: TokenSource,
    H: SemanticHooks,
{
    fn parse_rule(
        &mut self,
        rule_index: usize,
        invoking_state: isize,
        precedence: i32,
    ) -> DirectAdaptiveParseResult<ParseTree> {
        let start_state = *self.atn.rule_to_start_state().get(rule_index).ok_or(
            DirectAdaptiveParseControl::Fallback(DirectAdaptiveFallback::MissingAtn),
        )?;
        let stop_state = *self
            .atn
            .rule_to_stop_state()
            .get(rule_index)
            .filter(|state| **state != usize::MAX)
            .ok_or(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::MissingAtn,
            ))?;
        let start_index = self.parser.current_visible_index();
        let mut children = Vec::new();
        let mut state_number = start_state;
        let mut consumed_eof = false;
        while state_number != stop_state {
            self.step()?;
            let (transition, boundary) = self.next_transition(state_number, precedence)?;
            if boundary.is_some() {
                return Err(DirectAdaptiveParseControl::Fallback(
                    DirectAdaptiveFallback::LeftRecursiveBoundary,
                ));
            }
            match transition {
                Transition::Epsilon { target } => {
                    state_number = target;
                }
                Transition::Precedence {
                    target,
                    precedence: transition_precedence,
                } => {
                    if transition_precedence < precedence {
                        return Err(DirectAdaptiveParseControl::Fallback(
                            DirectAdaptiveFallback::Precedence,
                        ));
                    }
                    state_number = target;
                }
                Transition::Rule {
                    rule_index,
                    follow_state,
                    precedence: rule_precedence,
                    ..
                } => {
                    let child = self.parse_rule(
                        rule_index,
                        invoking_state_number(state_number),
                        rule_precedence,
                    )?;
                    if self.parser.build_parse_trees {
                        children.push(child);
                    }
                    state_number = follow_state;
                }
                Transition::Atom { .. }
                | Transition::Range { .. }
                | Transition::Set { .. }
                | Transition::NotSet { .. }
                | Transition::Wildcard { .. } => {
                    let (matched_eof, child) = self.consume_transition(&transition)?;
                    consumed_eof |= matched_eof;
                    if let Some(child) = child {
                        children.push(child);
                    }
                    state_number = transition.target();
                }
                Transition::Predicate { .. } => {
                    return Err(DirectAdaptiveParseControl::Fallback(
                        DirectAdaptiveFallback::Predicate,
                    ));
                }
                Transition::Action { .. } => {
                    return Err(DirectAdaptiveParseControl::Fallback(
                        DirectAdaptiveFallback::Action,
                    ));
                }
            }
        }

        let mut context = ParserRuleContext::with_child_capacity(
            rule_index,
            invoking_state,
            if self.parser.build_parse_trees {
                children.len()
            } else {
                0
            },
        );
        if let Some(token) = self.parser.token_ref_at(start_index) {
            context.set_start_ref(token);
        }
        let stop_index = self
            .parser
            .rule_stop_token_index(self.parser.input.index(), consumed_eof);
        if let Some(token) = stop_index.and_then(|index| self.parser.token_ref_at(index)) {
            context.set_stop_ref(token);
        }
        if self.parser.build_parse_trees {
            for child in children {
                context.add_child(child);
            }
        }
        Ok(self.parser.rule_node(context))
    }

    const fn step(&mut self) -> DirectAdaptiveParseResult<()> {
        self.steps += 1;
        if self.steps > ADAPTIVE_DIRECT_STEP_LIMIT {
            return Err(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::StepLimit,
            ));
        }
        Ok(())
    }

    fn next_transition(
        &mut self,
        state_number: usize,
        precedence: i32,
    ) -> DirectAdaptiveParseResult<(Transition, Option<usize>)> {
        let state = self
            .atn
            .state(state_number)
            .ok_or(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::MissingAtn,
            ))?;
        if state.is_rule_stop() {
            return Err(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::RuleStop,
            ));
        }
        let transition_index =
            self.transition_index(state_number, state.transitions.len(), precedence)?;
        let transition = state.transitions.get(transition_index).cloned().ok_or(
            DirectAdaptiveParseControl::Fallback(DirectAdaptiveFallback::NoTransition),
        )?;
        let boundary = match &transition {
            Transition::Epsilon { target } | Transition::Precedence { target, .. } => {
                left_recursive_boundary(self.atn, state, *target)
            }
            _ => None,
        };
        Ok((transition, boundary))
    }

    fn transition_index(
        &mut self,
        state_number: usize,
        transition_count: usize,
        precedence: i32,
    ) -> DirectAdaptiveParseResult<usize> {
        match transition_count {
            0 => Err(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::NoTransition,
            )),
            1 => Ok(0),
            _ => {
                if let Some(alt) = self.ll1_transition_index(state_number, transition_count)? {
                    return Ok(alt);
                }
                let decision = self
                    .decision_by_state
                    .get(state_number)
                    .and_then(|decision| *decision)
                    .ok_or(DirectAdaptiveParseControl::Fallback(
                        DirectAdaptiveFallback::UnknownDecision,
                    ))?;
                let prediction = self
                    .simulator
                    .adaptive_predict_stream_info_with_precedence(
                        decision,
                        direct_precedence(precedence),
                        &mut self.parser.input,
                    )
                    .map_err(|_| {
                        DirectAdaptiveParseControl::Fallback(DirectAdaptiveFallback::Prediction)
                    })?;
                if prediction.has_semantic_context {
                    return Err(DirectAdaptiveParseControl::Fallback(
                        DirectAdaptiveFallback::SemanticContext,
                    ));
                }
                prediction
                    .alt
                    .checked_sub(1)
                    .filter(|index| *index < transition_count)
                    .ok_or(DirectAdaptiveParseControl::Fallback(
                        DirectAdaptiveFallback::InvalidAlt,
                    ))
            }
        }
    }

    fn ll1_transition_index(
        &mut self,
        state_number: usize,
        transition_count: usize,
    ) -> DirectAdaptiveParseResult<Option<usize>> {
        let state = self
            .atn
            .state(state_number)
            .ok_or(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::MissingAtn,
            ))?;
        if state.precedence_rule_decision {
            return Ok(None);
        }
        let Some(rule_stop) = state
            .rule_index
            .and_then(|rule_index| self.atn.rule_to_stop_state().get(rule_index).copied())
        else {
            return Ok(None);
        };
        let symbol = self.parser.input.la_token(1);
        let entry = self
            .parser
            .cached_decision_lookahead(self.atn, state, rule_stop);
        Ok(ll1_greedy_alt(&entry, symbol, state.non_greedy).filter(|alt| *alt < transition_count))
    }

    fn consume_transition(
        &mut self,
        transition: &Transition,
    ) -> DirectAdaptiveParseResult<(bool, Option<ParseTree>)> {
        let symbol = self.parser.input.la_token(1);
        if !transition.matches(symbol, 1, self.atn.max_token_type()) {
            return Err(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::TokenMismatch,
            ));
        }
        let token = self
            .parser
            .input
            .lt_ref(1)
            .ok_or(DirectAdaptiveParseControl::Fallback(
                DirectAdaptiveFallback::TokenMismatch,
            ))?;
        let matched_eof = symbol == TOKEN_EOF;
        if !matched_eof {
            self.parser.consume();
        }
        let child = self
            .parser
            .build_parse_trees
            .then(|| ParseTree::Terminal(TerminalNode::from_ref(token)));
        Ok((matched_eof, child))
    }
}

/// Detects the loop edge where ANTLR would call `pushNewRecursionContext` for a
/// transformed left-recursive rule.
fn left_recursive_boundary(atn: &Atn, state: &AtnState, target: usize) -> Option<usize> {
    if !state.precedence_rule_decision {
        return None;
    }
    let target_state = atn.state(target)?;
    if target_state.kind == AtnStateKind::LoopEnd {
        return None;
    }
    state.rule_index
}

/// Selects the first outer alternative observed for a rule path.
///
/// ANTLR's alt-numbered tree contexts store the rule alternative chosen at the
/// outer decision. The metadata recognizer only needs this when a generated
/// grammar opts into that target template; otherwise the value remains `0` and
/// parse-tree rendering is unchanged.
const fn next_alt_number(
    state: &AtnState,
    transition_index: usize,
    current_alt_number: usize,
    track_alt_numbers: bool,
) -> usize {
    if !track_alt_numbers || current_alt_number != 0 || state.transitions.len() <= 1 {
        return current_alt_number;
    }
    if matches!(
        state.kind,
        AtnStateKind::Basic
            | AtnStateKind::BlockStart
            | AtnStateKind::PlusBlockStart
            | AtnStateKind::StarBlockStart
            | AtnStateKind::StarLoopEntry
    ) && !state.precedence_rule_decision
    {
        return transition_index + 1;
    }
    current_alt_number
}

/// Folds boundary markers emitted at precedence-loop entries into nested rule
/// nodes, matching ANTLR's recursive-context parse-tree shape.
fn fold_left_recursive_boundaries(nodes: Vec<RecognizedNode>) -> Vec<RecognizedNode> {
    let mut folded = Vec::new();
    for node in nodes {
        match node {
            RecognizedNode::LeftRecursiveBoundary { rule_index } => {
                if !folded.is_empty() {
                    let children = std::mem::take(&mut folded);
                    let start_index = recognized_nodes_start_index(&children).unwrap_or_default();
                    let stop_index = recognized_nodes_stop_index(&children);
                    folded.push(RecognizedNode::Rule {
                        rule_index,
                        invoking_state: -1,
                        alt_number: 0,
                        start_index,
                        stop_index,
                        return_values: BTreeMap::new(),
                        children,
                    });
                }
            }
            node => folded.push(node),
        }
    }
    folded
}

/// Mirrors [`fold_left_recursive_boundaries`] for [`FastRecognizedNode`].
fn fold_fast_left_recursive_boundaries(
    nodes: Vec<Rc<FastRecognizedNode>>,
) -> Vec<Rc<FastRecognizedNode>> {
    // Most rule invocations have no left-recursive boundaries, so skip the
    // fold work entirely when none are present. The boundary marker is only
    // emitted at precedence-rule loop entries, which are rare relative to
    // every rule call the recognizer fans out.
    if !nodes.iter().any(|node| {
        matches!(
            node.as_ref(),
            FastRecognizedNode::LeftRecursiveBoundary { .. }
        )
    }) {
        return nodes;
    }
    let mut folded: Vec<Rc<FastRecognizedNode>> = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node.as_ref() {
            FastRecognizedNode::LeftRecursiveBoundary { rule_index } => {
                if !folded.is_empty() {
                    let children = std::mem::take(&mut folded);
                    let start_index =
                        fast_recognized_nodes_start_index(&children).unwrap_or_default();
                    let stop_index = fast_recognized_nodes_stop_index(&children);
                    folded.push(Rc::new(FastRecognizedNode::Rule {
                        rule_index: *rule_index,
                        invoking_state: -1,
                        start_index,
                        stop_index,
                        children: NodeList::from_vec(children),
                    }));
                }
            }
            _ => folded.push(node),
        }
    }
    folded
}

fn fast_node_has_left_recursive_boundary(node: &FastRecognizedNode) -> bool {
    match node {
        FastRecognizedNode::LeftRecursiveBoundary { .. } => true,
        FastRecognizedNode::Rule { children, .. } => children.has_left_recursive_boundary(),
        FastRecognizedNode::Token { .. }
        | FastRecognizedNode::ErrorToken { .. }
        | FastRecognizedNode::MissingToken { .. } => false,
    }
}

fn fast_recognized_nodes_start_index(nodes: &[Rc<FastRecognizedNode>]) -> Option<usize> {
    nodes
        .iter()
        .find_map(|node| fast_recognized_node_start_index(node.as_ref()))
}

const fn fast_recognized_node_start_index(node: &FastRecognizedNode) -> Option<usize> {
    match node {
        FastRecognizedNode::Token { index } | FastRecognizedNode::ErrorToken { index } => {
            Some(*index)
        }
        FastRecognizedNode::MissingToken { at_index, .. } => Some(*at_index),
        FastRecognizedNode::Rule { start_index, .. } => Some(*start_index),
        FastRecognizedNode::LeftRecursiveBoundary { .. } => None,
    }
}

const fn fast_recognized_node_span(node: &FastRecognizedNode) -> Option<(usize, Option<usize>)> {
    match node {
        FastRecognizedNode::Token { index } | FastRecognizedNode::ErrorToken { index } => {
            Some((*index, Some(*index)))
        }
        FastRecognizedNode::MissingToken { at_index, .. } => Some((*at_index, None)),
        FastRecognizedNode::Rule {
            start_index,
            stop_index,
            ..
        } => Some((*start_index, *stop_index)),
        FastRecognizedNode::LeftRecursiveBoundary { .. } => None,
    }
}

fn fast_recognized_nodes_stop_index(nodes: &[Rc<FastRecognizedNode>]) -> Option<usize> {
    nodes
        .iter()
        .rev()
        .find_map(|node| fast_recognized_node_stop_index(node.as_ref()))
}

const fn fast_recognized_node_stop_index(node: &FastRecognizedNode) -> Option<usize> {
    match node {
        FastRecognizedNode::Token { index } | FastRecognizedNode::ErrorToken { index } => {
            Some(*index)
        }
        FastRecognizedNode::MissingToken { at_index, .. } => at_index.checked_sub(1),
        FastRecognizedNode::Rule { stop_index, .. } => *stop_index,
        FastRecognizedNode::LeftRecursiveBoundary { .. } => None,
    }
}

fn recognized_nodes_start_index(nodes: &[RecognizedNode]) -> Option<usize> {
    nodes.iter().find_map(recognized_node_start_index)
}

const fn recognized_node_start_index(node: &RecognizedNode) -> Option<usize> {
    match node {
        RecognizedNode::Token { index } | RecognizedNode::ErrorToken { index } => Some(*index),
        RecognizedNode::MissingToken { at_index, .. } => Some(*at_index),
        RecognizedNode::Rule { start_index, .. } => Some(*start_index),
        RecognizedNode::LeftRecursiveBoundary { .. } => None,
    }
}

fn recognized_nodes_stop_index(nodes: &[RecognizedNode]) -> Option<usize> {
    nodes.iter().rev().find_map(recognized_node_stop_index)
}

/// Converts an ATN state number into the signed invoking-state slot used by
/// ANTLR parse-tree contexts, saturating only for impossible platform widths.
fn invoking_state_number(state_number: usize) -> isize {
    isize::try_from(state_number).unwrap_or(isize::MAX)
}

fn direct_precedence(precedence: i32) -> usize {
    usize::try_from(precedence.max(0)).unwrap_or_default()
}

const fn recognized_node_stop_index(node: &RecognizedNode) -> Option<usize> {
    match node {
        RecognizedNode::Token { index } | RecognizedNode::ErrorToken { index } => Some(*index),
        RecognizedNode::MissingToken { at_index, .. } => at_index.checked_sub(1),
        RecognizedNode::Rule { stop_index, .. } => *stop_index,
        RecognizedNode::LeftRecursiveBoundary { .. } => None,
    }
}

fn token_input_display(token: &impl Token) -> String {
    format!("'{}'", token.text().unwrap_or("<EOF>"))
}

fn display_input_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn diagnostic_for_token(token: Option<&impl Token>, message: String) -> ParserDiagnostic {
    ParserDiagnostic {
        line: token.map(Token::line).unwrap_or_default(),
        column: token.map(Token::column).unwrap_or_default(),
        message,
    }
}

/// Emits parser diagnostics for the selected recovered parse path.
#[allow(clippy::print_stderr)]
fn report_parser_diagnostics(diagnostics: &[ParserDiagnostic]) {
    for diagnostic in diagnostics {
        eprintln!(
            "line {}:{} {}",
            diagnostic.line, diagnostic.column, diagnostic.message
        );
    }
}

/// Emits generated parser diagnostics and lexer diagnostics in the same
/// source-position order as ANTLR's lazy token stream reports them.
#[allow(clippy::print_stderr)]
fn report_generated_diagnostics(
    parser_diagnostics: &[ParserDiagnostic],
    token_errors: &[TokenSourceError],
) {
    #[derive(Clone, Copy)]
    enum DiagnosticSource {
        Token(usize),
        Parser(usize),
    }

    let mut ordered = Vec::with_capacity(parser_diagnostics.len() + token_errors.len());
    ordered.extend(token_errors.iter().enumerate().map(|(index, error)| {
        (
            error.line,
            error.column,
            0_usize,
            index,
            DiagnosticSource::Token(index),
        )
    }));
    ordered.extend(
        parser_diagnostics
            .iter()
            .enumerate()
            .map(|(index, diagnostic)| {
                (
                    diagnostic.line,
                    diagnostic.column,
                    1_usize,
                    index,
                    DiagnosticSource::Parser(index),
                )
            }),
    );
    ordered.sort_by_key(|(line, column, source_order, index, _)| {
        (*line, *column, *source_order, *index)
    });

    for (_, _, _, _, source) in ordered {
        match source {
            DiagnosticSource::Token(index) => {
                let error = &token_errors[index];
                eprintln!("line {}:{} {}", error.line, error.column, error.message);
            }
            DiagnosticSource::Parser(index) => {
                let diagnostic = &parser_diagnostics[index];
                eprintln!(
                    "line {}:{} {}",
                    diagnostic.line, diagnostic.column, diagnostic.message
                );
            }
        }
    }
}

/// Emits buffered token-source diagnostics after parser diagnostics that were
/// discovered while speculatively reading the same token stream.
#[allow(clippy::print_stderr)]
fn report_token_source_errors(errors: &[TokenSourceError]) {
    for error in errors {
        eprintln!("line {}:{} {}", error.line, error.column, error.message);
    }
}

fn expected_symbols_display(symbols: &BTreeSet<i32>, vocabulary: &Vocabulary) -> String {
    let items = symbols
        .iter()
        .map(|symbol| expected_symbol_display(*symbol, vocabulary))
        .collect::<Vec<_>>();
    if let [single] = items.as_slice() {
        return single.clone();
    }
    format!("{{{}}}", items.join(", "))
}

fn expected_symbol_display(symbol: i32, vocabulary: &Vocabulary) -> String {
    if symbol == TOKEN_EOF {
        return "<EOF>".to_owned();
    }
    vocabulary.display_name(symbol)
}

fn is_caller_follow_boundary_text(text: &str) -> bool {
    text.chars().any(|ch| ch == ';' || ch == '\n')
        && text.chars().all(|ch| ch.is_whitespace() || ch == ';')
}

fn is_caller_follow_boundary_gap_text(text: &str) -> bool {
    text.chars().all(|ch| ch.is_whitespace() || ch == ';')
}

/// Returns whether `state` belongs to an ANTLR-transformed left-recursive rule.
/// Inline insertion in those precedence loops can synthesize a missing operand
/// before an operator and then block the legitimate loop-exit path.
fn state_is_left_recursive_rule(atn: &Atn, state: &AtnState) -> bool {
    let Some(rule_index) = state.rule_index else {
        return false;
    };
    atn.rule_to_start_state()
        .get(rule_index)
        .and_then(|state_number| atn.state(*state_number))
        .is_some_and(|rule_start| rule_start.left_recursive_rule)
}

/// Picks the better of two `parse_atn_rule` passes (with and without the
/// FIRST-set prefilter). A clean outcome (no diagnostics) always wins over a
/// recovered one; among recovered outcomes the second pass is preferred
/// because the no-prefilter walk reaches ANTLR-style recovery inside child
/// rules. If both passes failed, the second pass's expected-token snapshot
/// is returned so the caller renders the same diagnostic ANTLR would.
fn select_better_top_outcome(
    first: Result<(FastRecognizeOutcome, ExpectedTokens), ExpectedTokens>,
    second: Result<(FastRecognizeOutcome, ExpectedTokens), ExpectedTokens>,
) -> Result<(FastRecognizeOutcome, ExpectedTokens), ExpectedTokens> {
    match (first, second) {
        (Ok(first), Ok(second)) => {
            if first.0.diagnostics.is_empty() {
                Ok(first)
            } else {
                Ok(second)
            }
        }
        (Ok(first), Err(_)) => Ok(first),
        (Err(_), Ok(second)) => Ok(second),
        (Err(_), Err(second_expected)) => Err(second_expected),
    }
}

/// Chooses the outermost parse result that consumed the most input.
///
/// The recognizer intentionally keeps shorter endpoints available while walking
/// nested rule transitions so callers can satisfy following tokens such as
/// `expr 'and' expr`. Only the public rule entry commits to one endpoint.
fn select_best_fast_outcome(
    outcomes: impl Iterator<Item = FastRecognizeOutcome>,
    prediction_mode: PredictionMode,
    caller_follow: Option<&TokenBitSet>,
    mut token_info_at: impl FnMut(usize) -> (i32, bool, bool),
) -> Option<FastRecognizeOutcome> {
    let mut best = None;
    let mut best_caller_follow = None;
    for outcome in outcomes {
        if matches!(
            prediction_mode,
            PredictionMode::Ll | PredictionMode::LlExactAmbigDetection
        ) && outcome.diagnostics.is_empty()
            && let Some(follow) = caller_follow
        {
            let (token_type, is_boundary, _) = token_info_at(outcome.index);
            if is_boundary && follow.contains(token_type) {
                let replace =
                    best_caller_follow
                        .as_ref()
                        .is_none_or(|existing: &FastRecognizeOutcome| {
                            (outcome.index, outcome.consumed_eof)
                                < (existing.index, existing.consumed_eof)
                        });
                if replace {
                    best_caller_follow = Some(outcome.clone());
                }
            }
        }
        let Some(existing) = best else {
            best = Some(outcome);
            continue;
        };
        let outcome_position = (outcome.index, outcome.consumed_eof);
        let best_position = (existing.index, existing.consumed_eof);
        let better = match prediction_mode {
            PredictionMode::Ll | PredictionMode::LlExactAmbigDetection => outcome_is_better(
                outcome_position,
                &outcome.diagnostics,
                best_position,
                &existing.diagnostics,
            ),
            PredictionMode::Sll => outcome.index > existing.index,
        };
        best = Some(if better { outcome } else { existing });
    }
    let should_use_caller_follow =
        best_caller_follow
            .as_ref()
            .zip(best.as_ref())
            .is_some_and(|(candidate, selected)| {
                if !selected.diagnostics.is_empty() {
                    return true;
                }
                candidate.index < selected.index
                    && (candidate.index..selected.index).all(|index| token_info_at(index).2)
            });
    if should_use_caller_follow {
        best_caller_follow
    } else {
        best
    }
}

fn select_best_outcome(
    outcomes: impl Iterator<Item = RecognizeOutcome>,
    prediction_mode: PredictionMode,
) -> Option<RecognizeOutcome> {
    let outcomes = outcomes.collect::<Vec<_>>();
    let prefer_first_tie = outcomes
        .iter()
        .any(|outcome| nodes_need_stable_tie(&outcome.nodes));
    outcomes.into_iter().reduce(|best, outcome| {
        let outcome_position = (outcome.index, outcome.consumed_eof);
        let best_position = (best.index, best.consumed_eof);
        let better = match prediction_mode {
            PredictionMode::Ll | PredictionMode::LlExactAmbigDetection => {
                outcome_is_better(
                    outcome_position,
                    &outcome.diagnostics,
                    best_position,
                    &best.diagnostics,
                ) || (!prefer_first_tie
                    && outcome_position == best_position
                    && outcome.diagnostics.len() == best.diagnostics.len()
                    && diagnostic_recovery_rank(&outcome.diagnostics)
                        == diagnostic_recovery_rank(&best.diagnostics)
                    && (outcome.decisions < best.decisions
                        || (outcome.decisions == best.decisions && outcome.actions > best.actions)))
            }
            PredictionMode::Sll => {
                outcome_position > best_position
                    || (outcome_position == best_position
                        && !prefer_first_tie
                        && (outcome.decisions < best.decisions
                            || (outcome.decisions == best.decisions
                                && outcome_is_better(
                                    outcome_position,
                                    &outcome.diagnostics,
                                    best_position,
                                    &best.diagnostics,
                                ))))
            }
        };
        if better {
            return outcome;
        }
        best
    })
}

/// Records the serialized transition order at parser decision states.
///
/// When two clean paths consume the same input, ANTLR's adaptive prediction
/// chooses by alternative order. Keeping this compact trace lets the metadata
/// recognizer distinguish greedy and non-greedy optional blocks without a full
/// prediction simulator.
fn transition_decision(
    atn: &Atn,
    state: &AtnState,
    transition_index: usize,
    predicates: &[(usize, usize, ParserPredicate)],
) -> Option<usize> {
    if state.transitions.len() <= 1
        || state.precedence_rule_decision
        || decision_reaches_unsupported_predicate(atn, state, predicates)
    {
        return None;
    }
    Some(transition_index)
}

/// Reports whether a state should reset the active no-viable decision start.
///
/// Loop entry/back states are continuations of the surrounding adaptive
/// prediction; resetting at those states would turn LL-star failures back into
/// ordinary mismatches.
const fn starts_prediction_decision(state: &AtnState) -> bool {
    state.transitions.len() > 1
        && !matches!(
            state.kind,
            AtnStateKind::PlusLoopBack | AtnStateKind::StarLoopBack | AtnStateKind::StarLoopEntry
        )
}

/// Marks a farthest expected-token set as no-viable when multiple alternatives
/// failed after the active decision had already consumed input.
fn record_no_viable_if_ambiguous(
    expected: &mut ExpectedTokens,
    decision_start_index: Option<usize>,
    index: usize,
) {
    if expected.index == Some(index) && expected.symbols.len() > 1 {
        if let Some(decision_start) = no_viable_decision_start(decision_start_index, index) {
            expected.record_no_viable(decision_start, index);
        }
    }
}

/// Records a no-viable decision caused by a failed semantic predicate before
/// any consuming transition can contribute an expected-token set.
const fn record_predicate_no_viable(
    expected: &mut ExpectedTokens,
    decision_start_index: Option<usize>,
    index: usize,
) {
    if let Some(decision_start) = decision_start_index {
        expected.record_no_viable(decision_start, index);
    }
}

/// Returns the active decision start only when the error is past that start.
const fn no_viable_decision_start(
    decision_start_index: Option<usize>,
    index: usize,
) -> Option<usize> {
    match decision_start_index {
        Some(start) if index > start => Some(start),
        _ => None,
    }
}

/// Restores expected-token bookkeeping when a child rule found a clean
/// consuming path; failures in longer child alternatives should not pollute the
/// caller's final expectation set.
fn restore_expected(
    children: &[RecognizeOutcome],
    child_start_index: usize,
    expected: &mut ExpectedTokens,
    snapshot: ExpectedTokens,
    preserve_child_expected: bool,
) {
    if preserve_child_expected {
        return;
    }
    if children
        .iter()
        .any(|child| child.diagnostics.is_empty() && child.index > child_start_index)
    {
        *expected = snapshot;
    }
}

/// Reports whether a decision can reach a predicate the generator did not
/// translate. Static alternative order is unsafe for those context predicates.
fn decision_reaches_unsupported_predicate(
    atn: &Atn,
    state: &AtnState,
    predicates: &[(usize, usize, ParserPredicate)],
) -> bool {
    state.transitions.iter().any(|transition| {
        transition_reaches_unsupported_predicate(atn, transition, predicates, &mut BTreeSet::new())
    })
}

/// Walks epsilon-like edges from one transition to find unsupported predicates.
fn transition_reaches_unsupported_predicate(
    atn: &Atn,
    transition: &Transition,
    predicates: &[(usize, usize, ParserPredicate)],
    visited: &mut BTreeSet<usize>,
) -> bool {
    match transition {
        Transition::Predicate {
            rule_index,
            pred_index,
            ..
        } => !predicates
            .iter()
            .any(|(rule, pred, _)| rule == rule_index && pred == pred_index),
        Transition::Epsilon { target }
        | Transition::Action { target, .. }
        | Transition::Rule { target, .. } => {
            state_reaches_unsupported_predicate(atn, *target, predicates, visited)
        }
        Transition::Precedence { .. }
        | Transition::Atom { .. }
        | Transition::Range { .. }
        | Transition::Set { .. }
        | Transition::NotSet { .. }
        | Transition::Wildcard { .. } => false,
    }
}

/// Finds an unsupported predicate reachable before a consuming transition.
fn state_reaches_unsupported_predicate(
    atn: &Atn,
    state_number: usize,
    predicates: &[(usize, usize, ParserPredicate)],
    visited: &mut BTreeSet<usize>,
) -> bool {
    if !visited.insert(state_number) {
        return false;
    }
    let Some(state) = atn.state(state_number) else {
        return false;
    };
    state.transitions.iter().any(|transition| {
        transition_reaches_unsupported_predicate(atn, transition, predicates, visited)
    })
}

/// Adds a decision step to the front of an already-recognized suffix path.
fn prepend_decision(outcome: &mut RecognizeOutcome, decision: Option<usize>) {
    if let Some(decision) = decision {
        outcome.decisions.insert(0, decision);
    }
}

fn outcome_is_better(
    outcome_position: (usize, bool),
    outcome_diagnostics: &[ParserDiagnostic],
    best_position: (usize, bool),
    best_diagnostics: &[ParserDiagnostic],
) -> bool {
    outcome_position > best_position
        || (outcome_position == best_position
            && (outcome_diagnostics.len() < best_diagnostics.len()
                || (outcome_diagnostics.len() == best_diagnostics.len()
                    && diagnostic_recovery_rank(outcome_diagnostics)
                        < diagnostic_recovery_rank(best_diagnostics))))
}

/// Ranks concrete recovery repairs ahead of generic non-EOF mismatch fallbacks
/// when speculative paths otherwise consume the same input.
fn diagnostic_recovery_rank(diagnostics: &[ParserDiagnostic]) -> usize {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("mismatched input ")
                && !diagnostic.message.starts_with("mismatched input '<EOF>' ")
        })
        .count()
}

fn discard_recovered_fast_outcomes_if_clean_path_exists(outcomes: &mut Vec<FastRecognizeOutcome>) {
    if outcomes
        .iter()
        .any(|outcome| outcome.diagnostics.is_empty())
    {
        outcomes.retain(|outcome| outcome.diagnostics.is_empty());
    }
}

fn discard_recovered_outcomes_if_clean_path_exists(outcomes: &mut Vec<RecognizeOutcome>) {
    if outcomes.iter().any(outcome_has_rule_failure_diagnostic) {
        return;
    }
    if outcomes
        .iter()
        .any(|outcome| outcome.diagnostics.is_empty())
    {
        outcomes.retain(|outcome| outcome.diagnostics.is_empty());
    }
}

/// Reports whether a recovered outcome came from an explicit predicate
/// fail-option and therefore should compete with shorter clean loop exits.
fn outcome_has_rule_failure_diagnostic(outcome: &RecognizeOutcome) -> bool {
    outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.starts_with("rule "))
}

/// Reports whether a candidate contains recursive tree structure where ANTLR's
/// first viable candidate preserves the correct left-recursive context shape.
fn nodes_need_stable_tie(nodes: &[RecognizedNode]) -> bool {
    nodes.iter().any(node_needs_stable_tie)
}

fn node_needs_stable_tie(node: &RecognizedNode) -> bool {
    match node {
        RecognizedNode::Token { .. }
        | RecognizedNode::ErrorToken { .. }
        | RecognizedNode::MissingToken { .. } => false,
        RecognizedNode::LeftRecursiveBoundary { .. } => true,
        RecognizedNode::Rule {
            rule_index,
            children,
            ..
        } => children.iter().any(|child| {
            matches!(
                child,
                RecognizedNode::Rule {
                    rule_index: child_rule,
                    ..
                } if child_rule == rule_index
            ) || node_needs_stable_tie(child)
        }),
    }
}

/// Removes equivalent endpoints before memoizing a state result while
/// preserving ATN transition-discovery order.
///
/// Outcomes are compared on observable recognition state — the input index,
/// EOF consumption, and diagnostics — without descending into the parse-tree
/// fragment carried by `nodes`. Two paths reaching the same point with
/// different node trees would otherwise prevent memoization from collapsing
/// equivalent suffixes and explode the speculative-path cache.
///
/// The first occurrence per recognition key wins, which matches ANTLR's
/// greedy alternative selection: serialized ATNs put greedy `*`/`+` loop-back
/// transitions before loop-exit, so the first-discovered outcome carries the
/// greedy parse-tree fragment.
fn dedupe_fast_outcomes(outcomes: &mut Vec<FastRecognizeOutcome>) {
    if outcomes.len() < 2 {
        return;
    }
    let mut keep = Vec::with_capacity(outcomes.len());
    let mut seen: BTreeMap<(usize, bool), Vec<usize>> = BTreeMap::new();
    'outcomes: for (index, outcome) in outcomes.iter().enumerate() {
        let bucket = seen
            .entry((outcome.index, outcome.consumed_eof))
            .or_default();
        for &previous in bucket.iter() {
            if outcomes[previous].diagnostics == outcome.diagnostics {
                continue 'outcomes;
            }
        }
        bucket.push(index);
        keep.push(index);
    }
    if keep.len() == outcomes.len() {
        return;
    }
    let mut iter = keep.into_iter();
    let mut next_keep = iter.next();
    let mut current = 0_usize;
    outcomes.retain(|_| {
        let result = next_keep == Some(current);
        if result {
            next_keep = iter.next();
        }
        current += 1;
        result
    });
}

fn dedupe_clean_fast_outcomes(outcomes: &mut Vec<FastRecognizeOutcome>) {
    if outcomes.len() < 2 {
        return;
    }
    // Most outcomes lists are 2-4 entries; an inline scan beats BTreeSet
    // here because BTreeSet's allocation + per-insert balancing dominates
    // O(log n) wins on tiny n. Retains the original order so callers that
    // depend on alt ordering (e.g. fast outcome selection) stay correct.
    //
    // Beyond the inline buffer we promote to a heap Vec so all kept entries
    // continue to participate in dedup — leaking duplicates here on
    // pathological grammars (e.g. ktor's deeply ambiguous Kotlin parse)
    // explodes the speculative cache one step up the recursion.
    let mut inline_keys: [(usize, bool); 8] = [(0, false); 8];
    let mut inline_len = 0_usize;
    let mut overflow: Vec<(usize, bool)> = Vec::new();
    outcomes.retain(|outcome| {
        let key = (outcome.index, outcome.consumed_eof);
        for &existing in &inline_keys[..inline_len] {
            if existing == key {
                return false;
            }
        }
        if !overflow.is_empty() {
            for &existing in &overflow {
                if existing == key {
                    return false;
                }
            }
        }
        if inline_len < inline_keys.len() {
            inline_keys[inline_len] = key;
            inline_len += 1;
        } else {
            overflow.push(key);
        }
        true
    });
}

/// Sorts and removes equivalent endpoints, including their action traces.
fn dedupe_outcomes(outcomes: &mut Vec<RecognizeOutcome>) {
    outcomes.sort_unstable();
    outcomes.dedup();
}

impl<S, H> Recognizer for BaseParser<S, H>
where
    S: TokenSource,
    H: SemanticHooks,
{
    fn data(&self) -> &RecognizerData {
        &self.data
    }

    fn data_mut(&mut self) -> &mut RecognizerData {
        &mut self.data
    }
}

impl<S, H> Parser for BaseParser<S, H>
where
    S: TokenSource,
    H: SemanticHooks,
{
    fn build_parse_trees(&self) -> bool {
        self.build_parse_trees
    }

    fn set_build_parse_trees(&mut self, build: bool) {
        self.build_parse_trees = build;
    }

    fn number_of_syntax_errors(&self) -> usize {
        Self::number_of_syntax_errors(self)
    }

    fn report_diagnostic_errors(&self) -> bool {
        self.report_diagnostic_errors
    }

    fn set_report_diagnostic_errors(&mut self, report: bool) {
        self.report_diagnostic_errors = report;
    }

    fn prediction_mode(&self) -> PredictionMode {
        self.prediction_mode
    }

    fn set_prediction_mode(&mut self, mode: PredictionMode) {
        self.prediction_mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atn::AtnType;
    use crate::atn::IntervalSet;
    use crate::atn::parser::{
        ParserAtnPredictionDiagnostic, ParserAtnPredictionDiagnosticKind, ParserAtnSimulator,
    };
    use crate::atn::serialized::{AtnDeserializer, SerializedAtn};
    use crate::token::{CommonToken, HIDDEN_CHANNEL, Token};
    use crate::token_stream::CommonTokenStream;
    use crate::vocabulary::Vocabulary;

    #[test]
    fn fx_hasher_write_matches_typed_methods_for_full_words() {
        // PR #5 review (Greptile P2): future key types whose `Hash` impl funnels
        // bytes through `Hasher::write` (e.g. `String`, `[u8; 8]`, slice-typed
        // fields) must hash the same as the typed methods, otherwise an
        // `FxHashMap` keyed on such a type silently disagrees with itself
        // depending on which entry point the caller used. Verify the
        // little-endian word equivalence this PR established.
        let value: u64 = 0x0102_0304_0506_0708;
        let mut typed = FxHasher::default();
        typed.write_u64(value);
        let mut bytewise = FxHasher::default();
        bytewise.write(&value.to_le_bytes());
        assert_eq!(typed.finish(), bytewise.finish());
    }

    #[derive(Debug)]
    struct Source {
        tokens: Vec<CommonToken>,
        index: usize,
    }

    impl TokenSource for Source {
        fn next_token(&mut self) -> CommonToken {
            let token = self
                .tokens
                .get(self.index)
                .cloned()
                .unwrap_or_else(|| CommonToken::eof("parser-test", self.index, 1, self.index));
            self.index += 1;
            token
        }

        fn line(&self) -> usize {
            1
        }

        fn column(&self) -> usize {
            self.index
        }

        fn source_name(&self) -> &'static str {
            "parser-test"
        }
    }

    fn mini_parser_data() -> RecognizerData {
        RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        )
        .with_rule_names(["s"])
    }

    fn mini_parser(tokens: Vec<CommonToken>) -> BaseParser<Source> {
        let data = mini_parser_data();
        BaseParser::new(CommonTokenStream::new(Source { tokens, index: 0 }), data)
    }

    fn mini_parser_with_hooks<H>(tokens: Vec<CommonToken>, hooks: H) -> BaseParser<Source, H>
    where
        H: SemanticHooks,
    {
        BaseParser::with_semantic_hooks(
            CommonTokenStream::new(Source { tokens, index: 0 }),
            mini_parser_data(),
            hooks,
        )
    }

    fn token_then_eof_atn() -> Atn {
        AtnDeserializer::new(&SerializedAtn::from_i32(&[
            4, 1, 2, // version, parser, max token type
            3, // states
            2, 0, // rule start
            1, 0, // basic
            7, 0, // rule stop
            0, // non-greedy states
            0, // precedence states
            1, // rules
            0, // rule 0 start
            0, // modes
            0, // sets
            2, // transitions
            0, 1, 5, 1, 0, 0, // match token 1
            1, 2, 5, -1, 0, 0, // match EOF
            0, // decisions
        ]))
        .deserialize()
        .expect("artificial parser ATN should deserialize")
    }

    fn eof_then_action_atn() -> Atn {
        AtnDeserializer::new(&SerializedAtn::from_i32(&[
            4, 1, 1, // version, parser, max token type
            3, // states
            2, 0, // rule start
            1, 0, // basic
            7, 0, // rule stop
            0, // non-greedy states
            0, // precedence states
            1, // rules
            0, // rule 0 start
            0, // modes
            0, // sets
            2, // transitions
            0, 1, 5, -1, 0, 0, // match EOF
            1, 2, 6, 0, 0, 0, // parser action
            0, // decisions
        ]))
        .deserialize()
        .expect("artificial parser ATN should deserialize")
    }

    fn two_alt_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::BlockStart).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(4, AtnStateKind::BlockEnd).with_rule_index(0));
        atn.add_state(AtnState::new(5, AtnStateKind::RuleStop).with_rule_index(0));
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![5]);
        atn.add_decision_state(1);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 2,
                label: 1,
            });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 3,
                label: 2,
            });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Epsilon { target: 4 });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Epsilon { target: 4 });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Epsilon { target: 5 });
        atn
    }

    /// ATN for `start : (A)? B EOF ;` (A=1, B=2, C=3, max token type 3).
    /// State 1 is the nullable optional-block decision; its sync set is {A, B}.
    fn optional_then_b_eof_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 3);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::BlockStart).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(4, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(5, AtnStateKind::RuleStop).with_rule_index(0));
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![5]);
        atn.add_decision_state(1);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        // Optional block: match A then fall through, or skip straight to state 3.
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 3,
                label: 1,
            });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 3 });
        // Match B, then EOF.
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Atom {
                target: 4,
                label: 2,
            });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Atom {
                target: 5,
                label: TOKEN_EOF,
            });
        atn
    }

    #[test]
    fn sync_decision_deletes_only_a_single_token() {
        // ANTLR sync recovery deletes exactly one token, only when LA(2) is
        // expected. `(A)? B EOF` at the optional-block decision:
        //  - `C B`   -> single-token deletion: one error node for the extra `C`.
        //  - `C C B` -> LA(2) is `C` (not expected), so NO deletion; sync returns
        //               without consuming and records the expected set for the
        //               subsequent mismatch (the parser must not over-consume both
        //               `C`s and accept the input).
        let atn = optional_then_b_eof_atn();

        let mut single = mini_parser(vec![
            CommonToken::new(3).with_text("c"),
            CommonToken::new(2).with_text("b"),
            CommonToken::eof("parser-test", 1, 2, 2),
        ]);
        single.rule_context_stack = vec![RuleContextFrame {
            rule_index: 0,
            invoking_state: 0,
        }];
        let children = single
            .sync_decision(&atn, 1, true, false)
            .expect("single extraneous token recovers");
        assert_eq!(children.len(), 1);
        assert!(matches!(children[0], ParseTree::Error(_)));
        assert_eq!(single.number_of_syntax_errors(), 1);
        // Exactly one token consumed (the cursor now sits on `b`).
        assert_eq!(single.la(1), 2);

        let mut double = mini_parser(vec![
            CommonToken::new(3).with_text("c"),
            CommonToken::new(3).with_text("c"),
            CommonToken::new(2).with_text("b"),
            CommonToken::eof("parser-test", 1, 3, 3),
        ]);
        double.rule_context_stack = vec![RuleContextFrame {
            rule_index: 0,
            invoking_state: 0,
        }];
        let result = double.sync_decision(&atn, 1, true, false);
        // No single-token deletion fires (LA(2) is `c`, not expected): sync must NOT
        // consume either `c`. It reports the mismatch at the first `c` (so the parser
        // does not over-consume both and accept the input). Nothing is consumed, so
        // the cursor still sits on the first `c` for rule-level recovery.
        let error = result.expect_err("two extraneous tokens must not be deleted by sync");
        match error {
            AntlrError::ParserError { message, .. } => {
                assert!(message.starts_with("mismatched input"), "got: {message}");
            }
            other => panic!("expected a mismatched-input ParserError, got {other:?}"),
        }
        assert_eq!(double.la(1), 3);
    }

    /// The real serialized ATN that `antlr4-rust-gen` emits for
    /// `grammar T; s : A* EOF; A:'a'; C:'c';` — a `*` loop whose follow set after
    /// the loop is `EOF`. The loop decision is state 5.
    fn star_loop_then_eof_atn() -> Atn {
        AtnDeserializer::new(&SerializedAtn::from_i32(&[
            4, 1, 3, 11, 2, 0, 7, 0, 1, 0, 5, 0, 4, 8, 0, 10, 0, 12, 0, 7, 9, 0, 1, 0, 1, 0, 1, 0,
            0, 0, 1, 0, 0, 0, 10, 0, 5, 1, 0, 0, 0, 2, 4, 5, 1, 0, 0, 3, 2, 1, 0, 0, 0, 4, 7, 1, 0,
            0, 0, 5, 3, 1, 0, 0, 0, 5, 6, 1, 0, 0, 0, 6, 8, 1, 0, 0, 0, 7, 5, 1, 0, 0, 0, 8, 9, 5,
            0, 0, 1, 9, 1, 1, 0, 0, 0, 1, 5,
        ]))
        .deserialize()
        .expect("star-loop-then-EOF ATN should deserialize")
    }

    #[test]
    fn sync_decision_deletes_token_before_eof_at_loop_back() {
        // `s : A* EOF` on `c`: the loop decision (state 5) can recover onto EOF.
        // At the loop ENTRY (loop_back = false) a single unexpected token before
        // EOF is deleted as an error node (then the generated EOF match consumes
        // the real EOF) — matching ANTLR's `(s c <EOF>)` + "extraneous input".
        // EOF must be a valid scan-stop for this to fire.
        let atn = star_loop_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(2).with_text("c"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);
        parser.rule_context_stack = vec![RuleContextFrame {
            rule_index: 0,
            invoking_state: 0,
        }];
        let children = parser
            .sync_decision(&atn, 5, true, false)
            .expect("single token before EOF recovers");
        assert_eq!(children.len(), 1);
        assert!(matches!(children[0], ParseTree::Error(_)));
        assert_eq!(parser.number_of_syntax_errors(), 1);
        assert_eq!(
            parser.la(1),
            TOKEN_EOF,
            "EOF is left for the rule's EOF match"
        );
    }

    #[test]
    fn sync_decision_does_not_delete_two_tokens_before_eof_at_loop_entry() {
        // `s : A* EOF` on `c c`: at the loop ENTRY (loop_back = false) ANTLR does
        // single-token deletion, which fails because LA(2) = `c` is not expected —
        // so it reports `mismatched input` and consumes nothing (ANTLR: `(s c c)`
        // with no EOF). The scan must NOT multi-token-consume both `c`s here.
        let atn = star_loop_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(2).with_text("c"),
            CommonToken::new(2).with_text("c"),
            CommonToken::eof("parser-test", 1, 2, 2),
        ]);
        parser.rule_context_stack = vec![RuleContextFrame {
            rule_index: 0,
            invoking_state: 0,
        }];
        let error = parser
            .sync_decision(&atn, 5, true, false)
            .expect_err("two tokens at the loop entry must not be deleted");
        match error {
            AntlrError::ParserError { message, .. } => {
                assert!(message.starts_with("mismatched input"), "got: {message}");
            }
            other => panic!("expected mismatched-input ParserError, got {other:?}"),
        }
        assert_eq!(
            parser.la(1),
            2,
            "nothing consumed; cursor still on first `c`"
        );
    }

    #[test]
    fn sync_decision_consumes_until_eof_at_loop_back() {
        // Same `s : A* EOF` decision, but at a loop-BACK (loop_back = true, i.e.
        // after ≥1 `A` matched). ANTLR uses multi-token `consumeUntil(recoverSet)`
        // there, so two unexpected tokens before EOF are BOTH deleted and the rule
        // recovers (matching `(s a c c <EOF>)` for input `a c c`). Here we feed the
        // post-`a` state directly: `c c <EOF>` with loop_back = true.
        let atn = star_loop_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(2).with_text("c"),
            CommonToken::new(2).with_text("c"),
            CommonToken::eof("parser-test", 1, 2, 2),
        ]);
        parser.rule_context_stack = vec![RuleContextFrame {
            rule_index: 0,
            invoking_state: 0,
        }];
        let children = parser
            .sync_decision(&atn, 5, false, true)
            .expect("loop-back multi-token deletion recovers onto EOF");
        assert_eq!(children.len(), 2, "both `c`s deleted as error nodes");
        assert!(children.iter().all(|c| matches!(c, ParseTree::Error(_))));
        assert_eq!(parser.number_of_syntax_errors(), 1);
        assert_eq!(parser.la(1), TOKEN_EOF, "EOF left for the rule's EOF match");
    }

    fn predicate_after_token_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(4, AtnStateKind::RuleStop).with_rule_index(0));
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![4]);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Atom {
                target: 1,
                label: 1,
            });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Predicate {
                target: 2,
                rule_index: 0,
                pred_index: 0,
                context_dependent: false,
            });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: 2,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Epsilon { target: 4 });
        atn
    }

    fn nested_nullable_context_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 1);
        for state_number in 0..=20 {
            let kind = match state_number {
                0 | 10 | 16 => AtnStateKind::RuleStart,
                9 | 15 | 20 => AtnStateKind::RuleStop,
                _ => AtnStateKind::Basic,
            };
            let rule_index = match state_number {
                0..=9 => 0,
                10..=15 => 1,
                _ => 2,
            };
            atn.add_state(AtnState::new(state_number, kind).with_rule_index(rule_index));
        }
        atn.set_rule_to_start_state(vec![0, 10, 16]);
        atn.set_rule_to_stop_state(vec![9, 15, 20]);
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Rule {
                target: 10,
                rule_index: 1,
                follow_state: 8,
                precedence: 0,
            });
        atn.state_mut(8)
            .expect("state 8")
            .add_transition(Transition::Atom {
                target: 9,
                label: 1,
            });
        atn.state_mut(8)
            .expect("state 8")
            .add_transition(Transition::Epsilon { target: 9 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Rule {
                target: 16,
                rule_index: 2,
                follow_state: 14,
                precedence: 0,
            });
        atn.state_mut(14)
            .expect("state 14")
            .add_transition(Transition::Epsilon { target: 15 });
        atn
    }

    fn generated_match_recovery_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::RuleStop).with_rule_index(0));
        atn.add_state(AtnState::new(4, AtnStateKind::RuleStart).with_rule_index(1));
        atn.add_state(AtnState::new(5, AtnStateKind::RuleStop).with_rule_index(1));
        atn.set_rule_to_start_state(vec![0, 4]);
        atn.set_rule_to_stop_state(vec![3, 5]);
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Rule {
                target: 4,
                rule_index: 1,
                follow_state: 2,
                precedence: 0,
            });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: TOKEN_EOF,
            });
        atn
    }

    fn complement_set_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 1);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::RuleStop).with_rule_index(0));
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![1]);
        let mut excluded = IntervalSet::new();
        excluded.add(1);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::NotSet {
                target: 1,
                set: excluded,
            });
        atn
    }

    /// ATN for `start : . EOF ;`: a wildcard whose follow state explicitly matches
    /// EOF. State 0 (`RuleStart`) -wildcard-> 2 -EOF-> 1 (`RuleStop`).
    fn wildcard_then_eof_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 1);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::RuleStop).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![1]);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Wildcard { target: 2 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 1,
                label: TOKEN_EOF,
            });
        atn
    }

    #[test]
    fn parser_matches_token_and_reports_mismatch() {
        let source = Source {
            tokens: vec![
                CommonToken::new(1).with_text("x"),
                CommonToken::eof("parser-test", 1, 1, 1),
            ],
            index: 0,
        };
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        );
        let mut parser = BaseParser::new(CommonTokenStream::new(source), data);
        assert_eq!(
            parser.match_token(1).expect("token 1 should match").text(),
            "x"
        );
        assert!(parser.match_token(1).is_err());
    }

    #[test]
    fn parser_matches_token_sets() {
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);

        assert_eq!(
            parser
                .match_set(&[(1, 1), (3, 4)])
                .expect("token set should match")
                .text(),
            "x"
        );
        assert!(parser.match_not_set(&[(1, 1)], 1, 4).is_err());
    }

    #[test]
    fn generated_rule_api_tracks_state_and_precedence() {
        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);

        let context = parser.enter_rule(7, 2);
        assert_eq!(context.rule_index(), 2);
        assert_eq!(parser.state(), 7);
        assert_eq!(
            parser.rule_context_stack,
            vec![RuleContextFrame {
                rule_index: 2,
                invoking_state: 7
            }]
        );

        let recursive = parser.enter_recursion_rule(11, 3, 4);
        assert_eq!(recursive.rule_index(), 3);
        assert!(parser.precpred(4));
        assert!(parser.precpred(5));
        assert!(!parser.precpred(3));

        let next = parser.push_new_recursion_context(13, 3);
        assert_eq!(next.invoking_state(), 13);
        parser.unroll_recursion_context();
        assert_eq!(parser.precedence_stack, vec![0]);
        assert_eq!(
            parser.rule_context_stack,
            vec![RuleContextFrame {
                rule_index: 2,
                invoking_state: 7
            }]
        );

        parser.exit_rule();
        assert!(parser.rule_context_stack.is_empty());
    }

    #[test]
    fn parser_predicates_support_token_adjacency() {
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("=").with_span(0, 0),
            CommonToken::new(1).with_text(">").with_span(1, 1),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);
        parser.consume();
        parser.consume();

        let predicates = [(0, 0, ParserPredicate::TokenPairAdjacent)];

        assert!(parser.parser_semantic_predicate_matches(&predicates, 0, 0));

        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("=").with_span(0, 0),
            CommonToken::new(1)
                .with_text(" ")
                .with_channel(HIDDEN_CHANNEL)
                .with_span(1, 1),
            CommonToken::new(1).with_text(">").with_span(2, 2),
            CommonToken::eof("parser-test", 3, 1, 3),
        ]);
        parser.consume();
        parser.consume();

        assert!(!parser.parser_semantic_predicate_matches(&predicates, 0, 0));
    }

    #[test]
    fn parser_predicates_support_context_child_text_checks() {
        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        let mut context = ParserRuleContext::new(1, 0);
        let mut child_context = ParserRuleContext::new(2, 0);
        child_context.add_child(ParseTree::Terminal(TerminalNode::new(
            CommonToken::new(1).with_text("var"),
        )));
        context.add_child(ParseTree::Rule(RuleNode::new(child_context)));
        let predicates = [(
            1,
            0,
            ParserPredicate::ContextChildRuleTextNotEquals {
                rule_index: 2,
                text: "var",
            },
        )];

        assert!(
            !parser.parser_semantic_predicate_matches_with_context_and_local(
                &predicates,
                1,
                0,
                &context,
                0,
            )
        );
    }

    #[test]
    fn context_expected_symbols_walks_nullable_parent_contexts() {
        let atn = nested_nullable_context_atn();
        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        parser.rule_context_stack = vec![
            RuleContextFrame {
                rule_index: 0,
                invoking_state: 0,
            },
            RuleContextFrame {
                rule_index: 1,
                invoking_state: 1,
            },
            RuleContextFrame {
                rule_index: 2,
                invoking_state: 2,
            },
        ];

        let expected = parser.context_expected_symbols(&atn);

        assert!(expected.contains(&1));
        assert!(expected.contains(&TOKEN_EOF));
    }

    #[test]
    fn prediction_context_reuses_cached_stack_until_rule_stack_changes() {
        let atn = nested_nullable_context_atn();
        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        parser.rule_context_stack = vec![
            RuleContextFrame {
                rule_index: 0,
                invoking_state: 0,
            },
            RuleContextFrame {
                rule_index: 1,
                invoking_state: 1,
            },
            RuleContextFrame {
                rule_index: 2,
                invoking_state: 2,
            },
        ];

        let first = parser.prediction_context(&atn);
        let second = parser.prediction_context(&atn);
        assert!(Rc::ptr_eq(&first, &second));

        parser.exit_rule();
        let after_pop = parser.prediction_context(&atn);
        assert!(!Rc::ptr_eq(&first, &after_pop));
    }

    #[test]
    fn generated_match_token_recovers_missing_token_from_context_follow() {
        let atn = generated_match_recovery_atn();
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new(
                [None, Some("'X'"), Some("'Y'")],
                [None, Some("X"), Some("Y")],
                [None::<&str>, None, None],
            ),
        );
        let mut parser = BaseParser::new(
            CommonTokenStream::new(Source {
                tokens: vec![CommonToken::eof("parser-test", 3, 1, 3)],
                index: 0,
            }),
            data,
        );
        parser.rule_context_stack = vec![
            RuleContextFrame {
                rule_index: 0,
                invoking_state: 0,
            },
            RuleContextFrame {
                rule_index: 1,
                invoking_state: 1,
            },
        ];
        assert_eq!(parser.number_of_syntax_errors(), 0);

        let node = parser
            .match_token_recovering(2, 5, &atn)
            .expect("generated match should insert missing token");

        assert_eq!(node.children().len(), 1);
        assert_eq!(node.children()[0].text(), "<missing 'Y'>");
        // Single-token insertion synthesizes a missing token and consumes nothing,
        // so no EOF terminal is consumed even though lookahead is EOF.
        assert!(!node.consumed_eof());
        assert_eq!(parser.la(1), TOKEN_EOF);
        assert_eq!(parser.number_of_syntax_errors(), 1);
        assert_eq!(
            parser.generated_parser_diagnostics,
            [ParserDiagnostic {
                line: 1,
                column: 3,
                message: "missing 'Y' at '<EOF>'".to_owned(),
            }]
        );
    }

    #[test]
    fn generated_match_token_counts_single_token_deletion_recovery() {
        let atn = generated_match_recovery_atn();
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new(
                [None, Some("'X'"), Some("'Y'"), Some("'Z'")],
                [None, Some("X"), Some("Y"), Some("Z")],
                [None::<&str>, None, None, None],
            ),
        );
        let mut parser = BaseParser::new(
            CommonTokenStream::new(Source {
                tokens: vec![
                    CommonToken::new(3).with_text("z"),
                    CommonToken::new(2).with_text("y"),
                    CommonToken::eof("parser-test", 3, 1, 3),
                ],
                index: 0,
            }),
            data,
        );

        let node = parser
            .match_token_recovering(2, 5, &atn)
            .expect("generated match should delete the extraneous token");

        assert_eq!(node.children().len(), 2);
        assert!(matches!(node.children()[0], ParseTree::Error(_)));
        assert_eq!(node.children()[0].text(), "z");
        assert_eq!(node.children()[1].text(), "y");
        assert_eq!(parser.number_of_syntax_errors(), 1);
    }

    #[test]
    fn generated_diagnostic_restore_rolls_back_syntax_error_count() {
        let atn = generated_match_recovery_atn();
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new(
                [None, Some("'X'"), Some("'Y'")],
                [None, Some("X"), Some("Y")],
                [None::<&str>, None, None],
            ),
        );
        let mut parser = BaseParser::new(
            CommonTokenStream::new(Source {
                tokens: vec![CommonToken::eof("parser-test", 3, 1, 3)],
                index: 0,
            }),
            data,
        );
        parser.rule_context_stack = vec![
            RuleContextFrame {
                rule_index: 0,
                invoking_state: 0,
            },
            RuleContextFrame {
                rule_index: 1,
                invoking_state: 1,
            },
        ];
        let marker = parser.generated_diagnostics_checkpoint();

        let _ = parser
            .match_token_recovering(2, 5, &atn)
            .expect("generated match should insert missing token");
        assert_eq!(parser.number_of_syntax_errors(), 1);

        parser.restore_generated_diagnostics(marker);

        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert!(parser.generated_parser_diagnostics.is_empty());
    }

    #[test]
    fn generated_prediction_diagnostics_use_adaptive_context() {
        let atn = two_alt_decision_atn();
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new(
                [None, Some("'x'"), Some("'y'")],
                [None, Some("X"), Some("Y")],
                [None::<&str>, None, None],
            ),
        )
        .with_rule_names(["s"]);
        let mut parser = BaseParser::new(
            CommonTokenStream::new(Source {
                tokens: vec![
                    CommonToken::new(1)
                        .with_text("x")
                        .with_position(1, 0)
                        .with_span(0, 0),
                    CommonToken::new(2)
                        .with_text("y")
                        .with_position(1, 2)
                        .with_span(1, 1),
                    CommonToken::eof("parser-test", 2, 1, 3),
                ],
                index: 0,
            }),
            data,
        );
        parser.set_report_diagnostic_errors(true);

        parser.record_generated_prediction_diagnostic(
            &atn,
            1,
            &ParserAtnPrediction {
                alt: 1,
                requires_full_context: true,
                has_semantic_context: false,
                diagnostic: Some(ParserAtnPredictionDiagnostic {
                    kind: ParserAtnPredictionDiagnosticKind::ContextSensitivity,
                    start_index: 0,
                    sll_stop_index: 1,
                    ll_stop_index: 0,
                    conflicting_alts: vec![1, 2],
                }),
            },
        );
        parser.record_generated_prediction_diagnostic(
            &atn,
            1,
            &ParserAtnPrediction {
                alt: 1,
                requires_full_context: true,
                has_semantic_context: false,
                diagnostic: Some(ParserAtnPredictionDiagnostic {
                    kind: ParserAtnPredictionDiagnosticKind::Ambiguity,
                    start_index: 0,
                    sll_stop_index: 1,
                    ll_stop_index: 1,
                    conflicting_alts: vec![1, 2],
                }),
            },
        );

        assert_eq!(
            parser.generated_parser_diagnostics,
            [
                ParserDiagnostic {
                    line: 1,
                    column: 2,
                    message: "reportAttemptingFullContext d=0 (s), input='xy'".to_owned(),
                },
                ParserDiagnostic {
                    line: 1,
                    column: 0,
                    message: "reportContextSensitivity d=0 (s), input='x'".to_owned(),
                },
                ParserDiagnostic {
                    line: 1,
                    column: 2,
                    message: "reportAttemptingFullContext d=0 (s), input='xy'".to_owned(),
                },
                ParserDiagnostic {
                    line: 1,
                    column: 2,
                    message: "reportAmbiguity d=0 (s): ambigAlts={1, 2}, input='xy'".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn generated_match_not_set_recovers_empty_complement_at_eof() {
        let atn = complement_set_atn();
        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        parser.rule_context_stack = vec![RuleContextFrame {
            rule_index: 0,
            invoking_state: 0,
        }];

        let node = parser
            .match_not_set_recovering(&[(1, 1)], 1, 1, 1, &atn)
            .expect("empty complement should recover at EOF");

        assert_eq!(node.children().len(), 1);
        // Recovery synthesizes a missing token without consuming EOF, so the
        // enclosing rule must not record EOF as its stop token.
        assert!(!node.consumed_eof());
        assert_eq!(parser.la(1), TOKEN_EOF);
        assert_eq!(
            parser.generated_parser_diagnostics,
            [ParserDiagnostic {
                line: 1,
                column: 1,
                message: "missing {} at '<EOF>'".to_owned(),
            }]
        );
    }

    #[test]
    fn wildcard_recovers_via_insertion_when_follow_expects_eof_at_eof() {
        // `start : . EOF ;` on empty input. The wildcard is modeled as an
        // empty-complement not-set; at EOF the follow state (the explicit EOF
        // match) expects EOF, so even in the start rule recovery must perform
        // single-token insertion (`<missing ...>`) rather than aborting — matching
        // ANTLR's `(start <missing ...> <EOF>)` / "missing ... at '<EOF>'".
        let atn = wildcard_then_eof_atn();
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        );
        let mut parser = BaseParser::new(
            CommonTokenStream::new(Source {
                tokens: vec![CommonToken::eof("parser-test", 1, 1, 1)],
                index: 0,
            }),
            data,
        );
        parser.rule_context_stack = vec![RuleContextFrame {
            rule_index: 0,
            invoking_state: 0,
        }];

        let node = parser
            .match_not_set_recovering(&[], 1, atn.max_token_type(), 2, &atn)
            .expect("wildcard at EOF should recover by insertion when follow expects EOF");

        // A single `<missing ...>` error node is inserted; EOF is not consumed.
        assert_eq!(node.children().len(), 1);
        assert!(!node.consumed_eof());
        assert!(node.children()[0].text().starts_with("<missing"));
        assert_eq!(parser.la(1), TOKEN_EOF);
        assert_eq!(
            parser.generated_parser_diagnostics,
            [ParserDiagnostic {
                line: 1,
                column: 1,
                message: "missing 'x' at '<EOF>'".to_owned(),
            }]
        );
    }

    #[test]
    fn generated_rule_recovery_consumes_to_parent_follow() {
        let atn = generated_match_recovery_atn();
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new(
                [None, Some("'X'"), Some("'Y'"), Some("'Z'")],
                [None, Some("X"), Some("Y"), Some("Z")],
                [None::<&str>, None, None, None],
            ),
        );
        let mut parser = BaseParser::new(
            CommonTokenStream::new(Source {
                tokens: vec![
                    CommonToken::new(3).with_text("z"),
                    CommonToken::eof("parser-test", 1, 1, 1),
                ],
                index: 0,
            }),
            data,
        );
        let _parent = parser.enter_rule(0, 0);
        let marker = parser.push_invoking_state(1);
        let mut child = parser.enter_rule(4, 1);
        parser.discard_invoking_state(marker);

        parser.recover_generated_rule(
            &mut child,
            &atn,
            AntlrError::ParserError {
                line: 1,
                column: 0,
                message: "mismatched input 'z' expecting {'X', 'Y'}".to_owned(),
            },
        );
        let tree = parser.finish_rule(child, false);

        assert_eq!(parser.la(1), TOKEN_EOF);
        assert_eq!(tree.to_string_tree(&["s", "a"]), "(a z)");
        assert_eq!(parser.number_of_syntax_errors(), 1);
        assert_eq!(
            parser.generated_parser_diagnostics,
            [ParserDiagnostic {
                line: 1,
                column: 0,
                message: "mismatched input 'z' expecting {'X', 'Y'}".to_owned(),
            }]
        );
        parser.exit_rule();
    }

    #[test]
    fn greedy_ll1_alt_handles_nullable_loop_exit() {
        let mut body_symbols = TokenBitSet::default();
        body_symbols.insert(1);
        let entry = DecisionLookahead {
            transitions: vec![
                TransitionLookSet {
                    symbols: body_symbols,
                    nullable: false,
                },
                TransitionLookSet {
                    symbols: TokenBitSet::default(),
                    nullable: true,
                },
            ],
        };

        assert_eq!(ll1_unique_alt(&entry, 2), None);
        assert_eq!(ll1_greedy_alt(&entry, 2, false), Some(1));
        assert_eq!(ll1_greedy_alt(&entry, 1, false), None);
        assert_eq!(ll1_greedy_alt(&entry, 1, true), None);
    }

    #[test]
    fn single_outcome_memo_probe_selects_sparse_or_promote_mode() {
        let key = |state_number| FastRecognizeKey {
            state_number,
            stop_state: 10,
            index: state_number,
            rule_start_index: 0,
            decision_start_index: None,
            precedence: 0,
            recovery_symbols_id: 0,
            recovery_state: None,
        };

        let mut sparse = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        for state_number in 0..(CLEAN_SINGLE_OUTCOME_MEMO_PROBE_LIMIT - 1) {
            assert!(sparse.should_memoize_single_outcome(&key(state_number)));
        }
        assert!(!sparse.should_memoize_single_outcome(&key(CLEAN_SINGLE_OUTCOME_MEMO_PROBE_LIMIT)));
        assert_eq!(
            sparse.single_outcome_memo_mode,
            SingleOutcomeMemoMode::Sparse
        );

        let mut promote = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        let repeated = key(1);
        for _ in 0..=CLEAN_SINGLE_OUTCOME_MEMO_REPEAT_LIMIT {
            assert!(promote.should_memoize_single_outcome(&repeated));
        }
        assert_eq!(
            promote.single_outcome_memo_mode,
            SingleOutcomeMemoMode::Promote
        );
    }

    #[test]
    fn clean_empty_multi_alt_outcomes_are_memoized() {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::BlockStart).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::RuleStop).with_rule_index(0));
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![2]);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 2,
                label: 1,
            });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 2,
                label: 2,
            });

        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 0, 1, 0)]);
        parser.fast_recovery_enabled = false;
        let mut visiting = FxHashSet::default();
        let mut memo = FxHashMap::default();
        let mut expected = ExpectedTokens::default();
        let outcomes = parser.recognize_state_fast(
            &atn,
            FastRecognizeRequest {
                state_number: 1,
                stop_state: 2,
                index: 0,
                rule_start_index: 0,
                decision_start_index: None,
                precedence: 0,
                depth: 0,
                recovery_symbols: parser.empty_recovery_symbols(),
                recovery_state: None,
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );

        assert!(outcomes.is_empty());
        assert_eq!(memo.len(), 1);
        assert!(memo.values().next().expect("memo entry").is_empty());
    }

    #[test]
    fn wildcard_matches_non_eof_only() {
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);
        assert_eq!(parser.match_wildcard().expect("wildcard").text(), "x");
        assert!(parser.match_wildcard().is_err());
    }

    #[test]
    fn add_parse_child_records_match_even_without_tree_building() {
        // `sync_decision`'s "is the current context empty" flag must reflect real
        // matches, not parse-tree children: when `build_parse_trees(false)`,
        // `children` stays empty but `has_matched_child` must still flip so nested
        // recovery does not wrongly suppress single-token deletion.
        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        let token = CommonToken::new(1).with_text("x");

        parser.set_build_parse_trees(false);
        let mut ctx = ParserRuleContext::new(0, 0);
        assert!(!ctx.has_matched_child());
        parser.add_parse_child(
            &mut ctx,
            ParseTree::Terminal(TerminalNode::new(token.clone())),
        );
        // Tree building is off, so no child is stored...
        assert!(ctx.children().is_empty());
        // ...but the match is recorded, so the context is no longer "empty".
        assert!(ctx.has_matched_child());

        // With tree building on, the child is stored and the match is recorded.
        parser.set_build_parse_trees(true);
        let mut ctx = ParserRuleContext::new(0, 0);
        parser.add_parse_child(&mut ctx, ParseTree::Terminal(TerminalNode::new(token)));
        assert_eq!(ctx.children().len(), 1);
        assert!(ctx.has_matched_child());
    }

    #[test]
    fn parser_interprets_simple_atn_rule() {
        let atn = token_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);

        let tree = parser
            .parse_atn_rule(&atn, 0)
            .expect("artificial parser rule should parse");
        assert_eq!(tree.text(), "x<EOF>");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert_eq!(
            tree.first_rule_stop(0)
                .expect("rule should stop at EOF")
                .token_type(),
            TOKEN_EOF
        );

        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);
        let (tree, actions) = parser
            .parse_atn_rule_with_runtime_options(&atn, 0, ParserRuntimeOptions::default())
            .expect("runtime-option parser rule should parse");
        assert!(actions.is_empty());
        assert_eq!(
            tree.first_rule_stop(0)
                .expect("rule should stop at EOF")
                .token_type(),
            TOKEN_EOF
        );
    }

    #[test]
    fn parser_exposes_buffered_token_stream_after_parse() {
        let atn = token_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);

        let tree = parser
            .parse_atn_rule(&atn, 0)
            .expect("artificial parser rule should parse");
        assert_eq!(tree.text(), "x<EOF>");

        let stream = parser.token_stream();
        let source_index_after_parse = stream.token_source().index;
        let buffered = stream.tokens();
        assert_eq!(buffered.len(), 2);
        assert_eq!(buffered[0].text(), Some("x"));
        assert_eq!(buffered[0].token_index(), 0);
        assert_eq!(buffered[1].token_type(), TOKEN_EOF);
        assert_eq!(stream.token_source().index, source_index_after_parse);

        let stream = parser.into_token_stream();
        assert_eq!(stream.token_source().index, source_index_after_parse);
        assert_eq!(stream.tokens()[0].text(), Some("x"));
        assert_eq!(stream.tokens()[1].token_type(), TOKEN_EOF);
    }

    #[test]
    fn parser_syntax_error_count_tracks_interpreted_recovery() {
        let atn = token_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);

        let tree = parser
            .parse_atn_rule(&atn, 0)
            .expect("invalid token should recover into an error node");

        assert_eq!(parser.number_of_syntax_errors(), 1);
        assert_eq!(
            tree.first_error_token()
                .expect("recovery should embed an error token")
                .text(),
            Some("y")
        );
    }

    #[test]
    fn parser_syntax_error_count_tracks_failed_interpreted_parse() {
        let atn = token_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);

        let error = parser
            .parse_atn_rule(&atn, 0)
            .expect_err("start-rule mismatch should remain a parser error");

        assert_eq!(parser.number_of_syntax_errors(), 1);
        assert!(matches!(error, AntlrError::ParserError { .. }));
    }

    #[test]
    fn adaptive_direct_rule_uses_simulator_decision() {
        let atn = two_alt_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut parser = mini_parser(vec![
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);

        let tree = parser
            .parse_atn_rule_adaptive_or_fallback(&atn, &mut simulator, 0)
            .expect("direct adaptive rule should parse");

        assert_eq!(tree.text(), "y");
        assert_eq!(parser.input.index(), 1);
    }

    #[test]
    fn adaptive_direct_rule_restores_input_on_fallback() {
        let atn = predicate_after_token_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);

        let tree = parser
            .parse_atn_rule_adaptive_or_fallback(&atn, &mut simulator, 0)
            .expect("fallback recognizer should parse");

        assert_eq!(tree.text(), "xy");
        assert_eq!(parser.input.index(), 2);
    }

    #[test]
    fn unknown_predicate_policy_defaults_to_assume_true() {
        let atn = predicate_after_token_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);

        let (tree, _) = parser
            .parse_atn_rule_with_runtime_options(&atn, 0, ParserRuntimeOptions::default())
            .expect("unknown predicate should pass under the default policy");

        assert_eq!(tree.text(), "xy");
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }

    #[test]
    fn unknown_predicate_policy_assume_false_kills_the_guarded_path() {
        let atn = predicate_after_token_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);

        let result = parser.parse_atn_rule_with_runtime_options(
            &atn,
            0,
            ParserRuntimeOptions {
                unknown_predicate_policy: UnknownSemanticPolicy::AssumeFalse,
                ..ParserRuntimeOptions::default()
            },
        );

        assert!(
            result.is_err(),
            "the only path is predicate-guarded, so assume-false must fail the parse"
        );
    }

    #[test]
    fn unknown_predicate_policy_error_names_the_coordinate() {
        let atn = predicate_after_token_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);

        let error = parser
            .parse_atn_rule_with_runtime_options(
                &atn,
                0,
                ParserRuntimeOptions {
                    unknown_predicate_policy: UnknownSemanticPolicy::Error,
                    ..ParserRuntimeOptions::default()
                },
            )
            .expect_err("evaluating an unknown predicate under Error policy must fail");

        let AntlrError::Unsupported(message) = error else {
            panic!("expected AntlrError::Unsupported, got {error:?}");
        };
        assert!(
            message.contains("unsupported semantic predicate"),
            "message should name the failure class: {message}"
        );
        assert!(
            message.contains("pred_index=0"),
            "message should carry the coordinate: {message}"
        );
    }

    #[derive(Debug, Default)]
    struct RecordingHooks {
        predicates: Vec<(usize, usize, usize, Option<String>)>,
        actions: Vec<(usize, String, Option<String>)>,
    }

    impl SemanticHooks for RecordingHooks {
        fn sempred<S>(
            &mut self,
            ctx: &mut ParserSemCtx<'_, S>,
            rule_index: usize,
            pred_index: usize,
        ) -> Option<bool>
        where
            S: TokenSource,
        {
            self.predicates.push((
                ctx.input_index(),
                rule_index,
                pred_index,
                ctx.token_text(1).map(str::to_owned),
            ));
            Some(true)
        }

        fn action<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>, action: ParserAction) -> bool
        where
            S: TokenSource,
        {
            self.actions.push((
                action.source_state(),
                ctx.action_text(),
                ctx.rule_name().map(str::to_owned),
            ));
            true
        }
    }

    #[test]
    fn semantic_hook_handles_unknown_predicate_before_error_policy() {
        let atn = predicate_after_token_atn();
        let mut parser = mini_parser_with_hooks(
            vec![
                CommonToken::new(1).with_text("x"),
                CommonToken::new(2).with_text("y"),
                CommonToken::eof("parser-test", 2, 1, 2),
            ],
            RecordingHooks::default(),
        );

        let (tree, _) = parser
            .parse_atn_rule_with_runtime_options(
                &atn,
                0,
                ParserRuntimeOptions {
                    unknown_predicate_policy: UnknownSemanticPolicy::Error,
                    ..ParserRuntimeOptions::default()
                },
            )
            .expect("hook supplies the missing predicate result");

        assert_eq!(tree.text(), "xy");
        assert_eq!(
            parser.semantic_hooks.predicates,
            vec![(1, 0, 0, Some("y".to_owned()))]
        );
    }

    #[test]
    fn semantic_hook_handles_committed_parser_action() {
        let atn = token_then_eof_atn();
        let mut parser = mini_parser_with_hooks(
            vec![
                CommonToken::new(1).with_text("x"),
                CommonToken::eof("parser-test", 1, 1, 1),
            ],
            RecordingHooks::default(),
        );
        let (tree, _) = parser
            .parse_atn_rule_with_runtime_options(&atn, 0, ParserRuntimeOptions::default())
            .expect("rule parses before action hook is tested");

        assert!(parser.parser_action_hook(ParserAction::new(42, 0, 0, Some(0)), &tree));
        assert_eq!(
            parser.semantic_hooks.actions,
            vec![(42, "x".to_owned(), Some("s".to_owned()))]
        );
    }

    #[test]
    fn translated_predicate_is_unaffected_by_error_policy() {
        let atn = predicate_after_token_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::new(2).with_text("y"),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);

        let (tree, _) = parser
            .parse_atn_rule_with_runtime_options(
                &atn,
                0,
                ParserRuntimeOptions {
                    predicates: &[(0, 0, ParserPredicate::True)],
                    unknown_predicate_policy: UnknownSemanticPolicy::Error,
                    ..ParserRuntimeOptions::default()
                },
            )
            .expect("a predicate covered by the table is not an unknown coordinate");

        assert_eq!(tree.text(), "xy");
    }

    #[test]
    fn parser_rule_start_skips_leading_hidden_tokens() {
        let atn = token_then_eof_atn();
        let mut parser = mini_parser(vec![
            CommonToken::new(99)
                .with_text(" ")
                .with_channel(HIDDEN_CHANNEL),
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 2, 1, 2),
        ]);

        let tree = parser
            .parse_atn_rule(&atn, 0)
            .expect("artificial parser rule should parse");
        let Some(ParseTree::Rule(rule)) = tree.first_rule(0) else {
            panic!("rule node should be present");
        };
        assert_eq!(
            rule.context()
                .start()
                .expect("rule should have a start token")
                .token_type(),
            1
        );
    }

    #[test]
    fn parser_action_after_eof_stops_at_eof_token() {
        let atn = eof_then_action_atn();
        let mut parser = mini_parser(vec![CommonToken::eof("parser-test", 0, 1, 0)]);

        let (_, actions) = parser
            .parse_atn_rule_with_runtime_options(&atn, 0, ParserRuntimeOptions::default())
            .expect("EOF action rule should parse");

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].stop_index(), Some(0));
        assert_eq!(
            parser.text_interval(actions[0].start_index(), actions[0].stop_index()),
            ""
        );
    }

    #[test]
    fn after_action_stop_uses_rule_context_stop_not_cursor() {
        // A rule that ends right before EOF without matching it (e.g. `a: ID;`
        // called from `start: a EOF;`): after matching ID the cursor parks on EOF,
        // but the rule did not consume it. The @after stop must follow the rule
        // context's recorded stop (ID at index 0), not the cursor's EOF (index 1).
        let mut id = CommonToken::new(1).with_text("x");
        id.set_token_index(0);
        let mut eof = CommonToken::eof("parser-test", 1, 1, 1);
        eof.set_token_index(1);
        let mut parser = mini_parser(vec![id.clone(), eof]);
        // Advance the cursor onto EOF, as it would be after `a` matched ID.
        parser.consume();
        assert_eq!(parser.la(1), TOKEN_EOF);

        // Rule `a` matched only ID, so its context stop is the ID token (index 0),
        // exactly what finish_rule(consumed_eof = false) records.
        let mut ctx = ParserRuleContext::new(0, 0);
        ctx.set_stop(id);
        let tree = ParseTree::Rule(RuleNode::new(ctx));

        let current_index = parser.input.index();
        // Cursor-only inference would wrongly pick EOF (the parked cursor)...
        assert_eq!(parser.after_action_stop_index(current_index), Some(1));
        // ...but the tree-aware helper follows the rule context stop (ID).
        assert_eq!(
            parser.after_action_stop_index_for_tree(&tree, current_index),
            Some(0)
        );
    }

    #[test]
    fn after_action_start_uses_rule_context_start_not_cursor() {
        // A rule that begins after leading hidden-channel tokens: the rule context
        // start (set by `enter_rule`) is the first visible token, not the raw cursor
        // that may still point at the hidden prefix. The @after start must follow
        // the context start so `$start`/`$text` excludes the hidden prefix.
        let parser = mini_parser(vec![CommonToken::eof("parser-test", 1, 1, 1)]);
        let mut id = CommonToken::new(1).with_text("x");
        // The first visible token sits at stream index 2 (after two hidden tokens).
        id.set_token_index(2);

        let mut ctx = ParserRuleContext::new(0, 0);
        ctx.set_start(id);
        let tree = ParseTree::Rule(RuleNode::new(ctx));

        // The raw fallback (pre-rule cursor) would be 0 (the hidden prefix)...
        // ...but the tree-aware helper follows the rule context start (index 2).
        assert_eq!(parser.after_action_start_index_for_tree(&tree, 0), 2);

        // With no rule start recorded, it falls back to the provided index.
        let empty = ParseTree::Rule(RuleNode::new(ParserRuleContext::new(0, 0)));
        assert_eq!(parser.after_action_start_index_for_tree(&empty, 7), 7);
    }

    #[test]
    fn fast_outcome_selection_respects_sll_tie_order() {
        let first = FastRecognizeOutcome {
            index: 1,
            consumed_eof: false,
            diagnostics: FastDiagnostics::from_vec(vec![ParserDiagnostic {
                line: 1,
                column: 0,
                message: "mismatched input 'x'".to_owned(),
            }]),
            nodes: NodeList::new(),
        };
        let second = FastRecognizeOutcome {
            index: first.index,
            consumed_eof: first.consumed_eof,
            diagnostics: FastDiagnostics::new(),
            nodes: NodeList::new(),
        };

        let selected = select_best_fast_outcome(
            [first.clone(), second.clone()].into_iter(),
            PredictionMode::Sll,
            None,
            |_| panic!("caller-follow token probe should not run"),
        )
        .expect("one outcome should be selected");
        assert_eq!(selected.diagnostics.len(), 1);
        let eof_second = FastRecognizeOutcome {
            index: second.index,
            consumed_eof: true,
            diagnostics: FastDiagnostics::new(),
            nodes: NodeList::new(),
        };
        let selected = select_best_fast_outcome(
            [first.clone(), eof_second].into_iter(),
            PredictionMode::Sll,
            None,
            |_| panic!("caller-follow token probe should not run"),
        )
        .expect("one outcome should be selected");
        assert!(!selected.consumed_eof);
        let selected = select_best_fast_outcome(
            [first, second].into_iter(),
            PredictionMode::Ll,
            None,
            |_| panic!("caller-follow token probe should not run"),
        )
        .expect("one outcome should be selected");
        assert!(selected.diagnostics.is_empty());
    }

    #[test]
    fn fast_outcome_selection_prefers_generated_caller_follow() {
        let earlier = FastRecognizeOutcome {
            index: 7,
            consumed_eof: false,
            diagnostics: FastDiagnostics::new(),
            nodes: NodeList::new(),
        };
        let later = FastRecognizeOutcome {
            index: 8,
            consumed_eof: false,
            diagnostics: FastDiagnostics::new(),
            nodes: NodeList::new(),
        };
        let mut follow = TokenBitSet::default();
        follow.insert(5);

        let selected = select_best_fast_outcome(
            [later.clone(), earlier.clone()].into_iter(),
            PredictionMode::Ll,
            Some(&follow),
            |index| (if index == 7 { 5 } else { TOKEN_EOF }, index == 7, true),
        )
        .expect("one outcome should be selected");
        assert_eq!(selected.index, 7);

        let selected = select_best_fast_outcome(
            [later.clone(), earlier.clone()].into_iter(),
            PredictionMode::Ll,
            Some(&follow),
            |index| (if index == 7 { 5 } else { TOKEN_EOF }, false, true),
        )
        .expect("one outcome should be selected");
        assert_eq!(selected.index, 8);

        let indented_next_statement = FastRecognizeOutcome {
            index: 9,
            consumed_eof: false,
            diagnostics: FastDiagnostics::new(),
            nodes: NodeList::new(),
        };
        let selected = select_best_fast_outcome(
            [indented_next_statement, earlier.clone()].into_iter(),
            PredictionMode::Ll,
            Some(&follow),
            |index| {
                let is_boundary = index == 7;
                let is_boundary_gap = matches!(index, 7 | 8);
                (
                    if index == 7 { 5 } else { TOKEN_EOF },
                    is_boundary,
                    is_boundary_gap,
                )
            },
        )
        .expect("one outcome should be selected");
        assert_eq!(selected.index, 7);

        let continuation = FastRecognizeOutcome {
            index: 10,
            consumed_eof: false,
            diagnostics: FastDiagnostics::new(),
            nodes: NodeList::new(),
        };
        let selected = select_best_fast_outcome(
            [continuation, earlier.clone()].into_iter(),
            PredictionMode::Ll,
            Some(&follow),
            |index| {
                let is_boundary = matches!(index, 7 | 9);
                (
                    if index == 7 { 5 } else { TOKEN_EOF },
                    is_boundary,
                    is_boundary,
                )
            },
        )
        .expect("one outcome should be selected");
        assert_eq!(selected.index, 10);

        let selected = select_best_fast_outcome(
            [earlier, later].into_iter(),
            PredictionMode::Sll,
            Some(&follow),
            |_| panic!("caller-follow token probe should not run in SLL mode"),
        )
        .expect("one outcome should be selected");
        assert_eq!(selected.index, 8);
    }

    #[test]
    fn caller_follow_boundary_text_requires_separator_shape() {
        assert!(is_caller_follow_boundary_text(";"));
        assert!(is_caller_follow_boundary_text("\n"));
        assert!(is_caller_follow_boundary_text("\r\n  "));
        assert!(is_caller_follow_boundary_text(";\n"));
        assert!(!is_caller_follow_boundary_text("\"\"\"line1\nline2\"\"\""));
        assert!(!is_caller_follow_boundary_text("/* line1\nline2 */"));
        assert!(!is_caller_follow_boundary_text("identifier"));
        assert!(is_caller_follow_boundary_gap_text(" \t "));
        assert!(is_caller_follow_boundary_gap_text("\n  "));
        assert!(is_caller_follow_boundary_gap_text(";\t"));
        assert!(!is_caller_follow_boundary_gap_text(
            "\"\"\"line1\nline2\"\"\""
        ));
        assert!(!is_caller_follow_boundary_gap_text("/* line1\nline2 */"));
    }

    #[test]
    fn caller_follow_token_info_treats_hidden_tokens_as_boundary_gaps() {
        let mut parser = mini_parser(vec![
            CommonToken::new(5).with_text("\n"),
            CommonToken::new(6)
                .with_text("// comment\n")
                .with_channel(HIDDEN_CHANNEL),
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 1, 2, 0),
        ]);

        assert_eq!(parser.caller_follow_token_info(0), (5, true, true));
        assert_eq!(parser.caller_follow_token_info(1), (6, false, true));
        assert_eq!(parser.caller_follow_token_info(2), (1, false, false));
    }

    #[test]
    fn caller_follow_token_info_uses_stream_visible_channel() {
        let source = Source {
            tokens: vec![
                CommonToken::new(5).with_text("\n").with_channel(2),
                CommonToken::new(1).with_text("x").with_channel(2),
                CommonToken::new(6)
                    .with_text("// comment\n")
                    .with_channel(HIDDEN_CHANNEL),
                CommonToken::eof("parser-test", 1, 2, 0),
            ],
            index: 0,
        };
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        );
        let mut parser = BaseParser::new(CommonTokenStream::with_channel(source, 2), data);

        assert_eq!(parser.caller_follow_token_info(0), (5, true, true));
        assert_eq!(parser.caller_follow_token_info(1), (1, false, false));
        assert_eq!(parser.caller_follow_token_info(2), (6, false, true));
    }

    #[test]
    fn reset_per_parse_caches_clears_state_expected_token_cache() {
        let atn = token_then_eof_atn();
        let mut parser = mini_parser(Vec::new());

        let _ = parser.cached_state_expected_token_set(&atn, 0);
        assert!(!parser.state_expected_token_cache.is_empty());

        parser.reset_per_parse_caches();
        assert!(parser.state_expected_token_cache.is_empty());
    }

    #[test]
    fn parser_error_with_empty_expected_set_omits_empty_set_display() {
        let source = Source {
            tokens: vec![
                CommonToken::new(1).with_text("x"),
                CommonToken::eof("parser-test", 1, 1, 1),
            ],
            index: 0,
        };
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        );
        let mut parser = BaseParser::new(CommonTokenStream::new(source), data);
        let expected = ExpectedTokens {
            index: Some(0),
            symbols: BTreeSet::new(),
            no_viable: None,
        };

        let (_, message) = parser.expected_error_message(0, 0, &expected);

        assert_eq!(message, "mismatched input 'x'");
    }

    #[test]
    fn eof_rule_stop_index_points_at_eof_token() {
        let source = Source {
            tokens: vec![
                CommonToken::new(1).with_text("x"),
                CommonToken::eof("parser-test", 1, 1, 1),
            ],
            index: 0,
        };
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        );
        let mut parser = BaseParser::new(CommonTokenStream::new(source), data);

        assert_eq!(parser.rule_stop_token_index(1, true), Some(1));
        assert_eq!(parser.rule_stop_token_index(1, false), Some(0));
    }

    #[test]
    fn generated_parser_action_uses_current_rule_stop_boundary() {
        let mut parser = mini_parser(vec![
            CommonToken::new(1).with_text("x"),
            CommonToken::eof("parser-test", 1, 1, 1),
        ]);

        parser.match_token(1).expect("token should match");
        let action = parser.parser_action_at_current(7, 0, 0, false);
        assert_eq!(action.source_state(), 7);
        assert_eq!(action.rule_index(), 0);
        assert_eq!(action.start_index(), 0);
        assert_eq!(action.stop_index(), Some(0));

        parser.match_eof().expect("EOF should match");
        let action = parser.parser_action_at_current(8, 0, 0, true);
        assert_eq!(action.stop_index(), Some(1));
    }

    #[test]
    fn folds_left_recursive_boundary_into_rule_node() {
        let nodes = fold_left_recursive_boundaries(vec![
            RecognizedNode::Token { index: 0 },
            RecognizedNode::LeftRecursiveBoundary { rule_index: 1 },
            RecognizedNode::Token { index: 1 },
        ]);

        assert_eq!(
            nodes,
            vec![
                RecognizedNode::Rule {
                    rule_index: 1,
                    invoking_state: -1,
                    alt_number: 0,
                    start_index: 0,
                    stop_index: Some(0),
                    return_values: BTreeMap::new(),
                    children: vec![RecognizedNode::Token { index: 0 }],
                },
                RecognizedNode::Token { index: 1 },
            ]
        );
    }

    #[test]
    fn outcome_ties_keep_later_non_recursive_alternative() {
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second = RecognizeOutcome {
            actions: vec![ParserAction::new(2, 0, 0, None)],
            ..first.clone()
        };

        let selected = select_best_outcome([first, second].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].source_state(), 2);
    }

    #[test]
    fn outcome_ties_prefer_more_actions_for_non_recursive_paths() {
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second = RecognizeOutcome {
            actions: vec![
                ParserAction::new(2, 0, 0, None),
                ParserAction::new(3, 0, 0, None),
            ],
            ..first.clone()
        };

        let selected = select_best_outcome([second, first].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions.len(), 2);
    }

    #[test]
    fn outcome_ties_prefer_later_action_stop_for_greedy_optional_paths() {
        let first = RecognizeOutcome {
            index: 7,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: vec![1, 0],
            actions: vec![
                ParserAction::new(23, 2, 2, Some(4)),
                ParserAction::new(23, 2, 0, Some(6)),
            ],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second = RecognizeOutcome {
            decisions: vec![0, 1],
            actions: vec![
                ParserAction::new(23, 2, 2, Some(6)),
                ParserAction::new(23, 2, 0, Some(6)),
            ],
            ..first.clone()
        };

        let selected = select_best_outcome([first, second].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].stop_index(), Some(6));
    }

    #[test]
    fn outcome_ties_keep_first_recursive_tree_shape() {
        let recursive_nodes = vec![RecognizedNode::Rule {
            rule_index: 1,
            invoking_state: -1,
            alt_number: 0,
            start_index: 0,
            stop_index: Some(0),
            return_values: BTreeMap::new(),
            children: vec![RecognizedNode::Rule {
                rule_index: 1,
                invoking_state: -1,
                alt_number: 0,
                start_index: 0,
                stop_index: Some(0),
                return_values: BTreeMap::new(),
                children: vec![RecognizedNode::Token { index: 0 }],
            }],
        }];
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: recursive_nodes.clone(),
        };
        let second = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(2, 0, 0, None)],
            nodes: recursive_nodes,
        };

        let selected = select_best_outcome([first, second].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].source_state(), 1);
    }

    #[test]
    fn sll_outcome_selection_keeps_earlier_recovered_alt() {
        let first_alt = RecognizeOutcome {
            index: 2,
            consumed_eof: true,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: vec![ParserDiagnostic {
                line: 1,
                column: 3,
                message: "missing 'Y' at '<EOF>'".to_owned(),
            }],
            decisions: vec![0],
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second_alt = RecognizeOutcome {
            diagnostics: Vec::new(),
            decisions: vec![1],
            actions: vec![ParserAction::new(2, 0, 0, None)],
            ..first_alt.clone()
        };

        let selected =
            select_best_outcome([second_alt, first_alt].into_iter(), PredictionMode::Sll)
                .expect("one outcome should be selected");
        assert_eq!(selected.diagnostics.len(), 1);
        assert_eq!(selected.decisions, [0]);
    }
}
