//! Semantic IR for grammar-embedded predicates and actions.
//!
//! ANTLR grammars embed target-language semantic predicates and actions that
//! a metadata-first runtime cannot execute directly (issue #9). This module
//! defines the small data-driven language those snippets are *translated
//! into*: heuristic template matching at codegen time, hand-written tables,
//! and (long term) a real Rust target all lower to the same IR, and the
//! runtime evaluates only the IR.
//!
//! Design constraints, in priority order:
//!
//! - **Prediction-safe**: predicates run speculatively inside adaptive
//!   prediction, possibly many times on abandoned paths. [`PExpr`] therefore
//!   has no mutating node — effects exist only in [`AStmt`], which the
//!   runtime executes on committed paths (or transactionally for
//!   member-state speculation).
//! - **Allocation-free on the hot path**: expression storage is a flat arena
//!   indexed by [`ExprId`], and text comparisons resolve borrowed `&str`
//!   operands without materializing `String`s (see `eval_text_cmp`).
//! - **Absence is explicit**: recognizer queries that can fail (missing
//!   lookahead token, absent context child, no rule argument) produce
//!   [`Value::Null`], and comparison semantics over Null are fixed here so
//!   every producer of IR agrees on them.
//!
//! # Null semantics
//!
//! - `Eq` is true iff both sides are present and equal, or both are Null.
//! - `Ne` is the negation of `Eq`.
//! - Ordering comparisons (`Lt`, `Le`, `Gt`, `Ge`) with any Null side are
//!   false.
//! - Arithmetic with any Null operand is Null; division/modulo by zero is
//!   Null.
//! - Truthiness: Null is false, `Bool(b)` is `b`, `Int(i)` is `i != 0`.
//!
//! These rules are load-bearing: `{...}?` lookahead-text predicates must fail
//! when the token is absent (`Eq(Null, "text") == false`), while
//! context-child text guards must pass when the child is absent
//! (`Ne(Null, "text") == true`). Predicates that are non-restrictive when a
//! value is absent (rule arguments) compose [`PExpr::IsNull`] with `Or`.

use std::borrow::Cow;
use std::fmt::Debug;

/// Index of an expression node inside a [`SemIr`] arena.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExprId(u32);

impl ExprId {
    /// Builds an expression id from a producer-assigned arena index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns this id's arena index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of a statement node inside a [`SemIr`] arena.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StmtId(u32);

impl StmtId {
    /// Builds a statement id from a producer-assigned arena index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns this id's arena index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of an interned string inside a [`SemIr`] arena.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StrId(u32);

impl StrId {
    /// Builds an interned-string id from a producer-assigned pool index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns this id's string-pool index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Opaque identifier of an externally implemented hook.
///
/// The IR deliberately cannot express arbitrary target code; a hook node
/// defers one predicate or action to the evaluation context, which maps the
/// id to grammar-specific behavior (a user trait method, or a runtime shim
/// such as the conformance suite's evaluation-reporting predicates).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HookId(u32);

impl HookId {
    /// Builds a hook id from a producer-assigned side-table index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Position of this hook in the producer's hook side table.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Comparison operator for [`PExpr::Cmp`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Arithmetic operator for [`PExpr::Arith`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// Pure predicate expression node.
///
/// Text-valued nodes ([`Self::Str`], [`Self::TokenText`],
/// [`Self::CtxRuleText`], [`Self::TokenTextSoFar`]) are only meaningful as
/// operands of [`Self::Cmp`] or [`Self::IsNull`]; evaluating one in any other
/// position yields [`Value::Null`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PExpr {
    /// Boolean literal.
    Bool(bool),
    /// Integer literal.
    Int(i64),
    /// Interned text literal (comparison operand only).
    Str(StrId),
    /// Token type of `LT(offset)` (parser) or lookahead char (lexer).
    La(isize),
    /// Text of the token at `LT(offset)`; Null when the token is absent.
    TokenText(isize),
    /// Whether the two most recently consumed tokens were adjacent in the
    /// token stream (`LT(-2).index + 1 == LT(-1).index`); false when either
    /// is absent.
    TokenIndexAdjacent,
    /// Text of the current rule context's first child with this rule index;
    /// Null when the context or child is absent.
    CtxRuleText(usize),
    /// Integer state slot declared by the grammar (`@members` counters).
    Member(usize),
    /// Integer argument of the current rule invocation; Null when the rule
    /// was invoked without one.
    LocalArg,
    /// Lexer: current character position within the line.
    Column,
    /// Lexer: character position of the current token's first character.
    TokenStartColumn,
    /// Lexer: text matched so far for the in-progress token.
    TokenTextSoFar,
    /// True when the operand evaluates to Null (or, for a text-valued
    /// operand, when its text is absent).
    IsNull(ExprId),
    /// Logical negation of the operand's truthiness.
    Not(ExprId),
    /// Short-circuit conjunction, evaluated left to right.
    And(Box<[ExprId]>),
    /// Short-circuit disjunction, evaluated left to right.
    Or(Box<[ExprId]>),
    /// Comparison; text operands take the text-comparison path.
    Cmp(CmpOp, ExprId, ExprId),
    /// Integer arithmetic with Null propagation.
    Arith(ArithOp, ExprId, ExprId),
    /// Defer to the context's hook table.
    Hook(HookId),
    /// Return a boolean while letting the recognizer report the evaluation.
    ///
    /// This keeps ANTLR runtime-testsuite `Invoke_pred` templates data-driven
    /// without making ordinary predicates effectful.
    EvalTrace(bool),
}

/// Effectful action statement node.
///
/// Statements never run during prediction unless the runtime explicitly
/// classifies them as speculation-eligible (member-only mutations evaluated
/// against a transactional member environment).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AStmt {
    /// `member = expr`.
    SetMember(usize, ExprId),
    /// `member += expr`.
    AddMember(usize, ExprId),
    /// Assign a rule return field by name.
    SetReturn(StrId, ExprId),
    /// Execute statements in order.
    Seq(Box<[StmtId]>),
    /// Defer to the context's action hook table.
    Hook(HookId),
}

/// Evaluation result of a non-text expression.
#[allow(variant_size_differences)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    /// An absent recognizer value (missing token, member, argument, …).
    Null,
    Bool(bool),
    Int(i64),
}

impl Value {
    /// Truthiness used by logical nodes and by [`eval_pred`]'s final result.
    #[must_use]
    pub const fn truthy(self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(value) => value,
            Self::Int(value) => value != 0,
        }
    }
}

/// Recognizer-state queries the predicate evaluator needs.
///
/// Implementations are thin adapters over a lexer or parser; queries that do
/// not exist for the implementing recognizer return `None` (evaluating to
/// Null). Lookahead methods take `&mut self` because token streams buffer
/// lazily.
pub trait PredContext {
    /// Token type (parser) or character (lexer) at the given lookahead.
    fn la(&mut self, offset: isize) -> i64;
    /// Text of the token at the given lookahead, if present.
    fn token_text(&mut self, offset: isize) -> Option<&str>;
    /// Whether `LT(-2)` and `LT(-1)` are adjacent token-stream entries.
    fn token_index_adjacent(&mut self) -> bool;
    /// Text of the current context's first child with this rule index.
    fn ctx_rule_text(&self, rule_index: usize) -> Option<String>;
    /// Integer member slot value.
    fn member(&self, member: usize) -> Option<i64>;
    /// Integer argument of the current rule invocation.
    fn local_arg(&self) -> Option<i64>;
    /// Lexer current character position within the line.
    fn column(&self) -> Option<i64>;
    /// Lexer character position of the current token's start.
    fn token_start_column(&self) -> Option<i64>;
    /// Lexer text matched so far for the in-progress token.
    fn token_text_so_far(&self) -> Option<String>;
    /// Evaluates an externally implemented predicate hook.
    fn hook(&mut self, hook: HookId) -> bool;
    /// Reports an observable predicate-evaluation template and returns `value`.
    fn trace_bool(&mut self, value: bool) -> bool {
        value
    }
}

/// Mutations the action evaluator needs, on top of predicate queries.
pub trait ActContext: PredContext {
    /// Writes an integer member slot.
    fn set_member(&mut self, member: usize, value: i64);
    /// Assigns a rule return field by name.
    fn set_return(&mut self, name: &str, value: i64);
    /// Runs an externally implemented action hook.
    fn action_hook(&mut self, hook: HookId);
}

/// Flat expression/statement arena with an interned string pool.
///
/// Producers append nodes through the builder methods and hand the finished
/// arena plus root ids to the runtime; evaluation never mutates the arena.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemIr {
    exprs: Vec<PExpr>,
    stmts: Vec<AStmt>,
    strings: Vec<Box<str>>,
}

impl SemIr {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an expression node and returns its id.
    pub fn expr(&mut self, node: PExpr) -> ExprId {
        let id = ExprId(u32::try_from(self.exprs.len()).expect("expression arena fits in u32"));
        self.exprs.push(node);
        id
    }

    /// Appends a statement node and returns its id.
    pub fn stmt(&mut self, node: AStmt) -> StmtId {
        let id = StmtId(u32::try_from(self.stmts.len()).expect("statement arena fits in u32"));
        self.stmts.push(node);
        id
    }

    /// Interns a string literal, reusing an existing pool entry when equal.
    pub fn intern(&mut self, value: &str) -> StrId {
        if let Some(position) = self.strings.iter().position(|entry| &**entry == value) {
            return StrId(u32::try_from(position).expect("string pool fits in u32"));
        }
        let id = StrId(u32::try_from(self.strings.len()).expect("string pool fits in u32"));
        self.strings.push(value.into());
        id
    }

    /// Resolves an interned string.
    #[must_use]
    pub fn text(&self, id: StrId) -> &str {
        &self.strings[id.0 as usize]
    }

    fn node(&self, id: ExprId) -> &PExpr {
        &self.exprs[id.0 as usize]
    }

    fn stmt_node(&self, id: StmtId) -> &AStmt {
        &self.stmts[id.0 as usize]
    }
}

/// Evaluates a predicate expression to its truthiness.
///
/// This is the runtime entry point for semantic predicate transitions; it is
/// side-effect-free except for [`PExpr::Hook`] nodes, whose implementations
/// own their replay-safety (they may run repeatedly on speculative paths).
pub fn eval_pred<C: PredContext>(ir: &SemIr, expr: ExprId, ctx: &mut C) -> bool {
    eval_value(ir, expr, ctx).truthy()
}

/// Executes an action statement against a mutable context.
pub fn exec_stmt<C: ActContext>(ir: &SemIr, stmt: StmtId, ctx: &mut C) {
    match ir.stmt_node(stmt) {
        AStmt::SetMember(member, value) => {
            let value = int_or_zero(eval_value(ir, *value, ctx));
            ctx.set_member(*member, value);
        }
        AStmt::AddMember(member, delta) => {
            let delta = int_or_zero(eval_value(ir, *delta, ctx));
            let current = ctx.member(*member).unwrap_or_default();
            ctx.set_member(*member, current + delta);
        }
        AStmt::SetReturn(name, value) => {
            let value = int_or_zero(eval_value(ir, *value, ctx));
            let name = ir.text(*name).to_owned();
            ctx.set_return(&name, value);
        }
        AStmt::Seq(stmts) => {
            for stmt in stmts {
                exec_stmt(ir, *stmt, ctx);
            }
        }
        AStmt::Hook(hook) => ctx.action_hook(*hook),
    }
}

const fn int_or_zero(value: Value) -> i64 {
    match value {
        Value::Int(value) => value,
        Value::Null | Value::Bool(_) => 0,
    }
}

fn eval_value<C: PredContext>(ir: &SemIr, expr: ExprId, ctx: &mut C) -> Value {
    match ir.node(expr) {
        // Text-valued nodes are comparison operands; anywhere else they have
        // no defined value.
        PExpr::Str(_) | PExpr::TokenText(_) | PExpr::CtxRuleText(_) | PExpr::TokenTextSoFar => {
            debug_assert!(false, "text-valued node evaluated outside a comparison");
            Value::Null
        }
        PExpr::Bool(value) => Value::Bool(*value),
        PExpr::Int(value) => Value::Int(*value),
        PExpr::La(offset) => Value::Int(ctx.la(*offset)),
        PExpr::TokenIndexAdjacent => Value::Bool(ctx.token_index_adjacent()),
        PExpr::Member(member) => ctx.member(*member).map_or(Value::Null, Value::Int),
        PExpr::LocalArg => ctx.local_arg().map_or(Value::Null, Value::Int),
        PExpr::Column => ctx.column().map_or(Value::Null, Value::Int),
        PExpr::TokenStartColumn => ctx.token_start_column().map_or(Value::Null, Value::Int),
        PExpr::IsNull(inner) => Value::Bool(eval_is_null(ir, *inner, ctx)),
        PExpr::Not(inner) => Value::Bool(!eval_value(ir, *inner, ctx).truthy()),
        PExpr::And(children) => Value::Bool(
            children
                .iter()
                .all(|child| eval_value(ir, *child, ctx).truthy()),
        ),
        PExpr::Or(children) => Value::Bool(
            children
                .iter()
                .any(|child| eval_value(ir, *child, ctx).truthy()),
        ),
        PExpr::Cmp(op, lhs, rhs) => eval_cmp(ir, *op, *lhs, *rhs, ctx),
        PExpr::Arith(op, lhs, rhs) => eval_arith(ir, *op, *lhs, *rhs, ctx),
        PExpr::Hook(hook) => Value::Bool(ctx.hook(*hook)),
        PExpr::EvalTrace(value) => Value::Bool(ctx.trace_bool(*value)),
    }
}

fn eval_is_null<C: PredContext>(ir: &SemIr, inner: ExprId, ctx: &mut C) -> bool {
    if let Some(source) = text_source(ir, inner) {
        return resolve_owned_text(ir, source, ctx).is_none();
    }
    eval_value(ir, inner, ctx) == Value::Null
}

fn eval_cmp<C: PredContext>(ir: &SemIr, op: CmpOp, lhs: ExprId, rhs: ExprId, ctx: &mut C) -> Value {
    let left_source = text_source(ir, lhs);
    let right_source = text_source(ir, rhs);
    if left_source.is_some() || right_source.is_some() {
        return eval_text_cmp(ir, op, (lhs, left_source), (rhs, right_source), ctx);
    }
    let left = eval_value(ir, lhs, ctx);
    let right = eval_value(ir, rhs, ctx);
    Value::Bool(match (left, right) {
        (Value::Null, Value::Null) => cmp_on_equality(op, true),
        (Value::Null, _) | (_, Value::Null) => cmp_on_equality(op, false),
        (Value::Bool(left), Value::Bool(right)) => cmp_on_equality(op, left == right),
        (Value::Int(left), Value::Int(right)) => cmp_ints(op, left, right),
        (Value::Bool(_), Value::Int(_)) | (Value::Int(_), Value::Bool(_)) => {
            cmp_on_equality(op, false)
        }
    })
}

/// Comparison outcome for operands that only carry equality (Null, Bool,
/// mismatched kinds): ordering operators are false.
const fn cmp_on_equality(op: CmpOp, equal: bool) -> bool {
    match op {
        CmpOp::Eq => equal,
        CmpOp::Ne => !equal,
        CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge => false,
    }
}

const fn cmp_ints(op: CmpOp, left: i64, right: i64) -> bool {
    match op {
        CmpOp::Eq => left == right,
        CmpOp::Ne => left != right,
        CmpOp::Lt => left < right,
        CmpOp::Le => left <= right,
        CmpOp::Gt => left > right,
        CmpOp::Ge => left >= right,
    }
}

/// Where a text-valued operand's characters come from.
///
/// Only [`Self::Lookahead`] holds a borrow of the context while its `&str`
/// is alive; the other sources either borrow the IR string pool or return an
/// owned `String`. `eval_text_cmp` resolves the non-lookahead side first so
/// the common `token-text == literal` comparison stays allocation-free.
#[derive(Clone, Copy, Debug)]
enum TextSource {
    Literal(StrId),
    Lookahead(isize),
    CtxRule(usize),
    SoFar,
}

fn text_source(ir: &SemIr, expr: ExprId) -> Option<TextSource> {
    match ir.node(expr) {
        PExpr::Str(id) => Some(TextSource::Literal(*id)),
        PExpr::TokenText(offset) => Some(TextSource::Lookahead(*offset)),
        PExpr::CtxRuleText(rule_index) => Some(TextSource::CtxRule(*rule_index)),
        PExpr::TokenTextSoFar => Some(TextSource::SoFar),
        _ => None,
    }
}

/// Resolves a non-lookahead text operand without holding a context borrow.
fn resolve_static_text<'ir, C: PredContext>(
    ir: &'ir SemIr,
    source: TextSource,
    ctx: &C,
) -> Option<Cow<'ir, str>> {
    match source {
        TextSource::Literal(id) => Some(Cow::Borrowed(ir.text(id))),
        TextSource::Lookahead(_) => unreachable!("lookahead operands are resolved last"),
        TextSource::CtxRule(rule_index) => ctx.ctx_rule_text(rule_index).map(Cow::Owned),
        TextSource::SoFar => ctx.token_text_so_far().map(Cow::Owned),
    }
}

/// Owned resolution used by [`PExpr::IsNull`] over text operands.
fn resolve_owned_text<C: PredContext>(
    ir: &SemIr,
    source: TextSource,
    ctx: &mut C,
) -> Option<String> {
    match source {
        TextSource::Lookahead(offset) => ctx.token_text(offset).map(str::to_owned),
        other => resolve_static_text(ir, other, ctx).map(Cow::into_owned),
    }
}

fn eval_text_cmp<C: PredContext>(
    ir: &SemIr,
    op: CmpOp,
    (lhs, left_source): (ExprId, Option<TextSource>),
    (rhs, right_source): (ExprId, Option<TextSource>),
    ctx: &mut C,
) -> Value {
    // A text operand compared against a non-text operand has no defined
    // value relationship; only equality semantics apply (never equal).
    let (Some(left_source), Some(right_source)) = (left_source, right_source) else {
        debug_assert!(false, "text operand compared with non-text operand");
        let _ = (lhs, rhs);
        return Value::Bool(cmp_on_equality(op, false));
    };
    Value::Bool(match (left_source, right_source) {
        (TextSource::Lookahead(left), TextSource::Lookahead(right)) => {
            // Two live lookahead borrows cannot coexist; own the left side.
            // No current producer emits this shape, so the allocation is
            // acceptable.
            let left = ctx.token_text(left).map(str::to_owned);
            let right = ctx.token_text(right);
            cmp_texts(op, left.as_deref(), right)
        }
        (TextSource::Lookahead(offset), other) => {
            let right = resolve_static_text(ir, other, ctx);
            let left = ctx.token_text(offset);
            cmp_texts(op, left, right.as_deref())
        }
        (other, TextSource::Lookahead(offset)) => {
            let left = resolve_static_text(ir, other, ctx);
            let right = ctx.token_text(offset);
            cmp_texts(op, left.as_deref(), right)
        }
        (left, right) => {
            let left = resolve_static_text(ir, left, ctx);
            let right = resolve_static_text(ir, right, ctx);
            cmp_texts(op, left.as_deref(), right.as_deref())
        }
    })
}

fn cmp_texts(op: CmpOp, left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => cmp_on_equality(op, true),
        (None, Some(_)) | (Some(_), None) => cmp_on_equality(op, false),
        (Some(left), Some(right)) => match op {
            CmpOp::Eq => left == right,
            CmpOp::Ne => left != right,
            CmpOp::Lt => left < right,
            CmpOp::Le => left <= right,
            CmpOp::Gt => left > right,
            CmpOp::Ge => left >= right,
        },
    }
}

fn eval_arith<C: PredContext>(
    ir: &SemIr,
    op: ArithOp,
    lhs: ExprId,
    rhs: ExprId,
    ctx: &mut C,
) -> Value {
    let (Value::Int(left), Value::Int(right)) =
        (eval_value(ir, lhs, ctx), eval_value(ir, rhs, ctx))
    else {
        return Value::Null;
    };
    let result = match op {
        ArithOp::Add => left.checked_add(right),
        ArithOp::Sub => left.checked_sub(right),
        ArithOp::Mul => left.checked_mul(right),
        ArithOp::Div => left.checked_div(right),
        ArithOp::Mod => left.checked_rem(right),
    };
    result.map_or(Value::Null, Value::Int)
}

#[cfg(test)]
mod tests {
    use super::{
        AStmt, ActContext, ArithOp, CmpOp, ExprId, HookId, PExpr, PredContext, SemIr, eval_pred,
        exec_stmt,
    };
    use std::collections::BTreeMap;

    /// Scriptable recognizer stand-in for evaluator tests.
    #[derive(Debug, Default)]
    struct MockCtx {
        tokens: Vec<(i64, Option<&'static str>)>,
        adjacent: bool,
        ctx_rule_texts: BTreeMap<usize, String>,
        members: BTreeMap<usize, i64>,
        local_arg: Option<i64>,
        column: Option<i64>,
        token_start_column: Option<i64>,
        text_so_far: Option<String>,
        hook_results: Vec<bool>,
        hook_calls: Vec<HookId>,
        la_calls: usize,
        returns: BTreeMap<String, i64>,
    }

    impl PredContext for MockCtx {
        fn la(&mut self, offset: isize) -> i64 {
            self.la_calls += 1;
            self.lookup(offset).map_or(-1, |(token_type, _)| token_type)
        }

        fn token_text(&mut self, offset: isize) -> Option<&str> {
            self.lookup(offset).and_then(|(_, text)| text)
        }

        fn token_index_adjacent(&mut self) -> bool {
            self.adjacent
        }

        fn ctx_rule_text(&self, rule_index: usize) -> Option<String> {
            self.ctx_rule_texts.get(&rule_index).cloned()
        }

        fn member(&self, member: usize) -> Option<i64> {
            self.members.get(&member).copied()
        }

        fn local_arg(&self) -> Option<i64> {
            self.local_arg
        }

        fn column(&self) -> Option<i64> {
            self.column
        }

        fn token_start_column(&self) -> Option<i64> {
            self.token_start_column
        }

        fn token_text_so_far(&self) -> Option<String> {
            self.text_so_far.clone()
        }

        fn hook(&mut self, hook: HookId) -> bool {
            self.hook_calls.push(hook);
            self.hook_results[hook.index()]
        }
    }

    impl ActContext for MockCtx {
        fn set_member(&mut self, member: usize, value: i64) {
            self.members.insert(member, value);
        }

        fn set_return(&mut self, name: &str, value: i64) {
            self.returns.insert(name.to_owned(), value);
        }

        fn action_hook(&mut self, hook: HookId) {
            self.hook_calls.push(hook);
        }
    }

    impl MockCtx {
        fn lookup(&self, offset: isize) -> Option<(i64, Option<&'static str>)> {
            // Offset 1 is the first entry, -1 the last, mirroring LT(k).
            let index = if offset > 0 {
                usize::try_from(offset - 1).ok()?
            } else {
                self.tokens.len().checked_sub(offset.unsigned_abs())?
            };
            self.tokens.get(index).copied()
        }
    }

    fn build(build: impl FnOnce(&mut SemIr) -> ExprId) -> (SemIr, ExprId) {
        let mut ir = SemIr::new();
        let root = build(&mut ir);
        (ir, root)
    }

    #[test]
    fn literals_and_truthiness() {
        for (value, expected) in [(true, true), (false, false)] {
            let (ir, root) = build(|ir| ir.expr(PExpr::Bool(value)));
            assert_eq!(eval_pred(&ir, root, &mut MockCtx::default()), expected);
        }
        let (ir, root) = build(|ir| ir.expr(PExpr::Int(2)));
        assert!(eval_pred(&ir, root, &mut MockCtx::default()));
        let (ir, root) = build(|ir| ir.expr(PExpr::Int(0)));
        assert!(!eval_pred(&ir, root, &mut MockCtx::default()));
    }

    #[test]
    fn lookahead_text_equals_literal_and_absent_token_fails() {
        let (ir, root) = build(|ir| {
            let text = ir.expr(PExpr::TokenText(1));
            let literal = ir.intern("of");
            let literal = ir.expr(PExpr::Str(literal));
            ir.expr(PExpr::Cmp(CmpOp::Eq, text, literal))
        });

        let mut ctx = MockCtx {
            tokens: vec![(7, Some("of"))],
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));

        ctx.tokens = vec![(7, Some("in"))];
        assert!(!eval_pred(&ir, root, &mut ctx));

        // Absent token: Eq against a present literal is false.
        ctx.tokens = Vec::new();
        assert!(!eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn ctx_rule_text_not_equals_passes_when_child_absent() {
        let (ir, root) = build(|ir| {
            let child = ir.expr(PExpr::CtxRuleText(4));
            let literal = ir.intern("static");
            let literal = ir.expr(PExpr::Str(literal));
            ir.expr(PExpr::Cmp(CmpOp::Ne, child, literal))
        });

        // Child absent: non-restrictive, passes.
        assert!(eval_pred(&ir, root, &mut MockCtx::default()));

        let mut ctx = MockCtx {
            ctx_rule_texts: std::iter::once((4, "static".to_owned())).collect(),
            ..MockCtx::default()
        };
        assert!(!eval_pred(&ir, root, &mut ctx));

        ctx.ctx_rule_texts = std::iter::once((4, "dynamic".to_owned())).collect();
        assert!(eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn absent_local_arg_composes_non_restrictive_guard() {
        // Legacy `LocalIntEquals` semantics: pass when the rule has no
        // argument, compare when it does.
        let (ir, root) = build(|ir| {
            let arg = ir.expr(PExpr::LocalArg);
            let absent = ir.expr(PExpr::IsNull(arg));
            let value = ir.expr(PExpr::Int(2));
            let equals = ir.expr(PExpr::Cmp(CmpOp::Eq, arg, value));
            ir.expr(PExpr::Or([absent, equals].into()))
        });

        assert!(eval_pred(&ir, root, &mut MockCtx::default()));
        let mut ctx = MockCtx {
            local_arg: Some(2),
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
        ctx.local_arg = Some(3);
        assert!(!eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn member_modulo_comparison() {
        let (ir, root) = build(|ir| {
            let member = ir.expr(PExpr::Member(0));
            let modulus = ir.expr(PExpr::Int(2));
            let remainder = ir.expr(PExpr::Arith(ArithOp::Mod, member, modulus));
            let expected = ir.expr(PExpr::Int(0));
            ir.expr(PExpr::Cmp(CmpOp::Eq, remainder, expected))
        });

        let mut ctx = MockCtx {
            members: std::iter::once((0, 4)).collect(),
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
        ctx.members.insert(0, 5);
        assert!(!eval_pred(&ir, root, &mut ctx));
        // Absent member is Null; Eq with a present value is false.
        ctx.members.clear();
        assert!(!eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn arithmetic_null_propagation_and_division_by_zero() {
        let (ir, root) = build(|ir| {
            let member = ir.expr(PExpr::Member(9));
            let zero = ir.expr(PExpr::Int(0));
            let modulo = ir.expr(PExpr::Arith(ArithOp::Mod, member, zero));
            ir.expr(PExpr::IsNull(modulo))
        });
        // member(9) present, but % 0 is Null.
        let mut ctx = MockCtx {
            members: std::iter::once((9, 3)).collect(),
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn and_or_short_circuit_left_to_right() {
        let (ir, root) = build(|ir| {
            let gate = ir.expr(PExpr::Bool(false));
            let la = ir.expr(PExpr::La(1));
            let one = ir.expr(PExpr::Int(1));
            let la_check = ir.expr(PExpr::Cmp(CmpOp::Eq, la, one));
            ir.expr(PExpr::And([gate, la_check].into()))
        });
        let mut ctx = MockCtx::default();
        assert!(!eval_pred(&ir, root, &mut ctx));
        assert_eq!(ctx.la_calls, 0, "false gate must short-circuit la()");

        let (ir, root) = build(|ir| {
            let gate = ir.expr(PExpr::Bool(true));
            let la = ir.expr(PExpr::La(1));
            let one = ir.expr(PExpr::Int(1));
            let la_check = ir.expr(PExpr::Cmp(CmpOp::Eq, la, one));
            ir.expr(PExpr::Or([gate, la_check].into()))
        });
        let mut ctx = MockCtx::default();
        assert!(eval_pred(&ir, root, &mut ctx));
        assert_eq!(ctx.la_calls, 0, "true gate must short-circuit la()");
    }

    #[test]
    fn token_index_adjacency_and_lookahead_type() {
        let (ir, root) = build(|ir| ir.expr(PExpr::TokenIndexAdjacent));
        let mut ctx = MockCtx {
            adjacent: true,
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
        ctx.adjacent = false;
        assert!(!eval_pred(&ir, root, &mut ctx));

        let (ir, root) = build(|ir| {
            let la = ir.expr(PExpr::La(-1));
            let expected = ir.expr(PExpr::Int(12));
            ir.expr(PExpr::Cmp(CmpOp::Ne, la, expected))
        });
        let mut ctx = MockCtx {
            tokens: vec![(12, None)],
            ..MockCtx::default()
        };
        assert!(!eval_pred(&ir, root, &mut ctx));
        ctx.tokens = vec![(13, None)];
        assert!(eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn lexer_column_predicates() {
        let (ir, root) = build(|ir| {
            let column = ir.expr(PExpr::Column);
            let limit = ir.expr(PExpr::Int(4));
            ir.expr(PExpr::Cmp(CmpOp::Ge, column, limit))
        });
        let mut ctx = MockCtx {
            column: Some(5),
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
        ctx.column = Some(3);
        assert!(!eval_pred(&ir, root, &mut ctx));
        // Unknown column: ordering against Null is false.
        ctx.column = None;
        assert!(!eval_pred(&ir, root, &mut ctx));

        let (ir, root) = build(|ir| {
            let start = ir.expr(PExpr::TokenStartColumn);
            let zero = ir.expr(PExpr::Int(0));
            ir.expr(PExpr::Cmp(CmpOp::Eq, start, zero))
        });
        let mut ctx = MockCtx {
            token_start_column: Some(0),
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn lexer_text_so_far_comparison() {
        let (ir, root) = build(|ir| {
            let text = ir.expr(PExpr::TokenTextSoFar);
            let literal = ir.intern("aa");
            let literal = ir.expr(PExpr::Str(literal));
            ir.expr(PExpr::Cmp(CmpOp::Eq, text, literal))
        });
        let mut ctx = MockCtx {
            text_so_far: Some("aa".to_owned()),
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
        ctx.text_so_far = Some("ab".to_owned());
        assert!(!eval_pred(&ir, root, &mut ctx));
    }

    #[test]
    fn hooks_defer_to_context() {
        let (ir, root) = build(|ir| ir.expr(PExpr::Hook(HookId(0))));
        let mut ctx = MockCtx {
            hook_results: vec![true],
            ..MockCtx::default()
        };
        assert!(eval_pred(&ir, root, &mut ctx));
        assert_eq!(ctx.hook_calls, vec![HookId(0)]);
    }

    #[test]
    fn statements_mutate_members_and_returns() {
        let mut ir = SemIr::new();
        let five = ir.expr(PExpr::Int(5));
        let set = ir.stmt(AStmt::SetMember(1, five));
        let two = ir.expr(PExpr::Int(2));
        let add = ir.stmt(AStmt::AddMember(1, two));
        let member = ir.expr(PExpr::Member(1));
        let name = ir.intern("y");
        let ret = ir.stmt(AStmt::SetReturn(name, member));
        let seq = ir.stmt(AStmt::Seq([set, add, ret].into()));

        let mut ctx = MockCtx::default();
        exec_stmt(&ir, seq, &mut ctx);

        assert_eq!(ctx.members.get(&1), Some(&7));
        assert_eq!(ctx.returns.get("y"), Some(&7));
    }

    #[test]
    fn string_interning_deduplicates() {
        let mut ir = SemIr::new();
        let first = ir.intern("of");
        let second = ir.intern("of");
        let third = ir.intern("in");
        assert_eq!(first, second);
        assert_ne!(first, third);
        assert_eq!(ir.text(third), "in");
    }
}
