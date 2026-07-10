# `Rust.test.stg` — honest reference & runtime-gap analysis

> **Migration status (July 2026): the render-then-compile pipeline is
> implemented and is the harness's only pipeline — the legacy
> template-recognition path (pattern-matching descriptor markup and
> simulating its output) has been deleted.** `antlr4-runtime-testsuite`
> renders each descriptor grammar through `Rust.test.stg` with the real
> StringTemplate engine (`tools/stg-render/RenderGrammar.java` via the
> ANTLR jar) and generates with `antlr4-rust-gen --actions embedded`,
> which splices the
> rendered Rust action/predicate bodies verbatim after `$`-attribute
> translation (`src/bin_support/embedded.rs` — the Rust analog of ANTLR's
> `ActionTranslator`). The four capability axes below are now generated:
> an output sink (`self.output()`), typed context views with positional
> accessors and public attribute fields (`FromRuleContext` downcast),
> typed attrs snapshots replacing the int-only `int_return` map, and
> `@members` as real struct fields / impl items. Listener traits and a
> typed walker bridge cover the listener suite. Validating the reference
> against the real ST engine also surfaced blind-authoring defects; each
> correction is documented in `Rust.test.stg.design-notes.md` under
> "Reference corrections". The historical analysis below is preserved
> as-written.

Companion to `Rust.test.stg` and `Rust.test.stg.design-notes.md` in this
directory. This document records **how** the reference `.stg` was produced and
**what diffing it against our actual runtime reveals**.

## What a `.test.stg` is, and why we want an honest one

ANTLR's `runtime-testsuite` descriptors are **target-neutral**: their grammars
contain StringTemplate calls like `<writeln("$e.v")>`, `<Cast("BinaryContext",
"$ctx")>`, `<SubContextLocal(...)>`. Before generating a parser, ANTLR renders
the descriptor grammar through the *current target's* `templates/<Lang>.test.stg`
group — a pure `.g4`-text → `.g4`-text StringTemplate render
(`new ST(group, descriptor.grammar).render()`, `RuntimeTests.prepareGrammars`).
The rendered `.g4` then contains real, target-language action code that the
target's code generator compiles. Every official target (Java, Go, Python3,
C#, C++, Swift, Dart, TypeScript, JavaScript, PHP) ships one such file.

We have none. Our conformance harness instead **recognizes a subset of template
markup inside the generator** (`src/bin_support/templates.rs`) rather than
rendering the `.stg` into `.g4`. Producing a real `Rust.test.stg` is the
framework-blessed path — but only if it is *honest*: it must describe an ideal,
idiomatic Rust target, not be reverse-engineered from whatever our current
runtime happens to expose (that would re-introduce fixture-fitting).

## How this reference was produced (blind methodology)

`Rust.test.stg` + `Rust.test.stg.design-notes.md` were authored by a fresh agent
**deliberately blind to this runtime**. It was allowed to read only the other
targets' `.test.stg` files (Java as the canonical/complete reference; Go as the
reference for getter/exported-name conventions) plus idiomatic Rust. It was
**forbidden** from reading `src/`, the code-generation template
(`tool/resources/org/antlr/v4/tool/templates/codegen/Rust/Rust.stg`), any
generated parser, or `src/bin_support/templates.rs`.

The point: force a specification of what an *ideal* Rust ANTLR target should
expose, uncontaminated by our current shortcuts. The design notes therefore
include a 21-capability **"Runtime API surface this assumes"** checklist — a
ready-made gap backlog — plus an honest "least certain" section flagging the
five renderings most likely to diverge from any real runtime.

The result covers all **70** templates with signatures matching `Java.test.stg`
exactly (it even caught that a hand-listed checklist of 69 omitted `StringType`).

## What diffing the ideal against our runtime reveals

Examining the blind reference against what we actually generate (from a kept
`target/antlr-runtime-testsuite/LeftRecursion_MultipleAlternativesWithCommonLabel_1`
work dir) shows a **systematic, four-axis divergence**:

| The ideal `.stg` assumes | Our runtime actually does |
| --- | --- |
| Output via an injected `self.output()` sink (`&mut dyn Write`) | `println!` to **stdout** |
| Typed context structs (`EContext`, `BinaryContext`) with positional child accessors `ctx.e(0)` / `ctx.e_all()` | **Zero** context structs; `fn e()` is a *rule method* returning `ParseTree` |
| Rule return attributes as public fields (`ctx.v`) | Values kept in a runtime `int_return` map keyed by rule-index (`src/tree.rs`) |
| `.downcast_ref::<BinaryContext>()` for labeled-alternative access | No labeled-alt context types to downcast to |

### Architectural crux: render vs. recognize

ANTLR renders `descriptor.grammar` through the `.stg` into a `.g4` full of real
target code — no dependency on the generated parser. Our harness does **not**
render: the `.g4` it writes still contains raw `<Cast(...)>` / `<writeln(...)>`
markup, and the *generator* pattern-matches template bodies. So "use
`Rust.test.stg` the ANTLR way" means render `.stg` → `.g4` **and then compile
it** — which our generator cannot do today, because it emits none of the four
capabilities above.

## Implications

- **Committing this as a reference/spec** (what this directory is) is cheap and
  valuable: it is the honest definition of an ideal Rust target plus a concrete,
  itemized gap backlog.
- **Actually using it during conformance runs the ANTLR way** is a large,
  multi-phase migration: add a StringTemplate render step to the harness *and*
  grow the generator to emit the assumed API (output sink, typed context
  structs, positional accessors, downcast). It is effectively the work of
  "becoming a real ANTLR Rust target," which is a much larger scope than the
  original goal (making `--sem-unknown=error` a viable default).
- **Trap to avoid:** adapting the `.stg` to render into our *current* idiom
  (`println!`, `int_return` map) so it "works" today. That re-introduces exactly
  the fixture-fitting removed when the `RuleValue` re-evaluator was deleted. The
  reference must stay honest.

## The five least-certain renderings (most likely to diverge)

From the design notes, ranked by risk — these are where a real Rust target is
most likely to need bespoke work:

1. **`RuleInvocationStack`** (and debug-formatted list prints): Rust `{:?}` on
   `Vec<String>` yields quoted `["a", "b"]`, not the Java-style `[a, b]` the
   descriptors byte-compare against. Every other target needed a bespoke
   formatter (Go `PrintArrayJavaStyle`, Python `str_list`). **Highest byte-match
   risk.**
2. **`PositionAdjustingLexerDef` / `PositionAdjustingLexer`**: the entire
   lexer-override model (`base_next_token`/`base_emit`, swappable interpreter,
   codepoint char indexing) is invented; a trait/hook-based customization point
   would reshape it.
3. **`Result` / `Production`**: bet on public return-attribute fields vs. getters
   or `Option`-wrapped returns (Go is the one target that diverged to getters).
4. **`Cast`**: assumes `Any`-style `downcast_ref` on context trait objects; if
   contexts are enums the idiomatic form is `match`/`if let`.
5. **Listener structs calling `self.output()`**: inside a listener method `self`
   is the listener, not the recognizer, so an ideal Rust listener likely needs
   the output sink injected into its struct (as Java/C# thread a `TextWriter`
   into `LeafListener`).
