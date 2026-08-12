// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Character-stream benchmarks.
//!
//! Every generated lexer walks an [`InputStream`] one code point at a time, so
//! stream construction, lookahead, text extraction, and position accounting are
//! the innermost loops of tokenization. ASCII and non-ASCII inputs are measured
//! separately because the stream keeps a dedicated representation for each.

use antlr4_runtime::{CharStream, InputStream, IntStream, TextInterval};

fn main() {
    divan::main();
}

const ASCII_LINE: &str = "grammar Sample; rule : ID '=' NUMBER ';' ; // trailing comment\n";
const UNICODE_LINE: &str = "règle : IDENTIFIÉ '≔' NOMBRE '；' ; // commentaire à la fin\n";

/// Roughly 64 KiB of input, the size of a mid-sized grammar or source file.
const REPEATS: usize = 1024;

fn ascii_source() -> String {
    ASCII_LINE.repeat(REPEATS)
}

fn unicode_source() -> String {
    UNICODE_LINE.repeat(REPEATS)
}

/// Consumes the whole stream through the lookahead API used by lexer loops.
fn scan(stream: &mut InputStream) -> i64 {
    let mut checksum = 0_i64;
    loop {
        let symbol = stream.la(1);
        if symbol == antlr4_runtime::EOF {
            return checksum;
        }
        checksum += i64::from(symbol);
        stream.consume();
    }
}

/// Stream construction, which scans the input and may build the index tables.
mod construct {
    use super::{InputStream, ascii_source, unicode_source};

    #[divan::bench]
    fn ascii(bencher: divan::Bencher<'_, '_>) {
        let source = ascii_source();
        bencher.bench_local(|| InputStream::new(&source));
    }

    #[divan::bench]
    fn unicode(bencher: divan::Bencher<'_, '_>) {
        let source = unicode_source();
        bencher.bench_local(|| InputStream::new(&source));
    }
}

/// Full-input lookahead and consume loop.
mod scan {
    use super::{InputStream, ascii_source, scan, unicode_source};

    #[divan::bench]
    fn ascii(bencher: divan::Bencher<'_, '_>) {
        let source = ascii_source();
        bencher
            .with_inputs(|| InputStream::new(&source))
            .bench_local_values(|mut stream| scan(&mut stream));
    }

    #[divan::bench]
    fn unicode(bencher: divan::Bencher<'_, '_>) {
        let source = unicode_source();
        bencher
            .with_inputs(|| InputStream::new(&source))
            .bench_local_values(|mut stream| scan(&mut stream));
    }
}

/// Token-text extraction: one interval per simulated token.
mod text {
    use super::{CharStream, InputStream, IntStream, TextInterval, ascii_source, unicode_source};

    const TOKEN_WIDTH: usize = 8;

    fn extract(stream: &InputStream) -> usize {
        let size = stream.size();
        let mut total = 0;
        let mut start = 0;
        while start + TOKEN_WIDTH < size {
            total += stream
                .text(TextInterval::new(start, start + TOKEN_WIDTH - 1))
                .len();
            start += TOKEN_WIDTH;
        }
        total
    }

    #[divan::bench]
    fn ascii(bencher: divan::Bencher<'_, '_>) {
        let stream = InputStream::new(ascii_source());
        bencher.bench_local(|| extract(&stream));
    }

    #[divan::bench]
    fn unicode(bencher: divan::Bencher<'_, '_>) {
        let stream = InputStream::new(unicode_source());
        bencher.bench_local(|| extract(&stream));
    }
}

/// Line/column accounting over the whole input.
mod position {
    use super::{CharStream, InputStream, IntStream, ascii_source, unicode_source};

    #[divan::bench]
    fn summary_ascii(bencher: divan::Bencher<'_, '_>) {
        let stream = InputStream::new(ascii_source());
        let size = stream.size();
        bencher.bench_local(|| stream.position_summary(0, size));
    }

    #[divan::bench]
    fn summary_unicode(bencher: divan::Bencher<'_, '_>) {
        let stream = InputStream::new(unicode_source());
        let size = stream.size();
        bencher.bench_local(|| stream.position_summary(0, size));
    }
}
