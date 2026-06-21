# Kotlin parse-tree parity smoke

Compares the Rust runtime's parse tree against `antlr4-python3-runtime` on the
upstream antlr/grammars-v4 Kotlin grammar.

## What runs

`run.sh` copies the Kotlin grammar from a local antlr/grammars-v4 checkout
(passed via `--grammars-v4` / `GRAMMARS_V4`), runs the official ANTLR jar
to generate both Python and Rust parsers, parses every `snippets/*.kt`
with the `kotlinFile` entry rule and every `script-snippets/*.kts` with the
`script` entry rule, and asserts the dumped trees are byte-identical. The
Python dumper emits Rust-Debug-shaped string literals so no normalization is
needed.

## Snippets

| File | What it covers |
| --- | --- |
| `snippets/01-nested-types.kt` | nested `interface` / `companion object` / `enum class` with a trailing comma / `sealed class` with inheritance call. Exercises the speculative recognizer's greedy loop tie-breaking on `(enumEntry NL*)+`, FIRST-set lookahead pruning across deeply nested rule alternatives, and left-recursive boundary folding under `expression`. |
| `snippets/02-dataframe.kt` | imports with wildcards, `@DataSchema` annotation, a `dataFrameOf("int")(1).group { int }.into("group").cast<B>(verify = false)` builder chain with a string literal arg, generic type arguments, and indexed access `df[0].group.int`. Exercises the lexer mode stack (string `"..."` open/close inside `(...)` parens — `popMode` must scope back to the right outer mode), function-call chains with multiple parenthesized argument lists, and named arguments. |
| `snippets/03-string-templates.kt` | line string templates with `${foo()}`, suffix text after `}`, and following declarations. Regresses the Kotlin `RCURL` mode-pop path: the generated lexer must return to line-string mode after `}` so template suffix text and later statements are tokenized correctly. |
| `script-snippets/01-top-level-calls.kts` | valid top-level call statements under `script`. |
| `script-snippets/02-top-level-val.kts` | issue #32 regression for a valid top-level `val` followed by newline. |
| `script-snippets/03-blank-separated-vals.kts` | issue #32 regression for blank-line-separated top-level `val` declarations followed by a call. |
| `script-snippets/04-newline-member-access.kts` | valid script expression where a newline before member access is a continuation, not a statement separator. |
| `script-snippets/05-indented-top-level-val.kts` | issue #32 regression with indentation trivia before the next top-level `val`. |

Add new file snippets by dropping `.kt` files into `snippets/`, and script
snippets by dropping `.kts` files into `script-snippets/`; the harness picks
them up automatically.

## Running locally

```sh
tests/kotlin-parity/run.sh \
    --antlr-jar /path/to/antlr-4.13.2-complete.jar \
    --grammars-v4 /path/to/antlr/grammars-v4/checkout
```

Both arguments can also be supplied via `ANTLR4_JAR` / `GRAMMARS_V4` env vars.
The CI workflow at `.github/workflows/kotlin-parity.yml` wires the same
script into a runner that fetches the jar and grammar repo on demand.
