use crate::errors::AntlrError;
use crate::recognizer::Recognizer;
use crate::token::{Token, TokenId, TokenStore, TokenView};
use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::mem::size_of;

const NONE: u32 = u32::MAX;
const FLAG_MATCHED_CHILD: u8 = 1 << 0;
const FLAG_START_PRESENT: u8 = 1 << 1;
const FLAG_STOP_PRESENT: u8 = 1 << 2;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(u32);

impl NodeId {
    pub(crate) const fn placeholder() -> Self {
        Self(NONE)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Compact parser result. The tree data lives in [`ParseTreeStorage`].
pub type ParseTree = NodeId;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Rule,
    Terminal,
    Error,
}

#[derive(Debug)]
struct ChildLink {
    node: NodeId,
    next: u32,
}

#[derive(Debug)]
struct RuleExtra {
    int_returns: BTreeMap<String, i64>,
    exception: Option<AntlrError>,
    attrs: Option<GeneratedAttrs>,
}

#[derive(Debug)]
enum ParseTreeExtra {
    Rule(RuleExtra),
}

/// Flat, structure-of-arrays concrete syntax tree storage.
///
/// Every node is addressed by [`NodeId`]. Rule children occupy one range in
/// `children`; `child_links` is parser scratch used only while rule contexts
/// are open and is never exposed as part of the completed tree.
#[derive(Debug, Default)]
pub struct ParseTreeStorage {
    kinds: Vec<NodeKind>,
    child_starts: Vec<u32>,
    child_lens: Vec<u32>,
    payload_a: Vec<u32>,
    payload_b: Vec<u32>,
    starts: Vec<u32>,
    stops: Vec<u32>,
    alt_numbers: Vec<u32>,
    context_alt_numbers: Vec<u32>,
    extra_ids: Vec<u32>,
    parents: Vec<u32>,
    flags: Vec<u8>,
    children: Vec<NodeId>,
    extras: Vec<ParseTreeExtra>,
    child_links: Vec<ChildLink>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParseTreeStats {
    pub nodes: usize,
    pub edges: usize,
    pub extras: usize,
    pub scratch_links: usize,
    pub allocated_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParseTreeCheckpoint {
    nodes: usize,
    children: usize,
    extras: usize,
    child_links: usize,
}

impl ParseTreeStorage {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kinds: Vec::new(),
            child_starts: Vec::new(),
            child_lens: Vec::new(),
            payload_a: Vec::new(),
            payload_b: Vec::new(),
            starts: Vec::new(),
            stops: Vec::new(),
            alt_numbers: Vec::new(),
            context_alt_numbers: Vec::new(),
            extra_ids: Vec::new(),
            parents: Vec::new(),
            flags: Vec::new(),
            children: Vec::new(),
            extras: Vec::new(),
            child_links: Vec::new(),
        }
    }

    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.kinds.len()
    }

    #[must_use]
    pub const fn edge_count(&self) -> usize {
        self.children.len()
    }

    #[must_use]
    pub const fn extra_count(&self) -> usize {
        self.extras.len()
    }

    #[must_use]
    pub const fn stats(&self) -> ParseTreeStats {
        ParseTreeStats {
            nodes: self.node_count(),
            edges: self.edge_count(),
            extras: self.extra_count(),
            scratch_links: self.child_links.len(),
            allocated_bytes: self.kinds.capacity() * size_of::<NodeKind>()
                + self.child_starts.capacity() * size_of::<u32>()
                + self.child_lens.capacity() * size_of::<u32>()
                + self.payload_a.capacity() * size_of::<u32>()
                + self.payload_b.capacity() * size_of::<u32>()
                + self.starts.capacity() * size_of::<u32>()
                + self.stops.capacity() * size_of::<u32>()
                + self.alt_numbers.capacity() * size_of::<u32>()
                + self.context_alt_numbers.capacity() * size_of::<u32>()
                + self.extra_ids.capacity() * size_of::<u32>()
                + self.parents.capacity() * size_of::<u32>()
                + self.flags.capacity() * size_of::<u8>()
                + self.children.capacity() * size_of::<NodeId>()
                + self.extras.capacity() * size_of::<ParseTreeExtra>()
                + self.child_links.capacity() * size_of::<ChildLink>(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.kinds.clear();
        self.child_starts.clear();
        self.child_lens.clear();
        self.payload_a.clear();
        self.payload_b.clear();
        self.starts.clear();
        self.stops.clear();
        self.alt_numbers.clear();
        self.context_alt_numbers.clear();
        self.extra_ids.clear();
        self.parents.clear();
        self.flags.clear();
        self.children.clear();
        self.extras.clear();
        self.child_links.clear();
    }

    pub(crate) fn release_scratch(&mut self) {
        self.child_links.clear();
    }

    pub(crate) fn discard_scratch(&mut self) {
        self.child_links = Vec::new();
    }

    pub(crate) const fn checkpoint(&self) -> ParseTreeCheckpoint {
        ParseTreeCheckpoint {
            nodes: self.kinds.len(),
            children: self.children.len(),
            extras: self.extras.len(),
            child_links: self.child_links.len(),
        }
    }

    pub(crate) fn rollback(&mut self, checkpoint: ParseTreeCheckpoint) {
        self.kinds.truncate(checkpoint.nodes);
        self.child_starts.truncate(checkpoint.nodes);
        self.child_lens.truncate(checkpoint.nodes);
        self.payload_a.truncate(checkpoint.nodes);
        self.payload_b.truncate(checkpoint.nodes);
        self.starts.truncate(checkpoint.nodes);
        self.stops.truncate(checkpoint.nodes);
        self.alt_numbers.truncate(checkpoint.nodes);
        self.context_alt_numbers.truncate(checkpoint.nodes);
        self.extra_ids.truncate(checkpoint.nodes);
        self.parents.truncate(checkpoint.nodes);
        self.flags.truncate(checkpoint.nodes);
        self.children.truncate(checkpoint.children);
        self.extras.truncate(checkpoint.extras);
        self.child_links.truncate(checkpoint.child_links);
    }

    pub(crate) fn terminal(&mut self, token: TokenId) -> NodeId {
        self.push_node(NodeRecord {
            kind: NodeKind::Terminal,
            payload_a: token.index() as u32,
            ..NodeRecord::default()
        })
    }

    pub(crate) fn error(&mut self, token: TokenId) -> NodeId {
        self.push_node(NodeRecord {
            kind: NodeKind::Error,
            payload_a: token.index() as u32,
            ..NodeRecord::default()
        })
    }

    pub(crate) fn add_child(&mut self, context: &mut ParserRuleContext, child: NodeId) {
        context.matched_child = true;
        let link = self.child_links.len_u32("parse-tree scratch child links");
        self.child_links.push(ChildLink {
            node: child,
            next: NONE,
        });
        if context.first_child == NONE {
            context.first_child = link;
        } else {
            self.child_links[context.last_child as usize].next = link;
        }
        context.last_child = link;
        context.child_count = context
            .child_count
            .checked_add(1)
            .expect("rule child count exceeds u32");
    }

    pub(crate) fn finish_rule(&mut self, context: ParserRuleContext) -> NodeId {
        let parent = NodeId(self.kinds.len_u32("parse-tree node pool"));
        let child_start = self.children.len_u32("parse-tree child pool");
        let mut link = context.first_child;
        while link != NONE {
            let child = &self.child_links[link as usize];
            self.children.push(child.node);
            self.parents[child.node.index()] = parent.0;
            link = child.next;
        }

        let extra_id = if context.int_returns.is_empty()
            && context.exception.is_none()
            && context.attrs.is_none()
        {
            NONE
        } else {
            let id = self.extras.len_u32("parse-tree extra pool");
            self.extras.push(ParseTreeExtra::Rule(RuleExtra {
                int_returns: context.int_returns,
                exception: context.exception,
                attrs: context.attrs,
            }));
            id
        };

        self.push_node(NodeRecord {
            kind: NodeKind::Rule,
            child_start,
            child_len: context.child_count,
            payload_a: u32::try_from(context.rule_index).expect("rule index exceeds u32"),
            payload_b: i32::try_from(context.invoking_state)
                .expect("invoking state exceeds i32")
                .cast_unsigned(),
            start: context.start.map_or(NONE, |token| token.index() as u32),
            stop: context.stop.map_or(NONE, |token| token.index() as u32),
            alt_number: u32::try_from(context.alt_number).expect("alternative number exceeds u32"),
            context_alt_number: u32::try_from(context.context_alt_number)
                .expect("context alternative number exceeds u32"),
            extra_id,
            flags: (u8::from(context.matched_child) * FLAG_MATCHED_CHILD)
                | (u8::from(context.start.is_some()) * FLAG_START_PRESENT)
                | (u8::from(context.stop.is_some()) * FLAG_STOP_PRESENT),
        })
    }

    fn push_node(&mut self, record: NodeRecord) -> NodeId {
        let id = NodeId(self.kinds.len_u32("parse-tree node pool"));
        self.kinds.push(record.kind);
        self.child_starts.push(record.child_start);
        self.child_lens.push(record.child_len);
        self.payload_a.push(record.payload_a);
        self.payload_b.push(record.payload_b);
        self.starts.push(record.start);
        self.stops.push(record.stop);
        self.alt_numbers.push(record.alt_number);
        if record.context_alt_number == 0 {
            if !self.context_alt_numbers.is_empty() {
                self.context_alt_numbers.push(0);
            }
        } else {
            if self.context_alt_numbers.is_empty() {
                self.context_alt_numbers.resize(id.index(), 0);
            }
            self.context_alt_numbers.push(record.context_alt_number);
        }
        self.extra_ids.push(record.extra_id);
        self.parents.push(NONE);
        self.flags.push(record.flags);
        id
    }

    #[must_use]
    pub fn node<'tree>(&'tree self, tokens: &'tree TokenStore, id: NodeId) -> Option<Node<'tree>> {
        (id.index() < self.node_count()).then_some(Node {
            storage: self,
            tokens,
            id,
        })
    }

    fn kind(&self, id: NodeId) -> NodeKind {
        self.kinds[id.index()]
    }

    fn child_ids(&self, id: NodeId) -> &[NodeId] {
        let index = id.index();
        let start = self.child_starts[index] as usize;
        let len = self.child_lens[index] as usize;
        &self.children[start..start + len]
    }

    const fn context_child_ids<'a>(
        &'a self,
        context: &'a ParserRuleContext,
    ) -> ContextChildIds<'a> {
        ContextChildIds {
            storage: self,
            next: context.first_child,
            remaining: context.child_count as usize,
        }
    }

    fn token_id(&self, id: NodeId) -> Option<TokenId> {
        match self.kind(id) {
            NodeKind::Terminal | NodeKind::Error => {
                Some(stored_token_id(self.payload_a[id.index()]))
            }
            NodeKind::Rule => None,
        }
    }

    fn rule_extra(&self, id: NodeId) -> Option<&RuleExtra> {
        let extra = *self.extra_ids.get(id.index())?;
        if extra == NONE {
            return None;
        }
        match &self.extras[extra as usize] {
            ParseTreeExtra::Rule(extra) => Some(extra),
        }
    }
}

#[derive(Clone, Copy)]
struct NodeRecord {
    kind: NodeKind,
    child_start: u32,
    child_len: u32,
    payload_a: u32,
    payload_b: u32,
    start: u32,
    stop: u32,
    alt_number: u32,
    context_alt_number: u32,
    extra_id: u32,
    flags: u8,
}

impl Default for NodeRecord {
    fn default() -> Self {
        Self {
            kind: NodeKind::Rule,
            child_start: 0,
            child_len: 0,
            payload_a: 0,
            payload_b: 0,
            start: NONE,
            stop: NONE,
            alt_number: 0,
            context_alt_number: 0,
            extra_id: NONE,
            flags: 0,
        }
    }
}

trait LenU32 {
    fn len_u32(&self, name: &str) -> u32;
}

impl<T> LenU32 for Vec<T> {
    fn len_u32(&self, name: &str) -> u32 {
        u32::try_from(self.len()).unwrap_or_else(|_| panic!("{name} exceeds u32"))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Node<'tree> {
    storage: &'tree ParseTreeStorage,
    tokens: &'tree TokenStore,
    id: NodeId,
}

impl<'tree> Node<'tree> {
    #[must_use]
    pub const fn id(self) -> NodeId {
        self.id
    }

    #[must_use]
    pub fn kind(self) -> NodeKind {
        self.storage.kind(self.id)
    }

    #[must_use]
    pub fn as_rule(self) -> Option<RuleNodeView<'tree>> {
        (self.kind() == NodeKind::Rule).then_some(RuleNodeView { node: self })
    }

    #[must_use]
    pub fn as_terminal(self) -> Option<TerminalNodeView<'tree>> {
        (self.kind() == NodeKind::Terminal).then_some(TerminalNodeView { node: self })
    }

    #[must_use]
    pub fn as_error(self) -> Option<ErrorNodeView<'tree>> {
        (self.kind() == NodeKind::Error).then_some(ErrorNodeView { node: self })
    }

    #[must_use]
    pub fn children(self) -> NodeChildren<'tree> {
        NodeChildren {
            storage: self.storage,
            tokens: self.tokens,
            ids: self.storage.child_ids(self.id).iter(),
        }
    }

    #[must_use]
    pub fn parent(self) -> Option<Self> {
        let parent = self.storage.parents[self.id.index()];
        (parent != NONE)
            .then(|| self.storage.node(self.tokens, NodeId(parent)))
            .flatten()
    }

    #[must_use]
    pub fn descendants(self) -> ParseTreeDescendants<'tree> {
        ParseTreeDescendants {
            storage: self.storage,
            tokens: self.tokens,
            stack: vec![self.id],
        }
    }

    #[must_use]
    pub fn pre_order(self) -> ParseTreeDescendants<'tree> {
        self.descendants()
    }

    #[must_use]
    pub fn text(self) -> String {
        match self.kind() {
            NodeKind::Terminal | NodeKind::Error => self
                .storage
                .token_id(self.id)
                .and_then(|id| self.tokens.text(id))
                .unwrap_or("")
                .to_owned(),
            NodeKind::Rule => {
                let mut text = String::new();
                let mut stack = self
                    .storage
                    .child_ids(self.id)
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>();
                while let Some(id) = stack.pop() {
                    match self.storage.kind(id) {
                        NodeKind::Rule => {
                            stack.extend(self.storage.child_ids(id).iter().rev().copied());
                        }
                        NodeKind::Terminal | NodeKind::Error => text.push_str(
                            self.storage
                                .token_id(id)
                                .and_then(|token| self.tokens.text(token))
                                .unwrap_or(""),
                        ),
                    }
                }
                text
            }
        }
    }

    #[must_use]
    pub fn to_string_tree_with_names<S: AsRef<str>>(self, rule_names: &[S]) -> String {
        match self.kind() {
            NodeKind::Rule => self
                .as_rule()
                .expect("rule node kind checked")
                .to_string_tree_with_names(rule_names),
            NodeKind::Terminal | NodeKind::Error => escape_tree_text(
                self.storage
                    .token_id(self.id)
                    .and_then(|id| self.tokens.text(id))
                    .unwrap_or(""),
            ),
        }
    }

    #[must_use]
    pub fn to_string_tree<R: Recognizer>(
        self,
        recognizer: Option<&R>,
        _tokens: &TokenStore,
    ) -> String {
        recognizer.map_or_else(
            || self.to_string_tree_with_names::<&str>(&[]),
            |recognizer| self.to_string_tree_with_names(recognizer.data().rule_names()),
        )
    }

    #[must_use]
    pub fn first_rule(self, rule_index: usize) -> Option<Self> {
        self.descendants().find(|node| {
            node.as_rule()
                .is_some_and(|rule| rule.rule_index() == rule_index)
        })
    }

    #[must_use]
    pub fn first_rule_stop(self, rule_index: usize) -> Option<TokenView<'tree>> {
        self.first_rule(rule_index)?.as_rule()?.stop()
    }

    #[must_use]
    pub fn first_rule_int_return(self, rule_index: usize, name: &str) -> Option<i64> {
        self.first_rule(rule_index)?.as_rule()?.int_return(name)
    }

    #[must_use]
    pub fn rule_attrs<T: Any>(self) -> Option<&'tree T> {
        self.as_rule()?.generated_attrs::<T>()
    }

    #[must_use]
    pub fn first_error_token(self) -> Option<TokenView<'tree>> {
        self.descendants()
            .find_map(Node::as_error)
            .map(ErrorNodeView::symbol)
    }

    #[must_use]
    pub fn rule_invocation_stack<S: AsRef<str>>(
        self,
        rule_index: usize,
        rule_names: &[S],
    ) -> Option<Vec<String>> {
        let mut stack = vec![(self.id, 0_usize)];
        let mut names = Vec::new();
        while let Some((id, child_index)) = stack.last_mut() {
            if *child_index == 0 {
                let Some(rule) = self.storage.node(self.tokens, *id).and_then(Node::as_rule) else {
                    stack.pop();
                    continue;
                };
                names.push(
                    rule_names
                        .get(rule.rule_index())
                        .map_or("<unknown>", |name| name.as_ref())
                        .to_owned(),
                );
                if rule.rule_index() == rule_index {
                    names.reverse();
                    return Some(names);
                }
            }
            let children = self.storage.child_ids(*id);
            let next = children.get(*child_index).copied();
            *child_index += 1;
            if let Some(child) = next {
                if self.storage.kind(child) == NodeKind::Rule {
                    stack.push((child, 0));
                }
            } else {
                stack.pop();
                names.pop();
            }
        }
        None
    }
}

#[derive(Clone, Debug)]
pub struct NodeChildren<'tree> {
    storage: &'tree ParseTreeStorage,
    tokens: &'tree TokenStore,
    ids: std::slice::Iter<'tree, NodeId>,
}

impl<'tree> Iterator for NodeChildren<'tree> {
    type Item = Node<'tree>;

    fn next(&mut self) -> Option<Self::Item> {
        self.ids
            .next()
            .and_then(|id| self.storage.node(self.tokens, *id))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.ids.size_hint()
    }
}

impl DoubleEndedIterator for NodeChildren<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.ids
            .next_back()
            .and_then(|id| self.storage.node(self.tokens, *id))
    }
}

impl ExactSizeIterator for NodeChildren<'_> {}

#[derive(Clone, Debug)]
pub struct ParseTreeDescendants<'tree> {
    storage: &'tree ParseTreeStorage,
    tokens: &'tree TokenStore,
    stack: Vec<NodeId>,
}

impl<'tree> Iterator for ParseTreeDescendants<'tree> {
    type Item = Node<'tree>;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        self.stack
            .extend(self.storage.child_ids(id).iter().rev().copied());
        Some(Node {
            storage: self.storage,
            tokens: self.tokens,
            id,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RuleNodeView<'tree> {
    node: Node<'tree>,
}

impl<'tree> RuleNodeView<'tree> {
    #[must_use]
    pub const fn node(self) -> Node<'tree> {
        self.node
    }

    #[must_use]
    pub fn rule_index(self) -> usize {
        self.node.storage.payload_a[self.node.id.index()] as usize
    }

    #[must_use]
    pub fn invoking_state(self) -> isize {
        self.node.storage.payload_b[self.node.id.index()].cast_signed() as isize
    }

    #[must_use]
    pub fn alt_number(self) -> usize {
        self.node.storage.alt_numbers[self.node.id.index()] as usize
    }

    #[doc(hidden)]
    #[must_use]
    pub fn context_alt_number(self) -> usize {
        self.node
            .storage
            .context_alt_numbers
            .get(self.node.id.index())
            .copied()
            .unwrap_or_default() as usize
    }

    #[must_use]
    pub fn start(self) -> Option<TokenView<'tree>> {
        self.start_id().and_then(|id| self.node.tokens.view(id))
    }

    #[must_use]
    pub fn start_id(self) -> Option<TokenId> {
        let index = self.node.id.index();
        (self.node.storage.flags[index] & FLAG_START_PRESENT != 0)
            .then(|| stored_token_id(self.node.storage.starts[index]))
    }

    #[must_use]
    pub fn stop(self) -> Option<TokenView<'tree>> {
        self.stop_id().and_then(|id| self.node.tokens.view(id))
    }

    #[must_use]
    pub fn stop_id(self) -> Option<TokenId> {
        let index = self.node.id.index();
        (self.node.storage.flags[index] & FLAG_STOP_PRESENT != 0)
            .then(|| stored_token_id(self.node.storage.stops[index]))
    }

    #[must_use]
    pub fn children(self) -> NodeChildren<'tree> {
        self.node.children()
    }

    #[must_use]
    pub fn child_count(self) -> usize {
        self.node.storage.child_lens[self.node.id.index()] as usize
    }

    #[must_use]
    pub fn child_rule(self, rule_index: usize) -> Option<Self> {
        self.child_rules(rule_index).next()
    }

    pub fn child_rules(self, rule_index: usize) -> impl DoubleEndedIterator<Item = Self> + 'tree {
        self.children().filter_map(move |child| {
            let rule = child.as_rule()?;
            (rule.rule_index() == rule_index).then_some(rule)
        })
    }

    pub fn child_rule_trees(
        self,
        rule_index: usize,
    ) -> impl DoubleEndedIterator<Item = Node<'tree>> + 'tree {
        self.child_rules(rule_index).map(Self::node)
    }

    #[must_use]
    pub fn child_token(self, token_type: i32) -> Option<TerminalNodeView<'tree>> {
        self.child_tokens(token_type).next()
    }

    pub fn child_tokens(
        self,
        token_type: i32,
    ) -> impl DoubleEndedIterator<Item = TerminalNodeView<'tree>> + 'tree {
        self.children().filter_map(move |child| {
            let terminal = match child.kind() {
                NodeKind::Terminal => child.as_terminal(),
                NodeKind::Error => child.as_error().map(ErrorNodeView::terminal),
                NodeKind::Rule => None,
            }?;
            (terminal.symbol().token_type() == token_type).then_some(terminal)
        })
    }

    pub fn terminal_children(
        self,
    ) -> impl DoubleEndedIterator<Item = TerminalNodeView<'tree>> + 'tree {
        self.children().filter_map(|child| match child.kind() {
            NodeKind::Terminal => child.as_terminal(),
            NodeKind::Error => child.as_error().map(ErrorNodeView::terminal),
            NodeKind::Rule => None,
        })
    }

    #[must_use]
    pub fn has_token(self, token_type: i32) -> bool {
        self.child_token(token_type).is_some()
    }

    #[must_use]
    pub fn text(self) -> String {
        self.node.text()
    }

    #[must_use]
    pub fn int_return(self, name: &str) -> Option<i64> {
        self.node
            .storage
            .rule_extra(self.node.id)?
            .int_returns
            .get(name)
            .copied()
    }

    #[must_use]
    pub fn generated_attrs<T: Any>(self) -> Option<&'tree T> {
        self.node
            .storage
            .rule_extra(self.node.id)?
            .attrs
            .as_ref()?
            .downcast_ref::<T>()
    }

    #[must_use]
    pub fn exception(self) -> Option<&'tree AntlrError> {
        self.node
            .storage
            .rule_extra(self.node.id)?
            .exception
            .as_ref()
    }

    #[must_use]
    pub fn downcast_ref<T: FromRuleNode<'tree>>(self) -> Option<T> {
        T::from_rule_node(self)
    }

    pub fn invocation_states(self) -> impl Iterator<Item = isize> + 'tree {
        std::iter::successors(Some(self), |rule| rule.node.parent()?.as_rule())
            .take_while(|rule| rule.node.parent().is_some() && rule.invoking_state() >= 0)
            .map(Self::invoking_state)
    }

    #[must_use]
    pub fn to_string_tree_with_names<S: AsRef<str>>(self, rule_names: &[S]) -> String {
        let name = rule_names
            .get(self.rule_index())
            .map_or("<unknown>", |name| name.as_ref());
        let display_name = if self.alt_number() == 0 {
            name.to_owned()
        } else {
            format!("{name}:{}", self.alt_number())
        };
        if self.child_count() == 0 {
            return display_name;
        }
        let children = self
            .children()
            .map(|child| child.to_string_tree_with_names(rule_names))
            .collect::<Vec<_>>()
            .join(" ");
        format!("({display_name} {children})")
    }

    #[must_use]
    pub fn to_string_tree<R: Recognizer>(self, recognizer: Option<&R>) -> String {
        recognizer.map_or_else(
            || self.to_string_tree_with_names::<&str>(&[]),
            |recognizer| self.to_string_tree_with_names(recognizer.data().rule_names()),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalNodeView<'tree> {
    node: Node<'tree>,
}

impl<'tree> TerminalNodeView<'tree> {
    #[must_use]
    pub const fn node(self) -> Node<'tree> {
        self.node
    }

    #[must_use]
    pub fn token_id(self) -> TokenId {
        self.node
            .storage
            .token_id(self.node.id)
            .expect("terminal node should contain a token ID")
    }

    #[must_use]
    pub fn symbol(self) -> TokenView<'tree> {
        self.node
            .tokens
            .view(self.token_id())
            .expect("terminal node token ID should remain valid")
    }

    #[must_use]
    pub fn text(self) -> &'tree str {
        self.node.tokens.text(self.token_id()).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ErrorNodeView<'tree> {
    node: Node<'tree>,
}

impl<'tree> ErrorNodeView<'tree> {
    #[must_use]
    pub const fn node(self) -> Node<'tree> {
        self.node
    }

    #[must_use]
    pub const fn terminal(self) -> TerminalNodeView<'tree> {
        TerminalNodeView { node: self.node }
    }

    #[must_use]
    pub fn token_id(self) -> TokenId {
        self.terminal().token_id()
    }

    #[must_use]
    pub fn symbol(self) -> TokenView<'tree> {
        self.terminal().symbol()
    }

    #[must_use]
    pub fn text(self) -> &'tree str {
        self.terminal().text()
    }
}

#[derive(Debug)]
struct ContextChildIds<'a> {
    storage: &'a ParseTreeStorage,
    next: u32,
    remaining: usize,
}

impl Iterator for ContextChildIds<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NONE {
            return None;
        }
        let link = &self.storage.child_links[self.next as usize];
        self.next = link.next;
        self.remaining -= 1;
        Some(link.node)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for ContextChildIds<'_> {}

/// Transient builder for one open rule.
///
/// This value owns no tree nodes or child vector. Child IDs are appended to
/// parser-owned scratch links and are copied once into the global child pool
/// when the rule completes.
#[derive(Debug)]
pub struct ParserRuleContext {
    rule_index: usize,
    invoking_state: isize,
    alt_number: usize,
    context_alt_number: usize,
    start: Option<TokenId>,
    stop: Option<TokenId>,
    int_returns: BTreeMap<String, i64>,
    first_child: u32,
    last_child: u32,
    child_count: u32,
    matched_child: bool,
    exception: Option<AntlrError>,
    attrs: Option<GeneratedAttrs>,
}

impl ParserRuleContext {
    #[must_use]
    pub const fn new(rule_index: usize, invoking_state: isize) -> Self {
        Self {
            rule_index,
            invoking_state,
            alt_number: 0,
            context_alt_number: 0,
            start: None,
            stop: None,
            int_returns: BTreeMap::new(),
            first_child: NONE,
            last_child: NONE,
            child_count: 0,
            matched_child: false,
            exception: None,
            attrs: None,
        }
    }

    pub(crate) const fn with_child_capacity(
        rule_index: usize,
        invoking_state: isize,
        _capacity: usize,
    ) -> Self {
        Self::new(rule_index, invoking_state)
    }

    #[must_use]
    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    #[must_use]
    pub const fn invoking_state(&self) -> isize {
        self.invoking_state
    }

    #[must_use]
    pub const fn alt_number(&self) -> usize {
        self.alt_number
    }

    pub const fn set_alt_number(&mut self, alt_number: usize) {
        self.alt_number = alt_number;
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn context_alt_number(&self) -> usize {
        self.context_alt_number
    }

    #[doc(hidden)]
    pub const fn set_context_alt_number(&mut self, alt_number: usize) {
        self.context_alt_number = alt_number;
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

    pub(crate) const fn set_start_id(&mut self, token: TokenId) {
        self.start = Some(token);
    }

    pub(crate) const fn set_stop_id(&mut self, token: TokenId) {
        self.stop = Some(token);
    }

    pub(crate) const fn set_start_from_context(&mut self, other: &Self) {
        self.start = other.start;
    }

    pub fn set_int_return(&mut self, name: impl Into<String>, value: i64) {
        self.int_returns.insert(name.into(), value);
    }

    #[must_use]
    pub fn int_return(&self, name: &str) -> Option<i64> {
        self.int_returns.get(name).copied()
    }

    pub fn set_generated_attrs(&mut self, attrs: GeneratedAttrs) {
        self.attrs = Some(attrs);
    }

    #[must_use]
    pub fn generated_attrs<T: Any>(&self) -> Option<&T> {
        self.attrs.as_ref().and_then(GeneratedAttrs::downcast_ref)
    }

    #[must_use]
    pub const fn exception(&self) -> Option<&AntlrError> {
        self.exception.as_ref()
    }

    pub fn set_exception(&mut self, error: AntlrError) {
        self.exception = Some(error);
    }

    #[must_use]
    pub const fn child_count(&self) -> usize {
        self.child_count as usize
    }

    #[must_use]
    pub const fn has_matched_child(&self) -> bool {
        self.matched_child
    }

    pub const fn note_matched_child(&mut self) {
        self.matched_child = true;
    }

    pub fn child_nodes<'a>(
        &'a self,
        storage: &'a ParseTreeStorage,
        tokens: &'a TokenStore,
    ) -> impl Iterator<Item = Node<'a>> + 'a {
        storage
            .context_child_ids(self)
            .filter_map(move |id| storage.node(tokens, id))
    }

    pub fn child_rules<'a>(
        &'a self,
        storage: &'a ParseTreeStorage,
        tokens: &'a TokenStore,
        rule_index: usize,
    ) -> impl Iterator<Item = RuleNodeView<'a>> + 'a {
        self.child_nodes(storage, tokens).filter_map(move |child| {
            let rule = child.as_rule()?;
            (rule.rule_index() == rule_index).then_some(rule)
        })
    }

    pub fn child_rule_trees<'a>(
        &'a self,
        storage: &'a ParseTreeStorage,
        tokens: &'a TokenStore,
        rule_index: usize,
    ) -> impl Iterator<Item = Node<'a>> + 'a {
        self.child_rules(storage, tokens, rule_index)
            .map(RuleNodeView::node)
    }

    pub fn child_tokens<'a>(
        &'a self,
        storage: &'a ParseTreeStorage,
        tokens: &'a TokenStore,
        token_type: i32,
    ) -> impl Iterator<Item = TerminalNodeView<'a>> + 'a {
        self.child_nodes(storage, tokens).filter_map(move |child| {
            let terminal = match child.kind() {
                NodeKind::Terminal => child.as_terminal(),
                NodeKind::Error => child.as_error().map(ErrorNodeView::terminal),
                NodeKind::Rule => None,
            }?;
            (terminal.symbol().token_type() == token_type).then_some(terminal)
        })
    }

    pub fn terminal_children<'a>(
        &'a self,
        storage: &'a ParseTreeStorage,
        tokens: &'a TokenStore,
    ) -> impl Iterator<Item = TerminalNodeView<'a>> + 'a {
        self.child_nodes(storage, tokens)
            .filter_map(|child| match child.kind() {
                NodeKind::Terminal => child.as_terminal(),
                NodeKind::Error => child.as_error().map(ErrorNodeView::terminal),
                NodeKind::Rule => None,
            })
    }

    #[must_use]
    pub fn text(&self, storage: &ParseTreeStorage, tokens: &TokenStore) -> String {
        self.child_nodes(storage, tokens).map(Node::text).collect()
    }

    #[must_use]
    pub fn to_string_tree_with_names<S: AsRef<str>>(
        &self,
        storage: &ParseTreeStorage,
        tokens: &TokenStore,
        rule_names: &[S],
    ) -> String {
        let name = rule_names
            .get(self.rule_index)
            .map_or("<unknown>", |name| name.as_ref());
        let display_name = if self.alt_number == 0 {
            name.to_owned()
        } else {
            format!("{name}:{}", self.alt_number)
        };
        if self.child_count == 0 {
            return display_name;
        }
        let children = self
            .child_nodes(storage, tokens)
            .map(|child| child.to_string_tree_with_names(rule_names))
            .collect::<Vec<_>>()
            .join(" ");
        format!("({display_name} {children})")
    }

    #[must_use]
    pub fn to_string_tree<R: Recognizer>(
        &self,
        recognizer: Option<&R>,
        storage: &ParseTreeStorage,
        tokens: &TokenStore,
    ) -> String {
        recognizer.map_or_else(
            || self.to_string_tree_with_names::<&str>(storage, tokens, &[]),
            |recognizer| {
                self.to_string_tree_with_names(storage, tokens, recognizer.data().rule_names())
            },
        )
    }
}

/// Type-erased generated-rule attributes stored only for rules that use them.
pub struct GeneratedAttrs(Box<dyn Any>);

impl GeneratedAttrs {
    #[must_use]
    pub fn new<T: Any>(attrs: T) -> Self {
        Self(Box::new(attrs))
    }

    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl fmt::Debug for GeneratedAttrs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GeneratedAttrs(..)")
    }
}

pub trait FromRuleNode<'tree>: Sized {
    fn from_rule_node(node: RuleNodeView<'tree>) -> Option<Self>;
}

pub trait ParseTreeListener {
    fn enter_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), AntlrError> {
        Ok(())
    }

    fn exit_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), AntlrError> {
        Ok(())
    }

    fn visit_terminal(&mut self, _node: TerminalNodeView<'_>) -> Result<(), AntlrError> {
        Ok(())
    }

    fn visit_error_node(&mut self, _node: ErrorNodeView<'_>) -> Result<(), AntlrError> {
        Ok(())
    }
}

/// Value-returning, caller-directed traversal over a completed parse tree.
///
/// Generated grammar visitors adapt typed rule and alternative callbacks to
/// this runtime contract. The default traversal returns the latest child's
/// result, matching ANTLR's base visitor behavior.
pub trait ParseTreeVisitor {
    type Result: Default;

    fn visit(&mut self, tree: Node<'_>) -> Self::Result {
        match tree.kind() {
            NodeKind::Rule => self.visit_rule(tree.as_rule().expect("rule node kind checked")),
            NodeKind::Terminal => {
                self.visit_terminal(tree.as_terminal().expect("terminal node kind checked"))
            }
            NodeKind::Error => {
                self.visit_error_node(tree.as_error().expect("error node kind checked"))
            }
        }
    }

    fn visit_rule(&mut self, node: RuleNodeView<'_>) -> Self::Result {
        self.visit_children(node)
    }

    fn visit_children(&mut self, node: RuleNodeView<'_>) -> Self::Result {
        let mut result = self.default_result();
        for child in node.children() {
            if !self.should_visit_next_child(node, &result) {
                break;
            }
            let child_result = self.visit(child);
            result = self.aggregate_result(result, child_result);
        }
        result
    }

    fn visit_terminal(&mut self, _node: TerminalNodeView<'_>) -> Self::Result {
        self.default_result()
    }

    fn visit_error_node(&mut self, _node: ErrorNodeView<'_>) -> Self::Result {
        self.default_result()
    }

    fn default_result(&mut self) -> Self::Result {
        Self::Result::default()
    }

    fn aggregate_result(
        &mut self,
        _aggregate: Self::Result,
        next_result: Self::Result,
    ) -> Self::Result {
        next_result
    }

    fn should_visit_next_child(
        &mut self,
        _node: RuleNodeView<'_>,
        _current_result: &Self::Result,
    ) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct ParseTreeWalker;

impl ParseTreeWalker {
    pub fn walk<L: ParseTreeListener>(listener: &mut L, tree: Node<'_>) -> Result<(), AntlrError> {
        enum Event {
            Enter(NodeId),
            Exit(NodeId),
        }

        let storage = tree.storage;
        let tokens = tree.tokens;
        let mut stack = vec![Event::Enter(tree.id)];
        while let Some(event) = stack.pop() {
            match event {
                Event::Enter(id) => {
                    let node = storage
                        .node(tokens, id)
                        .expect("walker node ID should remain valid");
                    match node.kind() {
                        NodeKind::Rule => {
                            let rule = node.as_rule().expect("rule node kind checked");
                            listener.enter_every_rule(rule)?;
                            stack.push(Event::Exit(id));
                            stack.extend(
                                storage
                                    .child_ids(id)
                                    .iter()
                                    .rev()
                                    .copied()
                                    .map(Event::Enter),
                            );
                        }
                        NodeKind::Terminal => listener.visit_terminal(
                            node.as_terminal().expect("terminal node kind checked"),
                        )?,
                        NodeKind::Error => {
                            listener.visit_error_node(
                                node.as_error().expect("error node kind checked"),
                            )?;
                        }
                    }
                }
                Event::Exit(id) => {
                    let rule = storage
                        .node(tokens, id)
                        .and_then(Node::as_rule)
                        .expect("walker exit node should remain a rule");
                    listener.exit_every_rule(rule)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ParsedFile {
    tokens: TokenStore,
    tree: ParseTreeStorage,
    root: NodeId,
}

impl ParsedFile {
    #[must_use]
    pub fn new(tokens: TokenStore, mut tree: ParseTreeStorage, root: NodeId) -> Self {
        tree.discard_scratch();
        Self { tokens, tree, root }
    }

    #[must_use]
    pub const fn tokens(&self) -> &TokenStore {
        &self.tokens
    }

    #[must_use]
    pub const fn storage(&self) -> &ParseTreeStorage {
        &self.tree
    }

    #[must_use]
    pub const fn root_id(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub fn tree(&self) -> Node<'_> {
        self.tree
            .node(&self.tokens, self.root)
            .expect("parsed file root ID should remain valid")
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<Node<'_>> {
        self.tree.node(&self.tokens, id)
    }

    #[must_use]
    pub fn into_parts(self) -> (TokenStore, ParseTreeStorage, NodeId) {
        (self.tokens, self.tree, self.root)
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

fn stored_token_id(raw: u32) -> TokenId {
    TokenId::try_from(raw as usize).expect("stored token ID should fit in u32")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenSpec;

    fn token(store: &mut TokenStore, token_type: i32, text: &str) -> TokenId {
        store
            .push(TokenSpec::explicit(token_type, text))
            .expect("test token should fit")
    }

    #[test]
    fn stores_rule_children_in_one_pooled_range() {
        let mut tokens = TokenStore::new(None, "");
        let first = token(&mut tokens, 1, "a");
        let second = token(&mut tokens, 2, "b");
        let mut storage = ParseTreeStorage::new();
        let first = storage.terminal(first);
        let second = storage.error(second);
        let mut context = ParserRuleContext::new(0, -1);
        storage.add_child(&mut context, first);
        storage.add_child(&mut context, second);
        let root = storage.finish_rule(context);
        let parsed = ParsedFile::new(tokens, storage, root);

        assert_eq!(parsed.tree().text(), "ab");
        assert_eq!(parsed.tree().children().count(), 2);
        assert_eq!(parsed.storage().stats().edges, 2);
        assert_eq!(parsed.storage().stats().scratch_links, 0);
        assert_eq!(
            parsed.tree().to_string_tree_with_names(&["root"]),
            "(root a b)"
        );
    }

    #[test]
    fn context_alt_number_does_not_change_public_tree_rendering() {
        let tokens = TokenStore::new(None, "");
        let mut storage = ParseTreeStorage::new();
        let mut context = ParserRuleContext::new(0, -1);
        context.set_context_alt_number(2);

        assert_eq!(context.alt_number(), 0);
        assert_eq!(context.context_alt_number(), 2);
        assert_eq!(
            context.to_string_tree_with_names(&storage, &tokens, &["root"]),
            "root"
        );

        let root = storage.finish_rule(context);
        let parsed = ParsedFile::new(tokens, storage, root);
        let rule = parsed.tree().as_rule().expect("root rule");
        assert_eq!(rule.alt_number(), 0);
        assert_eq!(rule.context_alt_number(), 2);
        assert_eq!(parsed.tree().to_string_tree_with_names(&["root"]), "root");
    }

    #[test]
    fn descendants_and_walker_preserve_antlr_order() {
        let mut tokens = TokenStore::new(None, "");
        let a = token(&mut tokens, 1, "a");
        let b = token(&mut tokens, 2, "b");
        let mut storage = ParseTreeStorage::new();
        let a = storage.terminal(a);
        let b = storage.terminal(b);
        let mut child = ParserRuleContext::new(1, 7);
        storage.add_child(&mut child, b);
        let child = storage.finish_rule(child);
        let mut root = ParserRuleContext::new(0, -1);
        storage.add_child(&mut root, a);
        storage.add_child(&mut root, child);
        let root = storage.finish_rule(root);
        let parsed = ParsedFile::new(tokens, storage, root);

        let visited = parsed
            .tree()
            .descendants()
            .map(|node| match node.kind() {
                NodeKind::Rule => format!(
                    "r{}",
                    node.as_rule().expect("rule node kind checked").rule_index()
                ),
                NodeKind::Terminal => node
                    .as_terminal()
                    .expect("terminal node kind checked")
                    .text()
                    .to_owned(),
                NodeKind::Error => node
                    .as_error()
                    .expect("error node kind checked")
                    .text()
                    .to_owned(),
            })
            .collect::<Vec<_>>();
        assert_eq!(visited, ["r0", "a", "r1", "b"]);

        #[derive(Default)]
        struct Listener(Vec<String>);
        impl ParseTreeListener for Listener {
            fn enter_every_rule(&mut self, ctx: RuleNodeView<'_>) -> Result<(), AntlrError> {
                self.0.push(format!("enter{}", ctx.rule_index()));
                Ok(())
            }

            fn exit_every_rule(&mut self, ctx: RuleNodeView<'_>) -> Result<(), AntlrError> {
                self.0.push(format!("exit{}", ctx.rule_index()));
                Ok(())
            }

            fn visit_terminal(&mut self, node: TerminalNodeView<'_>) -> Result<(), AntlrError> {
                self.0.push(node.text().to_owned());
                Ok(())
            }
        }
        let mut listener = Listener::default();
        ParseTreeWalker::walk(&mut listener, parsed.tree())
            .expect("test listener should accept every node");
        assert_eq!(listener.0, ["enter0", "a", "enter1", "b", "exit1", "exit0"]);
    }

    fn visitor_test_tree() -> ParsedFile {
        let mut tokens = TokenStore::new(None, "");
        let a = token(&mut tokens, 1, "a");
        let b = token(&mut tokens, 2, "b");
        let error = token(&mut tokens, 3, "!");
        let mut storage = ParseTreeStorage::new();
        let a = storage.terminal(a);
        let b = storage.terminal(b);
        let error = storage.error(error);
        let mut child = ParserRuleContext::new(1, 7);
        storage.add_child(&mut child, b);
        let child = storage.finish_rule(child);
        let mut root = ParserRuleContext::new(0, -1);
        storage.add_child(&mut root, a);
        storage.add_child(&mut root, child);
        storage.add_child(&mut root, error);
        let root = storage.finish_rule(root);
        ParsedFile::new(tokens, storage, root)
    }

    #[test]
    fn visitor_dispatches_and_aggregates_all_node_kinds() {
        #[derive(Default)]
        struct Visitor(Vec<String>);

        impl ParseTreeVisitor for Visitor {
            type Result = Vec<String>;

            fn visit_rule(&mut self, node: RuleNodeView<'_>) -> Self::Result {
                self.0.push(format!("rule{}", node.rule_index()));
                self.visit_children(node)
            }

            fn visit_terminal(&mut self, node: TerminalNodeView<'_>) -> Self::Result {
                vec![format!("terminal:{}", node.text())]
            }

            fn visit_error_node(&mut self, node: ErrorNodeView<'_>) -> Self::Result {
                vec![format!("error:{}", node.text())]
            }

            fn aggregate_result(
                &mut self,
                mut aggregate: Self::Result,
                next_result: Self::Result,
            ) -> Self::Result {
                aggregate.extend(next_result);
                aggregate
            }
        }

        let parsed = visitor_test_tree();
        let mut visitor = Visitor::default();
        assert_eq!(
            visitor.visit(parsed.tree()),
            ["terminal:a", "terminal:b", "error:!"]
        );
        assert_eq!(visitor.0, ["rule0", "rule1"]);
    }

    #[test]
    fn visitor_default_aggregation_returns_the_latest_child() {
        struct Visitor;

        impl ParseTreeVisitor for Visitor {
            type Result = String;

            fn visit_terminal(&mut self, node: TerminalNodeView<'_>) -> Self::Result {
                node.text().to_owned()
            }

            fn visit_error_node(&mut self, node: ErrorNodeView<'_>) -> Self::Result {
                node.text().to_owned()
            }
        }

        let parsed = visitor_test_tree();
        assert_eq!(Visitor.visit(parsed.tree()), "!");
    }

    #[test]
    fn visitor_can_short_circuit_before_any_or_later_children() {
        struct Visitor {
            limit: usize,
            visited: usize,
        }

        impl ParseTreeVisitor for Visitor {
            type Result = usize;

            fn visit_terminal(&mut self, _node: TerminalNodeView<'_>) -> Self::Result {
                self.visited += 1;
                1
            }

            fn visit_error_node(&mut self, _node: ErrorNodeView<'_>) -> Self::Result {
                self.visited += 1;
                1
            }

            fn aggregate_result(
                &mut self,
                aggregate: Self::Result,
                next_result: Self::Result,
            ) -> Self::Result {
                aggregate + next_result
            }

            fn should_visit_next_child(
                &mut self,
                _node: RuleNodeView<'_>,
                current_result: &Self::Result,
            ) -> bool {
                *current_result < self.limit
            }
        }

        let parsed = visitor_test_tree();
        let mut none = Visitor {
            limit: 0,
            visited: 0,
        };
        assert_eq!(none.visit(parsed.tree()), 0);
        assert_eq!(none.visited, 0);

        let mut one = Visitor {
            limit: 1,
            visited: 0,
        };
        assert_eq!(one.visit(parsed.tree()), 1);
        assert_eq!(one.visited, 1);
    }

    #[test]
    fn invocation_states_exclude_a_nonnegative_root_frame() {
        let tokens = TokenStore::new(None, "");
        let mut storage = ParseTreeStorage::new();
        let grandchild = storage.finish_rule(ParserRuleContext::new(2, 13));
        let mut child = ParserRuleContext::new(1, 7);
        storage.add_child(&mut child, grandchild);
        let child = storage.finish_rule(child);
        let mut root = ParserRuleContext::new(0, 4);
        storage.add_child(&mut root, child);
        let root = storage.finish_rule(root);
        let parsed = ParsedFile::new(tokens, storage, root);

        let root = parsed
            .node(root)
            .and_then(Node::as_rule)
            .expect("root rule should be stored");
        let child = parsed
            .node(child)
            .and_then(Node::as_rule)
            .expect("child rule should be stored");
        let grandchild = parsed
            .node(grandchild)
            .and_then(Node::as_rule)
            .expect("grandchild rule should be stored");

        assert_eq!(root.invocation_states().collect::<Vec<_>>(), []);
        assert_eq!(child.invocation_states().collect::<Vec<_>>(), [7]);
        assert_eq!(grandchild.invocation_states().collect::<Vec<_>>(), [13, 7]);
    }

    #[test]
    fn uncommon_rule_payloads_live_in_sparse_extras() {
        let tokens = TokenStore::new(None, "");
        let mut storage = ParseTreeStorage::new();
        let plain = storage.finish_rule(ParserRuleContext::new(0, -1));
        let mut rich = ParserRuleContext::new(1, 3);
        rich.set_int_return("value", 42);
        let rich = storage.finish_rule(rich);
        let parsed = ParsedFile::new(tokens, storage, rich);

        assert_eq!(parsed.storage().extra_count(), 1);
        assert_eq!(
            parsed
                .node(rich)
                .expect("rich rule should be stored")
                .as_rule()
                .expect("rich node should be a rule")
                .int_return("value"),
            Some(42)
        );
        assert!(
            parsed
                .node(plain)
                .expect("plain rule should be stored")
                .as_rule()
                .expect("plain node should be a rule")
                .int_return("value")
                .is_none()
        );
    }

    #[test]
    fn preserves_maximum_token_id_in_nodes_and_rule_spans() {
        let max = TokenId::try_from(u32::MAX as usize).expect("maximum token ID should fit");
        let tokens = TokenStore::new(None, "");
        let mut storage = ParseTreeStorage::new();
        let terminal = storage.terminal(max);
        assert_eq!(storage.token_id(terminal), Some(max));

        let mut context = ParserRuleContext::new(0, -1);
        context.set_start_id(max);
        context.set_stop_id(max);
        let rule = storage.finish_rule(context);
        let rule = storage
            .node(&tokens, rule)
            .and_then(Node::as_rule)
            .expect("rule should be stored");
        assert_eq!(rule.start_id(), Some(max));
        assert_eq!(rule.stop_id(), Some(max));
    }
}
