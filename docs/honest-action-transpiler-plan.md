# Plan: Honest embedded-action handling via known transformations

## Background

The generator had fixture-fitted codegen that **re-derived** expected conformance
output instead of implementing the grammar's semantics. The worst offender:
`render_rule_value_write` emitted a hardcoded arithmetic/string evaluator
(`parse_sum`/`parse_product`, even `let mut value = 3`) for `$rule.v` /
`$rule.result` references, re-parsing the matched token text and re-evaluating it
against the upstream `ExpressionGrammar`'s arithmetic rules. This made
`LeftRecursion/ReturnValueAndActions*`, `Performance/ExpressionGrammar*`, etc.
"pass" without ever executing the grammar's `{$v = $a.v + $b.v;}` actions. A
real user grammar with different `$v` semantics would silently mis-parse —
violating design goal G1 ("never silently mis-parse").

That evaluator is now removed: `$rule.v`/`$rule.result` fall through to the
honest `RuleReturnValue` path, which reads a return slot the runtime *actually*
captured. When the action that would set the slot was not translated, the read
yields empty — the honest result.

This plan replaces the fakery with a real (if narrow, extensible) transpiler for
embedded actions, integrated into the existing "known transformations" pipeline
(`parse_action_template` / `--sem-patterns`), so that:

- Actions we *can* faithfully translate produce correct output.
- Actions we *cannot* fail loud under `--sem-unknown=error` (no silent guessing).

## What the corpus actually contains

Distinct embedded `{...}` action bodies across the whole runtime-testsuite
descriptor corpus, grouped by what an honest implementation requires:

### A. Already honest (real tree/token reads) — keep

- `<ToStringTree("$ctx"):writeln()>` (80), `<InputText():writeln()>` (23),
  `<writeln("$text")>`, `<writeln("$A.text")>`, `<writeln("$t.text")>`,
  `<writeln("$label.text")>` — print real tree/token text.
- `<True()>`/`<False()>` predicates, `<ValEquals>`, column predicates,
  `<InitIntMember>`/`SetMember`/`AddMember` — translated to SemIR.
- `<DumpDFA()>`, `<LL_EXACT_AMBIG_DETECTION()>`, `<Pass()>` — recognized
  no-ops / diagnostics.

These read state the runtime genuinely has; no change.

### B. Return-value assignments over arithmetic/string expressions — the real gap

- `{$v = $INT.int;}` (13), `{$v = $x.v;}` (8), `{$v = $a.v + $b.v;}` (8),
  `{$v = $a.v * $b.v;}` (8), `{$v = $e.v;}` (5), `{$v = 3;}` (4),
  `{$v = $x.v+1;}` (4), `{$v = $left.v + 1;}` (5), `{$v = $left.v - 1;}` (5)
- `{$result = $ID.text;}` (3), `{$result = <AppendStr(...)>;}` (nested string
  builders, 3+3)
- Read side: `<Result("v")>` (20), `<writeln("$e.v")>` (13),
  `<writeln("$e.result")>` (3)

These are **structured `$`-attribute expressions**, not arbitrary target code.
ANTLR itself handles them cross-target (it abstracts the `$`-refs; the operators
`= + * ;` are C-family-universal). They are the honest transpiler's core scope.

### C. Genuinely target-specific / arbitrary — must stay hook-or-fail

- `{<ContextRuleFunction(Cast("UnaryContext","$ctx"), "INC()"):Concat(...):Assert()>...}`
  — context-class casts + method probes + assertions.
- JavaScript-grammar style: `{this.ProcessOpenBrace();}`, `{this.IsStrictMode()}?`
  — arbitrary user methods maintaining custom lexer state.
- kotlin-spec `{ if (!_modeStack.isEmpty()) { popMode(); } }` — Java-specific
  method call (though the guarded-popMode idiom has a portable equivalent we
  recognize).

No general transpiler can execute these; under `error` they fail loud with a
message pointing at `--sem-patterns` / `SemanticHooks` / portable commands.

## Honest baseline (measured, fakery removed)

`summary: 341 passed, 16 failed, 0 skipped, 357 run` (`--sem-unknown` at the
default `assume-true`). The 16 failures are exactly this feature — embedded
`$`-attribute return-value actions in left-recursive rules:
`ReturnValueAndActions_1..4`, `ReturnValueAndActionsAndLabels_1..4`,
`MultipleAlternativesWithCommonLabel_1..5`, `PrefixOpWithActionAndLabel_1..3`.
Nothing else regressed; the prior "357/357" was inflated by the removed
`RuleValue` re-derivation.

## How to parse the action expression (evaluated options)

Question raised: since we are a parser framework, can we reuse ANTLR tooling to
parse `{$lhs = <expr>;}` instead of hand-rolling / regex?

- **ANTLR's `ActionSplitter`** (in the jar) parses only the `$`-attribute layer:
  its listener emits `setAttr($v, valueExpr)`, `qualifiedAttr($a.v)`, `attr($x)`,
  `text(...)`. The assignment RHS (`$a.v + $b.v`) is a single opaque
  `ATTR_VALUE_EXPR` token — **ANTLR never parses or evaluates action
  expressions**; it rewrites `$`-refs and hands the rest to the *target
  compiler* as text. There is no reusable expression parser to borrow.
- **Official ANTLR has no Rust target** (`ANTLR cannot generate Rust code as of
  4.13.2`), so `-Dlanguage=Rust` is out.
- **Self-hosting a `.g4` parser via our own toolchain**: the ANTLRv4 *parser*
  grammar generates through us cleanly (it is action-free), but the ANTLRv4
  *lexer* fails under `error` on its own category-C actions
  (`{this.handleBeginArgument();}`) — the same wall one level up. And even a
  working `.g4` parser would not yield an expression AST (see `ActionSplitter`).
- **No regex** — the codebase uses zero regex; not introducing it.

### Chosen approach: DOGFOOD — parse action expressions with our own generated parser

Correction to "no Rust target": **this repo IS the Rust target** (the jar is the
ATN frontend; `antlr4-rust-gen` + `antlr-rust-runtime` are the Rust backend). So
we can, and should, dogfood: write the action-expression sublanguage as a small
`.g4`, generate a Rust parser for it *through our own toolchain*, and walk that
parse tree when lowering to SemIR — instead of hand-rolling a precedence parser.

**Proven end-to-end** (`/tmp/actionexpr-dogfood`): the grammar below generated a
Rust lexer+parser via our toolchain under `--sem-unknown=error`, built against
our runtime, and parsed real inputs correctly with proper precedence/parens:

```
$v = $a.v + $b.v * $c.v ;   -> tree ok ($v = $a.v + ($b.v * $c.v))
$v = f($a.v, 3) ;           -> tree ok
$v = (1+2)*3 ;              -> tree ok
```

```antlr
grammar ActionExpr;
assignment : ref '=' expr ';' ;
expr : expr ('*'|'/') expr | expr ('+'|'-') expr | atom ;
atom : INT | ref | ID '(' (expr (',' expr)*)? ')' | '(' expr ')' ;
ref  : '$' ID ('.' ID)? ;
INT  : [0-9]+ ; ID : [a-zA-Z_][a-zA-Z0-9_]* ; WS : [ \t\r\n]+ -> skip ;
```

Why this over hand-rolling:
- No bespoke precedence/associativity parser to get wrong — the ATN handles it.
- Strongest dogfood: exercises our generator on left-recursion + a real
  expression grammar, so generator bugs surface on our own code first.
- The generated `action_expr_*.rs` is checked in as generated source (same
  pattern as the kotlin-parity dumper) and regenerated when the sublanguage
  grammar changes — a build step, not a runtime dependency. No new crate dep.

**Scope note — dogfooding replaces PARSING, not LOWERING.** Walking the
`ActionExpr` parse tree to (a) bind `$a`/`$b`/`$INT` refs to the alternative's
real ATN children and (b) lower `+ - * ()` / `AppendStr` into evaluatable SemIR
is still ours to write (see below). A `ref`/atom the lowering does not support
(function calls other than recognized string builders, `$ctx` casts) makes the
whole action fail loud under `error` — never a guess.

Optional: keep a tiny fallback recognizer for the trivial `{$x = <int-literal>;}`
constant case (already handled by `parse_int_return_assignment`) so the dogfooded
parser is only invoked for non-trivial expressions.

## Design: a small expression IR for `$`-attribute actions

Extend the SemIR action layer with a faithful evaluator for category B. Integer
return attributes (`returns [int v]`) already have runtime slots
(`int_return` / storage in `IntReturns`); the missing pieces are (a) *correctly
binding* `$`-refs to the right child (concern #2), (b) *computing* the value
from those children instead of the author action, and (c) for string returns, a
typed value model that does not exist yet (concern #3). The `TokenText` /
string parts of the IR below are gated behind step 5.

### 1. Parse `{$lhs = <expr>;}` into an action IR

New `parse_return_assignment_action(body)` recognizing:

```
$<ret> = <expr> ;
<expr> := <int-literal>
        | $<tokenref>.int            // token text parsed as int
        | $<tokenref>.text           // token text as string
        | $<labelOrRule>.<attr>      // another rule's captured return value
        | <expr> ('+'|'-'|'*') <expr>
        | <expr> '+' <expr>          // string concat when operands are strings
        | '(' <expr> ')'
```

Lowered to a typed `ActionExpr` enum:

```rust
enum ActionExpr {
    IntLit(i64),
    TokenInt(TokenRef),          // $INT.int
    TokenText(TokenRef),         // $ID.text
    RuleAttr(RuleRef, AttrName), // $a.v / $left.v / $e.result
    Bin(Box<ActionExpr>, BinOp, Box<ActionExpr>),
}
enum ActionStmt { SetReturn(AttrName, ActionExpr) }
```

`AppendStr(a, b)` (string builder) maps to `Bin(_, Concat, _)`.

### 2. Bind `$`-refs to the alternative's children — THE HARD PART (review concern #2)

**Correction:** an earlier draft claimed bindings can be read off
`GeneratedParserStep`s "which already track labels for rule args." They do not.
`GeneratedParserStep::{CallRule, MatchToken, Action}` carry only
`source_state` / `rule_index` / `token_type` / `follow_state` / `precedence` —
**no grammar label (`a=e`, `left=e`), no occurrence ordinal, no list-label
info** (verified at src/bin/antlr4-rust-gen.rs:1814). And the existing read path
`first_rule_int_return` is a **depth-first FIRST match by rule_index**
(src/tree.rs:64), so `$a.v` and `$b.v` in `a=e '+' b=e` would both resolve to the
*same first* `e` child. Binding by rule-name / first-match would **recreate the
fixture-fit bug behind a new parser** — the exact failure we are removing.

So binding is a first-class artifact that must be built and tested on its own,
*before* any expression lowering:

- **A per-action-state `ActionBinding` map**, computed from the owning grammar
  rule source + the selected ATN alternative, recording for each `$`-ref in the
  action: the **label** (`a`, `b`, `left`, or none), the **symbol kind** (rule
  vs token), the **occurrence ordinal** (the k-th `e` / k-th `INT` in this
  alternative), and a concrete **child/token accessor** (a positional index into
  the built subtree, not a rule-name search).
- ANTLR itself resolves `$a`/`$b` by *label*, falling back to occurrence for
  unlabeled refs. We must mirror that: labels come from the grammar rule text
  (parsed for `label=ruleref` / `label=TOKEN`); occurrence ordinals come from
  walking the alternative's element order. `left`-recursive rewrites relocate
  the logical reference (the recursive operand), so the binding must be derived
  from the *rewritten* alternative structure the ATN actually produced, not the
  surface grammar order.
- **New tree accessors** keyed by occurrence, e.g. `nth_rule_int_return(rule,
  occurrence, name)` and `nth_token(token_type, occurrence)`, replacing the
  first-match `first_rule_int_return` for bound refs.

**Binding tests are the gate for this step** (must fail before, pass after):
- `$a.v + $b.v` reads two *distinct* `e` children (not the same one twice).
- `$left.v` selects the correct recursive context under LR rewrite.
- an unlabeled `$INT.int` binds to the intended token occurrence.

### 3. Evaluate at runtime against the real parse tree

Emit an evaluator that walks the alternative's actual children (from the tree the
runtime built) via the **occurrence-keyed bindings from step 2**, and folds the
`ActionExpr`: `TokenInt` reads the bound token's text and parses it; `RuleAttr`
reads the bound child rule's captured return slot (`nth_rule_int_return`); `Bin`
folds with the real operator. The result is stored into this rule's return slot
via the existing `SetIntReturn` machinery, so `<Result("v")>` /
`<writeln("$e.v")>` read a genuinely-computed value.

Compositional: `1+2*3` works because each sub-`e`'s slot is computed by its own
alternative's action bottom-up — real left-recursion, not a hardwired re-parse.

### 4. Integrate with `--sem-unknown` / known transformations

- The `$`-assignment parser plugs into `parse_action_template`'s or_else chain,
  same as other recognized idioms. Recognized → `Translated`; the disposition
  and `run_action` codegen flow unchanged.
- An `{$lhs = <expr>;}` whose `<expr>` uses an unsupported construct (category C:
  casts, method calls, `$ctx` navigation) does **not** match → falls to
  `UnsupportedAction` → fails loud under `error`, honestly. A ref that cannot be
  bound (step 2 fails) is likewise a hard fail, never a first-match guess.
- `--sem-patterns` can still override any coordinate to `hook`/`assume-*`.

### 5. String returns need a typed value model FIRST (review concern #3)

**Correction:** an earlier draft treated `$result = $ID.text` /
`AppendStr(...)` as "a small parallel string-return slot." That is not
sufficient. The current SemIR value model is integer-only:
`semir::Value = Null | Bool | Int` (src/semir.rs:222); text nodes evaluate to
`Null` outside comparisons (src/semir.rs:384); `AStmt::SetReturn` coerces through
`int_or_zero` (src/semir.rs:361); return storage is `BTreeMap<String, i64>`
(src/tree.rs:197). Lowering `$ID.text` or concatenation through this today would
silently become `0`/empty — another silent-wrong path.

String support therefore requires, *in order*:
1. Extend `semir::Value` with a `Text(...)` variant and give `eval_value` a real
   text path (not `Null`) for token-text / concat expressions.
2. Add typed action-evaluation semantics + a string return slot
   (`set_string_return` / `nth_rule_string_return`) parallel to the int slot.
3. Only then lower `$result = $ID.text` and `AppendStr(...)` chains.

This is a distinct, larger change than the integer path and is sequenced after
it — do not claim `$result`/`AppendStr` support until the value model lands.

## Non-goals / explicit failures

- No general target-language interpreter. Category C stays fail-loud/hook.
- No re-parsing matched text to guess a value (the removed anti-pattern).
- The hardcoded `ListenerWalk` fixture bodies are a separate cleanup: they walk
  the real tree but hardwire specific descriptor output formats. Decide
  separately whether to generalize (real listener API) or drop + fail those
  descriptors.

## Sequencing

0. **Finish the fakery removal (review concern #1) — prerequisite, blocks a
   clean commit.** Routing is changed, but the dead `RuleValue` variant,
   `RuleValueKind`, and `render_rule_value_write` remain, and the stale
   `parses_rule_value_print_template` test **fails** in the working tree today.
   Delete all three (fixing the `RuleValue { .. }` match sites at ~793 / ~6440 /
   ~8043 / ~8254 / ~8388) and replace the stale test with a regression proving
   `writeln("$e.v")` / `writeln("$e.result")` parse as `RuleReturnValue` and no
   longer re-evaluate matched text. Full `cargo test` + clippy green, then commit
   the honest-baseline change (341/357).
1. (done) Measure honest baseline: `341 passed, 16 failed, 0 skipped`.
2. **Dogfood the parser** (proven): commit the `ActionExpr.g4` + its
   generated-and-checked-in Rust parser (built via our own toolchain).
3. **Binding artifact + occurrence-keyed tree accessors (concern #2)** — the
   core risk. Build `ActionBinding` per action state (label / symbol kind /
   occurrence ordinal / accessor) and `nth_rule_int_return` / `nth_token`. Land
   the binding tests (distinct-children, `$left` under LR, unlabeled occurrence)
   FIRST, red→green, independent of expression evaluation.
4. **Integer `ActionExpr` lowering + evaluation** over the bindings. Re-run
   suite: `ReturnValueAndActions_1..4`, `PrefixOpWithActionAndLabel_1..3`, and
   the integer `MultipleAlternativesWithCommonLabel` cases pass *for real*.
5. **Typed value model (concern #3)**: add `semir::Value::Text`, real text
   eval, string return slot. Then lower `$result` / `AppendStr`; the remaining
   string-valued LR descriptors pass.
6. Audit `ListenerWalk` + any remaining fixture bodies; generalize or fail.
7. Only then revisit flipping `--sem-unknown` default to `error`, with the honest
   pass count as the gate (category-C descriptors are documented opt-outs / hook
   cases, not fakery).

## Risk / verification

- Every step gated by the full 357 sweep AND clippy/`cargo test`.
- The transpiler must never *lower confidence*: a partially-recognized `<expr>`
  must fail the whole action (fail loud), never emit a best-effort guess.
- Cross-check a handful of category-B outputs against real ANTLR Java output to
  confirm semantics match (e.g. `(1+2)*3 => 9`).
