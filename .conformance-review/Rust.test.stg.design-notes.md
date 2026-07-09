# `Rust.test.stg` — design notes

Companion to `Rust.test.stg`. This file explains the non-trivial renderings and,
critically, enumerates the **generated-code / runtime API surface** the templates
presuppose so a reader can later diff each assumption against a real Rust runtime
and see the gaps.

**Authored blind.** Written without reading any concrete Rust ANTLR runtime or
code generator. Modeled on the other targets' `.test.stg` files (Java canonical;
Go the reference for getter/naming conventions) plus idiomatic Rust. Treat every
"assumes …" below as a *hypothesis about the ideal API*, not a claim about what
exists.

## Count / signature reconciliation

- `Java.test.stg` defines **70** templates. The prompt's *enumerated* checklist
  lists **69** distinct names — it omits `StringType`, which Java and every other
  target `.stg` define and which the harness needs to render descriptors that use
  it. `Rust.test.stg` therefore defines all **70** (the prompt's 69 + `StringType`).
- All 70 signatures (name + argument arity/names) match `Java.test.stg` exactly,
  verified by set-diffing the template headers. The harness calls these positionally
  and by name, so bodies may be re-rendered freely but headers may not drift.

## Cross-cutting design decisions

### Output sink — `self.output()`
Every target routes `writeln`/`write` through a capturable sink: Java `outStream`,
C# `Output` (an injected `TextWriter`), Python `self._output`, Go `fmt.Println`
(stdout). The idiomatic Rust analog is a handle the generated recognizer exposes
over a `&mut dyn std::io::Write` the harness installs. I render `writeln!(self.output(), …)`.
The key subtlety: `write!`/`writeln!` return `io::Result`, and an ANTLR action
`{...}` is a statement position (fine, `let _ = …` is implicit for a trailing `;`),
but this must **not** leak a `Result` that fails to compile. Assumption: `self.output()`
yields a sink whose `write!`/`writeln!` are used in statement position only (all
three `write*` templates end in `;`), so the discarded `io::Result` is acceptable
exactly as `outStream.println(...)`'s `void` is in Java.

### Typed contexts + child accessors (Go-style split, Rust casing)
`ctx.e(0)` / `ctx.e_all()` for rule children; `ctx.INT(0)` / `ctx.INT_all()` for
token children. This mirrors Go's `E(0)`/`AllE()` "single-vs-list getter" split,
but in Rust naming: rule refs are snake_case identifiers so they stay lowercase;
token refs are ANTLR token *names* (conventionally uppercase) so they stay
uppercase. I chose the `_all()` suffix (over Go's `All*` prefix or Python/TS
`*_list()`) because `ContextListFunction` in the neutral tests renders `<rule>()` in
Java but Python/TS already diverge to `<rule>_list()`; a suffix reads most naturally
in snake_case Rust and keeps the single-child getter (`e`) and list getter (`e_all`)
lexically adjacent. **`ContextListFunction` is rendered `<ctx>.<rule>_all()` to match.**

### Rule return attributes as fields — `Result`/`Production` pass through
Java/C#/Python/Swift/Dart render `Result(r)`/`Production(p)` as a bare `<r>`/`<p>`
(the attribute is an in-scope local/field). **Go is the outlier**: it renders
`Get<r;format="cap">()` because Go exposes rule return values through generated
getters. The ideal Rust target should expose rule return attributes as **public
fields** on the returns/context value (no accessor ceremony), so I pass through
`<r>`/`<p>` like Java. This is a deliberate divergence from Go and one of the most
likely to mismatch a real runtime (see "Least certain").

### Members as an `impl` block
`@parser::members { … }` injects associated fns onto the generated parser; helpers
are invoked `self.foo()` / `self.pred(v)` (methods take `&mut self` because they
write to the output sink). Predicates `{...}?` and actions `{...}` are assumed to
execute in a scope where `self` is the parser (or lexer) recognizer.

### Listener trait `TListener`
Generated trait `TListener` with snake_case `enter_<rule>` / `exit_<rule>` and a
defaulted `visit_terminal(&mut self, node: &TerminalNode)`. Test listeners are unit
structs (`#[derive(Default)] struct LeafListener;`) implementing it, walked by a
`ParseTreeWalker::walk(&mut listener, tree)`. Grammar name `T` ⇒ trait `TListener`,
matching the neutral `<X>Listener`/`TBaseListener` convention (Rust has default
trait methods, so there is no separate "base" class — the trait *is* the base).

### Downcast to concrete/labeled-alt context
`Cast(t,v)` ⇒ `(<v>).downcast_ref::<<t>>().unwrap()` — the Rust analog of Java's
`((BinaryContext)$ctx)`. Assumes contexts are `dyn`-compatible / carry an `Any`-like
downcast (`downcast_ref::<T>()`), which is how a trait-object context tree in Rust
would expose labeled-alternative subtypes.

## Per-template notes (non-trivial only)

- **writeln / write / writeList** — `writeln!`/`write!` macros against `self.output()`.
  `writeList` uses ST `separator=" + "` to string-concatenate list elements before
  printing, matching Java's `separator="+"` (Rust needs the operands `Display`, which
  the neutral tests satisfy — they pass integer/string locals).
- **Assert** — `assert!(<s>)`. Go/Python/Swift render this empty (no cheap runtime
  assert in the test context); Rust has `assert!` natively, so I keep it like
  Java/C#/Dart/TS. If a real runtime's action scope can't panic safely mid-parse,
  this may need to become empty.
- **Cast** — see "Downcast" above. `.unwrap()` because the neutral tests only cast
  when the dynamic type is known-correct.
- **Append** — `<a> + &(<b>).to_string()`: `String + &str`. `AppendStr` is
  `String + &str` where `<b>` is already a string, so no `.to_string()`. `Concat`
  is raw token juxtaposition (no operator) exactly as every target.
- **AssertIsList** — `let __ttt__: &[_] = &<v>;`. Pure static-type assertion (like
  Java's `List<?> __ttt__ = <v>;` / C#'s cast): if `<v>` is not sliceable it won't
  compile. Chose a slice coercion over a `Vec` binding so it works whether the getter
  returns `Vec<_>` or `&[_]`.
- **InitIntMember / InitBooleanMember / InitIntVar** — `let mut <n>: T = <v>; let _ = <n>;`.
  The `let _ = <n>;` suppresses an `unused_variables`/`unused_assignment` warning
  (which, under the repo's `-D warnings`, would be a hard error), mirroring Go's
  `var _ int = <n>` trick. `mut` because some descriptors later assign via
  `AssignLocal`/`AddMember`.
- **GetMember/SetMember/AddMember/MemberEquals/Mod…** — `self.<n>` member access,
  assuming grammar-declared `@members` fields become fields on the recognizer struct.
- **DumpDFA** — `self.dump_dfa()`; assumes a debug method that writes to the same
  sink the harness captures (Java passes `outStream`; a Rust `dump_dfa` should target
  `self.output()` internally).
- **StringList** — `Vec<String>`. **StringType** — `String`.
- **BuildParseTrees** — `self.set_build_parse_trees(true)`. **BailErrorStrategy** —
  `self.set_error_handler(BailErrorStrategy::new())`.
- **ToStringTree** — `<s>.to_string_tree(Some(self))`: the recognizer is passed as
  `Option<&Recognizer>` so the printer can resolve rule names (Java passes `this`,
  Go `nil, p`).
- **Column/Text/InputText/LT/LA/TokenStartColumn** — snake_case accessors on the
  recognizer (`char_position_in_line()`, `text()`, `input().text()`,
  `input().lt(i).text()`, `input().la(i)`, `token_start_char_position_in_line()`).
  `la` returns the integer token type; `lt(i).text()` the lexeme.
- **GetExpectedTokenNames** — `self.expected_tokens().to_token_string(self.vocabulary())`.
  Assumes an interval-set → display-string method that takes a `Vocabulary`
  (Java `.toString(tokenNames)`, C# `.ToString(Vocabulary)`, Go `StringVerbose`).
- **RuleInvocationStack** — `format!("{:?}", self.rule_invocation_stack())`. The
  neutral test expects a Java-list-style `[a, b, c]` string; Rust `Vec<String>`'s
  `{:?}` yields `["a", "b", "c"]` (quoted). **This will not byte-match the Java-style
  `[a, b, c]` the descriptor expects** unless the runtime provides a bespoke
  formatter — Go needed `antlr.PrintArrayJavaStyle`, Swift strips quotes, Python has
  `str_list`. Flagged as least-certain; a real Rust target likely needs a
  `print_array_java_style`-style helper here.
- **LL_EXACT_AMBIG_DETECTION** — `self.interpreter().set_prediction_mode(PredictionMode::LlExactAmbigDetection)`.
  Enum variant is `UpperCamel` per Rust convention (`LlExactAmbigDetection`), a
  divergence from the `LL_EXACT_AMBIG_DETECTION` constant name.
- **ParserToken / ParserTokenType / ParserPropertyCall / ContextRuleFunction /
  ContextMember / SubContextLocal / SubContextMember** — path/field access with `::`
  for associated constants (`Parser::<t>`) and `.` for field/method access. Unlike
  Go, I do **not** capitalize the trailing accessor in `SubContext*` (Rust fields are
  snake_case and the neutral args are already snake_case), so these pass through
  plainly like Java/C#/Python.
- **ParserPropertyMember** — `@members { fn property(&self) -> bool { true } }`.
- **PositionAdjustingLexerDef / PositionAdjustingLexer** — I split like Dart:
  `Def` holds the `PositionAdjustingLexerATNSimulator` struct + its
  `reset_accept_position`, and `PositionAdjustingLexer` holds the `next_token`/`emit`
  overrides and the `handle_accept_position_*` helpers. This is the single most
  assumption-dense pair (see "Least certain"). Key idiomatic choices: char indexing
  goes through `text.chars().collect::<Vec<char>>()` (Rust strings aren't
  byte-indexable and ANTLR positions are codepoint offsets); `isize` for
  stream/line/column indices (ANTLR uses signed sentinels like `-1`); a downcast
  helper `interpreter_as::<T>()` and an `is::<T>()` type check to swap in the custom
  simulator; `input_mut()` to hand the ATN simulator a mutable `&mut dyn CharStream`.
  Assumes the generated lexer lets you *override* `next_token`/`emit` and call the
  base via `base_next_token()`/`base_emit()` (Rust has no `super`), and that the
  interpreter slot is a swappable `Box<dyn …>`.
- **BasicListener / TokenGetterListener / RuleGetterListener / LRListener /
  LRWithLabelsListener** — unit-struct listeners implementing `TListener`. Note the
  `TokenGetterListener` prints `ctx.INT_all()` via `{:?}` (Java printed the raw list
  `ctx.INT()`); like `RuleInvocationStack` the exact debug formatting is unlikely to
  byte-match Java's list rendering without a helper — flagged.
- **TreeNodeWithAltNumField** — a `MyRuleNode` struct wrapping
  `BaseParserRuleContext` with an `alt_num` field and a `ParserRuleContext` impl
  overriding `alt_number`/`set_alt_number`. Assumes contexts compose over a
  `BaseParserRuleContext` and that `alt_number` is an overridable trait method.
- **WalkListener** — `let mut listener = LeafListener::default(); ParseTreeWalker::walk(&mut listener, <s>);`.
- **DeclareContextListGettersFunction** — a compile-only shape check using the list
  getters (`a_all()`/`b_all()`) returning `Vec<Rc<…Context>>`. `Rc` because a parse
  tree in Rust is most naturally reference-counted shared nodes.
- **Declare_foo / Invoke_foo / Declare_pred / Invoke_pred** — helper fns on the
  recognizer (`&mut self` so they can write output); `pred` prints `eval={v}` and
  returns `v`. Invoked `self.foo()` / `self.pred(<v>)`.

## Runtime API surface this assumes

Grouped so each can be checked against a real Rust runtime.

### Output / recording
1. `self.output()` on both parser and lexer recognizers, returning a handle usable
   as the first arg to `write!`/`writeln!` (i.e. implements `std::io::Write` or a
   `fmt::Write`-like shim), wired to the sink the test harness captures.
2. Discarding the `io::Result` from `writeln!`/`write!` in statement position must
   compile clean under `-D warnings` (or `self.output()` returns a sink whose macro
   expansion doesn't yield a must-use `Result`).

### Recognizer accessors (snake_case methods on parser/lexer)
3. `text()`, `char_position_in_line()`, `token_start_char_position_in_line()`,
   `token_start_char_index()`, `token_start_line()`, `token_type()`.
4. `input()` → token/char stream with `text()`, `lt(i)->Token`, `la(i)->i32/isize`,
   `index()`, `seek(i)`; plus `input_mut()` for a mutable borrow.
5. `set_build_parse_trees(bool)`, `set_error_handler(BailErrorStrategy)`,
   `dump_dfa()` (writing to the captured sink), `expected_tokens()` →
   `.to_token_string(Vocabulary)`, `vocabulary()`, `rule_invocation_stack()` →
   sequence of rule names, `interpreter()` with `set_prediction_mode(PredictionMode)`.
6. Grammar `@members` fields materialize as fields on the recognizer struct,
   reachable as `self.<name>`; grammar `@members` fns materialize as methods.

### Generated context types
7. Per-rule context struct named `<Rule;cap>Context` (e.g. `AContext`, `EContext`,
   `CallContext`, `IntContext`, `SContext`, `BinaryContext`).
8. Positional child accessors: rule child `ctx.<rule>(i)` (single) + `ctx.<rule>_all()`
   (`Vec` of children); token child `ctx.<TOKEN>(i)` + `ctx.<TOKEN>_all()`.
9. `ctx.child_count()`, `ctx.start()` (→ token with `.text()`), and terminal nodes
   exposing `.symbol()` (→ token with `.text()`), plus `TerminalNode`.
10. Rule **return attributes exposed as public fields** on the returns/context value
    (`ctx.v`, bare `r`/`p`) — *not* getters. (Divergence from Go.)
11. Base-context downcast: `(<ctx>).downcast_ref::<ConcreteContext>()` (an `Any`-style
    facility on the context trait object) for labeled-alt access.
12. Contexts compose over a `BaseParserRuleContext`, and `ParserRuleContext` is a
    trait with overridable `alt_number()`/`set_alt_number()`; a
    `ParserRuleContextRef` (shared, e.g. `Rc`) type for parent links; trees are
    `Rc<…Context>`.

### Listener / walker
13. Generated listener trait `<Grammar>Listener` (e.g. `TListener`) with **defaulted**
    `enter_<rule>`/`exit_<rule>(&mut self, ctx: &<Rule>Context)` and a defaulted
    `visit_terminal(&mut self, &TerminalNode)`.
14. `ParseTreeWalker::walk(&mut impl TListener, tree)` free function / assoc fn.

### Lexer override / ATN-simulator plumbing (heaviest assumptions)
15. Ability to override `next_token`/`emit` on the generated lexer and call the base
    impl via `base_next_token()`/`base_emit()` (no `super` in Rust).
16. A swappable interpreter slot: `interpreter()` returning something with `is::<T>()`;
    `set_interpreter(Box<dyn …>)`; `interpreter_as::<T>()` for a downcast borrow.
17. A subclass-able `LexerATNSimulator` with `new(atn, decision_to_dfa, shared_context_cache)`,
    `set_line`, `set_char_position_in_line`, `consume(&mut dyn CharStream)`; and lexer
    accessors `atn()`, `decision_to_dfa()`, `shared_context_cache()`.
18. Generated per-token associated constants on the lexer (`Self::TOKENS`, `Self::LABEL`).

### Codegen name mappings the templates bake in
19. `PredictionMode::LlExactAmbigDetection` (UpperCamel enum variant).
20. `BailErrorStrategy::new()`, `BaseParserRuleContext::new(parent, invoking_state)`.
21. ANTLR positions are `isize` (signed, to carry `-1` sentinels); string positions
    are **codepoint** offsets, so lexer char scanning goes through `.chars()`.

## Templates I am least certain about (most likely to diverge from a real runtime)

1. **`RuleInvocationStack`** (and the debug-formatted list prints in
   `TokenGetterListener`) — I used `format!("{:?}", …)`, but Rust's `Debug` for
   `Vec<String>` quotes elements (`["a", "b"]`), whereas the descriptor expects
   Java-style `[a, b]`. Every other target needed a bespoke helper (Go
   `PrintArrayJavaStyle`, Python `str_list`, Swift quote-stripping). A real Rust
   target almost certainly needs a `print_array_java_style`-type formatter, and this
   template should call it. **Highest byte-match risk.**
2. **`PositionAdjustingLexerDef` + `PositionAdjustingLexer`** — the override model
   (`base_next_token`/`base_emit`, `set_interpreter`, `interpreter_as::<T>`,
   `is::<T>`), the mutable-stream borrow (`input_mut()`), and codepoint char indexing
   are all *inventions*. Whether an idiomatic Rust runtime even exposes lexer method
   overriding this way (vs. a hook/trait) is unknown; a trait-based customization
   point would change this template substantially.
3. **`Result` / `Production` (rule return attributes as fields)** — I bet on public
   fields (`<r>` pass-through, Java-style) rather than Go-style getters
   (`Get<r>()`). If the real Rust target generates getters or wraps returns in an
   `Option`, these break. Directly parallels Go being the one target that diverged.
4. **`Cast` downcast** — `(<v>).downcast_ref::<T>().unwrap()` assumes an `Any`-style
   downcast on context trait objects. If contexts are enums (not trait objects), the
   idiomatic form is a `match`/`if let`, and this template would need to change shape.
5. **Listener as `#[derive(Default)] struct` implementing a defaulted trait, and
   `writeln!(self.output(), …)` *inside* the listener** — inside a listener method,
   `self` is the listener, not the recognizer, so `self.output()` presumes the
   listener also carries the sink. Java/C# thread an explicit `TextWriter` into the
   `LeafListener` constructor for exactly this reason; an ideal Rust listener likely
   needs the sink injected too (e.g. `LeafListener { out }`), which would change the
   listener templates' struct/ctor shape.
