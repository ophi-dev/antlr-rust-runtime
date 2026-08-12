// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

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
    assert_eq!(
        parser
            .matches("antlr4_runtime::__antlr4_rust_context!")
            .count(),
        5
    );
    assert!(parser.contains("context_kind: exact(1)"));
    assert!(parser.contains("context_kind: exact(2)"));
    assert!(!parser.contains("fn __from_node("));
    assert!(!parser.contains("impl<State> std::fmt::Display for"));
    assert!(!parser.contains("pub struct __RuleAttrs"));
    for expected in [
        "pub struct StartContext {",
        "pub struct SingleLabelContext {",
        "pub struct ManyLabelContext {",
        "pub trait ShapesListener<E = std::convert::Infallible>",
        "pub struct ShapesTreeWalker",
        "pub type ParseTreeWalker = ShapesTreeWalker",
        "fn enter_every_rule(&mut self",
        "fn enter_single_label(&mut self",
        "fn enter_many_label(&mut self",
        "rule atom_children: many(AtomContext[",
        "label_rule first: required(nth(0), AtomContext[",
        "label_rule rest: many(skip(0), AtomContext[",
        "label_rule value: required(last_after(0), AtomContext[",
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
fn grouped_literal_tokens_are_exposed_on_typed_contexts() {
    let temp = temporary_directory("grouped-token-accessors");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/grouped-token-accessors/T.g4");
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
    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod grouped_token_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn reads_grouped_operators_and_context_text() {
        let lexer = TLexer::new(InputStream::new("left<=right=+="));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = TParser::new(tokens);
        let root = parser.root().expect("operator input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let root = parsed
            .tree()
            .as_rule()
            .expect("root rule")
            .downcast_ref::<RootContext>()
            .expect("typed root context");
        let expression = root.expression().expect("root expression");
        assert_eq!(expression.text(), "left<=right");
        assert_eq!(expression.bop().expect("operator label").to_string(), "<=");
        assert!(expression.le_token().is_some());
        assert!(expression.ge_token().is_none());
        assert!(expression.equal_token().is_none());
        assert!(expression.notequal_token().is_none());
        assert!(expression.assign_token().is_none());
        assert!(expression.add_assign_token().is_none());

        let identifier = expression
            .expression_children()
            .next()
            .expect("left expression")
            .identifier()
            .expect("left identifier");
        assert_eq!(identifier.text(), "left");

        let operators = root
            .operator_sequence()
            .expect("trailing operator sequence");
        assert_eq!(operators.assign_tokens().count(), 1);
        assert_eq!(operators.add_assign_tokens().count(), 1);
        assert_eq!(operators.le_tokens().count(), 0);

        let eof_choice = root.eof_choice().expect("EOF choice");
        assert!(eof_choice.eof_token().is_some());
        assert!(eof_choice.le_token().is_none());
    }
}
"#,
    );
}

#[test]
fn token_group_label_shared_across_alternatives_unions_the_sets() {
    let temp = temporary_directory("multi-alternative-label");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/multi-alternative-label/T.g4");
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
    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod multi_alternative_label_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn parse(input: &str) -> antlr4_runtime::ParsedFile {
        parse_rule(input, TParser::expr)
    }

    fn parse_rule(
        input: &str,
        entry: impl FnOnce(
            &mut TParser<TLexer<antlr4_runtime::InputStream>>,
        ) -> Result<antlr4_runtime::NodeId, antlr4_runtime::AntlrError>,
    ) -> antlr4_runtime::ParsedFile {
        let lexer = TLexer::new(InputStream::new(input));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = TParser::new(tokens);
        let root = entry(&mut parser).expect("input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        parser.into_parsed_file(root)
    }

    #[test]
    fn op_label_is_available_on_every_alternative_group() {
        let parsed = parse("a * b + c < d");
        let expr = parsed
            .tree()
            .as_rule()
            .expect("expr rule")
            .downcast_ref::<ExprContext>()
            .expect("typed expr context");
        let relation = expr.relation().expect("relation child");
        assert_eq!(relation.op().expect("relation operator").to_string(), "<");

        let calc = relation
            .relation_children()
            .next()
            .expect("left relation")
            .calc()
            .expect("calc child");
        assert_eq!(calc.op().expect("additive operator").to_string(), "+");

        let product = calc.calc_children().next().expect("left calc");
        assert_eq!(
            product.op().expect("multiplicative operator").to_string(),
            "*"
        );

        let leaf = product.calc_children().next().expect("leaf calc");
        assert!(leaf.op().is_none(), "primary alternative carries no operator");
    }

    // Issue #201: labels nested inside an unlabeled grouping block reach the
    // typed surface, and reading them agrees with the parsed text.
    #[test]
    fn labels_inside_grouping_blocks_read_their_own_children() {
        let parsed = parse_rule("doc in a, b 7", TParser::grouped);
        let grouped = parsed
            .tree()
            .as_rule()
            .expect("grouped rule")
            .downcast_ref::<GroupedContext>()
            .expect("typed grouped context");
        assert_eq!(grouped.doc().expect("doc token").to_string(), "doc");
        assert!(
            grouped.oneway().is_none(),
            "the throws branch carries no oneway token"
        );
        assert_eq!(
            grouped
                .errors()
                .map(|error| error.text())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );

        // The other branch of the same block, and both optionals absent.
        let parsed = parse_rule("* 7", TParser::grouped);
        let grouped = parsed
            .tree()
            .as_rule()
            .expect("grouped rule")
            .downcast_ref::<GroupedContext>()
            .expect("typed grouped context");
        assert!(grouped.doc().is_none(), "absent optional reads as None");
        assert_eq!(grouped.oneway().expect("oneway token").to_string(), "*");
        assert_eq!(grouped.errors().count(), 0);
    }

    // Issue #201: a single and a list label on the same rule must each resolve
    // past the other's children rather than by caller-side positional guessing.
    #[test]
    fn single_and_list_labels_on_one_rule_stay_disjoint() {
        let parsed = parse_rule("f ( x y ) in a, b", TParser::mixed);
        let mixed = parsed
            .tree()
            .as_rule()
            .expect("mixed rule")
            .downcast_ref::<MixedContext>()
            .expect("typed mixed context");
        assert_eq!(mixed.name().expect("name label").text(), "f");
        assert_eq!(
            mixed.errors().map(|error| error.text()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        // `name` is the sole unary outside the throws list, so the list accessor
        // must not include it and the two must partition `unary_children`.
        assert_eq!(mixed.unary_children().count(), 3);

        let parsed = parse_rule("f ( ) ", TParser::mixed);
        let mixed = parsed
            .tree()
            .as_rule()
            .expect("mixed rule")
            .downcast_ref::<MixedContext>()
            .expect("typed mixed context");
        assert_eq!(mixed.name().expect("name label").text(), "f");
        assert_eq!(
            mixed.errors().count(),
            0,
            "the absent throws clause contributes no errors"
        );
    }

    #[test]
    fn shared_optional_block_keeps_its_label_accessor() {
        let parsed = parse_rule("+*7", TParser::shared_optional_block);
        let context = parsed
            .tree()
            .as_rule()
            .expect("sharedOptionalBlock rule")
            .downcast_ref::<SharedOptionalBlockContext>()
            .expect("typed sharedOptionalBlock context");
        assert_eq!(context.shared().expect("first alternative label").to_string(), "+");

        let parsed = parse_rule("", TParser::shared_optional_block);
        let context = parsed
            .tree()
            .as_rule()
            .expect("sharedOptionalBlock rule")
            .downcast_ref::<SharedOptionalBlockContext>()
            .expect("typed sharedOptionalBlock context");
        assert!(context.shared().is_none(), "skipped block leaves the label unset");

        let parsed = parse_rule("*7", TParser::shared_optional_block);
        let context = parsed
            .tree()
            .as_rule()
            .expect("sharedOptionalBlock rule")
            .downcast_ref::<SharedOptionalBlockContext>()
            .expect("typed sharedOptionalBlock context");
        assert_eq!(context.shared().expect("second alternative label").to_string(), "*");
    }

    #[test]
    fn mixed_repetition_uses_the_last_assignment_on_both_alternatives() {
        let parsed = parse_rule("+-7", TParser::mixed_repetition);
        let context = parsed
            .tree()
            .as_rule()
            .expect("mixedRepetition rule")
            .downcast_ref::<MixedRepetitionContext>()
            .expect("typed mixedRepetition context");
        assert_eq!(context.latest().expect("repeated label").to_string(), "-");

        let parsed = parse_rule("/7", TParser::mixed_repetition);
        let context = parsed
            .tree()
            .as_rule()
            .expect("mixedRepetition rule")
            .downcast_ref::<MixedRepetitionContext>()
            .expect("typed mixedRepetition context");
        assert_eq!(context.latest().expect("single label").to_string(), "/");

        let parsed = parse_rule("++-7", TParser::prefixed_mixed_repetition);
        let context = parsed
            .tree()
            .as_rule()
            .expect("prefixedMixedRepetition rule")
            .downcast_ref::<PrefixedMixedRepetitionContext>()
            .expect("typed prefixedMixedRepetition context");
        assert_eq!(context.latest().expect("prefixed repeated label").to_string(), "-");

        let parsed = parse_rule("*/7", TParser::prefixed_mixed_repetition);
        let context = parsed
            .tree()
            .as_rule()
            .expect("prefixedMixedRepetition rule")
            .downcast_ref::<PrefixedMixedRepetitionContext>()
            .expect("typed prefixedMixedRepetition context");
        assert_eq!(context.latest().expect("prefixed single label").to_string(), "/");
    }
}
"#,
    );
}

#[test]
fn token_label_accessors_distinguish_deleted_and_inserted_recovery_tokens() {
    let temp = temporary_directory("token-label-recovery");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/token-label-recovery/T.g4");
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
    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod token_label_recovery_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _, Token as _};

    #[test]
    fn union_label_skips_a_deleted_token_from_another_alternative() {
        let lexer = TLexer::new(InputStream::new("x b a"));
        let mut parser = TParser::new(CommonTokenStream::new(lexer));
        let root = parser
            .different_types()
            .expect("single-token deletion should recover");
        assert_eq!(parser.number_of_syntax_errors(), 1);
        let parsed = parser.into_parsed_file(root);
        let context = parsed
            .tree()
            .as_rule()
            .expect("differentTypes rule")
            .downcast_ref::<DifferentTypesContext>()
            .expect("typed differentTypes context");

        assert_eq!(context.op().expect("operator label").to_string(), "a");
        assert_eq!(
            context
                .b_token()
                .expect("plain token accessors retain deleted tokens")
                .to_string(),
            "b"
        );
    }

    #[test]
    fn single_type_label_skips_a_deleted_token_of_the_same_type() {
        let lexer = TLexer::new(InputStream::new("x a y a"));
        let mut parser = TParser::new(CommonTokenStream::new(lexer));
        let root = parser
            .same_type()
            .expect("single-token deletion should recover");
        assert_eq!(parser.number_of_syntax_errors(), 1);
        let parsed = parser.into_parsed_file(root);
        let context = parsed
            .tree()
            .as_rule()
            .expect("sameType rule")
            .downcast_ref::<SameTypeContext>()
            .expect("typed sameType context");

        assert_eq!(context.op().expect("operator label").to_string(), "a");
        assert_eq!(
            context
                .a_token()
                .expect("plain token accessor retains the deleted token")
                .symbol()
                .column(),
            2
        );
        assert_eq!(context.op().expect("operator label").symbol().column(), 6);
    }

    #[test]
    fn label_keeps_a_synthesized_missing_token() {
        let lexer = TLexer::new(InputStream::new("x b"));
        let mut parser = TParser::new(CommonTokenStream::new(lexer));
        let root = parser
            .missing_token()
            .expect("single-token insertion should recover");
        assert_eq!(parser.number_of_syntax_errors(), 1);
        let parsed = parser.into_parsed_file(root);
        let context = parsed
            .tree()
            .as_rule()
            .expect("missingToken rule")
            .downcast_ref::<MissingTokenContext>()
            .expect("typed missingToken context");
        let label = context.op().expect("missing token remains assigned to label");

        assert_eq!(label.to_string(), "<missing 'a'>");
        assert_eq!(label.symbol().start(), usize::MAX);
        assert_eq!(
            label.symbol(),
            context.a_token().expect("plain token accessor").symbol()
        );
    }
}
"#,
    );
}

#[test]
fn validated_tree_makes_required_children_infallible_after_full_validation() {
    let temp = temporary_directory("validated-tree");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/validated-tree/T.g4");
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
    let parser = fs::read_to_string(out.join("t_parser.rs")).expect("parser should be emitted");
    for expected in [
        "pub type TValidatedTree = antlr4_runtime::ValidatedTree<ValidatedTreeContext>;",
        "pub type ValidatedRuleNode<'a> = antlr4_runtime::ValidatedRuleNode<'a, ValidatedTreeContext>;",
        "pub use antlr4_runtime::FromValidatedRuleNode;",
        "pub type TValidationError = antlr4_runtime::ValidationError;",
        "antlr4_runtime::__antlr4_rust_parser_entry_points! {",
        "validated_tree: TValidatedTree,",
        "validation_error: TValidationError,",
        "validate_tree: validate_tree_structure,",
        "pub trait TValidatedListener",
        "pub trait TValidatedVisitor",
        "antlr4_runtime::__antlr4_rust_context_accessors! {\n    StartContext {",
        "rule required_rule: required(RequiredRuleContext[",
        "token bang_token: required(",
        "rule optional_rule: optional(OptionalRuleContext[",
        "rule atom_children: many(AtomContext[",
        "label_token tails: many(skip(0), [",
        "context.required_rule()?;",
        "antlr4_runtime::require_min_count(context.atom_children().count(), 1, \"StartContext\", \"atom\")?;",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }

    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod validated_tree_tests {
    use std::convert::Infallible;

    use super::t_lexer::TLexer;
    use super::t_parser::*;
    use antlr4_runtime::{
        BaseParser, CommonTokenStream, InputStream, MissingChildError, ParserRuleContext,
    };

    fn strict(input: &str) -> Result<TValidatedTree, TValidationError> {
        parse_validated(input, TLexer::new, TParser::start)
    }

    fn start_context(tree: &TValidatedTree) -> StartContext<'_, ValidatedTreeContext> {
        tree.tree()
            .downcast_ref()
            .expect("validated start context")
    }

    #[derive(Default)]
    struct ValidatedTrace {
        starts: usize,
        wrapped: usize,
        bare: usize,
        atoms: usize,
    }

    impl TValidatedListener for ValidatedTrace {
        fn enter_start(
            &mut self,
            ctx: &StartContext<ValidatedTreeContext>,
        ) -> Result<(), Infallible> {
            self.starts += 1;
            assert_eq!(ctx.bang_token().to_string(), "!");
            assert_eq!(ctx.required().text(), "(head)");
            Ok(())
        }

        fn enter_wrapped_label(
            &mut self,
            ctx: &WrappedLabelContext<ValidatedTreeContext>,
        ) -> Result<(), Infallible> {
            self.wrapped += 1;
            assert_eq!(ctx.id_token().to_string(), "head");
            Ok(())
        }

        fn enter_bare_label(
            &mut self,
            _ctx: &BareLabelContext<ValidatedTreeContext>,
        ) -> Result<(), Infallible> {
            self.bare += 1;
            Ok(())
        }

        fn enter_atom(
            &mut self,
            ctx: &AtomContext<ValidatedTreeContext>,
        ) -> Result<(), Infallible> {
            self.atoms += 1;
            let _required_token = ctx.id_token();
            Ok(())
        }
    }

    #[derive(Default)]
    struct ValidatedVisitor {
        wrapped: usize,
        bare: usize,
        atoms: usize,
    }

    impl TValidatedVisitor for ValidatedVisitor {
        type Result = ();

        fn default_result(&mut self) -> Self::Result {}

        fn visit_wrapped_label(
            &mut self,
            ctx: &WrappedLabelContext<ValidatedTreeContext>,
        ) -> Self::Result {
            self.wrapped += 1;
            let _required_token = ctx.id_token();
            self.visit_children(ctx)
        }

        fn visit_bare_label(
            &mut self,
            ctx: &BareLabelContext<ValidatedTreeContext>,
        ) -> Self::Result {
            self.bare += 1;
            let _required_token = ctx.id_token();
            self.visit_children(ctx)
        }

        fn visit_atom(
            &mut self,
            ctx: &AtomContext<ValidatedTreeContext>,
        ) -> Self::Result {
            self.atoms += 1;
            let _required_token = ctx.id_token();
        }
    }

    #[test]
    fn clean_tree_exposes_direct_required_children_and_typed_traversal() {
        let validated = strict("(head) [maybe] ! : ? one two , ,").expect("clean parse validates");
        let start = start_context(&validated);

        assert_eq!(start.required_rule().text(), "(head)");
        assert_eq!(start.required().text(), "(head)");
        assert_eq!(
            start.optional_rule().expect("optional rule").text(),
            "[maybe]"
        );
        assert_eq!(start.optional().expect("optional label").text(), "[maybe]");
        assert_eq!(start.bang_token().to_string(), "!");
        assert_eq!(start.bang().to_string(), "!");
        assert_eq!(start.colon_token().to_string(), ":");
        assert_eq!(
            start.question_token().expect("optional token").to_string(),
            "?"
        );
        assert_eq!(start.question().expect("optional label").to_string(), "?");
        assert_eq!(
            start
                .atom_children()
                .map(|atom| atom.id_token().to_string())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(
            start
                .items()
                .map(|atom| atom.id_token().to_string())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(
            start
                .tails()
                .map(|token| token.to_string())
                .collect::<Vec<_>>(),
            [",", ","]
        );
        assert_eq!(start.eof_token().to_string(), "<EOF>");

        let mut listener = ValidatedTrace::default();
        listener.walk(validated.tree()).expect("validated walk");
        assert_eq!(listener.starts, 1);
        assert_eq!(listener.wrapped, 1);
        assert_eq!(listener.bare, 0);
        assert_eq!(listener.atoms, 2);

        let mut visitor = ValidatedVisitor::default();
        visitor.visit(validated.tree());
        assert_eq!(visitor.wrapped, 1);
        assert_eq!(visitor.bare, 0);
        assert_eq!(visitor.atoms, 2);
    }

    #[test]
    fn optional_absence_and_the_other_labeled_alternative_stay_typed() {
        let validated = strict("head ! : one").expect("clean bare parse validates");
        let start = start_context(&validated);

        assert!(start.optional_rule().is_none());
        assert!(start.optional().is_none());
        assert!(start.question_token().is_none());
        assert!(start.question().is_none());
        assert_eq!(start.items().count(), 1);
        assert_eq!(start.tails().count(), 0);

        let mut visitor = ValidatedVisitor::default();
        visitor.visit(validated.tree());
        assert_eq!(visitor.wrapped, 0);
        assert_eq!(visitor.bare, 1);
        assert_eq!(visitor.atoms, 1);
    }

    #[test]
    fn recovery_and_lexer_errors_cannot_cross_the_type_boundary() {
        // The first input inserts a missing COLON; the second deletes COMMA.
        for input in ["head ! one", "head , ! : one"] {
            let error = parse_with_parser(input, TLexer::new, TParser::start)
                .expect("ANTLR recovery returns a tree")
                .validate()
                .expect_err("recovered parse must not validate");
            assert!(
                matches!(
                    error,
                    TValidationError::SyntaxErrors {
                        lexer: 0,
                        parser: 1..
                    }
                ),
                "{input:?}: {error:?}"
            );

            let output = parse_with_parser(input, TLexer::new, TParser::start)
                .expect("ANTLR recovery returns a tree");
            let TParserParseOutput { result, parser } = output;
            let recovered = parser.into_parsed_file(result);
            assert!(
                matches!(
                    validate_tree_structure(&recovered),
                    Err(TValidationError::RecoveredErrorNode { .. })
                ),
                "{input:?}"
            );
        }

        assert_eq!(
            strict("head @ ! : one").expect_err("lexer diagnostics must reject validation"),
            TValidationError::SyntaxErrors {
                lexer: 1,
                parser: 0,
            }
        );

        assert!(matches!(
            strict("head : one"),
            Err(TValidationError::Recognition(_))
        ));
    }

    #[test]
    fn structural_validation_reports_missing_required_children() {
        let lexer = TLexer::new(InputStream::new(""));
        let tokens = CommonTokenStream::new(lexer);
        let data = TParser::<TLexer<InputStream>>::metadata().recognizer_data();
        let mut parser = BaseParser::new(tokens, data);
        let root = parser.rule_node(ParserRuleContext::new(0, -1));
        let malformed = parser.into_parsed_file(root);

        let error =
            validate_tree_structure(&malformed).expect_err("empty start context must be invalid");
        assert_eq!(
            error,
            TValidationError::MissingChild(MissingChildError::new(
                "StartContext",
                "requiredRule",
            ))
        );
        assert_eq!(
            error.to_string(),
            "required child requiredRule is missing from StartContext"
        );
    }

    #[test]
    fn structural_validation_reports_underfilled_repetitions() {
        let lexer = TLexer::new(InputStream::new("! :"));
        let tokens = CommonTokenStream::new(lexer);
        let data = TParser::<TLexer<InputStream>>::metadata().recognizer_data();
        let mut parser = BaseParser::new(tokens, data);
        let required_rule = parser.rule_node(ParserRuleContext::new(RULE_REQUIRED_RULE, -1));
        let bang = parser.match_token(BANG).expect("BANG token");
        let colon = parser.match_token(COLON).expect("COLON token");
        let mut root = ParserRuleContext::new(RULE_START, -1);
        parser.add_parse_child(&mut root, required_rule);
        parser.add_parse_child(&mut root, bang);
        parser.add_parse_child(&mut root, colon);
        let root = parser.rule_node(root);
        let malformed = parser.into_parsed_file(root);

        assert_eq!(
            validate_tree_structure(&malformed)
                .expect_err("start context must contain at least one atom"),
            TValidationError::InvalidChildCount {
                context: "StartContext",
                child: "atom",
                minimum: 1,
                actual: 0,
            }
        );
    }

    #[test]
    fn recovery_oriented_contexts_keep_fallible_required_accessors() {
        let parsed = parse("head ! : one", TLexer::new, TParser::start)
            .expect("clean recovery-oriented parse");
        let start = parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .downcast_ref::<StartContext>()
            .expect("stored start context");
        let required: Result<RequiredRuleContext<'_>, MissingChildError> =
            start.required_rule();
        assert_eq!(required.expect("required child").text(), "head");
    }
}
"#,
    );
}

#[test]
fn compile_parse_tree_pattern_matches_and_binds_against_generated_parser() {
    let temp = temporary_directory("tree-pattern-compile");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/typed-tree-walkers/Calculator.g4");
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

    let parser =
        fs::read_to_string(out.join("calculator_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("antlr4_runtime::__antlr4_rust_parser_facade!"),
        "generated parser must install the runtime-owned parser facade\n{parser}"
    );

    // End-to-end: compile a pattern rooted at the (left-recursive) `expression`
    // rule and match it against a real parse. Exercises the rule-bypass ATN's
    // left-recursive path through a genuinely generated grammar.
    assert_generated_project(
        temp.path(),
        &["calculator_lexer.rs", "calculator_parser.rs"],
        r#"
#[cfg(test)]
mod tree_pattern_tests {
    use super::calculator_lexer::CalculatorLexer;
    use super::calculator_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Node, Parser as _};

    /// Parses `input` and returns the owned tree plus the top-level expression
    /// node id, for matching against a pattern.
    fn parse_top_expression(input: &'static str) -> antlr4_runtime::ParsedFile {
        let lexer = CalculatorLexer::new(InputStream::new(input));
        let mut parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        let root = parser.start().expect("input parses");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        parser.into_parsed_file(root)
    }

    fn top_expression(parsed: &antlr4_runtime::ParsedFile) -> Node<'_> {
        parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .child_rule(RULE_EXPRESSION)
            .expect("top-level expression")
            .node()
    }

    #[test]
    fn compiles_and_matches_expression_pattern() {
        let lexer = CalculatorLexer::new(InputStream::new(""));
        let parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        // `<expr> + <expr>` rooted at the expression rule.
        let pattern = parser
            .compile_parse_tree_pattern(
                "<expression> + <expression>",
                RULE_EXPRESSION,
                CalculatorLexer::new,
            )
            .expect("pattern compiles");

        let parsed = parse_top_expression("2 + 8");
        let result = pattern.match_tree(top_expression(&parsed));
        assert!(result.succeeded(), "2 + 8 should match `<expr> + <expr>`");
        // Both operands bind under the rule name `expression`.
        let operands: Vec<_> = result
            .get_all("expression")
            .iter()
            .map(|node| node.text())
            .collect();
        assert_eq!(operands, vec!["2".to_owned(), "8".to_owned()]);
    }

    #[test]
    fn rejects_non_matching_structure() {
        let lexer = CalculatorLexer::new(InputStream::new(""));
        let parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        let pattern = parser
            .compile_parse_tree_pattern(
                "<expression> * <expression>",
                RULE_EXPRESSION,
                CalculatorLexer::new,
            )
            .expect("pattern compiles");

        let parsed = parse_top_expression("2 + 8");
        // Addition must not match a multiplication pattern.
        assert!(!pattern.match_tree(top_expression(&parsed)).succeeded());
    }

    #[test]
    fn trailing_eof_tag_requires_a_rule_that_consumes_it() {
        let lexer = CalculatorLexer::new(InputStream::new(""));
        let parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        // `start : expression EOF ;` consumes the tag: the pattern matches a
        // whole parse.
        let pattern = parser
            .compile_parse_tree_pattern(
                "<expression> <EOF>",
                RULE_START,
                CalculatorLexer::new,
            )
            .expect("EOF-consuming rule accepts a trailing <EOF> tag");
        let parsed = parse_top_expression("2 + 8");
        assert!(pattern.match_tree(parsed.tree()).succeeded());

        // `expression` never consumes EOF, so the tag would be silently
        // dropped from the pattern tree; that must be rejected.
        assert!(
            parser
                .compile_parse_tree_pattern(
                    "<expression> + <expression> <EOF>",
                    RULE_EXPRESSION,
                    CalculatorLexer::new,
                )
                .is_err(),
            "unconsumed trailing <EOF> tag must not compile"
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
        "rule expression_children: many(ExpressionContext[",
        "label_rule left: required(nth(0), ExpressionContext[",
        "label_rule right: required(nth(1), ExpressionContext[",
        "token star_token: optional(",
        "token int_token: required(",
        "token eof_token: required(-1, \"EOF\")",
        "label_token literal: required(",
        "label_token choice: required(nth(0), [2, 3], \"choice\")",
        "label_token other: required(",
        "label_token wildcard: required(",
        "token plus_token: required(",
        "token star_token: required(",
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
mod allocation_tracking {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    std::thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    pub struct TrackingAllocator;

    unsafe impl GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { System.alloc(layout) };
            record_allocation();
            pointer
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { System.alloc_zeroed(layout) };
            record_allocation();
            pointer
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) };
        }

        unsafe fn realloc(
            &self,
            pointer: *mut u8,
            layout: Layout,
            new_size: usize,
        ) -> *mut u8 {
            let pointer = unsafe { System.realloc(pointer, layout, new_size) };
            record_allocation();
            pointer
        }
    }

    fn record_allocation() {
        ENABLED.with(|enabled| {
            if enabled.get() {
                ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
            }
        });
    }

    pub fn count_allocations<T>(operation: impl FnOnce() -> T) -> (T, usize) {
        ALLOCATIONS.with(|allocations| allocations.set(0));
        ENABLED.with(|enabled| enabled.set(true));
        let value = operation();
        ENABLED.with(|enabled| enabled.set(false));
        let allocations = ALLOCATIONS.with(Cell::get);
        (value, allocations)
    }
}

#[cfg(test)]
#[global_allocator]
static ALLOCATOR: allocation_tracking::TrackingAllocator =
    allocation_tracking::TrackingAllocator;

#[cfg(test)]
mod typed_tree_tests {
    use super::calculator_lexer::CalculatorLexer;
    use super::calculator_parser::*;
    use super::allocation_tracking::count_allocations;
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
        let expected_display = format!(
            "[{}]",
            expression
                .invocation_states()
                .map(|state| state.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(add.to_string(), expected_display);
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

        let (child_ids, allocations) = count_allocations(|| {
            let add = expression
                .downcast_ref::<AddLabelContext>()
                .expect("top-level expression is addition");
            let left = add.left().expect("left expression");
            let right = add.right().expect("right expression");
            (
                add.rule_node().node().id(),
                left.rule_node().node().id(),
                right.rule_node().node().id(),
            )
        });
        assert_eq!(child_ids.0, expression.node().id());
        assert_eq!(
            allocations, 0,
            "stored typed contexts and child accessors must not allocate"
        );

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
        OsStr::new("--no-listener"),
        OsStr::new("--visitor"),
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
        "pub struct ObjectCreationExpressionContext {",
        "pub struct ObjectCreationExpressionLabelContext {",
        "pub struct ParenthesizedLabelContext {",
        "pub struct StoredTreeRuleContext {",
        "pub struct ValidatedTreeRuleContext {",
        "fn enter_object_creation_expression(&mut self",
        "fn enter_object_creation_expression_label(&mut self",
        "fn enter_parenthesized_label(&mut self",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert_generated_modules_compile(temp.path(), &["t.rs"]);
}

#[test]
fn colliding_context_accessor_names_compile() {
    let temp = temporary_directory("context-accessor-collision");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/context-accessor-collision/T.g4");
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
    assert_generated_modules_compile(temp.path(), &["t.rs"]);
}

#[test]
fn validated_rule_nodes_of_different_grammars_do_not_cross_downcast() {
    // The shared runtime validated surface is branded with each module's
    // `ValidatedTreeContext` marker, so downcasting one grammar's validated
    // node into another grammar's context must stay a type error (rule
    // indexes and context kinds are grammar-local numbers).
    let temp = temporary_directory("validated-cross-grammar");
    let out = temp.path().join("generated");
    for name in ["A", "B"] {
        let grammar = temp.path().join(format!("{name}.g4"));
        fs::write(
            &grammar,
            format!(
                "grammar {name};\n\
                 root : ID EOF ;\n\
                 ID : [a-z]+ ;\n\
                 WS : [ \\t\\r\\n]+ -> skip ;\n"
            ),
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
    }
    let modules = ["a_lexer.rs", "a_parser.rs", "b_lexer.rs", "b_parser.rs"];

    let output = run_generated_project(
        temp.path(),
        &modules,
        r#"
#[allow(dead_code)]
fn cross_grammar_downcast(node: crate::a_parser::ValidatedRuleNode<'_>) {
    let _ = node.downcast_ref::<crate::b_parser::RootContext<
        '_,
        crate::b_parser::ValidatedTreeContext,
    >>();
}
"#,
    );
    assert!(
        !output.status.success(),
        "cross-grammar validated downcast unexpectedly compiled"
    );
    assert!(
        utf8(&output.stderr).contains("error[E0271]"),
        "expected a Grammar associated-type mismatch, got: {}",
        utf8(&output.stderr)
    );

    assert_generated_project(
        temp.path(),
        &modules,
        r#"
#[cfg(test)]
mod tests {
    #[test]
    fn same_grammar_downcast_still_resolves() {
        use crate::a_lexer::ALexer;
        use crate::a_parser::{AParser, RootContext, ValidatedTreeContext};

        let validated = crate::a_parser::parse_validated(
            "hello",
            ALexer::new,
            AParser::root,
        )
        .expect("clean parse should validate");
        let root = validated
            .tree()
            .downcast_ref::<RootContext<'_, ValidatedTreeContext>>()
            .expect("entry rule downcasts within its own grammar");
        assert!(root.text().starts_with("hello"));
    }
}
"#,
    );
}
