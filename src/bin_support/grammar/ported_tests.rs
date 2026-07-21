use super::frontend::{DiagnosticStage, SourceId, parse_source};

#[test]
fn frontend_tool_syntax_cases_match_upstream_outcomes() {
    let accepted = [
        ("testA/parser-no-rules", "grammar A;\n"),
        ("testA/lexer-no-rules", "lexer grammar A;\n"),
        (
            "testEmptyGrammarOptions",
            "grammar A;\noptions {}\na : 'x' ;\n",
        ),
        ("testEmptyRuleOptions", "grammar A;\na options{} : 'x' ;\n"),
        (
            "testEmptyBlockOptions",
            "grammar A;\na : (options{} : 'x') ;\n",
        ),
        ("testEmptyTokensBlock", "grammar A;\ntokens {}\na : 'x' ;\n"),
    ];
    for (name, source) in accepted {
        parse_source(SourceId::new(0), name, source)
            .unwrap_or_else(|error| panic!("{name}: {:?}", error.diagnostics()));
    }

    let rejected = [
        (
            "testA/missing-grammar-keyword",
            "A;",
            DiagnosticStage::Parser,
        ),
        (
            "testA/missing-grammar-name",
            "grammar ;",
            DiagnosticStage::Parser,
        ),
        (
            "testA/missing-grammar-semi",
            "grammar A\na : ID ;\n",
            DiagnosticStage::Parser,
        ),
        (
            "testA/extra-rule-semi",
            "grammar A;\na : ID ;;\nb : B ;",
            DiagnosticStage::Parser,
        ),
        (
            "testA/extra-grammar-semi",
            "grammar A;;\na : ID ;\n",
            DiagnosticStage::Parser,
        ),
        (
            "testA/missing-rule-action",
            "grammar A;\na @init : ID ;\n",
            DiagnosticStage::Parser,
        ),
        (
            "testA/malformed-rule-prequel",
            "grammar A;\na  ( A | B ) D ;\nb : B ;",
            DiagnosticStage::Parser,
        ),
        (
            "testExtraColon",
            "grammar A;\na : : A ;\nb : B ;",
            DiagnosticStage::Parser,
        ),
        (
            "testMissingRuleSemi",
            "grammar A;\na : A \nb : B ;",
            DiagnosticStage::Parser,
        ),
        (
            "testMissingRuleSemi2",
            "lexer grammar A;\nA : 'a' \nB : 'b' ;",
            DiagnosticStage::Parser,
        ),
        (
            "testMissingRuleSemi3",
            "grammar A;\na : A \nb[int i] returns [int y] : B ;",
            DiagnosticStage::Parser,
        ),
        (
            "testMissingRuleSemi4",
            "grammar A;\na : b \n  catch [Exception e] {...}\nb : B ;\n",
            DiagnosticStage::Parser,
        ),
        (
            "testMissingRuleSemi5",
            "grammar A;\na : A \n  catch [Exception e] {...}\n",
            DiagnosticStage::Parser,
        ),
        (
            "testBadRulePrequelStart",
            "grammar A;\na @ options {k=1;} : A ;\nb : B ;",
            DiagnosticStage::Parser,
        ),
        (
            "testBadRulePrequelStart2",
            "grammar A;\na } : A ;\nb : B ;",
            DiagnosticStage::Parser,
        ),
        (
            "testUnterminatedStringLiteral",
            "grammar A;\na : 'x\n  ;\n",
            DiagnosticStage::Lexer,
        ),
        (
            "testParserRuleNameStartingWithUnderscore",
            "grammar A;\n_a : 'x' ;\n",
            DiagnosticStage::Lexer,
        ),
    ];
    for (name, source, expected_stage) in rejected {
        let error = parse_source(SourceId::new(0), name, source)
            .expect_err("invalid grammar must not return a CST");
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.stage == expected_stage),
            "{name}: {:?}",
            error.diagnostics()
        );
    }
}
