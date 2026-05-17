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
}
