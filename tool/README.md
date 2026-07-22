# Rust Target Design

The ANTLR tool integration will be implemented as a normal ANTLR target named `Rust`.

Clean-room target design:

- generate compact Rust modules that reference `antlr4_runtime`
- emit immutable `GrammarMetadata`
- emit lexer/parser wrappers over runtime base types and ATN simulators
- emit listener and visitor traits from the grammar model
- keep semantic predicates and actions as generated dispatch methods
- avoid copying another Rust target's template structure

The target implementation lives under:

- `tool/src/org/antlr/v4/codegen/target/RustTarget.java`
- `tool/resources/org/antlr/v4/tool/templates/codegen/Rust/Rust.stg`

The runtime ATN simulator is present in Rust. The production generator is
`src/bin/antlr4-rust-gen.rs`, which compiles `.g4` roots and their dependency
graph directly into Rust modules.

The checked-in Java target files remain intentionally small while the direct `-Dlanguage=Rust` templates are expanded. They should emit the same artifacts as `antlr4-rust-gen`: constants, metadata, serialized ATN arrays, lexer/parser wrappers, and semantic action/predicate dispatch hooks.
