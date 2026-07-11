# Issue #63: JavaScript and TypeScript target-action support plan

Status: implemented 2026-07-11

Issue: [#63](https://github.com/ophi-dev/antlr-rust-runtime/issues/63)

## Outcome

Generate faithful Rust lexers and parsers from the unmodified official
`antlr/grammars-v4` JavaScript and TypeScript grammars by combining:

- grammar-agnostic runtime and generator hook plumbing,
- JavaScript-family mappings in `patterns/javascript.toml`, and
- checked-in Rust lexer/parser support modules analogous to grammars-v4's
  JavaScript and TypeScript lexer/parser base implementations.

The delivered demos must build from the upstream `.g4` files, tokenize and
parse representative JavaScript and TypeScript correctly, and document the
complete builds from the ANTLR jar through runnable Rust binaries.

This is a target-support bundle, not an attempt to translate arbitrary
target-language actions into Rust. Grammar-family rule names, token names,
helper names, and state machines remain outside `src/`; the generic generator
learns their shape from grammar metadata plus the user-selected pattern file.

## Scope decisions

### In scope

- Wire generated lexers to `SemanticHooks` for stateful predicates and
  committed actions.
- Generate named, typed lexer hooks for helper calls selected by a pattern
  file, matching the typed parser-hook model already in the repository.
- Extend typed helper matching to a small, explicit call syntax: optional
  `this.`/`self.` receiver, optional boolean negation, and declared literal
  arguments. This covers parser calls such as `this.n("static")` without an
  arbitrary source-language parser.
- Provide a post-emission lexer callback so grammar-specific state can observe
  the last emitted default-channel token, as the official JavaScript bases do.
- Expose read-only raw-token access through `ParserSemCtx` for hidden-channel
  line-terminator checks.
- Add Rust JavaScript and TypeScript lexer/parser base modules, runnable parity
  demos, CI, and build documentation.
- Make strict generation succeed with every authored JavaScript and TypeScript
  semantic coordinate reported as `translated` or `hooked` in
  `semantics.json`.

### Not in scope

- No JavaScript or TypeScript special cases in `src/bin/antlr4-rust-gen.rs` or
  runtime modules.
- No execution or general transpilation of `{this.*}` source text.
- No edits to the upstream JavaScript grammars and no target-specific grammar
  preprocessor.
- No silent `assume-true`, `assume-false`, ignored-action, or
  `--allow-unsupported-lexer-actions` path in the working demo.
- Do not make JavaScript support depend on `--require-generated-parser`.
  The generated parser may use the existing ATN-interpreter fallback for rules
  the direct recursive-descent compiler cannot yet emit. Closing the 59-rule
  direct-codegen gap is independent performance work; predicates and actions
  must remain faithful on both paths.
- No arbitrary-expression argument lowering. Typed helpers accept declared
  string, boolean, and integer literals; the official TypeScript grammar's
  `p("of")` and `n("get"|"set")` calls are inside that boundary.

## Why this boundary

The official Go, Python, C#, C++, Java, and JavaScript targets do not put this
state in the generic ANTLR runtime. They ship grammar-specific base classes
beside the grammar. Rust should follow the same ownership boundary:

```text
official JavaScript/TypeScript .g4 + patterns/javascript.toml
                       |
                       v
       antlr4-rust-gen emits typed hook coordinates
                       |
              +--------+--------+
              |                 |
              v                 v
   generated lexer/parser    semantics.json
              |
              v
 checked-in grammar-specific Rust lexer/parser bases
              |
              v
       runnable Rust parser crate
```

Hard-coding `ProcessOpenBrace`, `IsRegexPossible`, grammar token constants,
or equivalent behavior in the generator/runtime would violate the repository's
codegen boundary and make the next stateful grammar another runtime patch.

`--actions embedded` is also the wrong path. The grammar bodies are written in
another target language, and embedded mode requires every parser rule to use
the direct generated path. Hooking the semantic coordinates works with both
direct rules and the interpreter fallback.

## Design

### 1. Give semantic helper patterns an explicit kind and signature

Extend the existing `[[helper]]` pattern model without changing the default
meaning of current files. Conceptually:

```toml
[[helper]]
kind = "lexer-predicate"
name = "IsRegexPossible"
arguments = ""
returns = "bool"
lower = "hook"

[[helper]]
kind = "lexer-action"
name = "ProcessOpenBrace"
arguments = ""
returns = "unit"
lower = "hook"

[[helper]]
kind = "parser-predicate"
name = "n"
arguments = "string"
returns = "bool"
lower = "hook"
```

`kind` defaults to `parser-predicate` so existing pattern files remain valid.
The first implementation supports zero arguments and one or more declared
string/bool/integer literals; it does not accept arbitrary expressions.

Replace the current bare-call recognizer with a small helper-call parser that
returns a structured value:

```rust
struct SemanticHelperCall {
    name: String,
    arguments: Vec<SemanticLiteral>,
    negated: bool,
}
```

Accepted forms are deliberately narrow:

- `helper()` / `this.helper()` / `self.helper()`
- `!helper()` / `!this.helper()` for boolean predicates
- `helper("literal")` when the pattern declares a string argument
- an optional trailing semicolon for action statements

Whitespace and escaped string literals must be handled, but member chains,
blocks, assignments, and computed arguments remain unknown semantics. Pattern
matching also remains scoped by semantic kind so a same-named parser helper
cannot accidentally match a lexer action.

The generated typed adapter stores the captured literals at the mapped ATN
coordinate. For example, the three `n(...)` coordinates call one required Rust
method with `"static"`, `"get"`, or `"set"`; a negated lexer predicate negates
the typed method result in the generated adapter.

### 2. Add generated-lexer hook ownership

Today a generated lexer owns only `BaseLexer<I>` and always selects a static
closure or no-hook token path. Change its shape to mirror generated parsers:

```rust
pub struct ExampleLexer<I, H = NoSemanticHooks>
where
    I: CharStream,
    H: SemanticHooks,
{
    base: BaseLexer<I>,
    hooks: H,
}
```

Emit:

- `new(input)` for the no-hook compatibility path,
- `with_hooks(input, hooks)` for the numeric `SemanticHooks` escape hatch,
- `with_typed_hooks(input, hooks)` for the normal generated typed-hook path,
- `ExampleLexerHooks`, containing required named predicate/action methods, and
- `ExampleLexerTypedHooks<T>`, adapting those methods to stable ATN
  coordinates.

Required typed methods make a JavaScript base implementation fail at compile
time when an upstream mapped helper is added and the Rust support module has
not caught up. The typed trait should also expose a default
`token_emitted(&CommonToken)` callback; the JavaScript implementation overrides
it to maintain `last_token_type`.

Keep name collision handling deterministic. If normalized action and predicate
names collide, suffix the generated method with `_action` or `_pred`; conflicting
signatures for the same normalized helper name are a generation error naming
both source bodies.

### 3. Compose translated lexer semantics with user hooks

Do not change the signatures of the existing public closure-based token APIs.
Add a composed runtime entry point, with interpreted and compiled-DFA variants,
used by generated lexers. Its dispatch order is:

For a custom action on the accepted path:

1. execute a built-in/generated translation such as the portable `popMode`
   lowering when it owns the coordinate;
2. otherwise call `SemanticHooks::lexer_action`;
3. if still unhandled, apply the configured unknown-semantic policy.

For a speculative predicate:

1. use a generated translation when it returns `Some(bool)`;
2. otherwise call `SemanticHooks::lexer_sempred`;
3. if it returns `None`, apply the configured policy.

This preserves mixed grammars: adding one hooked coordinate must not bypass
existing translated predicates, lexer commands, position adjusters, or the
compiled DFA.

After a non-skipped token is created, call
`SemanticHooks::lexer_token_emitted(&CommonToken)` before returning it. The
callback observes hidden/custom-channel tokens too; the JavaScript base itself
filters to the default channel, exactly like the official bases. It is never
called for `skip`/`more` intermediates.

For `UnknownSemanticPolicy::Error`, an unhandled lexer hook records a
coordinate-rich `TokenSourceError`; an unresolved predicate evaluates false so
lexing can terminate deterministically. Deduplicate repeated speculative hits
for the same coordinate and token start. The JavaScript parity harness must
assert that no such diagnostics were produced.

### 4. Make lexer actions honestly hookable

Pass `SemPatternFile` into lexer action collection instead of rejecting a
source action before patterns can classify it. Add a hook action template (or
equivalent side-table entry) distinct from both a translated action and an
unsupported action.

The semantic manifest and renderer must agree:

| Classification | Manifest | Runtime |
| --- | --- | --- |
| portable/known translation | `translated` | generated action/predicate |
| helper selected by pattern | `hooked` | typed or numeric hook |
| explicit assume policy | `assume-*` / `ignored` | documented fallback |
| unsupported under strict policy | `error` | generation fails |

Delete the current generated-lexer rejection for hook-routed predicates. Under
`--sem-unknown error`, an explicitly hooked coordinate is allowed; a coordinate
that matches neither a translation nor a hook pattern still fails generation.

### 5. Add the parser context needed by JavaScriptParserBase

`ParserSemCtx::la`, `lt`, and `token_text` already cover visible-token checks
such as `n("static")`, `p("of")`, `closeBrace`, and the next-token guards.
`lineTerminatorAhead` also needs the raw hidden token immediately before the
current visible token.

Add a read-only absolute buffered-token accessor, for example:

```rust
pub fn token_at(&mut self, index: usize) -> Option<&CommonToken>;
```

Together with the existing `input_index()`, this lets the support module inspect
the previous one or two raw tokens without exposing mutable stream structure or
moving the cursor. Unit tests must cover start-of-stream bounds, on-demand
buffering, hidden channels, and a multiline comment containing `\r`/`\n`.

### 6. Complete `patterns/javascript.toml`

Keep every JavaScript-family spelling in this file. Map:

- lexer predicates: `IsStartOfFile`, `IsRegexPossible`,
  `IsInTemplateString`, and positive/negated `IsStrictMode` calls;
- lexer actions: `ProcessOpenBrace`, `ProcessCloseBrace`,
  `ProcessStringLiteral`, `ProcessTemplateOpenBrace`, and
  `ProcessTemplateCloseBrace`;
- parser predicates: `notLineTerminator`, `lineTerminatorAhead`,
  `notOpenBraceAndNotFunction`, `closeBrace`, and the string-argument `n`
  helper.

Also map the TypeScript-only lexer actions `StartTemplateString`,
`IncreaseTemplateDepth`, and `DecreaseTemplateDepth`; the parser predicate
`notOpenBraceAndNotFunctionAndNotInterface`; and the string-argument `p`
helper. JavaScript generation must not depend on TypeScript coordinates.

With either official grammar, strict generation must produce no `assume-true`,
`assume-false`, `ignored`, or `error` disposition for an authored semantic
coordinate.

### 7. Add JavaScript-specific Rust support modules

Check in these demo sources under `tests/javascript-parity/dumper/src/`:

#### `javascript_lexer_base.rs`

Implement the generated `JavaScriptLexerHooks` trait. Port the behavior of the
official base, with Rust-owned state:

- `scope_strict_modes: Vec<bool>`
- `last_token_type: Option<i32>`
- `use_strict_default: bool`
- `use_strict_current: bool`
- `current_depth: i32`
- `template_depth_stack: Vec<i32>`

Implement the four predicate helpers, five committed action helpers, and
`token_emitted`. Token classification uses constants imported from the
generated lexer module; no numeric token IDs are handwritten. Provide a
constructor that can set the default strict-mode value before moving the base
into the lexer.

Preserve callback timing:

- predicates only read state;
- actions mutate state on the accepted path;
- `ProcessStringLiteral` sees the previous default-channel token and the
  current token text from `LexerSemCtx::text_so_far()`;
- `token_emitted` updates `last_token_type` only after the action has run;
- template and brace depth updates match the official implementation.

#### `javascript_parser_base.rs`

Implement the generated `JavaScriptParserHooks` trait:

- `n`/`next` compares visible token text through `ParserSemCtx`;
- `not_line_terminator` delegates to `line_terminator_ahead`;
- the open-brace/function guard and `close_brace` use generated token
  constants and visible lookahead;
- `line_terminator_ahead` inspects raw hidden tokens, including whitespace
  followed by a line terminator or a multiline comment containing a newline.

These files are examples users can copy into an application. They are not
compiled into the runtime crate and are not generated output.

### 8. Add TypeScript-specific Rust support modules

Check in `typescript_lexer_base.rs` and `typescript_parser_base.rs` under the
TypeScript parity demo. The lexer base mirrors the official TypeScript state
model for strict scopes, previous default-channel tokens, brace depth, and
template depth. The parser base implements `p` with visible lookbehind,
`n` with visible lookahead, hidden-token line-terminator checks, and the
open-brace/function/interface guard using generated token constants.

No TypeScript helper, rule, grammar, or token name is added under `src/`.

## Demo, parity, and documentation layout

Add:

```text
tests/javascript-parity/
  README.md
  run.sh
  dump_python.py
  snippets/
    01-hashbang.js
    02-regex-vs-division.js
    03-strict-mode.js
    04-template-nesting.js
    05-line-terminators.js
    06-class-lookahead.js
  dumper/
    Cargo.toml
    src/main.rs
    src/javascript_lexer_base.rs
    src/javascript_parser_base.rs
    src/generated/              # populated by run.sh, not committed

tests/typescript-parity/
  README.md
  run.sh
  TypeScriptParityDumper.java
  snippets/
    01-types.ts
    02-contextual-helpers.ts
    03-template-nesting.ts
    04-line-terminators.ts
    05-strict-mode.ts
  dumper/
    Cargo.toml
    src/main.rs
    src/typescript_lexer_base.rs
    src/typescript_parser_base.rs
    src/generated/              # populated by run.sh, not committed
```

`run.sh` follows the Kotlin parity harness:

1. accept `--antlr-jar` / `ANTLR4_JAR` and
   `--grammars-v4` / `GRAMMARS_V4`;
2. copy the JavaScript lexer/parser grammars to a temporary clean-room;
3. generate `.interp` metadata with ANTLR 4.13.2;
4. generate the Rust lexer and parser separately with
   `patterns/javascript.toml`, `--sem-unknown error`, and
   `--require-full-semantics`;
5. do not pass `--allow-unsupported-lexer-actions` or
   `--require-generated-parser`;
6. build the Rust dumper with the checked-in base modules;
7. generate a Python reference parser using grammars-v4's Python base files and
   target transform script (or an equivalently pinned official target);
8. compare default-channel token streams and parse trees byte-for-byte;
9. fail if either lexer or parser reports diagnostics.

Add `.github/workflows/javascript-parity.yml`, pinned to ANTLR 4.13.2, the jar
checksum, and the same grammars-v4 commit used by Kotlin parity. Sparse-checkout
only `javascript/javascript`.

Add a parallel pinned TypeScript parity workflow. Its oracle is the official
Java target and the Java base classes shipped beside the TypeScript grammar;
compare both tokens and parse trees byte-for-byte.

Add `docs/javascript-build.md` as the user-facing build guide. It must show:

- prerequisites and pinned downloads;
- ANTLR `.interp` generation from the unmodified grammars;
- the two strict `antlr4-rust-gen` commands;
- which generated modules and base files belong in an application crate;
- explicit construction using `with_typed_hooks` for both lexer and parser;
- selection of `program()` as the compilation-unit entry rule;
- how to inspect token-source and parser diagnostics;
- how to run the repository parity smoke;
- the deliberate omission of `--require-generated-parser` and what the
  interpreter fallback means.

Add `docs/typescript-build.md` with the same strict clean-room workflow, the
TypeScript base files, and explicit construction that exercises the generated
literal-argument hook signatures.

Link the guide from the main `README.md` semantic-actions section.

## Test plan

### Generator unit tests

- Parse helper kinds and zero/literal-argument signatures.
- Match receiverless, `this.`, and `self.` calls without confusing semantic
  kinds.
- Preserve string escapes and reject nonliteral arguments.
- Capture and apply predicate negation.
- Map multiple coordinates to one typed helper with different literals.
- Reject conflicting normalized method signatures.
- Render a generic lexer with `new`, `with_hooks`, `with_typed_hooks`, owned
  hook state, and the typed adapter.
- Render mixed translated/hooked actions and predicates with correct dispatch
  precedence.
- Confirm hook coordinates and `semantics.json` dispositions agree.
- Confirm unknown strict coordinates still fail at generation time.

### Runtime unit tests

- A hooked predicate can read state and select/reject an ATN path.
- A committed action mutates state exactly once and only for the accepting
  rule.
- Translated coordinates take precedence over external hooks.
- The token-emitted callback runs after actions, for default and hidden
  channels, and not for `skip`/`more` intermediates.
- Unhandled strict hooks produce a deduplicated token-source diagnostic.
- The compiled-DFA and interpreted token paths make the same hook calls.
- Existing closure-only and no-hook lexer APIs retain their behavior.
- Raw parser token access does not move the token-stream cursor.

### JavaScript behavior matrix

| Fixture | Required behavior |
| --- | --- |
| hashbang | `#!` is accepted only before any emitted default token |
| regex vs division | `/.../` after an operator/start, division after identifiers/literals |
| strict mode | `"use strict"` updates the current brace scope; strict-only and legacy octal tokens are selected correctly |
| template nesting | `${...}` with nested object braces closes at the correct template depth and restores lexer mode |
| line terminators | ASI predicates see hidden newlines and newlines inside multiline comments |
| class lookahead | `static`, `get`, and `set` exercise captured string arguments to `n(...)` |

Each fixture must match the pinned official target's tokens and parse tree and
produce zero unexpected lexer/parser errors.

### TypeScript behavior matrix

| Fixture | Required behavior |
| --- | --- |
| types | interfaces, generics, function types, and unions parse faithfully |
| contextual helpers | `p("of")` uses lookbehind and `n("get"|"set")` uses lookahead |
| template nesting | TypeScript template-depth actions preserve nested modes and regex tokens |
| line terminators | `notLineTerminator` and ASI helpers observe the official hidden-token rules |
| strict mode | string-literal actions and strict scopes match the official lexer base |

### Regression gates

Run:

```bash
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo run --release --quiet --bin antlr4-runtime-testsuite
tests/kotlin-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
tests/javascript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
tests/typescript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

## Implementation sequence

1. **Semantic call model**: extend the pattern schema and structured helper-call
   parser; update manifest classification; add focused generator tests.
2. **Lexer runtime composition**: add composed dispatch, unknown-policy
   diagnostics, and post-emission observation while preserving existing APIs.
3. **Generated lexer surface**: own hook state, emit typed lexer traits/adapters,
   and remove the current hook rejection.
4. **Parser support surface**: add literal arguments to typed parser adapters
   and raw buffered-token access to `ParserSemCtx`.
5. **JavaScript/TypeScript mappings and bases**: complete
   `patterns/javascript.toml` and add the four checked-in Rust base modules.
6. **Proof and docs**: add snippets, reference dumpers, strict parity runners,
   build guides, README links, and pinned CI workflows.
7. **Full regression pass**: runtime testsuite, Kotlin parity, clippy, unit
   tests, and JavaScript/TypeScript parity.

The sequence keeps each layer independently testable. Steps 1-4 contain no
JavaScript or TypeScript identifiers in generic code; grammar-family content
first appears in step 5.

## Acceptance criteria

- [x] The unmodified official `JavaScriptLexer.g4` and
      `JavaScriptParser.g4` generate Rust modules with ANTLR 4.13.2 metadata.
- [x] Both generator invocations pass `--sem-unknown error` and
      `--require-full-semantics` without
      `--allow-unsupported-lexer-actions`.
- [x] Every authored JavaScript lexer action/predicate and parser predicate is
      `translated` or `hooked` in its semantic manifest; none is assumed,
      ignored, or errored.
- [x] Generated lexers expose owned numeric and typed hook constructors and
      correctly compose translated semantics with hooks on compiled and
      interpreted paths.
- [x] Checked-in `javascript_lexer_base.rs` and
      `javascript_parser_base.rs` contain all grammar-specific behavior needed
      by the demo; no JavaScript rule/helper/token names are added under `src/`.
- [x] Both demo crates build without editing generated Rust or upstream grammar
      files.
- [x] All six behavior fixtures match a pinned official ANTLR target's token
      stream and parse tree byte-for-byte and report no unexpected diagnostics.
- [x] `docs/javascript-build.md` contains a copy/pasteable clean-room build and
      explicit typed-hook construction example, and the main README links it.
- [x] A pinned JavaScript parity workflow is included in CI.
- [x] The unmodified official `TypeScriptLexer.g4` and
      `TypeScriptParser.g4` generate strictly without ignored or assumed
      semantics, including `p("of")` and `n("get"|"set")` literal arguments.
- [x] Checked-in TypeScript Rust base modules cover its template actions,
      strict/brace state, parser lookaround, and interface guard.
- [x] All five TypeScript fixtures match the pinned official Java target's
      token stream and parse tree byte-for-byte.
- [x] `docs/typescript-build.md` provides a copy/pasteable strict build, and a
      pinned TypeScript parity workflow is included in CI.
- [x] Unit tests, strict clippy, the upstream runtime testsuite, and Kotlin
      parity continue to pass.

## Risks and mitigations

- **Source-to-coordinate drift**: pair source calls with ATN coordinates using
  the existing ordered inventory checks, fail on count mismatches, and pin the
  parity grammar commit.
- **Speculative side effects**: typed lexer predicate methods receive only the
  shared/read-only lexer context; only committed actions and token observation
  mutate the grammar-specific base.
- **Hook state bypasses translated behavior**: use a composed runtime API with
  explicit precedence and mixed-coordinate tests rather than switching the
  entire lexer to a hook-only path.
- **Generated API complexity**: mirror the already-established parser
  `with_hooks`/typed-adapter design and retain no-hook constructors for
  compatible grammars.
- **Upstream grammar changes**: required typed methods turn new mapped helpers
  into compile failures, while strict semantic manifests catch new unmapped
  coordinates before the demo can silently parse incorrectly.
- **Direct parser coverage distracts from correctness**: document and test the
  interpreter fallback; track full direct generation separately after parity is
  established.
