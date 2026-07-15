use crate::errors::AntlrError;
use crate::recognizer::Recognizer;
use std::any::Any;
use std::fmt;
use std::rc::Rc;

use crate::token::{TokenId, TokenStore, TokenView};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseTree {
    Rule(RuleNode),
    Terminal(TerminalNode),
    Error(ErrorNode),
}

impl ParseTree {
    /// Returns this node's direct children.
    ///
    /// Rule nodes return their context children; terminal and error nodes
    /// return an empty slice, so generic recursive walks can start from a
    /// `&ParseTree` without matching on every variant first.
    pub fn children(&self) -> &[Self] {
        match self {
            Self::Rule(rule) => rule.context().children(),
            Self::Terminal(_) | Self::Error(_) => &[],
        }
    }

    /// Iterates this tree in pre-order, starting with `self`.
    pub fn descendants(&self) -> ParseTreeDescendants<'_> {
        ParseTreeDescendants { stack: vec![self] }
    }

    /// Iterates this tree in pre-order, starting with `self`.
    ///
    /// This is an alias for [`Self::descendants`] for callers that prefer to
    /// name the traversal order explicitly.
    pub fn pre_order(&self) -> ParseTreeDescendants<'_> {
        self.descendants()
    }

    pub fn text(&self, tokens: &TokenStore) -> String {
        match self {
            Self::Rule(rule) => rule.text(tokens),
            Self::Terminal(node) => node.text(tokens).to_owned(),
            Self::Error(node) => node.text(tokens).to_owned(),
        }
    }

    pub fn to_string_tree_with_names<S: AsRef<str>>(
        &self,
        rule_names: &[S],
        tokens: &TokenStore,
    ) -> String {
        match self {
            Self::Rule(rule) => rule.to_string_tree_with_names(rule_names, tokens),
            Self::Terminal(node) => escape_tree_text(node.text(tokens)),
            Self::Error(node) => escape_tree_text(node.text(tokens)),
        }
    }

    /// Renders the LISP-style tree using rule names resolved through a
    /// recognizer, matching ANTLR's `toStringTree(parser)` shape used by
    /// generated test actions (`<tree>.to_string_tree(Some(self), tokens)`).
    pub fn to_string_tree<R: Recognizer>(
        &self,
        recognizer: Option<&R>,
        tokens: &TokenStore,
    ) -> String {
        recognizer.map_or_else(
            || self.to_string_tree_with_names::<&str>(&[], tokens),
            |recognizer| self.to_string_tree_with_names(recognizer.data().rule_names(), tokens),
        )
    }

    /// Finds the first rule node with `rule_index` in a depth-first walk.
    pub fn first_rule(&self, rule_index: usize) -> Option<&Self> {
        match self {
            Self::Rule(rule) => {
                if rule.context().rule_index() == rule_index {
                    return Some(self);
                }
                rule.context()
                    .children()
                    .iter()
                    .find_map(|child| child.first_rule(rule_index))
            }
            Self::Terminal(_) | Self::Error(_) => None,
        }
    }

    /// Finds the stop token for the first rule node with `rule_index`.
    pub fn first_rule_stop<'a>(
        &self,
        rule_index: usize,
        tokens: &'a TokenStore,
    ) -> Option<TokenView<'a>> {
        let Self::Rule(rule) = self else {
            return None;
        };
        if rule.context().rule_index() == rule_index {
            return rule.context().stop(tokens);
        }
        rule.context()
            .children()
            .iter()
            .find_map(|child| child.first_rule_stop(rule_index, tokens))
    }

    /// Reads an integer return value from the first rule node with
    /// `rule_index`, matching ANTLR's `$label.value` resolution for labeled
    /// rule references in the runtime testsuite.
    pub fn first_rule_int_return(&self, rule_index: usize, name: &str) -> Option<i64> {
        let Self::Rule(rule) = self else {
            return None;
        };
        if rule.context().rule_index() == rule_index {
            return rule.context().int_return(name);
        }
        rule.context()
            .children()
            .iter()
            .find_map(|child| child.first_rule_int_return(rule_index, name))
    }

    /// Reads the typed attribute snapshot from this tree's root rule node.
    ///
    /// Generated parsers use this for `$label.attr` / `$rule.attr` reads on a
    /// child subtree returned by a rule call.
    pub fn rule_attrs<T: Any>(&self) -> Option<&T> {
        let Self::Rule(rule) = self else {
            return None;
        };
        rule.context().generated_attrs::<T>()
    }

    /// Finds the first recovery error token in a depth-first walk.
    pub fn first_error_token<'a>(&self, tokens: &'a TokenStore) -> Option<TokenView<'a>> {
        match self {
            Self::Rule(rule) => rule
                .context()
                .children()
                .iter()
                .find_map(|child| child.first_error_token(tokens)),
            Self::Terminal(_) => None,
            Self::Error(node) => Some(node.symbol(tokens)),
        }
    }

    /// Returns the first rule invocation stack for `rule_index`, ordered from
    /// the selected rule outward to the root rule.
    pub fn rule_invocation_stack<S: AsRef<str>>(
        &self,
        rule_index: usize,
        rule_names: &[S],
    ) -> Option<Vec<String>> {
        let mut stack = Vec::new();
        if self.find_rule_path(rule_index, rule_names, &mut stack) {
            stack.reverse();
            return Some(stack);
        }
        None
    }

    fn find_rule_path<S: AsRef<str>>(
        &self,
        rule_index: usize,
        rule_names: &[S],
        stack: &mut Vec<String>,
    ) -> bool {
        let Self::Rule(rule) = self else {
            return false;
        };
        let current_index = rule.context().rule_index();
        stack.push(
            rule_names
                .get(current_index)
                .map_or("<unknown>", |name| name.as_ref())
                .to_owned(),
        );
        if current_index == rule_index
            || rule
                .context()
                .children()
                .iter()
                .any(|child| child.find_rule_path(rule_index, rule_names, stack))
        {
            return true;
        }
        stack.pop();
        false
    }
}

#[derive(Clone, Debug)]
pub struct ParseTreeDescendants<'a> {
    stack: Vec<&'a ParseTree>,
}

impl<'a> Iterator for ParseTreeDescendants<'a> {
    type Item = &'a ParseTree;

    fn next(&mut self) -> Option<Self::Item> {
        let tree = self.stack.pop()?;
        self.stack.extend(tree.children().iter().rev());
        Some(tree)
    }
}

fn escape_tree_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleNode {
    context: ParserRuleContext,
}

impl RuleNode {
    pub const fn new(context: ParserRuleContext) -> Self {
        Self { context }
    }

    pub const fn context(&self) -> &ParserRuleContext {
        &self.context
    }

    pub const fn context_mut(&mut self) -> &mut ParserRuleContext {
        &mut self.context
    }

    pub fn text(&self, tokens: &TokenStore) -> String {
        self.context.text(tokens)
    }

    pub fn to_string_tree_with_names<S: AsRef<str>>(
        &self,
        rule_names: &[S],
        tokens: &TokenStore,
    ) -> String {
        self.context.to_string_tree_with_names(rule_names, tokens)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserRuleContext {
    rule_index: usize,
    invoking_state: isize,
    alt_number: usize,
    start: Option<TokenId>,
    stop: Option<TokenId>,
    int_returns: Option<Box<IntReturns>>,
    children: Vec<ParseTree>,
    /// Whether any child has been offered to this context, independent of whether
    /// the tree was actually built. `children` stays empty when
    /// `Parser::set_build_parse_trees(false)`, so generated recovery uses this
    /// flag (not `children.is_empty()`) to tell whether the rule has matched
    /// anything yet.
    matched_child: bool,
    // Boxed: an `AntlrError` is large and only set on the rare error path, so
    // keeping it behind a pointer keeps `ParserRuleContext` (and thus the
    // `ParseTree::Rule` variant) compact.
    exception: Option<Box<AntlrError>>,
    /// Typed generated-rule attribute snapshot (see [`GeneratedAttrs`]).
    attrs: Option<GeneratedAttrs>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct IntReturns(BTreeMap<String, i64>);

/// Typed rule-attribute storage attached to a [`ParserRuleContext`].
///
/// Generated parsers keep each rule's `returns`/`locals`/argument attributes
/// in a generated per-rule struct and seal a shared snapshot onto the finished
/// context, so a parent rule (or a listener) can read `$child.attr` /
/// `ctx.attr` with its real Rust type — the analog of ANTLR's attribute
/// fields on generated context classes.
#[derive(Clone)]
pub struct GeneratedAttrs(Rc<dyn Any>);

impl GeneratedAttrs {
    pub fn new<T: Any>(attrs: T) -> Self {
        Self(Rc::new(attrs))
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl fmt::Debug for GeneratedAttrs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GeneratedAttrs(..)")
    }
}

// Attribute snapshots are sealed once per finished rule; two contexts are the
// same context (and thus equal) exactly when they share the same snapshot.
impl PartialEq for GeneratedAttrs {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for GeneratedAttrs {}

impl ParserRuleContext {
    pub const fn new(rule_index: usize, invoking_state: isize) -> Self {
        Self {
            rule_index,
            invoking_state,
            alt_number: 0,
            start: None,
            stop: None,
            int_returns: None,
            children: Vec::new(),
            matched_child: false,
            exception: None,
            attrs: None,
        }
    }

    pub(crate) fn with_child_capacity(
        rule_index: usize,
        invoking_state: isize,
        capacity: usize,
    ) -> Self {
        Self {
            rule_index,
            invoking_state,
            alt_number: 0,
            start: None,
            stop: None,
            int_returns: None,
            children: Vec::with_capacity(capacity),
            matched_child: false,
            exception: None,
            attrs: None,
        }
    }

    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    pub const fn invoking_state(&self) -> isize {
        self.invoking_state
    }

    pub const fn alt_number(&self) -> usize {
        self.alt_number
    }

    pub const fn set_alt_number(&mut self, alt_number: usize) {
        self.alt_number = alt_number;
    }

    pub fn start<'a>(&self, tokens: &'a TokenStore) -> Option<TokenView<'a>> {
        self.start.and_then(|id| tokens.view(id))
    }

    pub(crate) const fn start_id(&self) -> Option<TokenId> {
        self.start
    }

    pub fn stop<'a>(&self, tokens: &'a TokenStore) -> Option<TokenView<'a>> {
        self.stop.and_then(|id| tokens.view(id))
    }

    pub(crate) const fn stop_id(&self) -> Option<TokenId> {
        self.stop
    }

    pub(crate) const fn set_start_id(&mut self, token: TokenId) {
        self.start = Some(token);
    }

    pub(crate) const fn set_stop_id(&mut self, token: TokenId) {
        self.stop = Some(token);
    }

    pub(crate) const fn set_start_from_context(&mut self, other: &Self) {
        self.start = other.start;
    }

    /// Stores a generated integer return value on this rule context.
    pub fn set_int_return(&mut self, name: impl Into<String>, value: i64) {
        self.int_returns
            .get_or_insert_with(Box::default)
            .0
            .insert(name.into(), value);
    }

    /// Reads a generated integer return value from this rule context.
    pub fn int_return(&self, name: &str) -> Option<i64> {
        self.int_returns
            .as_ref()
            .and_then(|values| values.0.get(name).copied())
    }

    /// Seals the generated rule-attribute snapshot on this context.
    pub fn set_generated_attrs(&mut self, attrs: GeneratedAttrs) {
        self.attrs = Some(attrs);
    }

    /// Reads the typed generated rule-attribute snapshot, if sealed.
    pub fn generated_attrs<T: Any>(&self) -> Option<&T> {
        self.attrs.as_ref().and_then(GeneratedAttrs::downcast_ref)
    }

    pub fn exception(&self) -> Option<&AntlrError> {
        self.exception.as_deref()
    }

    pub fn set_exception(&mut self, error: AntlrError) {
        self.exception = Some(Box::new(error));
    }

    pub fn children(&self) -> &[ParseTree] {
        &self.children
    }

    /// Returns the number of direct children in this context.
    pub const fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Finds the first direct child rule with `rule_index`.
    pub fn child_rule(&self, rule_index: usize) -> Option<&Self> {
        self.child_rules(rule_index).next()
    }

    /// Iterates over direct child rules with `rule_index`.
    pub fn child_rules(&self, rule_index: usize) -> impl Iterator<Item = &Self> + '_ {
        self.children.iter().filter_map(move |child| match child {
            ParseTree::Rule(rule) if rule.context().rule_index() == rule_index => {
                Some(rule.context())
            }
            _ => None,
        })
    }

    /// Finds the first direct token child with `token_type`.
    ///
    /// Includes recovery error nodes, which ANTLR treats as terminal nodes for
    /// token-getter helpers.
    pub fn child_token<'a>(
        &'a self,
        token_type: i32,
        tokens: &'a TokenStore,
    ) -> Option<&'a TerminalNode> {
        self.child_tokens(token_type, tokens).next()
    }

    /// Iterates over direct child subtrees whose root rule has `rule_index`.
    ///
    /// Like [`Self::child_rules`] but yielding the full [`ParseTree`] child,
    /// for generated `$label.ctx` reads and listener walks over a labeled
    /// subtree.
    pub fn child_rule_trees(&self, rule_index: usize) -> impl Iterator<Item = &ParseTree> + '_ {
        self.children.iter().filter(move |child| match child {
            ParseTree::Rule(rule) => rule.context().rule_index() == rule_index,
            ParseTree::Terminal(_) | ParseTree::Error(_) => false,
        })
    }

    /// Iterates over direct token children with `token_type`, including
    /// recovery error nodes (ANTLR treats those as terminals for getters).
    pub fn child_tokens<'a>(
        &'a self,
        token_type: i32,
        tokens: &'a TokenStore,
    ) -> impl Iterator<Item = &'a TerminalNode> + 'a {
        self.children.iter().filter_map(move |child| match child {
            ParseTree::Terminal(node) if tokens.token_type(node.token_id()) == Some(token_type) => {
                Some(node)
            }
            ParseTree::Error(node) if tokens.token_type(node.token_id()) == Some(token_type) => {
                Some(node.terminal())
            }
            _ => None,
        })
    }

    /// Iterates over all direct terminal children regardless of token type.
    pub fn terminal_children(&self) -> impl Iterator<Item = &TerminalNode> + '_ {
        self.children.iter().filter_map(|child| match child {
            ParseTree::Terminal(node) => Some(node),
            ParseTree::Error(node) => Some(node.terminal()),
            ParseTree::Rule(_) => None,
        })
    }

    /// Downcast-style conversion to a generated typed context view.
    ///
    /// Generated parsers implement [`FromRuleContext`] for each context type;
    /// this mirrors ANTLR's `((BinaryContext) $ctx)` cast in test actions
    /// (`$ctx.downcast_ref::<BinaryContext>(tokens)`).
    pub fn downcast_ref<'a, T: FromRuleContext<'a>>(&self, tokens: &'a TokenStore) -> Option<T> {
        T::from_rule_context(self, tokens)
    }

    /// Returns whether this context has a direct token child with `token_type`.
    pub fn has_token(&self, token_type: i32, tokens: &TokenStore) -> bool {
        self.child_token(token_type, tokens).is_some()
    }

    pub fn add_child(&mut self, child: ParseTree) {
        self.matched_child = true;
        self.children.push(child);
    }

    /// Records that a child was matched without storing it (used when parse-tree
    /// construction is disabled). Keeps `has_matched_child` accurate even though
    /// `children` stays empty.
    pub const fn note_matched_child(&mut self) {
        self.matched_child = true;
    }

    /// Whether this context has matched at least one child (token, rule, or error
    /// node) so far, regardless of whether parse-tree construction is enabled.
    pub const fn has_matched_child(&self) -> bool {
        self.matched_child
    }

    pub fn text(&self, tokens: &TokenStore) -> String {
        self.children
            .iter()
            .map(|child| child.text(tokens))
            .collect()
    }

    pub fn to_string_tree_with_names<S: AsRef<str>>(
        &self,
        rule_names: &[S],
        tokens: &TokenStore,
    ) -> String {
        let name = rule_names
            .get(self.rule_index)
            .map_or("<unknown>", |name| name.as_ref());
        let display_name = if self.alt_number == 0 {
            name.to_owned()
        } else {
            format!("{name}:{}", self.alt_number)
        };
        if self.children.is_empty() {
            return display_name;
        }
        let children = self
            .children
            .iter()
            .map(|child| child.to_string_tree_with_names(rule_names, tokens))
            .collect::<Vec<_>>()
            .join(" ");
        format!("({display_name} {children})")
    }

    /// Renders the LISP-style tree using rule names resolved through a
    /// recognizer, matching ANTLR's `toStringTree(parser)` shape used by
    /// generated test actions on a mid-rule `$ctx`.
    pub fn to_string_tree<R: Recognizer>(
        &self,
        recognizer: Option<&R>,
        tokens: &TokenStore,
    ) -> String {
        recognizer.map_or_else(
            || self.to_string_tree_with_names::<&str>(&[], tokens),
            |recognizer| self.to_string_tree_with_names(recognizer.data().rule_names(), tokens),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalNode {
    token: TokenId,
}

impl TerminalNode {
    pub(crate) const fn from_id(token: TokenId) -> Self {
        Self { token }
    }

    #[must_use]
    pub const fn token_id(&self) -> TokenId {
        self.token
    }

    pub fn symbol<'a>(&self, tokens: &'a TokenStore) -> TokenView<'a> {
        tokens
            .view(self.token)
            .expect("terminal node token ID should remain valid")
    }

    pub fn text<'a>(&self, tokens: &'a TokenStore) -> &'a str {
        tokens.text(self.token).unwrap_or("")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorNode {
    terminal: TerminalNode,
}

impl ErrorNode {
    pub(crate) const fn from_id(token: TokenId) -> Self {
        Self {
            terminal: TerminalNode::from_id(token),
        }
    }

    const fn terminal(&self) -> &TerminalNode {
        &self.terminal
    }

    #[must_use]
    pub const fn token_id(&self) -> TokenId {
        self.terminal.token_id()
    }

    pub fn symbol<'a>(&self, tokens: &'a TokenStore) -> TokenView<'a> {
        self.terminal.symbol(tokens)
    }

    pub fn text<'a>(&self, tokens: &'a TokenStore) -> &'a str {
        tokens.text(self.token_id()).unwrap_or("")
    }
}

/// Conversion from a dynamic [`ParserRuleContext`] into a generated typed
/// context view.
///
/// Implemented by generated per-rule / per-labeled-alternative context types
/// so `ctx.downcast_ref::<XContext>(tokens)` can check the rule shape and
/// materialize the typed view with a token resolver, mirroring ANTLR's
/// context-class casts.
pub trait FromRuleContext<'a>: Sized {
    fn from_rule_context(context: &ParserRuleContext, tokens: &'a TokenStore) -> Option<Self>;
}

pub trait ParseTreeListener {
    fn enter_every_rule(
        &mut self,
        _ctx: &ParserRuleContext,
        _tokens: &TokenStore,
    ) -> Result<(), AntlrError> {
        Ok(())
    }

    fn exit_every_rule(
        &mut self,
        _ctx: &ParserRuleContext,
        _tokens: &TokenStore,
    ) -> Result<(), AntlrError> {
        Ok(())
    }

    fn visit_terminal(
        &mut self,
        _node: &TerminalNode,
        _tokens: &TokenStore,
    ) -> Result<(), AntlrError> {
        Ok(())
    }

    fn visit_error_node(
        &mut self,
        _node: &ErrorNode,
        _tokens: &TokenStore,
    ) -> Result<(), AntlrError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct ParseTreeWalker;

impl ParseTreeWalker {
    /// Walks a parse tree depth-first, invoking listener callbacks in ANTLR's
    /// enter/child/exit order for rule nodes.
    pub fn walk<L: ParseTreeListener>(
        listener: &mut L,
        tree: &ParseTree,
        tokens: &TokenStore,
    ) -> Result<(), AntlrError> {
        match tree {
            ParseTree::Rule(rule) => {
                listener.enter_every_rule(rule.context(), tokens)?;
                for child in rule.context().children() {
                    Self::walk(listener, child, tokens)?;
                }
                listener.exit_every_rule(rule.context(), tokens)
            }
            ParseTree::Terminal(node) => listener.visit_terminal(node, tokens),
            ParseTree::Error(node) => listener.visit_error_node(node, tokens),
        }
    }
}

/// A completed parse result paired with the canonical tokens referenced by it.
#[derive(Debug)]
pub struct ParsedFile<R> {
    tokens: TokenStore,
    tree: R,
}

impl<R> ParsedFile<R> {
    #[must_use]
    pub const fn new(tokens: TokenStore, tree: R) -> Self {
        Self { tokens, tree }
    }

    #[must_use]
    pub const fn tokens(&self) -> &TokenStore {
        &self.tokens
    }

    #[must_use]
    pub const fn tree(&self) -> &R {
        &self.tree
    }

    #[must_use]
    pub fn into_parts(self) -> (TokenStore, R) {
        (self.tokens, self.tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{TokenSpec, TokenStore};

    struct TreeToken(TokenSpec);

    impl TreeToken {
        fn new(token_type: i32) -> Self {
            Self(TokenSpec::explicit(token_type, ""))
        }

        fn with_text(mut self, text: impl Into<String>) -> Self {
            self.0.text = Some(text.into());
            self
        }
    }

    fn terminal(store: &mut TokenStore, token: TreeToken) -> TerminalNode {
        let id = store.push(token.0).expect("test token should fit");
        TerminalNode::from_id(id)
    }

    fn error(store: &mut TokenStore, token: TreeToken) -> ErrorNode {
        let id = store.push(token.0).expect("test token should fit");
        ErrorNode::from_id(id)
    }

    #[test]
    fn renders_rule_tree() {
        let mut tokens = TokenStore::new(None, "");
        let mut ctx = ParserRuleContext::new(0, -1);
        ctx.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(1).with_text("x"),
        )));
        let tree = ParseTree::Rule(RuleNode::new(ctx));
        assert_eq!(
            tree.to_string_tree_with_names(&["expr"], &tokens),
            "(expr x)"
        );
    }

    #[test]
    fn finds_first_rule_depth_first() {
        let mut tokens = TokenStore::new(None, "");
        let mut nested = ParserRuleContext::new(1, -1);
        nested.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(1).with_text("x"),
        )));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Rule(RuleNode::new(nested)));
        let tree = ParseTree::Rule(RuleNode::new(root));

        let rule = tree.first_rule(1).expect("nested rule should be found");
        assert_eq!(
            rule.to_string_tree_with_names(&["root".to_owned(), "child".to_owned()], &tokens),
            "(child x)"
        );
        assert!(tree.first_rule(2).is_none());
    }

    #[test]
    fn reports_rule_invocation_stack_from_leaf_to_root() {
        let mut tokens = TokenStore::new(None, "");
        let mut nested = ParserRuleContext::new(1, -1);
        nested.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(1).with_text("x"),
        )));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Rule(RuleNode::new(nested)));
        let tree = ParseTree::Rule(RuleNode::new(root));

        assert_eq!(
            tree.rule_invocation_stack(1, &["s", "a"]),
            Some(vec!["a".to_owned(), "s".to_owned()])
        );
    }

    #[test]
    fn parse_tree_children_returns_rule_children_and_empty_leaf_slices() {
        let mut tokens = TokenStore::new(None, "");
        let terminal = ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(1).with_text("terminal"),
        ));
        let error = ParseTree::Error(error(&mut tokens, TreeToken::new(2).with_text("error")));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(terminal);
        root.add_child(error);
        let tree = ParseTree::Rule(RuleNode::new(root));

        let children = tree.children();
        let [first, second] = children else {
            panic!("expected exactly 2 children");
        };
        assert_eq!(first.text(&tokens), "terminal");
        assert_eq!(second.text(&tokens), "error");
        assert!(first.children().is_empty());
        assert!(second.children().is_empty());
    }

    #[test]
    fn iterates_descendants_in_pre_order() {
        let mut tokens = TokenStore::new(None, "");
        let mut nested = ParserRuleContext::new(1, -1);
        nested.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(10).with_text("child"),
        )));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(11).with_text("prefix"),
        )));
        root.add_child(ParseTree::Rule(RuleNode::new(nested)));
        root.add_child(ParseTree::Error(error(
            &mut tokens,
            TreeToken::new(12).with_text("error"),
        )));
        let tree = ParseTree::Rule(RuleNode::new(root));

        let visited = tree
            .descendants()
            .map(|node| match node {
                ParseTree::Rule(rule) => format!("rule:{}", rule.context().rule_index()),
                ParseTree::Terminal(terminal) => {
                    format!("terminal:{}", terminal.text(&tokens))
                }
                ParseTree::Error(error) => format!("error:{}", error.text(&tokens)),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            visited,
            vec![
                "rule:0".to_owned(),
                "terminal:prefix".to_owned(),
                "rule:1".to_owned(),
                "terminal:child".to_owned(),
                "error:error".to_owned(),
            ]
        );

        let preorder = tree
            .pre_order()
            .map(|node| node.text(&tokens))
            .collect::<Vec<_>>();
        assert_eq!(
            preorder,
            vec!["prefixchilderror", "prefix", "child", "child", "error"]
        );
    }

    #[test]
    fn parsed_file_resolves_tokens_while_traversing() {
        let mut tokens = TokenStore::new(None, "");
        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(10).with_text("first"),
        )));
        root.add_child(ParseTree::Error(error(
            &mut tokens,
            TreeToken::new(11).with_text("second"),
        )));
        let parsed = ParsedFile::new(tokens, ParseTree::Rule(RuleNode::new(root)));

        let visited = parsed
            .tree()
            .descendants()
            .map(|node| node.text(parsed.tokens()))
            .collect::<Vec<_>>();

        assert_eq!(visited, vec!["firstsecond", "first", "second"]);
    }

    #[test]
    fn finds_direct_child_rules_by_index() {
        let mut tokens = TokenStore::new(None, "");
        let mut direct = ParserRuleContext::new(1, -1);
        direct.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(10).with_text("direct"),
        )));

        let mut nested_match = ParserRuleContext::new(1, -1);
        nested_match.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(11).with_text("nested"),
        )));
        let mut wrapper = ParserRuleContext::new(2, -1);
        wrapper.add_child(ParseTree::Rule(RuleNode::new(nested_match)));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(12).with_text("prefix"),
        )));
        root.add_child(ParseTree::Rule(RuleNode::new(direct)));
        root.add_child(ParseTree::Rule(RuleNode::new(wrapper)));

        assert_eq!(root.child_count(), 3);
        assert_eq!(root.child_rules(1).count(), 1);
        assert_eq!(
            root.child_rule(1).map(|context| context.text(&tokens)),
            Some("direct".to_owned())
        );
        assert_eq!(
            root.child_rule(2).map(ParserRuleContext::rule_index),
            Some(2)
        );
        assert!(root.child_rule(3).is_none());
    }

    #[test]
    fn finds_direct_terminal_children_by_token_type() {
        let mut tokens = TokenStore::new(None, "");
        let mut nested = ParserRuleContext::new(1, -1);
        nested.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(13).with_text("nested"),
        )));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Error(error(
            &mut tokens,
            TreeToken::new(12).with_text("error"),
        )));
        root.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(10).with_text("direct"),
        )));
        root.add_child(ParseTree::Terminal(terminal(
            &mut tokens,
            TreeToken::new(11).with_text("other"),
        )));
        root.add_child(ParseTree::Rule(RuleNode::new(nested)));

        assert_eq!(root.child_count(), 4);
        assert!(root.has_token(10, &tokens));
        assert!(root.has_token(12, &tokens));
        assert_eq!(
            root.child_token(10, &tokens).map(|node| node.text(&tokens)),
            Some("direct")
        );
        assert_eq!(
            root.child_token(11, &tokens).map(|node| node.text(&tokens)),
            Some("other")
        );
        assert_eq!(
            root.child_token(12, &tokens).map(|node| node.text(&tokens)),
            Some("error")
        );
        assert!(root.child_token(13, &tokens).is_none());
    }
}
