# Review concerns: honest action transpiler plan

## 1. `RuleValue` routing is removed, but the dead evaluator and stale test remain

The uncommitted generator change does remove the important routing: `$rule.v`
and `$rule.result` now parse as `RuleReturnValue`, so they read a captured
return slot instead of constructing `RuleValue`. That addresses the core
fakery.

The cleanup is still incomplete. The `RuleValue` enum variant, `RuleValueKind`,
and `render_rule_value_write` helper remain in the file even though the focused
unit build reports them as never constructed. The old unit test still expects
`writeln("$e.result")` to parse as `RuleValue` and fails under the current
working tree.

Evidence:

- `src/bin/antlr4-rust-gen.rs:7739` now routes all non-`text` `$rule.<name>`
  references to `ActionTemplate::RuleReturnValue`.
- `src/bin/antlr4-rust-gen.rs:5818` still defines `ActionTemplate::RuleValue`.
- `src/bin/antlr4-rust-gen.rs:9149` still defines `render_rule_value_write`.
- `src/bin/antlr4-rust-gen.rs:9158` through `src/bin/antlr4-rust-gen.rs:9248`
  still contains the arithmetic/string evaluator.
- `src/bin/antlr4-rust-gen.rs:13075` still has
  `parses_rule_value_print_template`, which expects `RuleValue`.
- `cargo test --features codegen --bin antlr4-rust-gen parses_rule_value_print_template` fails in
  this working tree because that expectation is stale.

Concern: the baseline direction is right, but leaving the dead evaluator around
keeps a misleading implementation path and stale test coverage in the codebase.
It also makes future changes more error-prone: a later parser path or manual
template construction could accidentally re-enable the old text re-evaluator.

Suggested correction: delete `RuleValue`, `RuleValueKind`, and
`render_rule_value_write`, then replace the stale unit with a regression that
proves `writeln("$e.v")` / `writeln("$e.result")` parse as `RuleReturnValue` and
no longer evaluate matched rule text.

## 2. Label and occurrence binding is the hard part, and the plan currently overstates the available metadata

The plan says bindings can be computed from `GeneratedParserStep`s because they
already track `CallRule`, `MatchToken`, and labels for rule args. Current
`GeneratedParserStep` only carries token type, rule index, source state, and
precedence. It does not carry grammar labels (`a=e`, `b=e`, `left=e`), occurrence
ids, or list-label information.

Evidence:

- `src/bin/antlr4-rust-gen.rs:1814` defines `GeneratedParserStep`.
- `src/bin/antlr4-rust-gen.rs:1815` through `src/bin/antlr4-rust-gen.rs:1843`
  show `MatchToken`, `Action`, and `CallRule` fields with no label/occurrence
  metadata.
- Existing label handling for `$label.y` resolves the label to a rule name only
  (`src/bin/antlr4-rust-gen.rs:7100`).
- The read path then uses `first_rule_int_return`, a depth-first first match by
  rule index (`src/tree.rs:61`).

Concern: resolving `$a.v`, `$b.v`, `$left.v`, `$e.v`, and common-label cases by
rule name or depth-first search will be wrong whenever an alternative contains
multiple children of the same rule or when left-recursive rewrites move the
logical reference away from the first descendant. That would recreate the same
class of fixture-fit bug, just behind a new parser.

Suggested correction: add an explicit binding artifact keyed by action source
state before lowering expressions. It should be built from the owning grammar
rule source plus the selected ATN alternative, and it should record label,
symbol kind, occurrence ordinal, and child/token accessor. Add tests that fail
if `$a.v + $b.v` reads the same `e` child twice, if `$left.v` selects the wrong
recursive context, or if an unlabeled `$INT.int` binds to the wrong token
occurrence.

## 3. String returns and token text do not fit the current SemIR/value model yet

The plan treats `$result = $ID.text` and `AppendStr(...)` as a small parallel
string-return addition. Current SemIR and parse contexts are integer-return
only: text expressions are comparison operands, not values that can be assigned,
and `SetReturn` coerces the evaluated value through `int_or_zero`.

Evidence:

- `src/semir.rs:219` defines `Value` as `Null | Bool | Int`.
- `src/semir.rs:384` says text-valued nodes evaluate to `Null` outside
  comparisons.
- `src/semir.rs:361` stores `AStmt::SetReturn` through `int_or_zero`.
- `src/semir.rs:275` exposes `ActContext::set_return(&str, i64)`.
- `src/parser.rs:1042` and `src/tree.rs:276` store return values as integer
  maps only.

Concern: lowering `$ID.text` or string concatenation through the current SemIR
would silently become `0`/empty unless the value model changes first. A parallel
`first_rule_string_return` is necessary but not sufficient; the IR needs a real
text value path, string-return storage, and typed action evaluation semantics.

Suggested correction: split the implementation sequence so integer return
actions land first, then add a typed value model (`Int`/`Text`) for action
expressions and return slots before claiming `$result` / `AppendStr` support.
