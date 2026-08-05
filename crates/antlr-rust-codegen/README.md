# antlr-rust-codegen

Rust library and `antlr4-rust-gen` command for compiling ANTLR v4 grammars
into source compatible with `antlr-rust-runtime`.

Use it from a build script:

```toml
# x-release-please-start-version
[dependencies]
antlr-rust-runtime = "0.29.0"

[build-dependencies]
antlr-rust-codegen = "0.29.0"
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
