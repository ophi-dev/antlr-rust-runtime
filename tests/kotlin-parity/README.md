# Kotlin parse-tree parity smoke

Compares the Rust runtime's parse tree against `antlr4-python3-runtime` on the
upstream antlr/grammars-v4 Kotlin grammar.

## What runs

`run.sh` clones the Kotlin grammar files, runs the official ANTLR jar to
generate both Python and Rust parsers, parses every `snippets/*.kt` with
both, and asserts the dumped trees are byte-identical (the Python dumper
emits Rust-Debug-shaped string literals so no normalization is needed).

## Snippets

| File | What it covers |
| --- | --- |
| `snippets/01-nested-types.kt` | nested `interface` / `companion object` / `enum class` with a trailing comma / `sealed class` with inheritance call. Exercises the speculative recognizer's greedy loop tie-breaking on `(enumEntry NL*)+`, FIRST-set lookahead pruning across deeply nested rule alternatives, and left-recursive boundary folding under `expression`. |
| `snippets/02-dataframe.kt` | imports with wildcards, `@DataSchema` annotation, a `dataFrameOf("int")(1).group { int }.into("group").cast<B>(verify = false)` builder chain with a string literal arg, generic type arguments, and indexed access `df[0].group.int`. Exercises the lexer mode stack (string `"..."` open/close inside `(...)` parens — `popMode` must scope back to the right outer mode), function-call chains with multiple parenthesized argument lists, and named arguments. |

Add new snippets by dropping `.kt` files into `snippets/`; the harness picks
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
