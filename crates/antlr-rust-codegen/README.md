# antlr-rust-codegen

Rust library and `antlr4-rust-gen` command for compiling ANTLR v4 grammars
into source compatible with `antlr-rust-runtime`.

Use it from a build script:

```toml
# x-release-please-start-version
[dependencies]
antlr-rust-runtime = "0.27.0"

[build-dependencies]
antlr-rust-codegen = "0.27.0"
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

Or install the command:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-gen
```

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
