//! Parses a Kotlin source file with the Rust runtime's generated parser and
//! prints the parse tree in the same diff-friendly form as
//! `tests/kotlin-parity/dump_python.py`. The smoke workflow compares the two
//! outputs to enforce parser-tree parity with antlr4-python3-runtime.
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use antlr4_runtime::{CommonTokenStream, InputStream, ParseTree};

mod generated {
    #![allow(dead_code, unused_imports, unreachable_pub, unused_qualifications)]
    pub mod kotlin_lexer;
    pub mod kotlin_parser;
}

use generated::kotlin_lexer::KotlinLexer;
use generated::kotlin_parser::KotlinParser;

fn dump(out: &mut dyn Write, tree: &ParseTree, rule_names: &[String], depth: usize) -> io::Result<()> {
    let pad = "  ".repeat(depth);
    match tree {
        ParseTree::Rule(rl) => {
            let name = rule_names
                .get(rl.context().rule_index())
                .map(String::as_str)
                .unwrap_or("<?>");
            writeln!(
                out,
                "{pad}Rule({name}, children={})",
                rl.context().children().len()
            )?;
            for c in rl.context().children() {
                dump(out, c, rule_names, depth + 1)?;
            }
        }
        ParseTree::Terminal(t) => writeln!(out, "{pad}Term({:?})", t.text())?,
        ParseTree::Error(e) => writeln!(out, "{pad}Err({:?})", e.text())?,
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = args.next().map(PathBuf::from),
            "--output" => output = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(input) = input else {
        eprintln!("missing --input <path>");
        return ExitCode::from(2);
    };
    let src = match fs::read_to_string(&input) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("failed to read {}: {err}", input.display());
            return ExitCode::from(1);
        }
    };

    let lexer = KotlinLexer::new(InputStream::new(&src));
    let tokens = CommonTokenStream::new(lexer);
    let mut parser = KotlinParser::new(tokens);
    let tree = match parser.kotlin_file() {
        Ok(tree) => tree,
        Err(err) => {
            eprintln!("parse failed: {err}");
            return ExitCode::from(1);
        }
    };

    let rule_names: Vec<String> = KotlinParser::<KotlinLexer<InputStream>>::metadata()
        .rule_names()
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    let mut sink: Box<dyn Write> = match output {
        Some(path) => match fs::File::create(&path) {
            Ok(file) => Box::new(io::BufWriter::new(file)),
            Err(err) => {
                eprintln!("failed to create {}: {err}", path.display());
                return ExitCode::from(1);
            }
        },
        None => Box::new(io::stdout().lock()),
    };
    if let Err(err) = dump(sink.as_mut(), &tree, &rule_names, 0) {
        eprintln!("write failed: {err}");
        return ExitCode::from(1);
    }
    if let Err(err) = sink.flush() {
        eprintln!("flush failed: {err}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
