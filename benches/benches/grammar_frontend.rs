//! End-to-end benchmarks for the `ANTLRv4` grammar frontend.
//!
//! Parsing a `.g4` source drives the whole runtime stack in one call: the
//! compiled lexer DFA, `CommonTokenStream` buffering, adaptive parser
//! prediction, and parse-tree construction. The grammars used here are the
//! checked-in bootstrap fixtures, so the benchmarks stay reproducible without
//! network access.

use antlr_rust_g4_parser::{SourceFile, SourceId, parse_source, parse_source_recovering};

fn main() {
    divan::main();
}

/// Small lexer grammar (~80 bytes): measures fixed frontend setup cost.
const LEXER_BASIC: &str = include_str!(
    "../../crates/antlr-rust-codegen/tests/codegen-direct/fixtures/lexer-basic/LexerBasic.g4"
);

/// The `ANTLRv4` lexer grammar (~8 KiB): modes, lexer commands, actions.
const ANTLR_V4_LEXER: &str = include_str!(
    "../../crates/antlr-rust-g4-parser/tests/frontend/bootstrap/src/grammars/ANTLRv4Lexer.g4"
);

/// The `ANTLRv4` parser grammar (~8 KiB): labeled alternatives, EBNF blocks.
const ANTLR_V4_PARSER: &str = include_str!(
    "../../crates/antlr-rust-g4-parser/tests/frontend/bootstrap/src/grammars/ANTLRv4Parser.g4"
);

/// The Java grammar (~31 KiB): the largest checked-in combined grammar.
const JAVA: &str = include_str!(
    "../../crates/antlr-rust-g4-parser/tests/frontend/bootstrap/tests/grammars/Java.g4"
);

/// Grammar with missing rule terminators, exercising parser error recovery.
const MALFORMED: &str = r"
grammar Malformed;

entry : first second ;

first
    : A B
    | C D

second
    : E F
    | G H
    ;

A : 'a' ;
B : 'b'
C : 'c' ;
D : 'd' ;
E : 'e' ;
F : 'f' ;
G : 'g' ;
H : 'h' ;
";

fn parse(source: &str) -> SourceFile {
    parse_source(SourceId::new(0), "bench.g4", source).expect("fixture grammar parses cleanly")
}

/// Full lex + parse + parse-tree construction of a grammar file.
mod parse_grammar {
    use super::{ANTLR_V4_LEXER, ANTLR_V4_PARSER, JAVA, LEXER_BASIC, parse};

    #[divan::bench]
    fn lexer_basic(bencher: divan::Bencher<'_, '_>) {
        bencher.bench_local(|| parse(LEXER_BASIC));
    }

    #[divan::bench]
    fn antlr_v4_lexer(bencher: divan::Bencher<'_, '_>) {
        bencher.bench_local(|| parse(ANTLR_V4_LEXER));
    }

    #[divan::bench]
    fn antlr_v4_parser(bencher: divan::Bencher<'_, '_>) {
        bencher.bench_local(|| parse(ANTLR_V4_PARSER));
    }

    #[divan::bench]
    fn java(bencher: divan::Bencher<'_, '_>) {
        bencher.bench_local(|| parse(JAVA));
    }
}

/// Error-recovery parsing, which additionally builds diagnostics.
mod recover {
    use super::{MALFORMED, SourceId, parse_source_recovering};

    #[divan::bench]
    fn malformed_grammar(bencher: divan::Bencher<'_, '_>) {
        bencher.bench_local(|| {
            parse_source_recovering(SourceId::new(0), "bench.g4", MALFORMED)
                .expect("recovering parse always produces a source file")
        });
    }
}

/// Post-parse traversal of the concrete syntax tree and its token table.
mod traverse {
    use super::{JAVA, parse};

    #[divan::bench]
    fn cst_descendants(bencher: divan::Bencher<'_, '_>) {
        let file = parse(JAVA);
        let cst = file.cst();
        bencher.bench_local(|| cst.descendants(cst.root_id()).count());
    }

    #[divan::bench]
    fn token_text(bencher: divan::Bencher<'_, '_>) {
        let file = parse(JAVA);
        bencher.bench_local(|| {
            file.tokens()
                .iter()
                .map(|token| file.token_text(token).len())
                .sum::<usize>()
        });
    }

    #[divan::bench]
    fn line_column_lookup(bencher: divan::Bencher<'_, '_>) {
        let file = parse(JAVA);
        bencher.bench_local(|| {
            file.tokens()
                .iter()
                .filter_map(|token| file.line_column(token.span.bytes.start))
                .count()
        });
    }
}
