use crate::errors::AntlrError;
use crate::token::{CommonToken, Token};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseTree {
    Rule(RuleNode),
    Terminal(TerminalNode),
    Error(ErrorNode),
}

impl ParseTree {
    pub fn text(&self) -> String {
        match self {
            Self::Rule(rule) => rule.text(),
            Self::Terminal(node) => node.text(),
            Self::Error(node) => node.text(),
        }
    }

    pub fn to_string_tree(&self, rule_names: &[String]) -> String {
        match self {
            Self::Rule(rule) => rule.to_string_tree(rule_names),
            Self::Terminal(node) => node.text(),
            Self::Error(node) => node.text(),
        }
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

    /// Returns the first rule invocation stack for `rule_index`, ordered from
    /// the selected rule outward to the root rule.
    pub fn rule_invocation_stack(
        &self,
        rule_index: usize,
        rule_names: &[String],
    ) -> Option<Vec<String>> {
        let mut stack = Vec::new();
        if self.find_rule_path(rule_index, rule_names, &mut stack) {
            stack.reverse();
            return Some(stack);
        }
        None
    }

    fn find_rule_path(
        &self,
        rule_index: usize,
        rule_names: &[String],
        stack: &mut Vec<String>,
    ) -> bool {
        let Self::Rule(rule) = self else {
            return false;
        };
        let current_index = rule.context().rule_index();
        stack.push(
            rule_names
                .get(current_index)
                .map_or("<unknown>", String::as_str)
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

    pub fn text(&self) -> String {
        self.context.text()
    }

    pub fn to_string_tree(&self, rule_names: &[String]) -> String {
        self.context.to_string_tree(rule_names)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserRuleContext {
    rule_index: usize,
    invoking_state: isize,
    start: Option<CommonToken>,
    stop: Option<CommonToken>,
    children: Vec<ParseTree>,
    exception: Option<AntlrError>,
}

impl ParserRuleContext {
    pub const fn new(rule_index: usize, invoking_state: isize) -> Self {
        Self {
            rule_index,
            invoking_state,
            start: None,
            stop: None,
            children: Vec::new(),
            exception: None,
        }
    }

    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    pub const fn invoking_state(&self) -> isize {
        self.invoking_state
    }

    pub const fn start(&self) -> Option<&CommonToken> {
        self.start.as_ref()
    }

    pub const fn stop(&self) -> Option<&CommonToken> {
        self.stop.as_ref()
    }

    pub fn set_start(&mut self, token: CommonToken) {
        self.start = Some(token);
    }

    pub fn set_stop(&mut self, token: CommonToken) {
        self.stop = Some(token);
    }

    pub const fn exception(&self) -> Option<&AntlrError> {
        self.exception.as_ref()
    }

    pub fn set_exception(&mut self, error: AntlrError) {
        self.exception = Some(error);
    }

    pub fn children(&self) -> &[ParseTree] {
        &self.children
    }

    pub fn add_child(&mut self, child: ParseTree) {
        self.children.push(child);
    }

    pub fn text(&self) -> String {
        self.children.iter().map(ParseTree::text).collect()
    }

    pub fn to_string_tree(&self, rule_names: &[String]) -> String {
        let name = rule_names
            .get(self.rule_index)
            .map_or("<unknown>", String::as_str);
        if self.children.is_empty() {
            return name.to_owned();
        }
        let children = self
            .children
            .iter()
            .map(|child| child.to_string_tree(rule_names))
            .collect::<Vec<_>>()
            .join(" ");
        format!("({name} {children})")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalNode {
    token: CommonToken,
}

impl TerminalNode {
    pub const fn new(token: CommonToken) -> Self {
        Self { token }
    }

    pub const fn symbol(&self) -> &CommonToken {
        &self.token
    }

    pub fn text(&self) -> String {
        self.token.text().unwrap_or("").to_owned()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorNode {
    token: CommonToken,
}

impl ErrorNode {
    pub const fn new(token: CommonToken) -> Self {
        Self { token }
    }

    pub const fn symbol(&self) -> &CommonToken {
        &self.token
    }

    pub fn text(&self) -> String {
        self.token.text().unwrap_or("").to_owned()
    }
}

pub trait ParseTreeListener {
    fn enter_every_rule(&mut self, _ctx: &ParserRuleContext) -> Result<(), AntlrError> {
        Ok(())
    }

    fn exit_every_rule(&mut self, _ctx: &ParserRuleContext) -> Result<(), AntlrError> {
        Ok(())
    }

    fn visit_terminal(&mut self, _node: &TerminalNode) -> Result<(), AntlrError> {
        Ok(())
    }

    fn visit_error_node(&mut self, _node: &ErrorNode) -> Result<(), AntlrError> {
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
    ) -> Result<(), AntlrError> {
        match tree {
            ParseTree::Rule(rule) => {
                listener.enter_every_rule(rule.context())?;
                for child in rule.context().children() {
                    Self::walk(listener, child)?;
                }
                listener.exit_every_rule(rule.context())
            }
            ParseTree::Terminal(node) => listener.visit_terminal(node),
            ParseTree::Error(node) => listener.visit_error_node(node),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::CommonToken;

    #[test]
    fn renders_rule_tree() {
        let mut ctx = ParserRuleContext::new(0, -1);
        ctx.add_child(ParseTree::Terminal(TerminalNode::new(
            CommonToken::new(1).with_text("x"),
        )));
        let tree = ParseTree::Rule(RuleNode::new(ctx));
        assert_eq!(tree.to_string_tree(&["expr".to_owned()]), "(expr x)");
    }

    #[test]
    fn finds_first_rule_depth_first() {
        let mut nested = ParserRuleContext::new(1, -1);
        nested.add_child(ParseTree::Terminal(TerminalNode::new(
            CommonToken::new(1).with_text("x"),
        )));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Rule(RuleNode::new(nested)));
        let tree = ParseTree::Rule(RuleNode::new(root));

        let rule = tree.first_rule(1).expect("nested rule should be found");
        assert_eq!(
            rule.to_string_tree(&["root".to_owned(), "child".to_owned()]),
            "(child x)"
        );
        assert!(tree.first_rule(2).is_none());
    }

    #[test]
    fn reports_rule_invocation_stack_from_leaf_to_root() {
        let mut nested = ParserRuleContext::new(1, -1);
        nested.add_child(ParseTree::Terminal(TerminalNode::new(
            CommonToken::new(1).with_text("x"),
        )));

        let mut root = ParserRuleContext::new(0, -1);
        root.add_child(ParseTree::Rule(RuleNode::new(nested)));
        let tree = ParseTree::Rule(RuleNode::new(root));

        assert_eq!(
            tree.rule_invocation_stack(1, &["s".to_owned(), "a".to_owned()]),
            Some(vec!["a".to_owned(), "s".to_owned()])
        );
    }
}
