# Runtime Requirements

This document records the ANTLR v4 runtime contract this crate implements.

## Runtime Surface

ANTLR generated code expects a target runtime to provide:

- `IntStream`: indexed lookahead, consuming, marking/releasing, seeking, size, and source name.
- `CharStream`: an `IntStream` over Unicode code points with text extraction over intervals.
- `Token`: type, channel, start/stop indices, token index, line, column, text, and source identity.
- `TokenSource`: lazy token production from a lexer or custom source.
- `TokenStream`: token lookahead/look-behind, indexed access, text extraction, and channel-aware buffering.
- `Vocabulary`: literal, symbolic, and display names for token types.
- `Recognizer`: grammar metadata, state, semantic predicate/action hooks, and error listeners.
- `Lexer`: token emission, modes, hidden/default channels, skip/more behavior, and EOF handling.
- `Parser`: token matching, parse tree construction, tracing/listeners, rule contexts, and error strategy integration.
- Parse trees: rule nodes, terminals, error nodes, listeners, visitors, and tree text rendering.
- ATN support: states, transitions, prediction contexts, DFA cache, semantic contexts, lexer actions, and serialized ATN loading.

## Target Contract

The Rust target should generate:

- one Rust module for each lexer/parser grammar
- stable public constants for token and rule indices
- static vocabulary and rule/token/channel/mode names
- serialized ATN data in a runtime-readable form
- lexer/parser structs that compose the runtime base types
- listener and visitor traits when requested
- rule entry methods matching grammar rule names
- action and semantic predicate dispatch hooks

Generated code should avoid global mutable state except for immutable metadata and thread-safe DFA caches.

## Compatibility Strategy

The runtime keeps generated-code shape stable by putting grammar execution behind metadata-backed ATN simulators. Generated lexers/parsers provide static names, vocabulary, and serialized ATN data; the runtime owns deserialization, token recognition, parser rule recognition, and shared stream/tree behavior.

Current parser recognition is intentionally separate from final parse-tree shaping. It validates token sequences through the parser ATN and returns a rule node over the consumed token interval; nested rule contexts, listener callbacks during parsing, adaptive prediction caches, and ANTLR-compatible error recovery are the next compatibility layers.
