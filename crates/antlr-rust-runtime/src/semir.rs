// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
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
//!
//! # Member state
//!
//! Grammars declare their own state in `@members` / `@lexer::members`. The IR
//! models it as [`MemberEnv`]: numbered slots that are either **scalar**
//! integers ([`PExpr::Member`], [`AStmt::SetMember`], [`AStmt::AddMember`]) or
//! **stacks** of integers ([`PExpr::MemberTop`], [`PExpr::MemberLen`],
//! [`AStmt::PushMember`], [`AStmt::PopMember`]). Stack slots cover the nesting
//! counters real grammars keep for string interpolation and mode tracking
//! (issue #206).
//!
//! Slot values are integers, so a boolean operand is coerced by
//! [`Value::truthy`]'s inverse — `true` is 1, `false` is 0 — and reads back
//! with the same truthiness. Empty-stack reads and pops are **defined, not
//! errors**:
//!
//! - [`PExpr::MemberTop`] on an empty (or never-pushed) stack is
//!   [`Value::Null`], which is falsy. This is exactly the
//!   `Count > 0 ? Peek() : false` idiom grammars write by hand.
//! - [`PExpr::MemberLen`] on a never-pushed stack is `0`, not Null: a stack
//!   that was never used is empty, not absent.
//! - [`AStmt::PopMember`] on an empty stack is a no-op. An unbalanced pop is a
//!   grammar bug the recognizer cannot diagnose, and panicking inside
//!   prediction would turn it into a crash on input the grammar merely
//!   mis-describes.

use std::borrow::Cow;
use std::collections::BTreeMap;
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
    /// Top of a stack-valued state slot; Null when the stack is empty or the
    /// slot was never pushed. See the module's "Member state" section.
    MemberTop(usize),
    /// Depth of a stack-valued state slot; `0` when never pushed.
    MemberLen(usize),
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
    /// `member.push(expr)` on a stack-valued slot.
    PushMember(usize, ExprId),
    /// `member.pop()` on a stack-valued slot; a no-op when empty.
    PopMember(usize),
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
    type TokenText<'a>: AsRef<str>
    where
        Self: 'a;

    /// Token type (parser) or character (lexer) at the given lookahead.
    fn la(&mut self, offset: isize) -> i64;
    /// Text of the token at the given lookahead, if present.
    fn token_text(&mut self, offset: isize) -> Option<Self::TokenText<'_>>;
    /// Whether `LT(-2)` and `LT(-1)` are adjacent token-stream entries.
    fn token_index_adjacent(&mut self) -> bool;
    /// Text of the current context's first child with this rule index.
    fn ctx_rule_text(&self, rule_index: usize) -> Option<String>;
    /// Integer member slot value.
    fn member(&self, member: usize) -> Option<i64>;
    /// Top of a stack-valued member slot; `None` when empty or never pushed.
    ///
    /// Recognizers with no grammar-declared stack state keep the default.
    fn member_top(&self, _member: usize) -> Option<i64> {
        None
    }
    /// Depth of a stack-valued member slot.
    fn member_len(&self, _member: usize) -> usize {
        0
    }
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
    /// Pushes onto a stack-valued member slot.
    ///
    /// Recognizers with no grammar-declared stack state keep the default
    /// no-op; [`AStmt::PushMember`] is only produced for grammars that declare
    /// a stack slot, so a silent drop here is unreachable rather than lossy.
    fn push_member(&mut self, _member: usize, _value: i64) {}
    /// Pops a stack-valued member slot, returning the removed value. A no-op
    /// returning `None` when the stack is empty.
    fn pop_member(&mut self, _member: usize) -> Option<i64> {
        None
    }
    /// Assigns a rule return field by name.
    fn set_return(&mut self, name: &str, value: i64);
    /// Runs an externally implemented action hook.
    fn action_hook(&mut self, hook: HookId);
}

/// Grammar-declared member state: numbered scalar and stack slots.
///
/// Recognition threads this by value along each speculative path (it is part
/// of the parser's memo key), so it is ordered and compares structurally.
/// Absent slots are not stored: a slot holding `0` is distinct from one never
/// written, but an *emptied* stack is canonicalized back to absent so two
/// logically identical paths stay `Eq` — and keep sharing memo entries.
///
/// Scalar and stack slot numbers live in separate namespaces; the generator
/// assigns each declared member to one or the other.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MemberEnv {
    scalars: BTreeMap<usize, i64>,
    stacks: BTreeMap<usize, Vec<i64>>,
}

impl MemberEnv {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scalars: BTreeMap::new(),
            stacks: BTreeMap::new(),
        }
    }

    /// Builds an environment holding a grammar's declared initial scalar values.
    ///
    /// Grammars write `private bool verbatium = true;` / `private int level =
    /// 1;`. Those initializers are part of the grammar's meaning: a predicate
    /// reading a slot that silently started at 0 instead would reject input the
    /// source grammar accepts. Generated recognizers seed with this so a fresh
    /// recognizer — and every [`Self::reset_to_initial`] afterwards — starts
    /// where the grammar says.
    #[must_use]
    pub fn with_initial_scalars(initial: impl IntoIterator<Item = (usize, i64)>) -> Self {
        Self {
            scalars: initial.into_iter().collect(),
            stacks: BTreeMap::new(),
        }
    }

    /// Clears all state back to the declared initial scalar values.
    ///
    /// This is what a recognizer reset needs: not "empty", but the state a
    /// freshly constructed recognizer had. Stacks always reset to empty, since
    /// a declaration cannot pre-seed one.
    pub fn reset_to_initial(&mut self, initial: impl IntoIterator<Item = (usize, i64)>) {
        self.scalars = initial.into_iter().collect();
        self.stacks.clear();
    }

    /// Whether no slot has been written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scalars.is_empty() && self.stacks.is_empty()
    }

    /// Reads a scalar slot; `None` when never written.
    #[must_use]
    pub fn scalar(&self, member: usize) -> Option<i64> {
        self.scalars.get(&member).copied()
    }

    /// Writes a scalar slot.
    pub fn set_scalar(&mut self, member: usize, value: i64) {
        self.scalars.insert(member, value);
    }

    /// Adds to a scalar slot (absent reads as `0`) and returns the new value.
    pub fn add_scalar(&mut self, member: usize, delta: i64) -> i64 {
        let value = self.scalars.entry(member).or_default();
        *value = value.saturating_add(delta);
        *value
    }

    /// Top of a stack slot; `None` when empty or never pushed.
    #[must_use]
    pub fn stack_top(&self, member: usize) -> Option<i64> {
        self.stacks.get(&member)?.last().copied()
    }

    /// Depth of a stack slot; `0` when never pushed.
    #[must_use]
    pub fn stack_len(&self, member: usize) -> usize {
        self.stacks.get(&member).map_or(0, Vec::len)
    }

    /// Pushes onto a stack slot.
    pub fn push_stack(&mut self, member: usize, value: i64) {
        self.stacks.entry(member).or_default().push(value);
    }

    /// Pops a stack slot, returning the removed value, or `None` when empty.
    ///
    /// An emptied stack drops its slot so it compares equal to one never
    /// pushed — otherwise `push`-then-`pop` would produce a memo key that no
    /// longer matches the equivalent untouched path.
    pub fn pop_stack(&mut self, member: usize) -> Option<i64> {
        let stack = self.stacks.get_mut(&member)?;
        let value = stack.pop();
        if stack.is_empty() {
            self.stacks.remove(&member);
        }
        value
    }

    /// Iterates written scalar slots in slot order.
    pub fn scalars(&self) -> impl Iterator<Item = (usize, i64)> + '_ {
        self.scalars.iter().map(|(slot, value)| (*slot, *value))
    }
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
            ctx.set_member(*member, current.saturating_add(delta));
        }
        AStmt::PushMember(member, value) => {
            let value = int_or_zero(eval_value(ir, *value, ctx));
            ctx.push_member(*member, value);
        }
        AStmt::PopMember(member) => {
            // An unbalanced pop is a grammar bug, not a recognizer error: drop
            // it rather than panicking inside prediction.
            let _ = ctx.pop_member(*member);
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

/// Coerces a statement operand to the integer a member slot stores.
///
/// Slots are integers, so a boolean operand (`{ verbatium = false; }`,
/// `interpolatedVerbatiums.Push(true)`) must survive the round trip through
/// [`Value::truthy`]: `true` is 1 so reading the slot back is truthy, `false`
/// is 0. Null has no value to store and becomes 0.
const fn int_or_zero(value: Value) -> i64 {
    match value {
        Value::Int(value) => value,
        Value::Bool(value) => value as i64,
        Value::Null => 0,
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
        // An empty stack reads as Null (falsy), which is the grammar idiom
        // `Count > 0 ? Peek() : false` without the guard.
        PExpr::MemberTop(member) => ctx.member_top(*member).map_or(Value::Null, Value::Int),
        // A never-pushed stack is empty, not absent, so depth is 0 not Null.
        PExpr::MemberLen(member) => {
            Value::Int(i64::try_from(ctx.member_len(*member)).unwrap_or(i64::MAX))
        }
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
        TextSource::Lookahead(offset) => {
            ctx.token_text(offset).map(|text| text.as_ref().to_owned())
        }
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
            // Holding the first token-text borrow would keep `ctx` borrowed,
            // so own this unsupported producer shape's first operand.
            let left = ctx.token_text(left).map(|text| text.as_ref().to_owned());
            let right = ctx.token_text(right);
            cmp_texts(op, left.as_deref(), right.as_ref().map(AsRef::as_ref))
        }
        (TextSource::Lookahead(offset), other) => {
            let right = resolve_static_text(ir, other, ctx);
            let left = ctx.token_text(offset);
            cmp_texts(op, left.as_ref().map(AsRef::as_ref), right.as_deref())
        }
        (other, TextSource::Lookahead(offset)) => {
            let left = resolve_static_text(ir, other, ctx);
            let right = ctx.token_text(offset);
            cmp_texts(op, left.as_deref(), right.as_ref().map(AsRef::as_ref))
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
        AStmt, ActContext, ArithOp, CmpOp, ExprId, HookId, MemberEnv, PExpr, PredContext, SemIr,
        Value, eval_pred, eval_value, exec_stmt,
    };
    use std::collections::BTreeMap;

    /// Scriptable recognizer stand-in for evaluator tests.
    #[derive(Debug, Default)]
    struct MockCtx {
        tokens: Vec<(i64, Option<&'static str>)>,
        adjacent: bool,
        ctx_rule_texts: BTreeMap<usize, String>,
        members: BTreeMap<usize, i64>,
        stacks: MemberEnv,
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
        type TokenText<'a>
            = &'a str
        where
            Self: 'a;

        fn la(&mut self, offset: isize) -> i64 {
            self.la_calls += 1;
            self.lookup(offset).map_or(-1, |(token_type, _)| token_type)
        }

        fn token_text(&mut self, offset: isize) -> Option<Self::TokenText<'_>> {
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

        fn member_top(&self, member: usize) -> Option<i64> {
            self.stacks.stack_top(member)
        }

        fn member_len(&self, member: usize) -> usize {
            self.stacks.stack_len(member)
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

        fn push_member(&mut self, member: usize, value: i64) {
            self.stacks.push_stack(member, value);
        }

        fn pop_member(&mut self, member: usize) -> Option<i64> {
            self.stacks.pop_stack(member)
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

    /// The C# interpolation idiom: `Push`/`Pop` a nesting stack and read its
    /// top as a boolean guard. `MemberTop` on an empty stack must be falsy
    /// rather than panic — the grammar writes
    /// `Count > 0 ? Peek() : false` and relies on exactly that.
    #[test]
    fn stack_member_push_pop_and_empty_reads() {
        let mut ir = SemIr::new();
        let verbatium = ir.expr(PExpr::Bool(true));
        let push_true = ir.stmt(AStmt::PushMember(0, verbatium));
        let regular = ir.expr(PExpr::Bool(false));
        let push_false = ir.stmt(AStmt::PushMember(0, regular));
        let pop = ir.stmt(AStmt::PopMember(0));
        let top = ir.expr(PExpr::MemberTop(0));
        let depth = ir.expr(PExpr::MemberLen(0));

        let mut ctx = MockCtx::default();

        // Never pushed: top is Null (falsy), depth is 0.
        assert!(!eval_pred(&ir, top, &mut ctx));
        assert_eq!(eval_value(&ir, depth, &mut ctx), Value::Int(0));

        exec_stmt(&ir, push_true, &mut ctx);
        assert!(
            eval_pred(&ir, top, &mut ctx),
            "pushed true reads back truthy"
        );
        assert_eq!(eval_value(&ir, depth, &mut ctx), Value::Int(1));

        // A `false` push must shadow the `true` beneath it, not vanish.
        exec_stmt(&ir, push_false, &mut ctx);
        assert!(!eval_pred(&ir, top, &mut ctx));
        assert_eq!(eval_value(&ir, depth, &mut ctx), Value::Int(2));

        // Popping restores the enclosing frame's value.
        exec_stmt(&ir, pop, &mut ctx);
        assert!(eval_pred(&ir, top, &mut ctx));
        assert_eq!(eval_value(&ir, depth, &mut ctx), Value::Int(1));

        exec_stmt(&ir, pop, &mut ctx);
        assert_eq!(eval_value(&ir, top, &mut ctx), Value::Null);
        assert_eq!(eval_value(&ir, depth, &mut ctx), Value::Int(0));

        // Underflow is a defined no-op, not a panic.
        exec_stmt(&ir, pop, &mut ctx);
        assert_eq!(eval_value(&ir, top, &mut ctx), Value::Null);
        assert_eq!(eval_value(&ir, depth, &mut ctx), Value::Int(0));
    }

    /// Slots hold integers, so a boolean assignment must round-trip through
    /// truthiness — `{ verbatium = true; }` then `{ verbatium }?` passes.
    #[test]
    fn bool_member_assignment_round_trips_through_truthiness() {
        let mut ir = SemIr::new();
        let yes = ir.expr(PExpr::Bool(true));
        let set_true = ir.stmt(AStmt::SetMember(3, yes));
        let no = ir.expr(PExpr::Bool(false));
        let set_false = ir.stmt(AStmt::SetMember(3, no));
        let read = ir.expr(PExpr::Member(3));

        let mut ctx = MockCtx::default();
        exec_stmt(&ir, set_true, &mut ctx);
        assert!(eval_pred(&ir, read, &mut ctx));
        exec_stmt(&ir, set_false, &mut ctx);
        assert!(!eval_pred(&ir, read, &mut ctx));
    }

    /// Emptying a stack must return the env to a state that compares equal to
    /// one never pushed. The parser's memo key contains this env, so a
    /// lingering empty `Vec` would silently stop matching equivalent paths.
    #[test]
    fn emptied_stack_slot_compares_equal_to_untouched_env() {
        let mut env = MemberEnv::new();
        env.push_stack(1, 7);
        assert_ne!(env, MemberEnv::new());
        assert_eq!(env.pop_stack(1), Some(7));
        assert_eq!(env, MemberEnv::new(), "emptied stack must canonicalize");
        assert!(env.is_empty());
        // Underflow leaves it canonical too.
        assert_eq!(env.pop_stack(1), None);
        assert_eq!(env, MemberEnv::new());
    }

    /// Scalar and stack slots are separate namespaces: slot 0 as a counter and
    /// slot 0 as a stack must not alias.
    #[test]
    fn scalar_and_stack_slots_do_not_alias() {
        let mut env = MemberEnv::new();
        env.set_scalar(0, 5);
        env.push_stack(0, 9);
        assert_eq!(env.scalar(0), Some(5));
        assert_eq!(env.stack_top(0), Some(9));
        assert_eq!(env.pop_stack(0), Some(9));
        assert_eq!(env.scalar(0), Some(5), "popping a stack leaves scalars");
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
