use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_antlr4_rust_gen(args: &[impl AsRef<OsStr>]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_antlr4-rust-gen"))
        .args(args)
        .output()
        .expect("antlr4-rust-gen should run")
}

fn assert_generated_modules_compile(temp_dir: &Path, modules: &[&str]) {
    assert_generated_project(temp_dir, modules, "");
}

fn assert_generated_project(temp_dir: &Path, modules: &[&str], test_source: &str) {
    let project = temp_dir.join("compile-generated");
    let source = project.join("src");
    fs::create_dir_all(&source).expect("generated-module check should be writable");
    fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\n\
             name = \"compile-generated\"\n\
             version = \"0.0.0\"\n\
             edition = \"2024\"\n\
             \n\
             [dependencies]\n\
             antlr-rust-runtime = {{ path = {:?} }}\n",
            env!("CARGO_MANIFEST_DIR")
        ),
    )
    .expect("generated-module manifest should be writable");
    let declarations = modules
        .iter()
        .map(|module| {
            let module_name = module.strip_suffix(".rs").unwrap_or(module);
            format!("#[path = {module:?}]\nmod {module_name};")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        source.join("lib.rs"),
        format!("{declarations}\n{test_source}"),
    )
    .expect("generated-module crate root should be writable");
    for module in modules {
        fs::copy(temp_dir.join("generated").join(module), source.join(module))
            .expect("generated module should be copied into the check crate");
    }

    let output = Command::new(env!("CARGO"))
        .args([
            if test_source.is_empty() {
                "check"
            } else {
                "test"
            },
            "--quiet",
            "--offline",
            "--manifest-path",
            project
                .join("Cargo.toml")
                .to_str()
                .expect("temporary path should be UTF-8"),
        ])
        .env("CARGO_TARGET_DIR", project.join("target"))
        .output()
        .expect("cargo check should run");
    assert!(
        output.status.success(),
        "generated project failed\nstdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
}

fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("process output should be UTF-8")
}

fn temporary_directory(label: &str) -> TempDirectory {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "antlr4-rust-gen-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary directory should be writable");
    TempDirectory(path)
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn long_help_describes_source_only_cli() {
    let output = run_antlr4_rust_gen(&["--help"]);

    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status.code(),
        utf8(&output.stderr)
    );
    assert_eq!(utf8(&output.stderr), "");

    let stdout = utf8(&output.stdout);
    assert!(
        stdout.starts_with("Usage: antlr4-rust-gen [OPTIONS] ROOT.g4...\n"),
        "{stdout}"
    );
    assert!(stdout.contains("  -I, --lib DIR"), "{stdout}");
    assert!(stdout.contains("  --option-hook KEY=VALUE"), "{stdout}");
    assert!(stdout.contains("  -listener, --listener"), "{stdout}");
    assert!(stdout.contains("  -no-listener, --no-listener"), "{stdout}");
    assert!(stdout.contains("  -visitor, --visitor"), "{stdout}");
    assert!(stdout.contains("  -no-visitor, --no-visitor"), "{stdout}");
    assert!(!stdout.contains("--lexer "), "{stdout}");
    assert!(!stdout.contains("--parser "), "{stdout}");
    assert!(!stdout.contains("--grammar "), "{stdout}");
    assert!(stdout.contains("  -V, --version"), "{stdout}");
    assert!(stdout.contains("  -h, --help"), "{stdout}");
}

#[test]
fn short_help_exits_successfully_on_stdout() {
    let output = run_antlr4_rust_gen(&["-h"]);

    assert!(output.status.success(), "stderr: {}", utf8(&output.stderr));
    assert_eq!(utf8(&output.stderr), "");
    assert!(utf8(&output.stdout).contains("Usage: antlr4-rust-gen"));
}

#[test]
fn long_and_short_version_exit_successfully_on_stdout() {
    for flag in ["--version", "-V"] {
        let output = run_antlr4_rust_gen(&[flag]);

        assert!(
            output.status.success(),
            "{flag} status: {:?}\nstderr: {}",
            output.status.code(),
            utf8(&output.stderr)
        );
        assert_eq!(utf8(&output.stderr), "");
        assert_eq!(
            utf8(&output.stdout),
            concat!("antlr4-rust-gen ", env!("CARGO_PKG_VERSION"), "\n")
        );
    }
}

#[test]
fn help_flag_as_option_value_is_not_intercepted() {
    let output = run_antlr4_rust_gen(&["--option-hook", "--help"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn version_flag_as_option_value_is_not_intercepted() {
    let output = run_antlr4_rust_gen(&["--option-hook", "--version"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn missing_roots_report_usage_on_stderr() {
    let args: [&str; 0] = [];
    let output = run_antlr4_rust_gen(&args);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("at least one grammar root is required"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn unknown_arguments_report_usage_on_stderr() {
    let output = run_antlr4_rust_gen(&["--bogus"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("unknown argument --bogus"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn legacy_interp_flags_are_rejected() {
    for flag in [
        "--lexer",
        "--parser",
        "--grammar",
        "--lexer-name",
        "--parser-name",
    ] {
        let output = run_antlr4_rust_gen(&[flag, "Legacy.interp"]);
        assert!(!output.status.success(), "{flag} unexpectedly succeeded");
        let stderr = utf8(&output.stderr);
        assert!(
            stderr.contains(&format!("unknown argument {flag}")),
            "{stderr}"
        );
    }
}

#[test]
fn option_hook_requires_a_key_value_assignment() {
    let output = run_antlr4_rust_gen(&["--option-hook", "superClass"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");
    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn positional_lexer_root_emits_rust_and_manifest() {
    let temp = temporary_directory("lexer");
    let grammar = temp.path().join("Letters.g4");
    let out = temp.path().join("generated");
    fs::write(&grammar, "lexer grammar Letters;\nA: 'a';\n").expect("grammar should be writable");

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
    assert!(out.join("letters.rs").is_file());
    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    assert!(manifest.contains("\"name\": \"Letters\""), "{manifest}");
    assert!(manifest.contains("\"kind\": \"lexer\""), "{manifest}");
}

#[test]
fn combined_root_suffixes_alternative_contexts_and_listener_methods() {
    let temp = temporary_directory("combined-contexts");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/combined-contexts/Shapes.g4");
    let out = temp.path().join("generated");

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
    assert!(out.join("shapes_lexer.rs").is_file());
    let parser =
        fs::read_to_string(out.join("shapes_parser.rs")).expect("parser should be emitted");
    for expected in [
        "pub struct StartContext<'a, State = StoredTreeContext>",
        "pub struct SingleLabelContext<'a, State = StoredTreeContext>",
        "pub struct ManyLabelContext<'a, State = StoredTreeContext>",
        "pub trait ShapesListener<E = std::convert::Infallible>",
        "pub struct ShapesTreeWalker",
        "pub type ParseTreeWalker = ShapesTreeWalker",
        "fn enter_every_rule(&mut self",
        "fn enter_single_label(&mut self",
        "fn enter_many_label(&mut self",
        "pub fn atom_children(&self) -> impl Iterator<Item = AtomContext<'a>>",
        "pub fn first(&self) -> Result<AtomContext<'a>, MissingChildError>",
        "pub fn rest(&self) -> impl Iterator<Item = AtomContext<'a>>",
        "pub fn value(&self) -> Result<AtomContext<'a>, MissingChildError>",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert!(
        !parser.contains("_all(&self)"),
        "generated contexts must not expose allocating Java-style list accessors\n{parser}"
    );
    assert!(
        !parser.contains("antlr4_runtime::{{"),
        "generated imports must not contain redundant nested braces\n{parser}"
    );
    assert!(
        !parser.contains("pub trait ShapesVisitor"),
        "visitor generation must remain opt-in\n{parser}"
    );
    assert_generated_project(
        temp.path(),
        &["shapes_lexer.rs", "shapes_parser.rs"],
        r#"
#[cfg(test)]
mod typed_label_tests {
    use super::shapes_lexer::ShapesLexer;
    use super::shapes_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn list_and_repeated_single_labels_keep_antlr_semantics() {
        let lexer = ShapesLexer::new(InputStream::new("a,b,c"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = ShapesParser::new(tokens);
        let root = parser.start().expect("list input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let many = parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .downcast_ref::<ManyLabelContext>()
            .expect("comma-separated input uses the many alternative");
        assert_eq!(
            many
                .rest()
                .map(|atom| atom.rule_node().node().text())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        let lexer = ShapesLexer::new(InputStream::new("a b c"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = ShapesParser::new(tokens);
        let root = parser.latest().expect("repeated input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let latest = parsed
            .tree()
            .as_rule()
            .expect("latest rule")
            .downcast_ref::<LatestContext>()
            .expect("latest context");
        assert_eq!(latest.atom_children().count(), 3);
        assert_eq!(
            latest
                .value()
                .expect("one or more atoms guarantees a value")
                .rule_node()
                .node()
                .text(),
            "c"
        );
    }
}
"#,
    );
}

#[test]
fn visitor_and_typed_walk_dispatch_labeled_left_recursion() {
    let temp = temporary_directory("typed-tree-walkers");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/typed-tree-walkers/Calculator.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--visitor"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser =
        fs::read_to_string(out.join("calculator_parser.rs")).expect("parser should be emitted");
    for expected in [
        "pub trait CalculatorVisitor",
        "pub trait CalculatorVisitable",
        "pub trait CalculatorListener",
        "pub struct CalculatorTreeWalker",
        "fn visit_multiply_label(&mut self",
        "fn visit_add_label(&mut self",
        "fn visit_number_label(&mut self",
        "fn default_result(&mut self) -> Self::Result;",
        "pub trait CalculatorListener<E = std::convert::Infallible>",
        "pub fn expression_children(&self) -> impl Iterator<Item = ExpressionContext<'a>>",
        "pub fn left(&self) -> Result<ExpressionContext<'a>, MissingChildError>",
        "pub fn right(&self) -> Result<ExpressionContext<'a>, MissingChildError>",
        "pub fn star_token(&self) -> Option<TerminalNode<'a>>",
        "pub fn int_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn eof_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn literal(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn choice(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn other(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn wildcard(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn plus_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn star_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "__token_children_matching(self.__node",
        "track_context_alt_numbers: true",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert!(
        !parser.contains("pub fn INT(") && !parser.contains("_all(&self)"),
        "generated contexts must expose Rust-shaped token and collection accessors\n{parser}"
    );

    assert_generated_project(
        temp.path(),
        &["calculator_lexer.rs", "calculator_parser.rs"],
        r#"
#[cfg(test)]
mod typed_tree_tests {
    use super::calculator_lexer::CalculatorLexer;
    use super::calculator_parser::*;
    use antlr4_runtime::{
        CommonTokenStream, InputStream, MissingChildError, Parser as _, RuleNodeView,
    };

    struct Eval;

    impl CalculatorVisitor for Eval {
        type Result = Result<i64, MissingChildError>;

        fn default_result(&mut self) -> Self::Result {
            Ok(0)
        }

        fn visit_start(&mut self, ctx: &StartContext) -> Self::Result {
            self.visit(ctx.expression()?)
        }

        fn visit_number_label(&mut self, ctx: &NumberLabelContext) -> Self::Result {
            Ok(ctx
                .int_token()?
                .to_string()
                .parse()
                .expect("integer token"))
        }

        fn visit_multiply_label(&mut self, ctx: &MultiplyLabelContext) -> Self::Result {
            let left = self.visit(ctx.left()?)?;
            let right = self.visit(ctx.right()?)?;
            if ctx.star_token().is_some() {
                Ok(left * right)
            } else {
                Ok(left / right)
            }
        }

        fn visit_add_label(&mut self, ctx: &AddLabelContext) -> Self::Result {
            let left = self.visit(ctx.left()?)?;
            let right = self.visit(ctx.right()?)?;
            if ctx.plus_token().is_some() {
                Ok(left + right)
            } else {
                Ok(left - right)
            }
        }
    }

    #[derive(Default)]
    struct Trace {
        events: Vec<&'static str>,
        entered_rules: usize,
        exited_rules: usize,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct TraceError;

    impl CalculatorListener<TraceError> for Trace {
        fn enter_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), TraceError> {
            self.entered_rules += 1;
            Ok(())
        }

        fn exit_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), TraceError> {
            self.exited_rules += 1;
            Ok(())
        }

        fn enter_multiply_label(
            &mut self,
            _ctx: &MultiplyLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("enter:multiply");
            Ok(())
        }

        fn exit_multiply_label(
            &mut self,
            _ctx: &MultiplyLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("exit:multiply");
            Ok(())
        }

        fn enter_add_label(&mut self, _ctx: &AddLabelContext) -> Result<(), TraceError> {
            self.events.push("enter:add");
            Ok(())
        }

        fn exit_add_label(&mut self, _ctx: &AddLabelContext) -> Result<(), TraceError> {
            self.events.push("exit:add");
            Ok(())
        }

        fn enter_number_label(
            &mut self,
            _ctx: &NumberLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("enter:number");
            Ok(())
        }

        fn exit_number_label(
            &mut self,
            _ctx: &NumberLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("exit:number");
            Ok(())
        }
    }

    struct FailingTrace;

    impl CalculatorListener<&'static str> for FailingTrace {
        fn enter_multiply_label(
            &mut self,
            _ctx: &MultiplyLabelContext,
        ) -> Result<(), &'static str> {
            Err("stop at multiply")
        }
    }

    #[test]
    fn evaluates_and_walks_exact_typed_alternatives() {
        let lexer = CalculatorLexer::new(InputStream::new("2 + 8 / 2"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = CalculatorParser::new(tokens);
        let root = parser.start().expect("calculator input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        assert!(
            parsed
                .tree()
                .descendants()
                .filter_map(antlr4_runtime::Node::as_rule)
                .all(|rule| rule.alt_number() == 0),
            "typed dispatch metadata must not become display-visible alt numbers"
        );
        let start = parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .downcast_ref::<StartContext>()
            .expect("typed start context");
        assert_eq!(start.eof_token().expect("required EOF").to_string(), "<EOF>");

        assert_eq!(Eval.visit(parsed.tree()).expect("evaluation succeeds"), 6);

        let mut trace = Trace::default();
        trace.walk(parsed.tree()).expect("typed listener walk");
        assert_eq!(
            trace.events,
            [
                "enter:add",
                "enter:number",
                "exit:number",
                "enter:multiply",
                "enter:number",
                "exit:number",
                "enter:number",
                "exit:number",
                "exit:multiply",
                "exit:add",
            ]
        );
        assert_eq!(trace.entered_rules, 6);
        assert_eq!(trace.exited_rules, 6);

        assert_eq!(
            FailingTrace.walk(parsed.tree()),
            Err("stop at multiply"),
            "listener domain errors must stop and escape the generated walker"
        );

        let start = parsed.tree().as_rule().expect("start rule");
        let expression = start
            .child_rule(RULE_EXPRESSION)
            .expect("top-level expression");
        let add = expression
            .downcast_ref::<AddLabelContext>()
            .expect("top-level expression is addition");
        assert_eq!(add.rule_node().node().id(), expression.node().id());
        assert_eq!(add.expression_children().count(), 2);
        assert!(add.plus_token().is_some());
        assert!(add.minus_token().is_none());
        assert_eq!(
            add.left().expect("left expression").rule_node().node().id(),
            expression
                .child_rules(RULE_EXPRESSION)
                .next()
                .expect("left expression")
                .node()
                .id()
        );
        assert!(expression.downcast_ref::<MultiplyLabelContext>().is_none());

        let right = expression
            .child_rules(RULE_EXPRESSION)
            .nth(1)
            .expect("right expression");
        assert!(right.downcast_ref::<MultiplyLabelContext>().is_some());
        assert!(right.downcast_ref::<AddLabelContext>().is_none());

        let lexer = CalculatorLexer::new(InputStream::new("+*1-"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = CalculatorParser::new(tokens);
        let root = parser
            .labeled_tokens()
            .expect("labeled token input should parse");
        let parsed = parser.into_parsed_file(root);
        let labeled = parsed
            .tree()
            .as_rule()
            .expect("labeledTokens rule")
            .downcast_ref::<LabeledTokensContext>()
            .expect("typed labeledTokens context");
        assert_eq!(labeled.literal().expect("literal label").to_string(), "+");
        assert_eq!(labeled.choice().expect("set label").to_string(), "*");
        assert_eq!(labeled.other().expect("not-set label").to_string(), "1");
        assert_eq!(labeled.wildcard().expect("wildcard label").to_string(), "-");

        let lexer = CalculatorLexer::new(InputStream::new("+*"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = CalculatorParser::new(tokens);
        let root = parser
            .literal_tokens()
            .expect("literal token input should parse");
        let parsed = parser.into_parsed_file(root);
        let literal_tokens = parsed
            .tree()
            .as_rule()
            .expect("literalTokens rule")
            .downcast_ref::<LiteralTokensContext>()
            .expect("typed literalTokens context");
        assert_eq!(
            literal_tokens
                .plus_token()
                .expect("required literal PLUS")
                .to_string(),
            "+"
        );
        assert_eq!(
            literal_tokens
                .star_token()
                .expect("required literal STAR")
                .to_string(),
            "*"
        );
        assert_eq!(
            literal_tokens
                .eof_token()
                .expect("required literal EOF")
                .to_string(),
            "<EOF>"
        );
    }
}
"#,
    );
}

#[test]
fn listener_and_visitor_generation_can_be_disabled_independently() {
    let temp = temporary_directory("tree-walker-flags");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/combined-contexts/Shapes.g4");
    let visitor_only = temp.path().join("visitor-only");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("-no-listener"),
        OsStr::new("-visitor"),
        OsStr::new("--out-dir"),
        visitor_only.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(visitor_only.join("shapes_parser.rs"))
        .expect("parser should be emitted");
    assert!(parser.contains("pub trait ShapesVisitor"), "{parser}");
    assert!(!parser.contains("pub trait ShapesListener"), "{parser}");
    assert!(!parser.contains("pub struct ShapesTreeWalker"), "{parser}");
    assert!(!parser.contains("pub type ParseTreeWalker"), "{parser}");

    let neither = temp.path().join("neither");
    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--no-listener"),
        OsStr::new("--visitor"),
        OsStr::new("--no-visitor"),
        OsStr::new("--out-dir"),
        neither.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser =
        fs::read_to_string(neither.join("shapes_parser.rs")).expect("parser should be emitted");
    assert!(!parser.contains("pub trait ShapesVisitor"), "{parser}");
    assert!(!parser.contains("pub trait ShapesListener"), "{parser}");
}

#[test]
fn colliding_rule_and_alternative_label_context_names_compile() {
    let temp = temporary_directory("context-name-collision");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/context-name-collision/T.g4");
    let out = temp.path().join("generated");

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
    let parser = fs::read_to_string(out.join("t.rs")).expect("parser should be emitted");
    for expected in [
        "pub struct ObjectCreationExpressionContext<'a, State = StoredTreeContext>",
        "pub struct ObjectCreationExpressionLabelContext<'a, State = StoredTreeContext>",
        "pub struct ParenthesizedLabelContext<'a, State = StoredTreeContext>",
        "fn enter_object_creation_expression(&mut self",
        "fn enter_object_creation_expression_label(&mut self",
        "fn enter_parenthesized_label(&mut self",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert_generated_modules_compile(temp.path(), &["t.rs"]);
}

#[test]
fn embedded_parser_semantics_satisfy_strict_manifest_checks() {
    let temp = temporary_directory("embedded-parser-semantics");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/embedded-parser-semantics/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    assert_eq!(
        manifest.matches("\"disposition\": \"translated\"").count(),
        2
    );
    assert_eq!(manifest.matches("\"template\": \"Embedded\"").count(), 2);
    assert_generated_modules_compile(temp.path(), &["t_lexer.rs", "t_parser.rs"]);
}

#[test]
fn imported_predicate_manifest_uses_its_structural_source_owner() {
    let temp = temporary_directory("imported-predicate");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         delegated: {featureEnabled()}? ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(
        &tokens,
        "lexer grammar Tokens;\n\
         ID: [a-z]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    assert!(manifest.contains("\"name\": \"Root\""), "{manifest}");
    assert!(
        manifest.contains("\"body\": \"featureEnabled()\""),
        "{manifest}"
    );
    assert!(manifest.contains("\"line\": 2"), "{manifest}");
}

#[test]
fn imported_parser_predicate_generates_typed_hook_from_structural_body() {
    let temp = temporary_directory("imported-parser-hook");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\ndelegated: {isTypeName()}? ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(parser.contains("pub trait RootHooks"), "{parser}");
    assert!(parser.contains("fn is_type_name"), "{parser}");
    assert!(
        parser.contains("(1, 0) => Some(self.0.is_type_name(ctx))"),
        "{parser}"
    );
}

#[test]
fn imported_lexer_action_generates_typed_hook_from_structural_body() {
    let temp = temporary_directory("imported-lexer-hook");
    let root = temp.path().join("RootLexer.g4");
    let delegate = temp.path().join("DelegateLexer.g4");
    let patterns = temp.path().join("patterns.toml");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "lexer grammar RootLexer;\nimport DelegateLexer;\nB: 'b';\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "lexer grammar DelegateLexer;\nA: 'a' {this.handle(\"a\");};\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(
        &patterns,
        "version = 1\n\
         [[helper]]\n\
         kind = \"lexer-action\"\n\
         name = \"handle\"\n\
         arguments = \"string\"\n\
         returns = \"unit\"\n\
         lower = \"hook\"\n",
    )
    .expect("semantic patterns should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--sem-patterns"),
        patterns.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let lexer = fs::read_to_string(out.join("root_lexer.rs")).expect("lexer should be emitted");
    assert!(lexer.contains("pub trait RootLexerHooks"), "{lexer}");
    assert!(lexer.contains("fn handle"), "{lexer}");
    assert!(lexer.contains("self.0.handle(ctx, \"a\")"), "{lexer}");
}

#[test]
fn imported_rule_arguments_and_locals_use_structural_call_owners() {
    let temp = temporary_directory("imported-rule-arguments");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: outer EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         outer locals [boolean seen=false]\n\
             : {$seen=true;} {$seen}? target[true]\n\
             ;\n\
         target[boolean enabled]: ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("let mut __antlr_local_seen = false;"),
        "{parser}"
    );
    assert!(
        parser.contains("parse_generated_rule_2_dispatch(1, false)"),
        "{parser}"
    );
}

#[test]
fn imported_embedded_action_uses_structural_rule_and_transition_owner() {
    let temp = temporary_directory("imported-embedded-action");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         delegated: {writeln!(self.output(), \"delegated\").unwrap();} ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("writeln!(self.output(), \"delegated\").unwrap();"),
        "{parser}"
    );
}

#[test]
fn multiple_roots_and_repeatable_library_paths_are_resolved() {
    let temp = temporary_directory("roots");
    let first_lib = temp.path().join("first-lib");
    let second_lib = temp.path().join("second-lib");
    let out = temp.path().join("generated");
    fs::create_dir_all(&first_lib).expect("first library directory should be writable");
    fs::create_dir_all(&second_lib).expect("second library directory should be writable");
    fs::write(
        first_lib.join("Shared.g4"),
        "lexer grammar Shared;\nA: 'a';\n",
    )
    .expect("import should be writable");
    let root = temp.path().join("Root.g4");
    let other = temp.path().join("Other.g4");
    fs::write(&root, "lexer grammar Root;\nimport Shared;\nB: 'b';\n")
        .expect("root should be writable");
    fs::write(&other, "lexer grammar Other;\nC: 'c';\n").expect("second root should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        other.as_os_str(),
        OsStr::new("-I"),
        first_lib.as_os_str(),
        OsStr::new("--lib"),
        second_lib.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert!(out.join("root.rs").is_file());
    assert!(out.join("other.rs").is_file());
    assert!(!out.join("shared.rs").exists());
}

#[test]
fn invalid_source_emits_diagnostics_without_partial_outputs() {
    let temp = temporary_directory("invalid");
    let grammar = temp.path().join("Broken.g4");
    let out = temp.path().join("generated");
    fs::write(&grammar, "lexer grammar Broken;\nA: 'unterminated;\n")
        .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");
    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("Broken.g4"), "{stderr}");
    assert!(stderr.contains("G4F002"), "{stderr}");
    assert!(!stderr.contains("unknown argument"), "{stderr}");
    assert!(
        !out.exists()
            || fs::read_dir(&out)
                .expect("output should be readable")
                .next()
                .is_none(),
        "failed compilation emitted partial output"
    );
}

#[test]
fn imported_source_diagnostics_report_the_import_path() {
    let temp = temporary_directory("imported-diagnostic");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/imported-diagnostic");
    let root = fixture.join("Root.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("--lib"),
        fixture.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    let stderr = utf8(&output.stderr);
    let delegate_diagnostic = format!("error[G4F003]: {}", fixture.join("Delegate.g4").display());
    let wrong_root_diagnostic = format!("error[G4F003]: {}", root.display());
    assert!(stderr.contains(&delegate_diagnostic), "{stderr}");
    assert!(!stderr.contains(&wrong_root_diagnostic), "{stderr}");
    assert!(!out.exists(), "failed compilation emitted output");
}

#[test]
fn unsupported_grammar_options_warn_and_exact_hooks_acknowledge_them() {
    let temp = temporary_directory("options");
    let grammar = temp.path().join("OptionsLexer.g4");
    fs::write(
        &grammar,
        "lexer grammar OptionsLexer;\noptions { superClass = MyLexerBase; }\nA: 'a';\n",
    )
    .expect("grammar should be writable");

    let unsupported_out = temp.path().join("unsupported");
    let unsupported = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        unsupported_out.as_os_str(),
        OsStr::new("--require-full-semantics"),
    ]);
    assert!(!unsupported.status.success());
    let stderr = utf8(&unsupported.stderr);
    assert!(
        stderr.contains("warning: unsupported grammar option: superClass=MyLexerBase at 2:10"),
        "{stderr}"
    );
    assert!(stderr.contains("--option-hook KEY=VALUE"), "{stderr}");
    assert!(!unsupported_out.exists());

    let acknowledged_out = temp.path().join("acknowledged");
    let acknowledged = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        acknowledged_out.as_os_str(),
        OsStr::new("--option-hook"),
        OsStr::new("superClass=MyLexerBase"),
        OsStr::new("--require-full-semantics"),
    ]);
    assert!(
        acknowledged.status.success(),
        "stderr: {}",
        utf8(&acknowledged.stderr)
    );
    let stderr = utf8(&acknowledged.stderr);
    assert!(!stderr.contains("unsupported grammar option"), "{stderr}");
    assert!(
        !stderr.contains("require caller-owned target behavior"),
        "{stderr}"
    );
    assert!(acknowledged_out.join("options_lexer.rs").is_file());
}
