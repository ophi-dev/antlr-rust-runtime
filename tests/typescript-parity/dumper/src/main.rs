#![allow(clippy::disallowed_methods)]

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use antlr4_runtime::{
    CommonTokenStream, InputStream, ParseTree, Parser, TOKEN_EOF, Token, TokenStore,
};

#[allow(dead_code, unused_imports, unreachable_pub, unused_qualifications)]
mod generated {
    pub mod type_script_lexer;
    pub mod type_script_parser;
}
mod typescript_lexer_base;
mod typescript_parser_base;

use generated::type_script_lexer::TypeScriptLexer;
use generated::type_script_parser::{self, TypeScriptParser};
use typescript_lexer_base::TypeScriptLexerBase;
use typescript_parser_base::TypeScriptParserBase;

fn dump_tree<S: AsRef<str>>(
    out: &mut dyn Write,
    tree: &ParseTree,
    tokens: &TokenStore,
    rule_names: &[S],
    depth: usize,
) -> io::Result<()> {
    let pad = "  ".repeat(depth);
    match tree {
        ParseTree::Rule(rule) => {
            let name = rule_names
                .get(rule.context().rule_index())
                .map_or("<?>", AsRef::as_ref);
            writeln!(
                out,
                "{pad}Rule({name}, children={})",
                rule.context().children().len()
            )?;
            for child in rule.context().children() {
                dump_tree(out, child, tokens, rule_names, depth + 1)?;
            }
        }
        ParseTree::Terminal(token) => writeln!(out, "{pad}Term({:?})", token.text(tokens))?,
        ParseTree::Error(token) => writeln!(out, "{pad}Err({:?})", token.text(tokens))?,
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut tokens_only = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = args.next().map(PathBuf::from),
            "--tokens" => tokens_only = true,
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
    let source = match fs::read_to_string(&input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("failed to read {}: {error}", input.display());
            return ExitCode::FAILURE;
        }
    };

    if tokens_only {
        let lexer = TypeScriptLexer::with_typed_hooks(
            InputStream::new(&source),
            TypeScriptLexerBase::with_strict_default(false),
        );
        let mut stream = CommonTokenStream::new(lexer);
        stream.fill();
        let errors = stream.drain_source_errors();
        if !errors.is_empty() {
            for error in errors {
                eprintln!("line {}:{} {}", error.line, error.column, error.message);
            }
            return ExitCode::FAILURE;
        }
        for token in stream.tokens() {
            if token.token_type() != TOKEN_EOF {
                println!(
                    "{}\t{}\t{:?}",
                    token.token_type(),
                    token.channel(),
                    token.text()
                );
            }
        }
        return ExitCode::SUCCESS;
    }

    let lexer = TypeScriptLexer::with_typed_hooks(
        InputStream::new(&source),
        TypeScriptLexerBase::with_strict_default(false),
    );
    let tokens = CommonTokenStream::new(lexer);
    let mut parser = TypeScriptParser::with_typed_hooks(tokens, TypeScriptParserBase);
    let tree = match parser.program() {
        Ok(tree) => tree,
        Err(error) => {
            eprintln!("parse failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    if parser.number_of_syntax_errors() != 0 {
        eprintln!(
            "parse produced {} syntax error(s)",
            parser.number_of_syntax_errors()
        );
        return ExitCode::FAILURE;
    }
    if let Err(error) = dump_tree(
        &mut io::stdout().lock(),
        &tree,
        parser.token_store(),
        type_script_parser::rule_names(),
        0,
    ) {
        eprintln!("write failed: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
