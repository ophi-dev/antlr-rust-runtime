# Issue #151: mutual (indirect) left-recursion support

Status: design accepted, implementation in progress
Prepared: 2026-07-26
Repository baseline: `7eb93072c4fd551db8ce7550e43c35f080f197c8`
Issue: <https://github.com/ophi-dev/antlr-rust-runtime/issues/151>
Validation target: `dotnet/roslyn` `CSharp.Generated.g4`

## 1. Executive decision

Accept the class of mutually-left-recursive grammars that ANTLR 4.13.2 rejects
with `error(119)` by **rewriting them, before ATN construction, into an
equivalent grammar that uses only direct left-recursion** — the form our
existing precedence machinery (`rewrite_immediate_left_recursion`) and ANTLR
itself already handle. This is approach (1) from the issue ("transform to an
equivalent accepted form"), chosen over approach (2) ("handle the cycle directly
in prediction") because it is provably correct by construction and requires zero
new runtime surface.

The transform is **left-corner substitution** (informally, "hub inlining"): for
each left-recursive cycle, one rule is designated the *hub*; every other cycle
member (a *satellite*) has its body substituted into the hub's left-corner
positions until the hub is directly left-recursive, after which the satellite
either disappears (if it was referenced only from within the cycle) or is
retained as a non-recursive copy (if referenced externally).

Two hard sub-cases are handled explicitly rather than guessed:

- **Leading-optional recursion** (`hub? rest`) is expanded to `hub rest | rest`
  before substitution — a union-preserving rewrite that ANTLR accepts.
- **Anything the direct-LR rewriter still cannot classify** after substitution
  (recursive calls carrying arguments, mixed labeled/unlabeled alternatives,
  ambiguous cycles with no non-recursive exit) is **rejected with a precise
  diagnostic** naming the exact rule and reason — strictly better than ANTLR's
  generic `error(119)`. This is staged acceptance criterion 1, and it ships
  even for grammars we decline to transform.

The correctness oracle is ANTLR itself: because the transform emits precisely
the direct-LR grammar we would otherwise hand to ANTLR, and our direct-LR
handling is already conformance-verified byte-for-byte against ANTLR's runtime,
tree-equality on the transformed grammar is both provable and differentially
testable.

This does **not** touch direct-left-recursion handling, the runtime, prediction,
or the ATN interpreter. It is a model-level grammar transform confined to
`src/bin_support/grammar/`.

## 2. Grounded current state

The generator already has every piece this builds on:

- **A mutual-left-recursion *detector*.** `indirect_left_recursive_components`
  (`src/bin_support/grammar/atn/analysis.rs:250`) builds a left-corner graph
  over the finalized ATN, runs Tarjan SCC, and emits `G4A005`
  ("mutually left-recursive rules: [...]"). This is ATN-level and fires *after*
  ATN construction. It stays as the backstop for cycles the new pass declines.
- **A direct-left-recursion *rewriter*.** `rewrite_immediate_left_recursion`
  (`src/bin_support/grammar/left_recursion.rs:11`) classifies each alternative
  of a directly-LR rule as Primary/Prefix/Binary/Suffix, rebuilds the rule into
  the precedence-climbing `primary (operator)*` shape with precedence
  predicates, and records `LeftRecursionInfo` for downstream codegen. It runs in
  `analyze` (`src/bin_support/grammar/semantics.rs:90`), on the model, before
  semantic analysis and ATN build.
- **A model-level transform boundary.** `TransformRegistry`
  (`src/bin_support/grammar/transform.rs:88`) runs ordered `GrammarTransform`
  passes over the integrated model with analysis invalidation
  (`AnalysisInvalidation::{NAMES,CALLS,NULLABILITY,...}`) and `validate_model`
  after each mutating pass. The default registry is currently empty.
- **Model-level reachability analysis.** `TransformAnalysis`
  (`src/bin_support/grammar/transform_analysis.rs:29`) already computes
  `call_graph`, `nullable`, and `recursive_components` over the model
  (not the ATN). This is exactly the input the new pass needs, at exactly the
  right phase.

The new pass slots between "integrate/split" and
`rewrite_immediate_left_recursion`: it turns indirect LR into direct LR, then
the existing rewriter does the rest.

```text
integrate + combined split
  -> [NEW] mutual-left-recursion elimination (this pass)
  -> rewrite_immediate_left_recursion   (unchanged)
  -> semantic analysis, numbering
  -> ATN construction + G4A005 detector  (unchanged backstop)
  -> Rust emission
```

## 3. The tractable subclass, stated precisely

Let the **left-corner relation** be: rule `A` left-calls `B` if some alternative
of `A` can reach a reference to `B` through only nullable/epsilon elements
(actions, predicates, optional/star quantifiers, and nullable rule calls) before
consuming any token. A **left-recursion cycle** is a strongly-connected
component (size > 1, or a self-loop) of this relation. This is the same relation
both existing analyses use.

We **accept** a cycle when, after choosing a hub and performing left-corner
substitution, every resulting alternative of the hub is one the direct-LR
rewriter classifies as Primary, Prefix, Binary, or Suffix — i.e. its left corner
is either non-recursive (Primary/Prefix) or a single non-optional reference to
the hub (Binary/Suffix). Empirically (§4) this covers:

- **Hub-and-spoke cycles** — one hub lists satellites as plain alternatives;
  each satellite's left corner is the hub. All four Roslyn cycles are this
  shape. Satellites collapse into the hub directly.
- **Chained/indirect cycles** — a satellite's left corner is *another*
  satellite, not the hub (`a : b ... ; b : a ...`). Resolved by transitive
  substitution: substitute along the left-corner chain until the reference
  reaches the hub. Genuinely mutual `a <-> b` collapses this way too.
- **Multi-alternative satellites** — a satellite with several alternatives
  contributes each alternative to the hub.

We **decline** — leaving the grammar bit-for-bit unchanged, so the existing
ATN-level `G4A005` cycle diagnostic reports it exactly as it does today — when
any of the following holds. Each is a precondition checked *before* the model is
touched, and each has a dedicated decline test:

- The grammar is a **lexer grammar**. Precedence rewriting is a parser-rule
  construct; routing lexer rules through it produced an "unsupported embedded
  lexer action" naming an action the grammar never declared. (Left-recursive
  lexer rules are invalid in ANTLR regardless — `error(119)` — and diagnosing
  them properly is tracked as issue #236.)
- The cycle has **no token-consuming base alternative** — the language is
  ill-founded. Mirrors ANTLR's `error(169)` for the immediate case.
- A corner to be substituted is **not bare**: quantified (`b*`, `b+`),
  **labelled** (`x=b`), or **argument-bearing** (`b[3]`). Splicing one satellite
  body in place of `b*` would drop the closure and change the accepted language;
  removing a labelled corner would leave `$x` dangling in surviving actions.
- The corner sits **behind a nullable prefix** (`a : n b`, `n :`), so which rule
  the author meant as the left corner is genuinely ambiguous. We decline rather
  than guess.
- A **satellite carries rule-level state** inlining would silently drop:
  `arguments`, `returns`, `locals`, `throws`, `@init`/`@after`, `catch`,
  `finally`, rule options, or `#`-labelled alternatives. The last also avoids
  ANTLR's all-or-none alternative-label rule (`error(122)`); synthesizing labels
  for spliced satellites remains deferred (§8).
- Splicing would **merge two overlapping label scopes** (caller and satellite
  both bind `x`), which would rebind caller actions to the satellite's element.
- The resulting hub is not **Primary/Prefix/Binary/Suffix** throughout, or
  substitution does not terminate within a bound (defensive; see §6).

Because these are preconditions rather than post-hoc repairs, the pass either
emits a grammar the conformance-verified direct-recursion path accepts, or it
changes nothing — it can never accept-and-miscompile.

### 3.1 The leading-optional wrinkle

C#'s range operator is `range_expression : expression? '..' expression?`. Its
**leading** `expression?` is an *optional* left-recursive reference. The direct-LR
rewriter (and ANTLR) require the recursive left corner to be non-optional, so
this alternative alone re-triggers `error(119)` even after the rest of the
`expression` cycle collapses.

The fix is a standard, union-preserving expansion applied *before*
substitution: an alternative of the form `X? rest` (where `X` is a cycle member)
becomes two alternatives `X rest | rest`. The first is now a well-formed
left-recursive alternative (Binary/Suffix); the second is a Primary. This is
verified to make ANTLR accept the grammar (§4) and does not change the accepted
language.

## 4. Empirical validation (ANTLR 4.13.2 as oracle)

Every claim below was checked by generating with ANTLR 4.13.2 and, where noted,
compiling and running the generated Java parser. Artifacts under
`target/roslyn-lab/` and the scratch harness.

**The Roslyn grammar's actual blockers.** With the six empty rules corrected
(the epsilon `omitted_*` nodes made optional at use sites, the four lexical
stubs pointed at token names — the "minimal lexer adjustment"), ANTLR 4.13.2's
*entire* remaining error output is one line:

```text
error(119): The following sets of rules are mutually left-recursive
  [type, array_type, nullable_type, pointer_type]
  and [name, qualified_name]
  and [expression, assignment_expression, binary_expression, ...13 rules]
  and [pattern, binary_pattern]
```

Four cycles. (The issue body also lists a `member_declaration` declaration
cycle; in the corrected staging it is **not** left-recursive — `record_declaration`
reaches `member_declaration` only behind a non-nullable token, so it never
appears in `error(119)`. One fewer cycle to handle.)

**All four cycles are hub-and-spoke.** Every satellite is referenced only by its
hub, with one exception: `array_type` is also used by
`array_creation_expression`, so it must be retained as a non-recursive copy.

**Hand-inlining resolves 3 of 4 cycles immediately.** A mechanical inliner that
splices each satellite into its hub and deletes hub-only satellites produces a
grammar ANTLR reduces to a single residual `error(119)` on `[expression]`.

**The residual is exactly the leading-optional range operator.** Applying the
`X? rest -> X rest | rest` expansion to `expression? '..' expression?` yields a
grammar ANTLR **accepts with zero errors**.

**The accepted grammar produces a working parser.** Generated as a combined
grammar with a minimal lexer, ANTLR compiles it and the parser cleanly parses
real C# exercising all four cycles — records, `switch` expressions, `is not`
patterns, `and`/`or` patterns, array/nullable types, dotted names, chained
invocation/element access — with no parse errors.

**Boundary probes** (synthetic grammars, ANTLR verdicts):

| Shape | ANTLR verdict | Disposition |
|---|---|---|
| `e : e '+' e \| e? '..' e? \| ID` (leading-opt) | `error(119)` | expand optional, then accept |
| `e : e '+' e \| e '..' e? \| '..' e? \| ID` (expanded) | accepted | the fix works |
| hub-and-spoke, satellites inlined | accepted | primary target shape |
| genuinely mutual `a<->b`, substituted along chain | accepted | broader than hubs |
| multi-alt satellite, all alts inlined | accepted | supported |
| recursive call with args `h '+' h[3]` | `error(80)` | reject with diagnostic |
| mixed labeled/unlabeled alts | `error(122)` | reject (label synth deferred) |

## 5. Semantics and correctness argument

Roslyn's grammar is machine-generated from the compiler's syntax model. Its
alternatives are ordered **alphabetically**, and every binary operator is lumped
into a single `expression op expression` alternative. The grammar therefore
encodes **no operator precedence or associativity** — real C# precedence lives
in Roslyn's hand-written recursive-descent parser, not in this `.g4`. The tree
it defines is the flat, precedence-agnostic tree ANTLR would build.

This sharpens the issue's correctness bar. We are **not** claiming to reconstruct
C# precedence (the source grammar makes no such claim). We claim the narrower,
fully-provable property:

> For any grammar we accept, the parser we generate produces the identical parse
> tree that ANTLR's own runtime produces from the direct-LR grammar our
> transform emits.

The argument is compositional:

1. **The transform preserves the language and the intended tree.** Left-corner
   substitution is textual inlining of a rule body into a call site in
   left-corner position — the classic indirect-to-direct left-recursion
   elimination, language-preserving by construction. The optional-expansion step
   is a union-preserving alternative split. We prove equivalence *differentially*:
   for the accepted grammar, both ANTLR-from-transformed and
   ours-from-transformed must agree, and both must agree with
   ANTLR-from-original on the *set of accepted strings* (tree shape legitimately
   changes when a satellite node is inlined — see §8 fidelity).
2. **The direct-LR rewriter is already correct.** It is exercised across the
   full ANTLR runtime testsuite with zero skips; the transform feeds it only
   inputs it already accepts (Primary/Prefix/Binary/Suffix alternatives).
3. **The rejection path is sound.** Any cycle we cannot reduce to that shape is
   declined with a diagnostic; we never accept-and-miscompile. This is the
   issue's hard requirement ("must never mean accepts and silently miscompiles").

## 6. Algorithm

Input: the integrated parser model (`Vec<GrammarUnit>`), `TransformAnalysis`.

1. **Find cycles.** Compute the left-corner relation over the model (reuse the
   nullable set and call graph from `TransformAnalysis`; refine call-graph edges
   to left-corner edges by walking each alternative's prefix through nullable
   elements). Tarjan SCC → cycles. Non-cyclic grammars are untouched (pass
   reports `changed = false`).
2. **Per cycle, expand leading-optionals.** For every alternative in every cycle
   member whose left corner is an *optional* reference to a cycle member,
   rewrite `X? rest -> X rest | rest`.
3. **Choose the hub.** Prefer the rule that (a) has a non-recursive base
   alternative and (b) is referenced from outside the cycle (the cycle's public
   entry). For the Roslyn cycles this is unambiguous (`type`, `name`, `pattern`,
   `expression`). Ties broken by lowest `RuleId` for determinism.
4. **Substitute to the hub.** Repeatedly: for each hub alternative whose left
   corner is a satellite `S`, replace that leading `S` reference by inlining
   `S`'s alternatives (one hub alternative per `S` alternative), carrying through
   the trailing elements. Continue transitively until every hub alternative's
   left corner is either non-recursive or the hub itself. Bound the iteration by
   the total alternative count across the cycle; exceeding it is a defensive
   rejection (should be unreachable for well-formed finite cycles).
5. **Classify and gate.** Run the direct-LR classifier on the rebuilt hub. If any
   alternative is Nonconforming (args, etc.), reject the whole cycle with a
   specific diagnostic and leave the model untouched (the ATN-level `G4A005`
   backstop will then fire, or the specific new diagnostic supersedes it).
6. **Retire satellites.** Delete satellites referenced only within the cycle.
   Retain a non-recursive copy for any satellite referenced externally
   (`array_type`): its body already refers to the hub, which is now the
   precedence rule, so external callers see an ordinary rule.
7. **Provenance + labels.** Record substitution provenance so diagnostics and
   generated-code comments can trace an inlined alternative back to its original
   satellite rule. Preserve element labels and `$`-attribute references from
   satellite bodies.

The pass then hands off to `rewrite_immediate_left_recursion`, unchanged.

## 7. Diagnostics (staged acceptance criterion 1)

Even where we decline to transform, we improve on ANTLR. New codes:

- `G4R010` — "mutually left-recursive cycle through rules [...] cannot be made
  direct: <reason>", with the specific blocker (argument-bearing recursion,
  mixed labels, no base case) and related spans on each cycle member. This is
  the actionable message ANTLR's generic `error(119)` lacks.
- On success, an *info*-level transform report entry records which satellites
  were inlined into which hub, surfaced via the existing `TransformReport`.

## 8. Fidelity and scope boundaries

- **Tree shape changes for inlined satellites — by design and by necessity.**
  Once `binary_expression` is inlined into `expression`, there is no
  `BinaryExpressionContext` node; the operator alternative lives directly under
  `expression`. This matches what ANTLR produces from the transformed grammar.
  Recovering per-satellite node types would require synthesizing alternative
  **labels** (`# BinaryExpression`) so codegen emits typed context subclasses —
  but ANTLR forbids mixing labeled and unlabeled alternatives in one rule, so
  this is all-or-nothing per hub and is **deferred** (future enhancement, tracked
  separately). The initial delivery produces a correct, precedence-climbing hub
  with a flat alternative set, which is sufficient to consume the grammar and
  parse the language.
- **No runtime change.** If a future grammar needs a cycle that cannot be
  reduced to direct LR at all (none known; all probed shapes reduce), that would
  motivate approach (2). Out of scope here.
- **The `member_declaration` cycle is a non-issue** in the corrected staging, as
  measured. If a future grammar presents a genuinely nullable declaration cycle,
  it falls under the same algorithm.

## 9. Test plan

- **Unit** (`left_recursion.rs` / new `mutual_recursion.rs` module, insta
  snapshots per house style): the boundary matrix from §4 as model-level
  fixtures — hub-and-spoke, chained, multi-alt, leading-optional, and each
  rejection case with its exact diagnostic.
- **Codegen-direct fixtures** (`tests/codegen-direct/fixtures/`): small
  mutually-recursive grammars whose transformed form is snapshotted, plus
  parse-and-compare against the existing direct-LR path.
- **Differential Roslyn validation**: generate a recognizer from the corrected
  Roslyn grammar through our pipeline; parse a corpus of real C# files; compare
  trees against ANTLR's runtime on the identical transformed grammar. This is
  the headline acceptance test.
- **Regression**: full ANTLR runtime testsuite, zero skips — the pass must be a
  no-op on every grammar that has no mutual-LR cycle (guarded by the
  `changed = false` fast path and asserted on the testsuite corpus).

## 10. Deliverables

1. `src/bin_support/grammar/mutual_recursion.rs` — the pass (detector +
   left-corner substitution + optional-expansion + gating), registered in the
   pipeline before `rewrite_immediate_left_recursion`.
2. Diagnostic `G4R010` and transform-report wiring.
3. Unit + codegen-direct fixtures; Roslyn differential test (behind the existing
   cleanroom-jar gating, like the Kotlin parity suite).
4. `README`/docs note: which mutual-recursion shapes are supported vs declined
   (acceptance criterion 3).
5. This document.
