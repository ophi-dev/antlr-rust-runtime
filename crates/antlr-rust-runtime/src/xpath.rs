//! ANTLR parse-tree `XPath` queries.
//!
//! This is ANTLR's small tree-path dialect, not W3C `XPath`. It supports child
//! (`/`) and descendant-or-self (`//`) selection, rule and token names,
//! single-quoted token literals, wildcards, and inverted node tests.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::token::{Token, TokenStoreError};
use crate::{
    CommonTokenStream, InputStream, Node, NodeId, NodeKind, Recognizer, TOKEN_EOF, Vocabulary,
};

mod generated {
    pub(super) mod x_path_lexer;
}

use generated::x_path_lexer::{ANYWHERE, BANG, ID, ROOT, STRING, WILDCARD, XPathLexer};

/// A compiled ANTLR parse-tree `XPath` expression.
#[derive(Clone, Debug)]
pub struct XPath {
    path: String,
    elements: Vec<PathElement>,
}

impl XPath {
    /// Compiles `path` against the rule and token names exposed by `recognizer`.
    pub fn new<R>(recognizer: &R, path: &str) -> Result<Self, XPathError>
    where
        R: Recognizer + ?Sized,
    {
        let tokens = tokenize(path)?;
        let elements = compile_elements(&tokens, recognizer.rule_names(), recognizer.vocabulary())?;
        if elements.is_empty() {
            return Err(XPathError::MissingPathElement);
        }
        Ok(Self {
            path: path.to_owned(),
            elements,
        })
    }

    /// Returns the source expression used to compile this query.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Evaluates this expression relative to `root`.
    ///
    /// Results retain parse-tree order and contain each node at most once.
    /// ANTLR's synthetic evaluation root is retained between path steps so a
    /// wildcard prefix can select `root` later. A final synthetic-root match is
    /// omitted because it is not part of the caller's parse tree.
    #[must_use]
    pub fn evaluate<'tree>(&self, root: Node<'tree>) -> Vec<Node<'tree>> {
        let mut work = vec![EvaluationNode::VirtualRoot(root)];
        for element in &self.elements {
            let mut next = Vec::new();
            let mut seen = BTreeSet::new();
            for node in work {
                if !node.has_children() {
                    continue;
                }
                extend_unique(&mut next, &mut seen, evaluate_element(*element, node));
            }
            work = next;
        }
        work.into_iter()
            .filter_map(EvaluationNode::tree_node)
            .collect()
    }

    /// Compiles and evaluates `path` relative to `root`.
    pub fn find_all<'tree, R>(
        root: Node<'tree>,
        path: &str,
        recognizer: &R,
    ) -> Result<Vec<Node<'tree>>, XPathError>
    where
        R: Recognizer + ?Sized,
    {
        Ok(Self::new(recognizer, path)?.evaluate(root))
    }
}

/// An invalid ANTLR parse-tree `XPath` expression.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum XPathError {
    #[error("Invalid tokens or characters at index {index} in path '{path}'")]
    InvalidCharacters { index: usize, path: String },
    #[error("Missing path element at end of path")]
    MissingPathElement,
    #[error("{name} at index {index} isn't a valid token name")]
    InvalidTokenName { name: String, index: usize },
    #[error("{name} at index {index} isn't a valid rule name")]
    InvalidRuleName { name: String, index: usize },
    #[error("Unknown path element {element} at index {index}")]
    UnknownPathElement { element: String, index: usize },
    #[error("Could not tokenize path: {message}")]
    Tokenization { message: String },
}

#[derive(Clone, Copy, Debug)]
struct PathElement {
    axis: Axis,
    test: NodeTest,
    invert: bool,
}

#[derive(Clone, Copy, Debug)]
enum Axis {
    Child,
    DescendantOrSelf,
}

#[derive(Clone, Copy, Debug)]
enum NodeTest {
    Rule(usize),
    Token(i32),
    Wildcard,
}

#[derive(Clone, Copy, Debug)]
enum EvaluationNode<'tree> {
    VirtualRoot(Node<'tree>),
    Tree(Node<'tree>),
}

impl<'tree> EvaluationNode<'tree> {
    fn has_children(self) -> bool {
        match self {
            Self::VirtualRoot(_) => true,
            Self::Tree(node) => node.children().next().is_some(),
        }
    }

    const fn id(self) -> Option<NodeId> {
        match self {
            Self::VirtualRoot(_) => None,
            Self::Tree(node) => Some(node.id()),
        }
    }

    const fn tree_node(self) -> Option<Node<'tree>> {
        match self {
            Self::VirtualRoot(_) => None,
            Self::Tree(node) => Some(node),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LexemeKind {
    Anywhere,
    Root,
    Wildcard,
    Bang,
    Identifier,
    String,
    Eof,
}

#[derive(Clone, Debug)]
struct Lexeme {
    kind: LexemeKind,
    text: String,
    index: usize,
}

fn tokenize(path: &str) -> Result<Vec<Lexeme>, XPathError> {
    let mut lexer = XPathLexer::new(InputStream::new(path));
    lexer.remove_error_listeners();
    let mut stream =
        CommonTokenStream::try_new(lexer).map_err(|error| tokenization_error(&error))?;
    stream.fill();

    if let Some(error) = stream.drain_source_errors().into_iter().next() {
        return Err(XPathError::InvalidCharacters {
            index: source_index(path, error.line, error.column),
            path: path.to_owned(),
        });
    }

    stream
        .tokens()
        .map(|token| {
            let kind = match token.token_type() {
                ANYWHERE => LexemeKind::Anywhere,
                ROOT => LexemeKind::Root,
                WILDCARD => LexemeKind::Wildcard,
                BANG => LexemeKind::Bang,
                ID => LexemeKind::Identifier,
                STRING => LexemeKind::String,
                TOKEN_EOF => LexemeKind::Eof,
                token_type => {
                    return Err(XPathError::Tokenization {
                        message: format!("unexpected XPath lexer token type {token_type}"),
                    });
                }
            };
            Ok(Lexeme {
                kind,
                text: token.text_or_empty().to_owned(),
                index: token.start(),
            })
        })
        .collect()
}

fn tokenization_error(error: &TokenStoreError) -> XPathError {
    XPathError::Tokenization {
        message: error.to_string(),
    }
}

fn source_index(path: &str, line: usize, column: usize) -> usize {
    let mut current_line = 1;
    let mut line_start = 0;
    for (index, ch) in path.chars().enumerate() {
        if current_line == line {
            return line_start + column;
        }
        if ch == '\n' {
            current_line += 1;
            line_start = index + 1;
        }
    }
    line_start + column
}

fn compile_elements(
    tokens: &[Lexeme],
    rule_names: &[String],
    vocabulary: &Vocabulary,
) -> Result<Vec<PathElement>, XPathError> {
    let mut elements = Vec::new();
    let mut cursor = 0;
    while let Some(token) = tokens.get(cursor) {
        match token.kind {
            LexemeKind::Anywhere | LexemeKind::Root => {
                let axis = if token.kind == LexemeKind::Anywhere {
                    Axis::DescendantOrSelf
                } else {
                    Axis::Child
                };
                cursor += 1;
                let invert = tokens
                    .get(cursor)
                    .is_some_and(|next| next.kind == LexemeKind::Bang);
                cursor += usize::from(invert);
                let word = tokens.get(cursor).ok_or(XPathError::MissingPathElement)?;
                if word.kind == LexemeKind::Eof {
                    return Err(XPathError::MissingPathElement);
                }
                elements.push(compile_element(word, axis, invert, rule_names, vocabulary)?);
                cursor += 1;
            }
            LexemeKind::Identifier | LexemeKind::Wildcard => {
                elements.push(compile_element(
                    token,
                    Axis::Child,
                    false,
                    rule_names,
                    vocabulary,
                )?);
                cursor += 1;
            }
            LexemeKind::Eof => break,
            LexemeKind::Bang | LexemeKind::String => {
                return Err(XPathError::UnknownPathElement {
                    element: token.text.clone(),
                    index: token.index,
                });
            }
        }
    }
    Ok(elements)
}

fn compile_element(
    token: &Lexeme,
    axis: Axis,
    invert: bool,
    rule_names: &[String],
    vocabulary: &Vocabulary,
) -> Result<PathElement, XPathError> {
    let node_test = match token.kind {
        LexemeKind::Wildcard => NodeTest::Wildcard,
        LexemeKind::String => NodeTest::Token(resolve_token(token, vocabulary)?),
        LexemeKind::Identifier if token.text.starts_with(char::is_uppercase) => {
            NodeTest::Token(resolve_token(token, vocabulary)?)
        }
        _ => NodeTest::Rule(resolve_rule(token, rule_names)?),
    };
    Ok(PathElement {
        axis,
        test: node_test,
        invert,
    })
}

fn resolve_token(token: &Lexeme, vocabulary: &Vocabulary) -> Result<i32, XPathError> {
    vocabulary
        .token_type(&token.text)
        .ok_or_else(|| XPathError::InvalidTokenName {
            name: token.text.clone(),
            index: token.index,
        })
}

fn resolve_rule(token: &Lexeme, rule_names: &[String]) -> Result<usize, XPathError> {
    rule_names
        .iter()
        .rposition(|name| name == &token.text)
        .ok_or_else(|| XPathError::InvalidRuleName {
            name: token.text.clone(),
            index: token.index,
        })
}

fn evaluate_element<'tree>(
    element: PathElement,
    node: EvaluationNode<'tree>,
) -> Box<dyn Iterator<Item = EvaluationNode<'tree>> + 'tree> {
    match node {
        EvaluationNode::VirtualRoot(root) => evaluate_virtual_root(element, root),
        EvaluationNode::Tree(node) => evaluate_tree_element(element, node),
    }
}

fn evaluate_virtual_root<'tree>(
    element: PathElement,
    root: Node<'tree>,
) -> Box<dyn Iterator<Item = EvaluationNode<'tree>> + 'tree> {
    match element.axis {
        Axis::Child => Box::new(
            std::iter::once(root)
                .filter(move |node| matches_direct(*node, element.test, element.invert))
                .map(EvaluationNode::Tree),
        ),
        Axis::DescendantOrSelf if matches!(element.test, NodeTest::Wildcard) && !element.invert => {
            Box::new(
                std::iter::once(EvaluationNode::VirtualRoot(root))
                    .chain(evaluate_anywhere(element, root).map(EvaluationNode::Tree)),
            )
        }
        Axis::DescendantOrSelf => {
            Box::new(evaluate_anywhere(element, root).map(EvaluationNode::Tree))
        }
    }
}

fn evaluate_tree_element<'tree>(
    element: PathElement,
    node: Node<'tree>,
) -> Box<dyn Iterator<Item = EvaluationNode<'tree>> + 'tree> {
    match element.axis {
        Axis::Child => Box::new(
            node.children()
                .filter(move |child| matches_direct(*child, element.test, element.invert))
                .map(EvaluationNode::Tree),
        ),
        Axis::DescendantOrSelf => {
            Box::new(evaluate_anywhere(element, node).map(EvaluationNode::Tree))
        }
    }
}

fn evaluate_anywhere<'tree>(
    element: PathElement,
    node: Node<'tree>,
) -> Box<dyn Iterator<Item = Node<'tree>> + 'tree> {
    match element.test {
        NodeTest::Wildcard if element.invert => Box::new(std::iter::empty()),
        NodeTest::Wildcard => Box::new(node.descendants()),
        NodeTest::Rule(rule_index) => Box::new(node.descendants().filter(move |candidate| {
            candidate
                .as_rule()
                .is_some_and(|rule| rule.rule_index() == rule_index)
        })),
        NodeTest::Token(token_type) => Box::new(
            node.descendants()
                .filter(move |candidate| node_token_type(*candidate) == Some(token_type)),
        ),
    }
}

fn matches_direct(node: Node<'_>, node_test: NodeTest, invert: bool) -> bool {
    match node_test {
        NodeTest::Wildcard => !invert,
        NodeTest::Rule(rule_index) => node
            .as_rule()
            .is_some_and(|rule| (rule.rule_index() == rule_index) != invert),
        NodeTest::Token(token_type) => {
            node_token_type(node).is_some_and(|actual| (actual == token_type) != invert)
        }
    }
}

fn node_token_type(node: Node<'_>) -> Option<i32> {
    match node.kind() {
        NodeKind::Terminal => node
            .as_terminal()
            .map(|terminal| terminal.symbol().token_type()),
        NodeKind::Error => node.as_error().map(|error| error.symbol().token_type()),
        NodeKind::Rule => None,
    }
}

fn extend_unique<'tree>(
    nodes: &mut Vec<EvaluationNode<'tree>>,
    seen: &mut BTreeSet<Option<NodeId>>,
    matches: impl IntoIterator<Item = EvaluationNode<'tree>>,
) {
    for node in matches {
        if seen.insert(node.id()) {
            nodes.push(node);
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::RecognizerData;
    use crate::token::{TokenSpec, TokenStore};
    use crate::tree::{ParseTreeStorage, ParsedFile, ParserRuleContext};

    const PROG: usize = 0;
    const FUNC: usize = 1;
    const BODY: usize = 2;
    const ARG: usize = 3;
    const STAT: usize = 4;
    const EXPR: usize = 5;
    const PRIMARY: usize = 6;

    const DEF: i32 = 1;
    const LPAREN: i32 = 2;
    const COMMA: i32 = 3;
    const RPAREN: i32 = 4;
    const LBRACE: i32 = 5;
    const RBRACE: i32 = 6;
    const SEMI: i32 = 7;
    const ASSIGN: i32 = 8;
    const MUL: i32 = 9;
    const ADD: i32 = 11;
    const RETURN: i32 = 13;
    const IDENTIFIER: i32 = 14;
    const INTEGER: i32 = 15;

    enum TreeSpec {
        Rule(usize, Vec<Self>),
        Token(i32, &'static str),
    }

    fn rule(index: usize, children: Vec<TreeSpec>) -> TreeSpec {
        TreeSpec::Rule(index, children)
    }

    const fn token(token_type: i32, text: &'static str) -> TreeSpec {
        TreeSpec::Token(token_type, text)
    }

    fn primary_expr(token_type: i32, text: &'static str) -> TreeSpec {
        rule(EXPR, vec![rule(PRIMARY, vec![token(token_type, text)])])
    }

    fn binary_expr(left: TreeSpec, operator: TreeSpec, right: TreeSpec) -> TreeSpec {
        rule(EXPR, vec![left, operator, right])
    }

    fn first_function() -> TreeSpec {
        let assignment = rule(
            STAT,
            vec![
                token(IDENTIFIER, "x"),
                token(ASSIGN, "="),
                binary_expr(
                    primary_expr(INTEGER, "3"),
                    token(ADD, "+"),
                    primary_expr(INTEGER, "4"),
                ),
                token(SEMI, ";"),
            ],
        );
        let print = rule(STAT, vec![primary_expr(IDENTIFIER, "y"), token(SEMI, ";")]);
        let body = rule(
            BODY,
            vec![
                token(LBRACE, "{"),
                assignment,
                print,
                rule(STAT, vec![token(SEMI, ";")]),
                token(RBRACE, "}"),
            ],
        );
        rule(
            FUNC,
            vec![
                token(DEF, "def"),
                token(IDENTIFIER, "f"),
                token(LPAREN, "("),
                rule(ARG, vec![token(IDENTIFIER, "x")]),
                token(COMMA, ","),
                rule(ARG, vec![token(IDENTIFIER, "y")]),
                token(RPAREN, ")"),
                body,
            ],
        )
    }

    fn second_function() -> TreeSpec {
        let product = binary_expr(
            primary_expr(INTEGER, "2"),
            token(MUL, "*"),
            primary_expr(IDENTIFIER, "x"),
        );
        let returned = binary_expr(primary_expr(INTEGER, "1"), token(ADD, "+"), product);
        let body = rule(
            BODY,
            vec![
                token(LBRACE, "{"),
                rule(
                    STAT,
                    vec![token(RETURN, "return"), returned, token(SEMI, ";")],
                ),
                token(RBRACE, "}"),
            ],
        );
        rule(
            FUNC,
            vec![
                token(DEF, "def"),
                token(IDENTIFIER, "g"),
                token(LPAREN, "("),
                rule(ARG, vec![token(IDENTIFIER, "x")]),
                token(RPAREN, ")"),
                body,
            ],
        )
    }

    fn materialize(
        spec: TreeSpec,
        tokens: &mut TokenStore,
        storage: &mut ParseTreeStorage,
    ) -> NodeId {
        match spec {
            TreeSpec::Token(token_type, text) => {
                let token = tokens
                    .push(TokenSpec::explicit(token_type, text))
                    .expect("test token should fit");
                storage.terminal(token)
            }
            TreeSpec::Rule(rule_index, children) => {
                let mut context = ParserRuleContext::new(rule_index, -1);
                for child in children {
                    let child = materialize(child, tokens, storage);
                    storage.add_child(&mut context, child);
                }
                storage.finish_rule(context)
            }
        }
    }

    fn sample_tree() -> ParsedFile {
        let mut tokens = TokenStore::new(None, "Expr");
        let mut storage = ParseTreeStorage::new();
        let root = materialize(
            rule(PROG, vec![first_function(), second_function()]),
            &mut tokens,
            &mut storage,
        );
        ParsedFile::new(tokens, storage, root)
    }

    #[derive(Debug)]
    struct TestRecognizer {
        data: RecognizerData,
    }

    impl Recognizer for TestRecognizer {
        fn data(&self) -> &RecognizerData {
            &self.data
        }

        fn data_mut(&mut self) -> &mut RecognizerData {
            &mut self.data
        }
    }

    fn expr_recognizer() -> TestRecognizer {
        let vocabulary = Vocabulary::new(
            [
                None,
                Some("'def'"),
                Some("'('"),
                Some("','"),
                Some("')'"),
                Some("'{'"),
                Some("'}'"),
                Some("';'"),
                Some("'='"),
                Some("'*'"),
                Some("'/'"),
                Some("'+'"),
                Some("'-'"),
                Some("'return'"),
                None,
                None,
            ],
            [
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some("MUL"),
                Some("DIV"),
                Some("ADD"),
                Some("SUB"),
                Some("RETURN"),
                Some("ID"),
                Some("INT"),
            ],
            [None::<&str>; 16],
        );
        TestRecognizer {
            data: RecognizerData::new("Expr.g4", vocabulary)
                .with_rule_names(["prog", "func", "body", "arg", "stat", "expr", "primary"]),
        }
    }

    fn display_nodes(nodes: Vec<Node<'_>>, rule_names: &[String]) -> Vec<String> {
        nodes
            .into_iter()
            .map(|node| {
                node.as_rule()
                    .map_or_else(|| node.text(), |rule| rule_names[rule.rule_index()].clone())
            })
            .collect()
    }

    #[test]
    fn upstream_valid_paths() {
        let tree = sample_tree();
        let recognizer = expr_recognizer();
        let paths = [
            "/prog/func",
            "/prog/*",
            "/*/func",
            "prog",
            "/prog",
            "/*",
            "*",
            "//ID",
            "//expr/primary/ID",
            "//body//ID",
            "//'return'",
            "//RETURN",
            "//primary/*",
            "//func/*/stat",
            "/prog/func/'def'",
            "//stat/';'",
            "//expr/primary/!ID",
            "//expr/!primary",
            "//!*",
            "/!*",
            "//expr//ID",
        ];
        let results = paths
            .into_iter()
            .map(|path| {
                let nodes =
                    XPath::find_all(tree.tree(), path, &recognizer).expect("valid XPath query");
                (path, display_nodes(nodes, recognizer.data().rule_names()))
            })
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!("upstream_valid_paths", results);
    }

    #[test]
    fn upstream_invalid_paths() {
        let recognizer = expr_recognizer();
        let paths = ["&", "//w&e/", "///", "//", "//Ick", "/prog/ick"];
        let errors = paths
            .into_iter()
            .map(|path| {
                (
                    path,
                    XPath::new(&recognizer, path)
                        .expect_err("invalid XPath query")
                        .to_string(),
                )
            })
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!("upstream_invalid_paths", errors);
    }

    #[test]
    fn lexer_and_parser_edge_cases_are_explicit() {
        let recognizer = expr_recognizer();
        let paths = ["", "// ID", "//'return", "!", "//!"];
        let errors = paths
            .into_iter()
            .map(|path| {
                (
                    path,
                    XPath::new(&recognizer, path)
                        .expect_err("invalid XPath query")
                        .to_string(),
                )
            })
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!("lexer_and_parser_edge_cases", errors);
    }

    #[test]
    fn generated_lexer_accepts_upstream_unicode_name_ranges() {
        let recognizer = TestRecognizer {
            data: RecognizerData::new(
                "Unicode.g4",
                Vocabulary::new([None::<&str>; 2], [None, Some("ÄTOKEN")], [None::<&str>; 2]),
            )
            .with_rule_names(["éclair", "文"]),
        };

        assert!(XPath::new(&recognizer, "//ÄTOKEN").is_ok());
        assert!(XPath::new(&recognizer, "//éclair").is_ok());
        assert!(XPath::new(&recognizer, "//文").is_ok());
    }

    #[test]
    fn named_anywhere_inversion_matches_java_4_13_2() {
        let tree = sample_tree();
        let recognizer = expr_recognizer();
        let paths = ["//ID", "//!ID", "//expr", "//!expr"];
        let results = paths
            .into_iter()
            .map(|path| {
                let nodes =
                    XPath::find_all(tree.tree(), path, &recognizer).expect("valid XPath query");
                (path, display_nodes(nodes, recognizer.data().rule_names()))
            })
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!("named_anywhere_inversion_matches_java_4_13_2", results);
    }

    #[test]
    fn wildcard_anywhere_preserves_java_virtual_root_for_later_steps() {
        let tree = sample_tree();
        let recognizer = expr_recognizer();
        let nodes =
            XPath::find_all(tree.tree(), "//*/prog", &recognizer).expect("valid XPath query");

        insta::assert_debug_snapshot!(
            "wildcard_anywhere_virtual_root",
            display_nodes(nodes, recognizer.data().rule_names())
        );
    }
}
