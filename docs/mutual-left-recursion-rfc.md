# Eliminating Mutual Left Recursion by Left-Corner Substitution: a Conservative Pre-Pass over ANTLR's Precedence Rewrite

**Status:** Request for Comments — addressed to ANTLR maintainers and grammar-analysis researchers
**Implementation:** shipped in [`antlr-rust-runtime`](https://github.com/ophi-dev/antlr-rust-runtime) PR [#221](https://github.com/ophi-dev/antlr-rust-runtime/pull/221) (issue [#151](https://github.com/ophi-dev/antlr-rust-runtime/issues/151))
**Validation oracle:** ANTLR 4.13.2 (Java tool + runtime)
**Date:** 2026-07-26 (rev. 2026-07-27: added the Visual Basic replication, §1.2/§3.1)

---

## Abstract

ANTLR 4 rewrites *immediate* left recursion into an unambiguous
precedence-climbing form using precedence predicates, but rejects *mutual*
(indirect) left recursion with `error(119)` (`LEFT_RECURSION_CYCLES`), even
when the cycle has a perfectly well-defined meaning under the same
alt-order-precedence convention. We describe a small, conservative grammar
transformation — **left-corner substitution into a designated hub rule** —
that reduces a useful subclass of mutual left recursion to the immediate form
ANTLR already handles, applied before `LeftRecursiveRuleTransformer` would run.
The transform is gated so that it either produces a rule that satisfies the
existing `binaryAlt`/`prefixAlt`/`suffixAlt`/`otherAlt` classification, or
declines and changes nothing, preserving today's diagnostics. Correctness is
established differentially: the transform's output is, by construction, a
grammar the reference tool accepts, so the reference runtime's parse trees are
a machine-checkable oracle. We validate on the grammar that motivated the work
— Roslyn's `CSharp.Generated.g4`, the C# compiler team's own generated
grammar, whose only blocker after trivial repairs is `error(119)` on four rule
cycles — achieving byte-identical parse trees against ANTLR's runtime, with
the full 357-descriptor runtime testsuite unperturbed. A replication on
Roslyn's second generated grammar, `VisualBasic.Grammar.g4` (419 rules, four
cycles including a 32-rule expression cycle), succeeds with the identical
pass, unmodified, supporting the claim that the covered subclass is the
natural shape of syntax-model-generated grammars. We state precisely
which cycle shapes are reduced, which are declined, and why the known-hard
cases (argument-bearing recursion, label mixing, epsilon-only cycles) remain
declined. We invite critique of the subclass boundary, the tree-shape
concession, and the possibility of adopting a similar pre-pass upstream.

---

## 1. Problem statement

ANTLR 4's celebrated left-recursion support ([Parr, Harwell, Fisher, *Adaptive
LL(\*) Parsing*, OOPSLA 2014]; `doc/left-recursion.md`) is scoped to rules with
**immediate** self-reference: `LeftRecursiveRuleTransformer` selects rules for
which `LeftRecursiveRuleAnalyzer.hasImmediateRecursiveRuleRefs(r.ast, r.name)`
holds, and rewrites them into the `primary (op …)*` loop with `{p >= _p}?`
precedence predicates. A left-recursive **cycle through two or more rules**
never enters that path; it survives to `AnalysisPipeline`, where
`LeftRecursionDetector` computes rule-start SCCs over the ATN and reports:

```text
error(119): The following sets of rules are mutually left-recursive [a, b]
```

The grammar is expressible in ANTLR syntax; the tool declines it. That is a
reasonable engineering boundary — but it bites hardest on grammars nobody can
edit: **machine-generated grammars published by language owners.**

### 1.1 The motivating instance

[`dotnet/roslyn`'s `CSharp.Generated.g4`](https://github.com/dotnet/roslyn/blob/main/src/Compilers/CSharp/Portable/Generated/CSharp.Generated.g4)
(≈1 800 lines, ≈340 rules, generated from the compiler's syntax model,
current with C# 12/13) is, to our knowledge, the only complete and maintained
ANTLR-syntax grammar of modern C#; grammars-v4's `csharp` stops around C# 7
(zero occurrences of `switch_expression`, `record_declaration`, or any pattern
rule). Measured with ANTLR 4.13.2, after repairing six trivially-empty rules
(two `/* epsilon */` bodies and four `/* see lexical specification */` stubs),
the **entire** remaining error output is one `error(119)` naming four cycles:

| Cycle | Shape |
|---|---|
| `type, array_type, nullable_type, pointer_type` | 1 hub + 3 satellites |
| `name, qualified_name` | 1 hub + 1 satellite |
| `expression, assignment_expression, …` (13 rules) | 1 hub + 12 satellites |
| `pattern, binary_pattern` | 1 hub + 1 satellite |

Representative excerpts:

```antlr
name : alias_qualified_name | qualified_name | simple_name ;
qualified_name : name '.' simple_name ;

pattern : binary_pattern | constant_pattern | … ;
binary_pattern : pattern ('or' | 'and') pattern ;

expression : … | binary_expression | … ;               // 46 alternatives
binary_expression : expression ('+'|'-'|…|'as'|'??') expression ;
range_expression  : expression? '..' expression? ;      // note the leading '?'
```

Each cycle is a **hub** (`name`, `pattern`, `type`, `expression`) whose
left-recursive **satellites** are plain alternatives of the hub, and each
satellite's left corner refers back to the hub. Every satellite is referenced
*only* by its hub, with a single exception (`array_type`, also used by
`array_creation_expression`). This is not a coincidence of C#: it is the
natural shape a syntax-model-driven generator produces, because each syntax
node class becomes a rule and the abstract base (`ExpressionSyntax`) becomes
the hub. We conjecture this hub-and-spoke shape is the dominant shape of
mutual left recursion in machine-generated grammars generally.

### 1.2 A replication: Roslyn's Visual Basic grammar

The conjecture invites an obvious test: Roslyn ships a *second*
syntax-model-generated grammar,
[`VisualBasic.Grammar.g4`](https://github.com/dotnet/roslyn/blob/main/src/Compilers/VisualBasic/Portable/Generated/VisualBasic.Grammar.g4)
(2 040 lines, 419 rules), produced by the analogous VB syntax generator and,
as far as we can tell, never before run through the ANTLR tool in anger. It is
a strictly harsher specimen. Reaching the left-recursion question required
repairing, in order: an unescaped `'\='` literal (VB's integer-divide-assign;
`error(156)`); **three duplicate rule definitions** (`error(51)`:
`resume_statement`, `case_block`, `if_directive_trivia`, each emitted once as
a union and once concrete); fourteen empty lexical stubs (C# had four); three
outright generator bugs — the multi-line lambda rules are published *without
their introducing header and with swapped end markers*
(`multi_line_function_lambda_expression : statement* end_sub_statement`),
`array_type : type array_rank_specifier*` (star, not plus — an epsilon
self-loop), and `invocation_expression : expression? argument_list?` (both
sides optional — matches the empty string); and seven intrinsically-nullable
rule bodies (`xml_text : xml_text_token*`, …). The defects we could root-cause
to the VB grammar emitter are reported upstream with fixes identified:
[dotnet/roslyn#84633](https://github.com/dotnet/roslyn/issues/84633)
(duplicate rules from a structure/node-kind name collision),
[#84634](https://github.com/dotnet/roslyn/issues/84634) (lambda header dropped,
and `End` markers swapped by positional kind-pairing),
[#84635](https://github.com/dotnet/roslyn/issues/84635) (the `'\='` escape),
and [#84636](https://github.com/dotnet/roslyn/issues/84636) (required list
children emitted `*` instead of `+`, which subsumes the `array_type` and
several nullable-root repairs). After those repairs — none of which touches
the recursion structure — the **entire** remaining error output is again one
`error(119)`, naming four cycles:

| Cycle | Shape |
|---|---|
| `expression` + 31 satellites | VB splits *every* binary operator into its own rule (`add_expression : expression '+' expression`, ×25) plus a member-access family using the leading-optional pattern (`expression? '.' identifier_name`) four times |
| `type, array_type, nullable_type` | 1 hub + 2 satellites |
| `name, qualified_name, qualified_cref_operator_reference` | 1 hub + 2 satellites |
| `xml_node, xml_attribute, base_xml_attribute` | VB XML literals: `xml_attribute : xml_node '=' xml_node` |

All four are hub-and-spoke; the externally-referenced satellites
(`qualified_name` from `implements_clause`, `xml_attribute` from
`xml_declaration_option`, `array_type` again) are exactly the retained-copy
case. The pass of §2, **unmodified**, reduces all four (§3.1). Two grammars
from two independent syntax models is still a small sample, but the
replication is consistent with the conjecture — and the VB expression cycle
(32 rules, 25 of them isomorphic binary-operator satellites) is a usefully
extreme instance of it.

### 1.3 Why the human fix is unsatisfying

A human can inline `binary_pattern` into `pattern` by hand — that is exactly
what grammar authors do today to appease `error(119)`. But for a published,
regenerated-on-every-release grammar, hand edits mean a permanently diverging
fork. The question is whether the *tool* can perform that inlining, safely,
with a proof obligation rather than a shrug.

---

## 2. The transformation

### 2.1 Definitions

Let *G* be a parser grammar. For rules *A*, *B*, say **A left-calls B** iff
some alternative of *A* can reach a reference to *B* before consuming a token
— i.e. through a (possibly empty) prefix of actions, semantic predicates,
epsilon elements, optional/star-quantified elements, and references to
*nullable* rules. This is the same left-corner relation
`LeftRecursionDetector` computes over ATN epsilon/rule transitions; we compute
it over the grammar model instead, before any ATN exists. A **cycle** is an
SCC of size ≥ 2 of this relation. (Size-1 SCCs are immediate left recursion
and are exactly the existing transformer's territory; we never touch them.)

### 2.2 Algorithm

For each cycle *C*:

1. **Choose the hub** *H* ∈ *C*: prefer a member that (a) has at least one
   alternative whose left corner is *not* in *C* (a token-consuming base
   case), and (b) is referenced from outside *C* (the cycle's public entry).
   Ties break deterministically. If no member satisfies (a), the cycle is
   ill-founded (its language is empty); **decline**.

2. **Expand leading optionals.** In every alternative of every member of *C*
   whose left corner is an *optional* reference `X?` with X ∈ *C*, rewrite

   ```text
   α X? β   →   α X β  |  α β        (α epsilon-only)
   ```

   This is the standard union-preserving expansion of a regular operator; it
   is required because the immediate-recursion pattern (both ANTLR's and ours)
   demands a non-optional recursive left corner. C#'s
   `range_expression : expression? '..' expression?` is the live instance.
   Only *greedy* optionals are expanded — the split's present-branch-first
   order is the greedy preference, so a nongreedy `X??` corner **declines** —
   and the expansion declines if a surviving action still references `X`.

3. **Substitute to the hub (left-corner inlining).** Maintain a worklist of
   *H*'s alternatives. For each alternative whose left corner is a satellite
   *S* ∈ *C* \ {*H*}: replace the leading reference to *S* by each of *S*'s
   alternatives in turn (one output alternative per alternative of *S*),
   concatenating the remainder. Repeat until every alternative's left corner
   is either outside *C* or is *H* itself. This terminates on every cycle
   whose members' left corners eventually reach *H* (a budget guards the
   pathological case; exceeding it **declines**).

4. **Gate on the immediate-form classification.** Classify the rebuilt hub
   exactly as `LeftRecursiveRuleAnalyzer` would: every alternative must be
   *primary*, *prefix*, *binary*, or *suffix*; at least one primary and one
   recursive alternative must exist; no recursive reference may carry
   arguments; a bare `H : H | …` self-loop (the image of an epsilon-only
   cycle) is nonconforming. Recursion is keyed on the **literal first
   element** — the classifier's reading — not on the first
   token-consuming one, and every alternative filed as primary is
   additionally checked to have a left-corner closure disjoint from *C*, so
   nothing still left-recursive can slip through as a "primary". If the gate
   fails, **decline: the grammar is left bit-for-bit unchanged**, and the
   existing SCC detector reports `error(119)` exactly as today.

5. **Commit.** Install the rebuilt hub. Delete satellites no retained rule
   references — where "retained" includes the **rebuilt hub body itself**:
   substitution consumes only the corner occurrence of a satellite, so a
   second reference in the same alternative (`e : s s | ID`) or in an
   alternative left verbatim (`t : arr | t '?' arr | ID`) keeps that
   satellite alive. A satellite referenced from outside
   the cycle (`array_type`) is retained verbatim: its body references *H*,
   which is now an ordinary immediate-left-recursive rule, so the external
   caller is unaffected. Then hand the grammar to the *unchanged* immediate
   left-recursion rewrite.

### 2.3 What the transform deliberately does not do

Every item below is a **precondition tested before the model is touched**, not a
repair attempted afterwards. That ordering is load-bearing: an earlier draft
checked admissibility only after splicing and consequently dropped
`<assoc=right>`, element labels, rule arguments and `*`-quantified corners,
in one case emitting a left-associative tree where the reference tool emits a
right-associative one. Deciding first turns each of those into a decline.

- It does **not** handle cycles where no member has a token-consuming base
  alternative (`a: b; b: c; c: a | X` reduces to `a : a | X`, whose recursive
  alternative consumes nothing — the same shape ANTLR rejects as
  `error(169)`/`NONCONFORMING_LR_RULE` in the immediate case).
- It requires every substituted corner to be **bare**. A quantified corner
  (`a : b* 'x'`) is not one satellite occurrence, so splicing a single body in
  its place would silently drop the closure; a **labelled** corner (`x=b`) would
  leave `$x` dangling in surviving actions; an **argument-bearing** corner
  (`b[3]`) has nowhere to put its arguments once the callee is gone; an
  **option-bearing** corner would have its options silently discarded. One
  shared predicate defines "bare" for every corner derivation.
- It declines a corner that a surviving **action still references by rule
  name** (`$b.text` after the `b` element is spliced or split away) — the
  reference would dangle — and it declines **nongreedy** optional corners
  (§2.2 step 2).
- It declines a corner reachable only **past a nullable rule call**
  (`a : n b`, `n :`), where which rule the author meant as the left corner is
  genuinely ambiguous.
- It declines a cycle for which the planner can make **no substitution step**
  (the cycle enters through a position the transform does not rewrite, such
  as a corner inside a nested block): a zero-step plan would otherwise be
  re-selected forever.
- It declines a satellite carrying **rule-level state** — arguments, returns,
  locals, `@init`/`@after`, `catch`/`finally`, rule options — since those attach
  to the rule and vanish with it.
- It does **not** synthesize alternative labels, and declines a satellite whose
  alternatives are `#`-labelled. Inlined *unlabelled* satellites lose their
  per-rule context type (§4.3); transferring an authored `#Add` would require
  labelling *all* hub alternatives (`error(122)`: "must label all alternatives
  or none"), a mechanical but API-affecting follow-up we chose to defer rather
  than bundle. An authored label is API, so silently dropping it is not an
  option — hence the decline.
- It does **not** touch **lexer** grammars: precedence rewriting is a
  parser-rule construct. (Left recursion in a lexer rule is invalid in ANTLR
  either way — `error(119)`.)
- It does **not** modify prediction, the ATN, or any runtime component. It is
  a source-model-to-source-model function running where
  `SemanticPipeline` invokes `LeftRecursiveRuleTransformer` in the reference
  tool — i.e. strictly before ATN construction.

None of these preconditions excludes any cycle in the two Roslyn grammars (§3),
which is the evidence that the subclass is narrow in the right places.

### 2.4 Precedence and associativity semantics of the result

Alt-order precedence composes through substitution in the obvious way: the
inlined alternatives occupy the position of the satellite reference in the
hub's alternative list, so the hub's declared order remains the single source
of precedence truth, and the standard rewrite's left-associativity default
(and `<assoc=right>` option) applies unchanged. For Roslyn specifically this
is even simpler than it sounds: both generated grammars are deliberately
**precedence-agnostic** — alternatives are listed alphabetically, with real
precedence living in Roslyn's hand-written parsers. C# lumps *all* binary
operators into one `expression op expression` alternative; VB splits them
into 25 one-per-operator satellite rules, likewise unordered. Either way the
grammar defines a flat operator tree, and the transformed parser reproduces
exactly that tree (§3). A user who wants the language's true precedence must
edit the grammar to order the operator alternatives — in the hub, exactly as
they would today for an immediate-recursive rule. The transform neither helps
nor hinders that.

---

## 3. Correctness argument and validation

Our correctness claim is deliberately narrow and machine-checkable:

> **Claim.** For every grammar the pass rewrites, the resulting grammar is
> accepted by ANTLR 4.13.2 without error, and our generated parser and
> ANTLR's runtime produce identical parse trees on identical input.

The claim's structure removes the need to trust our judgment about language
equivalence: step 3 is textbook indirect→immediate left-recursion elimination
(substitution of nonterminal bodies at left-corner positions), step 2 is a
regular-operator identity, and — decisively — the *output* is itself an
ANTLR-legal grammar, so the reference implementation adjudicates every case.

Validation performed (all artifacts reproducible; ANTLR 4.13.2 as oracle):

1. **Acceptance flip.** The repaired Roslyn grammar: reference tool →
   `error(119)` (sole error); our pipeline → accepted, parser generated and
   compiled. A distilled fixture (`MutualExpr.g4`, all three cycle shapes
   including the leading-optional operator) is likewise `error(119)` upstream
   and accepted by us — checked in CI both ways.
2. **Tree equality.** For inputs exercising all four Roslyn cycles — dotted
   names, array/nullable types, `is` + `and`/`or`/relational patterns,
   records, switch expressions, chained calls/indexing, `..` ranges — the
   reference runtime (running the *hand-inlined equivalent* grammar) and our
   parser (running the *mechanically transformed* original) print
   **byte-identical** LISP trees. The same equality holds on the distilled
   fixture, asserted in CI (`1+2*3`, `a.b.c`, `x..y`, `f()..g()`).
3. **Non-perturbation.** ANTLR's full runtime-testsuite conformance sweep:
   357/357 descriptors pass, zero skips, before and after. Both pre-existing
   mutual-recursion rejection fixtures still produce their diagnostic —
   confirming the gate declines them and the legacy path is intact.
4. **Boundary probes.** Each declined shape was probed against the reference
   tool to confirm the decline mirrors an upstream refusal (`error(80)`,
   `error(122)`, `error(169)`) rather than our own limitation.
5. **Decline is observable.** Thirteen unit tests, one per precondition of
   §2.3, each asserting the model is byte-identical to its *pre-pass*
   rendering, that no model IDs were consumed and no provenance recorded. A
   CLI-level fixture additionally confirms a declined cycle still reports the
   pre-existing cycle diagnostic naming the original rules, and emits no
   parser artifact.

### 3.1 Replication on the Visual Basic grammar

The identical protocol was run on the repaired `VisualBasic.Grammar.g4`
(§1.2), with **no change to the pass**:

- **Acceptance flip.** Reference tool → `error(119)` (sole error, four
  cycles); our pipeline → accepted, parser generated and compiled.
- **Collapse shape.** All hub-only satellites vanish (the 25 binary-operator
  rules, `member_access_expression`, `invocation_expression`,
  `nullable_type`, `base_xml_attribute`, …); the three externally-referenced
  satellites (`qualified_name`, `xml_attribute`, `array_type`) are retained
  as non-recursive copies, as specified in §2.2 step 5.
- **Tree equality.** On inputs exercising each cycle — operator chains
  (`a + b * c - d / e`), member/call chains (`a.b.c(x).d(y)`), dotted
  imports/namespaces, array/nullable/qualified types — the reference runtime
  on the hand-inlined equivalent and our parser on the mechanically
  transformed original print **byte-identical** trees, all clean parses.
- **Growth.** The 32-rule expression cycle collapses into a hub of 59
  alternatives (from 30): each single-alternative satellite contributes one
  alternative, plus one per leading-optional expansion. `type` and `name`
  stay at 5 alternatives; `xml_node` grows 15 → 17. Linear in the cycle's own
  alternative count, as predicted in §4.5.

Beyond replication, VB stresses two aspects C# barely exercises: the
leading-optional expansion fires **four times** (the whole
`expression? '.' …` member-access family, vs. C#'s single range operator),
and the XML-literal cycle (`xml_attribute : xml_node '=' xml_node`) shows the
pattern arising outside expression/type/name territory. One honest caveat:
the published VB file needed the §1.2 repairs *before* the recursion question
could even be posed — three of those repairs (the swapped lambda ends, the
`*`-quantified `array_type`, the doubly-optional `invocation_expression`) are
defects in Roslyn's grammar emitter that no parser-side mechanism can absorb,
and in the raw file they entangle `statement` and the lambda rules into the
expression SCC. Mutual-recursion support makes such grammars *consumable*; it
does not make them *correct*.

What we do **not** claim: that the transform preserves ANTLR's *ambiguity
resolution* on grammars that were ambiguous across the cycle in ways
alt-order does not capture. The gate's requirement that the result fit the
immediate-form pattern — whose semantics ANTLR defines and we inherit — is
precisely what bounds the claim.

---

## 4. Discussion & requested comments

### 4.1 Is the subclass boundary right?

Empirically, left-corner substitution reduced every cycle we probed that has
a well-defined base case, *including* genuinely mutual `a ↔ b` cycles that
are not hub-shaped (`a : b '+' a | ID; b : a '.' b | ID` reduces cleanly once
`b` is substituted along the chain). The shapes that remain declined are
exactly the shapes whose *immediate* images ANTLR also rejects.

We asked this question of our own automated reviewers first, and it was
productive enough to be worth reporting, because it shows how the boundary was
actually located. The exact two hazards anticipated below — substitution order
changing alternative order, and a nullable prefix making the left-corner
relation ambiguous — were both realised, alongside three more:

| Adversarial shape | Failure it caused |
|---|---|
| `expr : power \| ID; power : <assoc=right> expr '^' expr` | `<assoc=right>` dropped → **left**-associative tree where the reference tool gives right-associative |
| `a : b* 'x'; b : a 'b'` | `b*` replaced by one satellite body → closure lost, accepted language changed |
| `a : n b; b : a 'b'; n :` | corner *decided* as `b`, *spliced* at `n` |
| `e : s \| e '+' e; s : e '*' e` | spliced alternatives appended rather than positioned → precedence reordered |
| `e : x=s \| ID; s : e '+' ID` (also `s[int x]`, `s @init{}`, `s : … #Add`) | labels, arguments, rule-level actions silently dropped |

Every one traced to a single architectural error — admissibility was checked
*after* mutation, and the corner's identity was derived twice — and every one is
now a decline or a correct rewrite (§2.3). The lesson generalises beyond this
pass: for a transform whose contract is "provably correct or nothing", the
decision must be a pure function of the untouched input.

A second review round then probed the rewritten, decide-first implementation
and located a further family, all in the *bookkeeping* that accompanies the
splice rather than in the splice itself:

| Adversarial shape | Failure it caused |
|---|---|
| `e : (s \| ID) \| e '+' e; s : e '*' e` | a plan that made no substitution step was re-selected verbatim forever (non-termination) |
| `e : s s \| ID; s : e '+' ID` (also `t : arr \| t '?' arr; arr : t '[' ']'`) | removability judged against the *original* hub body → the surviving non-corner reference dangled after the satellite was deleted |
| `e : s #ViaSatellite \| ID #Atom; s : e '+' ID` | spliced alternative took the satellite's (empty) label → the authored context class silently vanished |
| `e : s \| ID; s : {p}? e '+' ID` | admissibility gate skipped the leading predicate while the downstream classifier keys on the literal first element → committed, then failed naming the wrong rule set |
| `r : e?? '..'` | nongreedy optional split with the greedy branch order → authored match preference inverted |
| `e : s {… $s.text …} \| ID; s : e '+' ID` | corner deleted while a surviving action still referenced it by rule name |

The shared root cause this time: each decision read the original model where it
had to read the *planned* one (or vice versa). Removability is now computed
against the planned alternatives; label and option attribution is split
explicitly (the `#label` names the hub's alternative position, the
`<assoc=…>` option describes the satellite's operator); the gate mirrors the
downstream classifier's literal-first reading and backstops every
non-recursive alternative with a left-corner-closure check; zero-step plans,
nongreedy corners and corners still referenced by surviving actions decline.
One reviewer claim was *refuted* by the reference oracle rather than fixed:
the optional split places the same `#label` on both product alternatives, and
ANTLR accepts that (both map to one context class) — the split is exactly as
label-preserving as the reference tool requires.

**Question that remains open to reviewers:** with the preconditions of §2.3 in
force, is there a cycle family with well-defined alt-order semantics that the
gate *accepts* but transforms wrongly? A counterexample to that would be the
most valuable outcome of this RFC; the shapes above are now regression tests
rather than open risks.

### 4.2 Hub choice

When several cycle members have base alternatives and external callers, hub
choice affects which rule survives as the precedence rule (and therefore tree
labels), not the language. We currently prefer external-referenced-with-base,
tie-broken deterministically; Roslyn's cycles have a unique natural hub. Is
there a principled criterion we're missing — e.g. always the member with the
maximal alternative count, or an explicit grammar option
(`options { lrHub=expression; }`)?

### 4.3 The tree-shape concession

Inlined satellites vanish from the tree: there is no `Binary_patternContext`;
the operator alternative lives directly under `pattern`, matching what ANTLR
itself produces for the hand-inlined grammar. For Roslyn this is arguably
*more* faithful to the language (Roslyn's own `BinaryPatternSyntax` is a
child of the pattern hierarchy, not a wrapper rule), but it is a real API
difference from a hypothetical native-mutual-recursion parser. The obvious
remedy — auto-labeling every inlined alternative with its satellite's name,
lifting the all-or-none label restriction tool-side — is mechanical but
changes generated-API surface. Would upstream consider label synthesis
acceptable, or is the flattened tree the honest answer?

### 4.4 Could ANTLR adopt this?

The pass is self-contained, language-target-independent, and sits at a point
in the pipeline ANTLR already owns (`SemanticPipeline`, immediately before
`LeftRecursiveRuleTransformer.translateLeftRecursiveRules()`). The gate reuses
the classification `LeftRecursiveRuleAnalyzer` already implements
(`binaryAlt`/`prefixAlt`/`suffixAlt`/`otherAlt`); the SCC computation
duplicates `LeftRecursionDetector` at the AST level. A Java port would be a
few hundred lines plus tests, and `error(119)` would then fire only for
cycles that are declined — with a message that could finally distinguish
"inherently ill-founded" from "well-defined but unsupported". We are glad to
contribute this if there is appetite; we are equally interested in hearing
why it was left out originally — whether as a deliberate scoping decision or
because the generated-grammar use case (§1.1) postdates the design.

### 4.5 Relation to prior art

Indirect→direct left-recursion elimination by substitution is classical
(Paull's algorithm; Moore, *Removing Left Recursion from Context-Free
Grammars*, ANLP 2000, discusses the size blow-up that makes the general
algorithm unattractive). The contribution here is not the substitution but
the **scoping and gating**: substituting only within left-corner SCCs, only
into a designated hub, only when the result lands in ANTLR's
precedence-pattern subclass — which keeps the blow-up bounded by the cycle's
own alternative count (C#'s 13-rule expression cycle: 46 → 47 hub
alternatives; VB's 32-rule cycle: 30 → 59; in both, each single-alternative
satellite contributes one alternative and each leading-optional expansion one
more) and inherits, rather than re-derives, the precedence semantics of
[OOPSLA 2014]. Moore-style worst cases are exactly what the budget + gate
decline.

---

## 5. Implementation notes (for the curious; Rust knowledge not required)

The pass is one file, `src/bin_support/grammar/mutual_recursion.rs` (~600
lines + ~400 of tests), in a Rust reimplementation of the ANTLR toolchain
that consumes `.g4` source directly. Correspondences to the Java tool:

| This work | ANTLR 4 (Java) |
|---|---|
| model-level left-corner SCC (Tarjan) | `LeftRecursionDetector` over ATN rule-start states |
| `eliminate_mutual_left_recursion` (the pass) | — (proposed pre-pass) |
| immediate-form gate | `LeftRecursiveRuleAnalyzer` alt classification |
| downstream immediate rewrite | `LeftRecursiveRuleTransformer` + `LeftRecursiveRuleWalker.g` |
| backstop diagnostic `G4A005` | `ErrorType.LEFT_RECURSION_CYCLES` (119) |

Design doc with the full empirical log:
[`docs/issue-151-mutual-left-recursion-plan.md`](./issue-151-mutual-left-recursion-plan.md).
Repro for the Roslyn measurements is scripted in the PR — for C#, the
six-rule repair, staged error output, and tree-diff harness; for VB, the
§1.2 repair sequence (escape, duplicate rules, lexical stubs, the three
emitter-bug corrections, nullable roots) followed by the same
generate/compile/tree-diff protocol.

---

## 6. Summary of questions for reviewers

1. Counterexamples: a cycle with well-defined alt-order semantics that the
   gate *accepts* but whose transformed parser diverges from intent (§4.1)?
2. Hub selection: is deterministic-with-preference sufficient, or should the
   author name the hub (§4.2)?
3. Trees: flattened satellites vs. synthesized labels — which is the right
   default for generated APIs (§4.3)?
4. Upstream interest: is a Java port of this pre-pass worth proposing against
   `antlr4`, and was mutual recursion originally excluded by design or by
   priority (§4.4)?

Feedback via issues/discussions on
[`ophi-dev/antlr-rust-runtime`](https://github.com/ophi-dev/antlr-rust-runtime)
is very welcome.
