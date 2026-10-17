# antlr-rust-codegen

Rust library and `antlr4-rust-gen` command for compiling ANTLR v4 grammars
into source compatible with `antlr-rust-runtime`.

Use it from a build script:

```toml
# x-release-please-start-version
[dependencies]
antlr-rust-runtime = "0.35.0"

[build-dependencies]
antlr-rust-codegen = "0.35.0"
# x-release-please-end
```

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let generation = antlr_rust_codegen::Builder::new()
        .grammar("grammar/MyLexer.g4")
        .grammar("grammar/MyParser.g4")
        .library_directory("grammar")
        .out_dir(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
        .generate()?;
    generation.emit_rerun_if_changed();
    Ok(())
}
```

Successful generations expose structured compiler warnings through
`Generation::diagnostics()`, including the diagnostic code, severity, source
path, and exact UTF-8 byte span. `Generation::warnings()` retains the rendered
CLI messages.

Or install the command:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-gen
```

## Semantic pattern TOML

`--sem-patterns FILE` is parsed by the lockstep
`antlr-rust-toml-parser` implementation package. Its lexer and parser are
checked-in outputs from this generator, and its handwritten facade builds TOML
values through the generated validated listener. Codegen owns only the semantic
pattern schema layered over those values.

Pattern files must now be valid TOML. Quote string-valued fields; unknown
fields, malformed sections, and trailing junk fail generation instead of being
silently accepted by the former permissive scalar reader.

The pinned grammar and regeneration instructions live under
`third_party/toml-grammar/`. Generated recognizers are refreshed with
`tools/toml-syntax/update-generated.sh --update` and verified with `--check`.

## Trusted grammars-v4 Rust support

`antlr4-rust-gen` recognizes a sibling `Rust/transformGrammar.py` for the
current C and Java target-support folders in `antlr/grammars-v4`. Before
executing one, an interactive terminal shows its repository, revision, and
content fingerprint and offers four choices: trust once, trust the exact
revision, trust the repository, or abort.

The approved transform runs with bundled RustPython in a child process over a
disposable copy of the complete grammar directory. This is a trusted-source
workflow, not a security sandbox. The original checkout is never modified.
The child has a 30-second deadline and bounded output capture; use
`--rust-support-timeout SECONDS` for a slower trusted transform.
Transformed grammars, shipped `.rs` support files, and `rust-support.json` are
emitted with the generated sources so the inputs remain inspectable. Embedded
actions, strict semantics, generated-only parser coverage, and `superClass`
acknowledgement are selected automatically for those staged sources.
Shipped helpers are available through the generated recognizer module's stable
`rust_support` re-export, for example
`c_parser::rust_support::c_parser_base::reset_symbol_table()`.

Non-interactive use fails before execution and prints the exact opt-in:

```bash
antlr4-rust-gen JavaLexer.g4 JavaParser.g4 \
    --trust-rust-support sha256:<fingerprint> \
    --out-dir src/generated
```

Persisted choices live in
`$XDG_CONFIG_HOME/antlr4-rust/trusted-support.json` (or the platform config
directory). Set `ANTLR4_RUST_TRUST_STORE` to use a different file. Automatic
transform execution is currently an `antlr4-rust-gen` CLI feature; the
`Builder` API does not execute sibling scripts.

The bundled Python compatibility target is deliberately limited to the imports
and filesystem operations used by the two existing grammars-v4 transforms
(`c/Rust` and `java/java/Rust`). A future transform with additional Python
requirements is a new compatibility decision, not implicitly supported.

The package also provides `antlr4-rust-testrig` for running a grammar directly
against UTF-8 files or stdin. A grammar-only project needs no `Cargo.toml`,
generated Rust sources, or runtime dependency; only a Rust toolchain is
required because TestRig invokes Cargo internally:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-testrig
antlr4-rust-testrig JSON.g4 json --tokens --tree example.json
```

Use `tokens` as the start rule for a lexer grammar. For split grammars, pass
the parser grammar first and pair it with `--lexer-grammar`:

```bash
antlr4-rust-testrig MyParser.g4 start \
    --lexer-grammar MyLexer.g4 --lib grammar inputs/*.txt
```

The command generates and compiles a temporary grammar-specific runner with the
matching runtime because Rust cannot reflectively load recognizer types or call
a rule by name. It removes the temporary package afterward while retaining
Cargo build artifacts in the current user's cache for subsequent invocations;
set `ANTLR4_RUST_TESTRIG_TARGET_DIR` to override that location. `--trace`,
`--diagnostics`, and `--sll` expose the corresponding parser modes;
exact-ambiguity diagnostics and SLL prediction are mutually exclusive. The
command processes every input and exits non-zero if generation, compilation,
input reading, lexing, or parsing reports an error, so the same command can be
used as a test runner.

The runtime, codegen, and internal parser packages are released in lockstep.

## Internal module ownership

`src/grammar/semantics.rs` is the documented exception to the 2,000-line
production-module guideline. Its semantic pass deliberately keeps vocabulary
numbering, symbol diagnostics, action binding, and the resulting
`SemanticBindings` mutation in one owner because they share source-coordinate
and declaration-order invariants. The independently mutable grammar
integration, validation, transform registry, transform analyses, and transform
passes are separate modules; splitting this remaining pass would expose its
partially built vocabulary and binding state as a wider internal contract
without creating an independent optimization or rendering boundary.
