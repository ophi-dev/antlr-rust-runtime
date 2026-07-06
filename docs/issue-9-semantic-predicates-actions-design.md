# Issue #9 — Semantic predicates & actions: system design and implementation plan

Status: draft design (2026-07-04)
Scope: `antlr4-rust-gen`, runtime predicate/action execution, hook API surface
Issue: <https://github.com/ophidiarium/antlr-rust-runtime/issues/9>

## 0. Where we actually are (the issue predates the current code)

Issue #9 was filed against a runtime that ignored predicates/actions. The
codebase has since grown a working — but *closed* — heuristic system:

| Layer | What exists today | Where |
|---|---|---|
| Grammar-source scraping | Brace/template matcher that finds action & predicate blocks in `.g4` text, in ANTLR serialization order | `src/bin_support/templates.rs` |
| Generator-side classification | Closed enums `ActionTemplate` (~20 variants), `PredicateTemplate` (~16 variants), `RuleArgTemplate`, `IntMemberTemplate`; per-variant recognizers | `src/bin/antlr4-rust-gen.rs` |
| Runtime predicate table | `ParserRuntimeOptions { predicates: &[(rule, pred, ParserPredicate)], rule_args, member_actions, return_actions }` — static data interpreted at parse/prediction time | `src/parser.rs:305-442` |
| Lexer hook surface | `next_token_with_hooks(lexer, atn, custom_action, semantic_predicate, accept_adjuster)` — closure hooks; generator renders `run_action` / `run_predicate` match arms into the generated lexer | `src/atn/lexer.rs:199`, gen `render_lexer_predicate_method` |
| Prediction semantics | `SemanticContext` (And/Or/Predicate) collected during closure, "action hides predicates" rule, `has_semantic_context` plumbed through DFA states | `src/prediction.rs:839`, `src/atn/parser.rs:1186` |
| Fail-loud (lexer only, codegen-time) | Unsupported lexer action templates are a codegen **error** unless `--allow-unsupported-lexer-actions` | gen `lexer_action_templates` |

This passes the full upstream conformance sweep (357/357), so the *semantics*
of predicate collection, evaluation order, and speculative member state are
already correct for the covered shapes.

### What is still wrong / missing

1. **Silent-true fallback in the parser.** A predicate coordinate absent from
   the table evaluates to `true` (`BaseParser::parser_predicate_matches`,
   `src/parser.rs:7016`). This is exactly the silent correctness bug the issue
   warns about, and there is no parser-side codegen error either.
2. **Closed-world extensibility.** Supporting a new grammar (say
   `grammars-v4/javascript`) means writing a new recognizer function + enum
   variant in the generator *and* a matching variant + evaluator in the
   runtime, in Rust, per idiom. The marginal cost per grammar is high and the
   enums accrete testsuite-specific noise (`Invoke { value }` prints
   `eval=...` to stdout).
3. **No user escape hatch on the parser side.** The lexer takes closures; the
   parser only takes static tables. A user who *can* write the predicate in
   Rust today has nowhere to put it.
4. **No machine-readable compatibility report.** Users can't ask "which
   predicates/actions in my grammar did the generator understand, and what did
   it do with the rest?"

## 1. Design goals

- **G1 — Never silently mis-parse.** Every predicate/action coordinate is
  accounted for: translated, hooked, or an explicit, configurable policy
  (error / assume-true / assume-false) chosen by the user *knowingly*.
- **G2 — Heuristics as data, not code.** Recognizing grammar idioms should be
  a *pattern library* consulted by one generic translator, extensible without
  editing generator source — this is the "most flexible heuristic" core.
- **G3 — One IR, three producers.** Built-in heuristics, user-supplied
  pattern files, and (later) a real Rust target all lower to the same small
  semantic IR the runtime evaluates. The existing closed enums become library
  entries, not parallel mechanisms.
- **G4 — Zero cost when unused.** Grammars without predicates/actions keep the
  compiled-DFA lexer path and the current parser hot loop untouched.
- **G5 — Prediction-safe.** Predicates evaluate speculatively during adaptive
  prediction; the design must keep them side-effect-free and keep action
  effects transactional, preserving ANTLR's collection/deferral semantics that
  the conformance suite already locks in.

## 2. Architecture overview

Four layers, ordered by how much they know about the grammar. Each layer
falls back to the next; the *policy* layer guarantees the fallback chain
terminates loudly instead of silently.

```text
.g4 source + .interp
        │
        ▼
┌───────────────────────────────────────────────┐
│ L1  Heuristic translator (codegen time)       │
│     normalizer → matcher → SemIR              │
│     pattern library: built-ins + user TOML    │
└──────────────┬────────────────────────────────┘
               │ untranslatable coordinates
               ▼
┌───────────────────────────────────────────────┐
│ L2  Hook dispatch (runtime)                   │
│     numeric SemanticHooks trait +             │
│     generated typed trait per grammar        │
└──────────────┬────────────────────────────────┘
               │ unhooked coordinates
               ▼
┌───────────────────────────────────────────────┐
│ L0  Policy: Error (default) | AssumeTrue |    │
│     AssumeFalse — per coordinate or global    │
└───────────────────────────────────────────────┘

L3 (long-term): full Rust ANTLR target emits SemIR or native Rust directly;
    out of scope here but the IR is designed to be its backend.
```

Manifest: the generator always emits a `semantics.json` (and a Rust const
mirror) listing every `(rule, pred|action)` coordinate, its grammar source
span, the raw body text, and its disposition (`translated(ir)`, `hooked`,
`policy(...)`). This is the compatibility boundary made explicit and testable.

## 3. Layer 0 — Accountability and policy (do this first)

Smallest change, biggest correctness win.

### 3.1 Runtime policy

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnknownSemanticPolicy {
    /// Return a FailedPredicate-style error naming the coordinate. Default.
    #[default]
    Error,
    AssumeTrue,   // current behavior, now opt-in
    AssumeFalse,
}
```

- Add `unknown_predicate_policy` / `unknown_action_policy` to
  `ParserRuntimeOptions` and to the lexer hook wrappers' default closures
  (today `|_, _| true`).
- `parser_predicate_matches` (src/parser.rs:7016): the `else` branch consults
  the policy instead of hardcoding `true`. Error shape follows the issue:

  ```text
  unsupported semantic predicate: rule=expr(12) pred=3 at Grammar.g4:57:9
    body: {isTypeName()}?
  ```

  Source spans come from the manifest (the scraper already knows
  `open_brace` offsets; add a line/col mapping).
- Predicates evaluated *during prediction* can't return `Result` cheaply; on
  `Error` policy the predicate evaluates to `false` in prediction and the
  coordinate is recorded on the parser, then surfaced as a hard error when the
  committed path crosses it (mirrors ANTLR: prediction-time failures just
  kill an alternative; parse-time failures throw `FailedPredicateException`).

### 3.2 Codegen strictness

- Parser side gets what the lexer side already has:
  untranslatable predicate/action → codegen **error** by default, with
  `--sem-unknown=error|hook|assume-true|assume-false` (global) and per-
  coordinate overrides in the pattern file (§5.3). `hook` means "leave it to
  the generated hook trait" (§6).
- `--allow-unsupported-lexer-actions` becomes an alias for
  `--sem-unknown=assume-true` scoped to lexer actions (kept for compat).

## 4. The semantic IR (`SemIR`)

The pivot from "enum variant per idiom" to "small language, many idioms".
Lives in a new `src/semir.rs`; both the generator (producer) and runtime
(evaluator) depend on it.

### 4.1 Expressions (predicates — pure by construction)

```rust
pub enum PExpr {
    Bool(bool),
    Int(i64),
    Str(StrId),                       // interned in a per-grammar pool
    // Recognizer state (read-only views)
    La(i8),                           // parser: token type at LT(k); lexer: char LA(k)
    TokenField(i8, TokenField),       // text/line/column/channel/index of LT(k)
    Column,                           // lexer: current char position in line
    TokenStartColumn,                 // lexer
    TextSince(TextAnchor),            // lexer token text so far, parser rule text
    LocalArg,                         // rule argument (existing ParserRuleArg plumbing)
    Member(MemberId),                 // declared state slot
    StackTop(MemberId),
    StackDepth(MemberId),
    CtxChildRuleText(usize),          // existing ContextChildRuleTextNotEquals need
    TokenIndexAdjacent,               // existing TokenPairAdjacent need
    // Combinators
    Not(ExprId), And(Box<[ExprId]>), Or(Box<[ExprId]>),
    Cmp(CmpOp, ExprId, ExprId),      // Eq Ne Lt Le Gt Ge
    Arith(ArithOp, ExprId, ExprId),  // Add Sub Mul Div Mod
    // Escape hatch: defer to hook at runtime
    Hook(HookId),
}
```

Storage is a flat arena (`Vec<PExpr>` + `ExprId = u32` indices), not boxed
trees: cache-friendly, trivially serializable into generated code as a
`const` table, and cheap to evaluate iteratively. `And`/`Or` short-circuit.

### 4.2 Statements (actions — effects, committed-path only unless flagged)

```rust
pub enum AStmt {
    SetMember(MemberId, ExprId),
    AddMember(MemberId, ExprId),      // covers existing ParserMemberAction deltas
    Push(MemberId, ExprId), Pop(MemberId),
    SetReturn(RetId, ExprId),         // covers ParserReturnAction
    // Lexer built-ins already handled by LexerAction stay there; these are
    // the *custom-action* bodies:
    LexerSetType(ExprId), LexerSetChannel(ExprId), LexerSkip,
    LexerPushMode(ModeId), LexerPopMode, LexerSetMode(ModeId),
    Emit(EmitKind),                   // diagnostics-style writeln templates
    Hook(HookId),
    Seq(Box<[StmtId]>),
    If(ExprId, StmtId, Option<StmtId>),
}
```

Every `AStmt` carries a `speculative: bool` classification computed at
codegen: an action is speculative-eligible iff it only mutates the member
environment (the existing `can_run_inline` predicate on `ActionTemplate`
generalizes to this). Speculative-eligible actions run during prediction so
same-rule predicates observe them — exactly the semantics `ParserMemberAction`
implements today via `member_values: BTreeMap<usize, i64>` threading; that
map generalizes to a `MemberEnv` (small copy-on-write vector of `Value`s)
with the same snapshot/rollback discipline.

### 4.3 Evaluation contexts

Two thin views over recognizer internals so hooks and IR share one interface
and borrows stay simple:

```rust
pub struct LexerSemCtx<'a> { /* char stream cursor, token start, mode stack, column, text-so-far accessor */ }
pub struct ParserSemCtx<'a> { /* token stream (seek/lt/la), rule ctx view, local arg, member env */ }
```

`BaseParser::parser_predicate_matches` becomes
`semir::eval_pred(ir, expr_id, &mut ParserSemCtx, hooks)` — the existing
`PredicateEval` struct is already 90 % of `ParserSemCtx`.

### 4.4 Existing enums lower into SemIR

Each `ParserPredicate` / `ActionTemplate` variant is expressible:

| Today | SemIR |
|---|---|
| `LookaheadTextEquals { offset, text }` | `Cmp(Eq, TokenField(offset, Text), Str(s))` |
| `MemberModuloEquals { m, k, v, eq }` | `Cmp(eq, Arith(Mod, Member(m), Int(k)), Int(v))` |
| `TokenPairAdjacent` | `TokenIndexAdjacent` |
| `ColumnGreaterOrEqual(n)` (lexer) | `Cmp(Ge, Column, Int(n))` |
| `SetIntReturn { name, value }` | `SetReturn(r, Int(value))` |
| C# split-token, Kotlin `NL` checks | library patterns (§5.2) |

Migration keeps the public `ParserPredicate` table constructor as a
deprecated adapter that lowers into IR, so existing generated crates keep
compiling for one release cycle.

## 5. Layer 1 — the heuristic translator (codegen)

Three stages, each independently testable.

### 5.1 Normalizer: many target syntaxes → one canonical form

Action bodies in the wild are Java, Python, Go, C#, JS, or C++ — but the
subset that appears in predicates is overwhelmingly a C-like expression
sublanguage. The normalizer is a tiny tokenizer + Pratt parser (not a full
target-language parser) that produces a `SurfaceExpr` AST:

- Strip/record receiver prefixes: `this.`, `self.`, `p.`, `l.`, `$`,
  `_input.`, `_localctx.` — mapped to canonical roots (`member`, `input`,
  `ctx`, `arg`).
- Canonicalize call names across targets:
  `getCharPositionInLine()` = `self.column` = `l.GetCharPositionInLine()`
  → `input.column`. A built-in alias table covers the ANTLR runtime API
  surface (LA, LT, getText, more).
- Literals, `! && || == != < <= > >= + - * / %`, parenthesization, ternary
  → `If`.
- Anything unrecognized becomes an opaque `Unknown(call_name, args)` node —
  *not* a failure yet; the matcher may still resolve it.

This stage subsumes today's per-shape string matching (`ValEquals("$i",…)`
etc.), which becomes a handful of built-in patterns instead of bespoke code.

### 5.2 Matcher + pattern library

A pattern maps a `SurfaceExpr` shape (with metavariables) to a SemIR
template. Patterns come from three sources, later sources overriding earlier:

1. **Built-ins** (in the generator binary): everything the closed enums cover
   today, including the conformance-suite `writeln`/`Invoke` templates and the
   real-grammar idioms (C# `TokenPairAdjacent`, Kotlin lookahead checks,
   Python-style column predicates).
2. **Grammar-family library** (`patterns/*.toml` shipped in-repo): curated
   entries for popular grammars-v4 grammars (JavaScript's
   `notLineTerminator`, `isRegexPossible`; Python3 indent/dedent helpers).
   These encode *whole named helper methods from `@members`*, keyed by
   method name + grammar fingerprint, since the method body itself is often
   too gnarly to translate but its *meaning* is well known and finite.
3. **User pattern file** (`--sem-patterns my-grammar.toml`): the flexibility
   valve. Users teach the generator idioms without forking it.

Pattern file schema (TOML):

```toml
version = 1

# Expression-level rewrite: metavariables $a, $b bind SurfaceExpr subtrees.
[[pattern]]
match   = "input.la(1) != $t"            # canonical surface syntax
where   = { t = "token_ref" }            # guards on metavariable kind
lower   = "cmp(ne, la(1), token($t))"    # SemIR constructor DSL

# Named-helper resolution: the grammar calls a @members method; translate
# calls to it as if the body were this expression.
[[helper]]
name    = "notLineTerminator"
returns = "bool"
lower   = "not(cmp(eq, token_field(-1, channel), int(1)))"   # example

# Per-coordinate escape: don't translate, route to hook / policy.
[[coordinate]]
kind    = "predicate"      # or "action"
rule    = "expr"           # rule name (resolved via .interp rule_names)
index   = 0
dispose = "hook"           # hook | assume-true | assume-false | error
```

The `lower` DSL is a direct textual constructor for §4 IR — deliberately not
Turing-complete. Guards (`where`) keep matches honest. Match ambiguity
(two patterns match one body) is a codegen error listing both patterns.

### 5.3 Verifier + manifest emission

- **Constant folding**: `Bool(true)` predicates drop to no-ops; the many
  always-true testsuite predicates cost nothing at runtime.
- **Purity check**: any `AStmt`-producing pattern matched in predicate
  position is a codegen error (G5).
- **Speculation classification**: mark member-only actions speculative
  (§4.2); everything else defers to the committed path, and the existing
  "action hides predicates" collection rule (`src/atn/parser.rs:1186`)
  continues to govern what prediction may see.
- **Manifest**: `semantics.json` next to the generated code +
  `pub const SEMANTICS: …` table. Each entry: coordinate, source span, raw
  body, disposition, pattern id that matched (provenance for debugging).
  `--require-full-semantics` fails codegen if any entry's disposition is a
  policy fallback — CI-friendly.

## 6. Layer 2 — hook traits (runtime escape hatch)

### 6.1 Numeric trait (issue Option 2)

```rust
pub trait SemanticHooks {
    fn sempred(&mut self, ctx: &mut ParserSemCtx<'_>, rule: usize, pred: usize) -> Option<bool> { None }
    fn action (&mut self, ctx: &mut ParserSemCtx<'_>, rule: usize, action: usize) -> bool { false } // handled?
    fn lexer_sempred(&mut self, ctx: &mut LexerSemCtx<'_>, rule: usize, pred: usize) -> Option<bool> { None }
    fn lexer_action (&mut self, ctx: &mut LexerSemCtx<'_>, rule: usize, action: usize) -> bool { false }
}
```

`Option<bool>` / handled-`bool` returns make the fallback chain explicit:
`None`/`false` falls through to L0 policy. Dispatch order at a coordinate:
SemIR (if translated) → `Hook(id)` nodes or untranslated coordinates → user
hooks → policy.

State ownership: the hook object is user state. `BaseParser` gains a
`hooks: H` generic defaulting to `NoHooks` (unit struct), mirroring how
`BaseLexer<I, F>` already threads generated closures — no `dyn`, no `Any`,
zero cost when unused (G4). The generated wrapper exposes
`Parser::with_hooks(input, hooks)`.

Speculation warning, documented loudly: `sempred` may be called during
prediction on paths that are later abandoned, possibly multiple times per
coordinate. Hooks must be pure w.r.t. observable parse state; mutable
*user* state is allowed but the user owns replay-safety (same contract as
every official ANTLR target).

### 6.2 Generated typed trait (issue Option 3)

When a predicate body normalizes to a bare helper call (`isTypeName()`,
`this.n > 0` does *not* qualify), the generator emits:

```rust
pub trait MyGrammarHooks: Sized {
    fn is_type_name(&mut self, ctx: &mut ParserSemCtx<'_>) -> bool;
    // one method per distinct helper name reached from predicates
    fn custom_action(&mut self, ctx: &mut ParserSemCtx<'_>, rule: usize, action: usize) {}
}
```

plus a blanket `impl<T: MyGrammarHooks> SemanticHooks for Typed<T>` adapter
that maps coordinates → named methods (the mapping table comes straight from
the manifest, so it is stable and inspectable). Index fragility disappears
for users; the numeric trait remains for the general case.

## 7. Performance notes (guarding the parse-bench wins)

- Predicate-free grammars: no IR tables, `NoHooks`, compiled lexer DFA path
  untouched — the `has_semantic_context` short-circuits already in place
  (`src/dfa.rs:163`, `src/lexer.rs:234`) keep gating everything (G4).
- IR evaluation is an iterative walk over a flat arena with short-circuit
  And/Or; no allocation. For the hot case (single comparison) it's a couple
  of array reads — comparable to today's enum match.
- `MemberEnv` replaces `BTreeMap<usize, i64>` with an indexed
  `SmallVec<[Value; 4]>` (member ids are dense, generator-assigned) — this is
  likely a *speedup* over the current map threading in `recognize_state_fast`.
- Lexer predicates force the ATN interpreter path per current design
  (`next_token_compiled_with_hooks` re-runs interpreter for predicate-bearing
  modes); unchanged. Constant-folded `true` predicates are dropped at codegen
  so they no longer force the slow path (small win available today).
- Measure with `tools/parse-bench` (Kotlin + C# fixtures; C# exercises
  `TokenPairAdjacent` on the IR path) and the perf-counters feature before/
  after each phase.

## 8. Testing strategy

1. **Conformance**: full 357-descriptor sweep must stay green at every phase;
   the suite is the strongest regression net for prediction-time semantics
   (SemanticContext collection, action-hides-predicates, fail-message
   predicates).
2. **SemIR unit tests**: golden eval tests per node kind; property test that
   `And`/`Or` short-circuit order matches ANTLR (left-to-right).
3. **Lowering equivalence**: for every legacy enum variant, a test asserting
   the lowered IR evaluates identically to the old evaluator across a state
   corpus (differential harness inside `src/parser.rs` tests, then delete the
   old evaluator).
4. **Pattern-file tests**: fixture grammars + TOML files under
   `tests/sem-patterns/`; assert manifest dispositions and generated-code
   snapshots.
5. **Real-grammar parity**: extend the kotlin-parity approach with one
   actionful grammar (JavaScript from grammars-v4 is the canonical stress
   case: `IsRegexPossible`, `notLineTerminator`, lexer mode state) — dump
   trees against `antlr4-python3-runtime` byte-for-byte, hooks implemented
   once in Rust via the typed trait.
6. **Fail-loud tests**: grammars with untranslatable predicates must fail
   codegen with the documented error shape; `AssumeTrue` policy must
   reproduce today's behavior bit-for-bit.

## 9. Phased implementation plan

Phases are independently shippable; each ends green on conformance + bench.

### Phase 1 — Accountability (S, ~1 PR) — ✅ implemented 2026-07-05

- Manifest emission (coordinates, spans, raw bodies, disposition) from data
  the scraper already has. → `semantics.json` written on every generator run.
- `UnknownSemanticPolicy` in `ParserRuntimeOptions` + lexer default-closure
  wiring; kill the silent `true` (default stays `AssumeTrue` for one release
  with a deprecation note in the manifest, flips to `Error` next minor).
  → `src/parser.rs` `unknown_predicate_result` / `unknown_semantic_error`;
  the lexer side is handled at codegen (assume-false emits an `|_, _| false`
  predicate hook and forces the hook-taking token path).
- Parser-side codegen strictness flag `--sem-unknown`.
- Docs: compatibility-boundary section in README (issue acceptance criterion).

### Phase 2 — Parser hooks (M, ~1 PR) — ✅ implemented 2026-07-05

- `ParserSemCtx` view for user hooks; it exposes lookahead, local integer
  args, member snapshots, committed action text, and the completed action tree.
- `SemanticHooks` trait, `hooks: H = NoSemanticHooks` generic on `BaseParser`,
  and `with_hooks` constructors in generated parser wrappers.
- Dispatch chain for parser predicates: generated metadata → hook → policy.
  Parser action events without a generated arm fall through to
  `SemanticHooks::action` on the committed path.

Implemented follow-up:
- `LexerSemCtx` plus `next_token_with_semantic_hooks` /
  `next_token_compiled_with_semantic_hooks` re-express lexer closure hooks
  through the same `SemanticHooks` trait.
- Generated typed hook traits (§6.2) for bare helper-call predicates.

### Phase 3 — SemIR core + enum lowering (M/L, ~2-3 PRs) — ✅ implemented 2026-07-05

- ✅ `src/semir.rs`: arena, `PExpr`/`AStmt`, evaluator, hook nodes, and unit
  tests for lookahead text, token adjacency, member/local predicates,
  short-circuiting, lexer column/text predicates, and action execution.
- `ParserPredicate`, `ParserMemberAction`, and `ParserReturnAction` lower to
  SemIR; generated parsers now emit a `parser_semantics()` SemIR table consumed
  by both generated-direct predicate checks and the interpreter fallback.
- The public legacy enum/table surface remains as a deprecated compatibility
  adapter for older generated modules.

### Phase 4 — Heuristic translator (L, ~3 PRs) — ◐ partially implemented 2026-07-05

- Built-in recognizers now lower through SemIR instead of remaining a parallel
  parser enum execution path.
- TOML pattern/helper/coordinate file loading via `--sem-patterns`; exact
  predicate rewrites and bare helper-call rewrites lower into SemIR or hooks.
- Ambiguity detection for matching user patterns and `--require-full-semantics`
  to fail CI on policy fallbacks.
- Grammar-source block scanning skips quoted literals/comments/charsets
  (`find_significant_open_brace`), so span/hook pairing survives real grammars
  whose rules reference brace tokens (`'{' statementList? '}'`).
- Still open: the §5.1 normalizer (tokenizer + Pratt parser over canonical
  surface syntax) and argument-capturing patterns. Today's matching is
  exact-body and bare no-arg helper calls, so idioms like JavaScript's
  `this.n("static")` (next-token-text-equals with an argument) cannot be
  mapped yet.

### Phase 5 — Typed hooks + real-grammar proof (M) — ◐ partially implemented 2026-07-05

- Typed trait generation + blanket adapter (`MyParserHooks` and
  `MyParserTypedHooks<T>`) for bare helper-call predicates.
- Grammar-family pattern file added at `patterns/javascript.toml` for common
  JavaScript helper predicates, routing them to hooks without adding
  JavaScript-specific logic to the generator. Verified against grammars-v4
  `JavaScriptParser.g4`: 11 of 16 predicate coordinates route to the typed
  hook trait; the remaining 5 are the argument-taking `n("...")` helpers
  (see Phase 4 gap).
- Lexer-side hooks: `next_token_with_semantic_hooks` exists as a manual
  facade, but generated lexers have no hook plumbing — a hook-routed lexer
  predicate is a codegen error, not a panic.
- Still open: a JavaScript parity fixture (trees vs `antlr4-python3-runtime`
  with hooks implemented in Rust) wired into CI.

### Phase 6 — (separate milestone, out of scope) Rust target

- A real ANTLR `RustTarget` emitting native predicate/action code. SemIR and
  the hook traits are its compatibility layer: grammars generated the
  metadata-first way and the target way share runtime semantics. Tracked
  separately per the issue's own suggestion.

## 10. Risks and mitigations

- **Heuristic mistranslation is worse than no translation.** Mitigation:
  patterns are precise (guards, ambiguity errors), provenance lands in the
  manifest, and parity fixtures pin real grammars. Default for *unmatched*
  is loud, never guessed.
- **IR scope creep toward a full language.** Mitigation: the IR only grows a
  node when a pattern in the curated libraries needs it; arbitrary code
  belongs in hooks (that's what they're for).
- **Generic `hooks: H` ripples through `BaseParser` signatures.** Mitigation:
  default type parameter keeps existing call sites source-compatible; the
  generated wrappers are regenerated anyway.
- **Prediction-time hook calls surprise users** (called speculatively,
  repeatedly). Mitigation: documented contract identical to upstream ANTLR
  targets; `ParserSemCtx` exposes no mutation of parse state, so the damage
  radius is user state only.
- **Conformance suite couples to legacy templates** (`Invoke` printing
  `eval=…`). Mitigation: keep testsuite-only patterns in a builtin group
  tagged `testsuite`, excluded from user-facing docs and from the typed-trait
  generator.
