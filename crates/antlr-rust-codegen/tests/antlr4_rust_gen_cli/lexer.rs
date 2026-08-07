#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

/// Editors on Windows commonly save `.g4` sources with a UTF-8 byte order mark
/// and CRLF line endings. Both must generate exactly like the plain spelling.
#[test]
fn byte_order_mark_and_crlf_grammars_generate_like_plain_sources() {
    let temp = temporary_directory("bom-crlf");
    let plain = "lexer grammar Letters;\nA: 'a';\nWS: [ \\t\\r\\n]+ -> skip;\n";
    let crlf = "lexer grammar Letters;\r\nA: 'a';\r\nWS: [ \\t\\r\\n]+ -> skip;\r\n";
    let cases = [
        ("plain", plain.to_owned()),
        ("bom", format!("\u{feff}{plain}")),
        ("crlf", crlf.to_owned()),
        ("bom-crlf", format!("\u{feff}{crlf}")),
    ];

    let mut generated = Vec::new();
    for (name, text) in cases {
        let case = temp.path().join(name);
        let grammar = case.join("Letters.g4");
        let out = case.join("generated");
        fs::create_dir_all(&case).expect("case directory should be writable");
        fs::write(&grammar, &text).expect("grammar should be writable");

        let output = run_antlr4_rust_gen(&[
            grammar.as_os_str(),
            OsStr::new("--out-dir"),
            out.as_os_str(),
        ]);
        assert!(
            output.status.success(),
            "{name}: stdout: {}\nstderr: {}",
            utf8(&output.stdout),
            utf8(&output.stderr)
        );
        generated.push((
            name,
            fs::read_to_string(out.join("letters.rs")).expect("lexer should be emitted"),
        ));
    }

    let (_, expected) = &generated[0];
    for (name, actual) in &generated[1..] {
        assert_eq!(
            actual, expected,
            "{name} output differs from the plain source"
        );
    }
    assert!(
        !expected.contains('\r'),
        "generated code should not carry carriage returns"
    );
}

/// A `.tokens` vocabulary is a generated sidecar parsed line by line, so it
/// never reaches the grammar lexer's byte order mark handling. Both a marked
/// and a CRLF sidecar must still supply the recorded token numbers.
#[test]
fn byte_order_mark_and_crlf_token_vocabularies_are_honored() {
    for (name, vocabulary) in [
        ("plain", "ID=1\nNUM=2\n".to_owned()),
        ("bom", "\u{feff}ID=1\nNUM=2\n".to_owned()),
        ("crlf", "ID=1\r\nNUM=2\r\n".to_owned()),
        ("bom-crlf", "\u{feff}ID=1\r\nNUM=2\r\n".to_owned()),
    ] {
        let temp = temporary_directory("vocab-bom");
        let grammar = temp.path().join("P.g4");
        let out = temp.path().join("generated");
        fs::write(temp.path().join("V.tokens"), &vocabulary)
            .expect("vocabulary should be writable");
        fs::write(
            &grammar,
            "parser grammar P;\n\
             options { tokenVocab=V; }\n\
             r: ID NUM;\n",
        )
        .expect("grammar should be writable");

        let output = run_antlr4_rust_gen(&[
            grammar.as_os_str(),
            OsStr::new("--lib"),
            temp.path().as_os_str(),
            OsStr::new("--out-dir"),
            out.as_os_str(),
        ]);
        assert!(
            output.status.success(),
            "{name}: stdout: {}\nstderr: {}",
            utf8(&output.stdout),
            utf8(&output.stderr)
        );
        let parser = fs::read_to_string(out.join("p.rs")).expect("parser should be emitted");
        assert!(
            parser.contains("ID: i32 = 1;") && parser.contains("NUM: i32 = 2;"),
            "{name}: vocabulary numbers were not imported"
        );
    }
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn combined_literal_tokens_are_public_and_lexable() {
    let temp = temporary_directory("combined-literal-tokens");
    let grammar = temp.path().join("T.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar T;\n\
         greeting : 'hello' NAME 'world' ;\n\
         NAME : [a-zA-Z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let constants = ["t_lexer.rs", "t_parser.rs"].map(|module| {
        let generated =
            fs::read_to_string(out.join(module)).expect("generated module should be readable");
        let (_, after_eof) = generated
            .split_once("pub const EOF: i32 = antlr4_runtime::TOKEN_EOF;")
            .expect("generated token constants should start with EOF");
        let (after_eof, _) = after_eof
            .split_once("\n\n")
            .expect("generated token constants should form their own block");
        (
            module,
            format!("pub const EOF: i32 = antlr4_runtime::TOKEN_EOF;{after_eof}"),
        )
    });
    insta::assert_debug_snapshot!("combined_literal_token_constants", constants);

    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod combined_literal_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::{self, TParser};
    use antlr4_runtime::{
        ByteStream, CommonTokenStream, InputStream, Parser as _, Token as _,
    };
    use std::io::Cursor;

    #[test]
    fn recognizes_implicit_literal_rules() {
        let lexer = TLexer::new(InputStream::new("hello Alice world"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = TParser::new(tokens);
        parser.greeting().expect("literal input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }

    #[test]
    fn generated_helpers_accept_named_text_and_byte_streams() {
        let input = InputStream::from_reader_with_source_name(
            Cursor::new(b"hello Alice world"),
            "greeting.txt",
        )
        .expect("in-memory UTF-8 should be readable");
        let output =
            t_parser::parse_stream_with_parser(input, TLexer::new, TParser::greeting)
                .expect("named text stream should parse");
        assert_eq!(output.parser.number_of_syntax_errors(), 0);
        assert!(
            output
                .parser
                .token_store()
                .iter()
                .all(|token| token.source_name() == "greeting.txt")
        );

        let parsed = t_parser::parse_stream(
            ByteStream::new(b"hello Alice world".to_vec()),
            TLexer::new,
            TParser::greeting,
        )
        .expect("byte stream should parse through the generic helper");
        assert_eq!(parsed.tokens().len(), 4);
        assert_eq!(
            parsed
                .tokens()
                .iter()
                .map(|token| token.byte_span())
                .collect::<Vec<_>>(),
            [Some(0..5), Some(6..11), Some(12..17), Some(17..17)]
        );
    }
}
"#,
    );
}

#[test]
fn generated_lex_helpers_expose_hidden_and_custom_channels() {
    let temp = temporary_directory("lex-helper-channels");
    let grammar = temp.path().join("Channels.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "lexer grammar Channels;\n\
         channels { COMMENTS }\n\
         WORD: [a-z]+;\n\
         COMMENT: '#' ~[\\r\\n]* -> channel(COMMENTS);\n\
         WS: [ \\t]+ -> channel(HIDDEN);\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    assert_generated_project(
        temp.path(),
        &["channels.rs"],
        r##"
#[cfg(test)]
mod lex_helper_tests {
    use super::channels::{
        self, CHANNEL_COMMENTS, COMMENT, Channels, WORD, WS,
    };
    use antlr4_runtime::{
        ByteStream, DEFAULT_CHANNEL, HIDDEN_CHANNEL, TOKEN_EOF, Token as _,
    };

    #[test]
    fn buffers_every_emitted_channel_without_a_parser() {
        let tokens = channels::lex("alpha # note", Channels::new);
        let observed = tokens
            .tokens()
            .map(|token| (
                token.token_type(),
                token.channel(),
                token.text_or_empty(),
            ))
            .collect::<Vec<_>>();

        assert_eq!(
            &observed[..3],
            [
                (WORD, DEFAULT_CHANNEL, "alpha"),
                (WS, HIDDEN_CHANNEL, " "),
                (COMMENT, CHANNEL_COMMENTS, "# note"),
            ]
        );
        assert_eq!(observed[3].0, TOKEN_EOF);
        assert_eq!(tokens.number_of_source_errors(), 0);
    }

    #[test]
    fn accepts_arbitrary_character_streams() {
        let tokens =
            channels::lex_stream(ByteStream::new(b"beta".to_vec()), Channels::new);
        let first = tokens.get(0).expect("word token");

        assert_eq!(first.token_type(), WORD);
        assert_eq!(first.byte_span(), Some(0..4));
    }
}
"##,
    );
}
