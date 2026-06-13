//! Abstract Transition Network structures shared by generated lexers and
//! parsers.
//!
//! ANTLR serializes grammars into an ATN. Generated Rust code stores that
//! serialized data in static metadata, while the runtime deserializes it into
//! these compact Rust structures for simulation.

pub mod lexer;
pub mod parser;
pub mod serialized;

/// Distinguishes lexer ATNs from parser ATNs in serialized grammar metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtnType {
    Lexer,
    Parser,
}

/// Deserialized ANTLR Abstract Transition Network.
///
/// The structure keeps the state graph plus ANTLR side tables such as
/// rule-to-start, rule-to-token, mode-to-start, decisions, and lexer actions.
/// The side tables are part of the runtime contract because generated grammars
/// should only need to provide metadata; simulation stays in this crate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Atn {
    grammar_type: AtnType,
    max_token_type: i32,
    states: Vec<AtnState>,
    rule_to_start_state: Vec<usize>,
    rule_to_stop_state: Vec<usize>,
    rule_to_token_type: Vec<i32>,
    mode_to_start_state: Vec<usize>,
    decision_to_state: Vec<usize>,
    lexer_actions: Vec<LexerAction>,
}

impl Atn {
    /// Creates an empty ATN with the grammar kind and maximum token type read
    /// from the serialized header.
    pub const fn new(grammar_type: AtnType, max_token_type: i32) -> Self {
        Self {
            grammar_type,
            max_token_type,
            states: Vec::new(),
            rule_to_start_state: Vec::new(),
            rule_to_stop_state: Vec::new(),
            rule_to_token_type: Vec::new(),
            mode_to_start_state: Vec::new(),
            decision_to_state: Vec::new(),
            lexer_actions: Vec::new(),
        }
    }

    pub const fn grammar_type(&self) -> AtnType {
        self.grammar_type
    }

    pub const fn max_token_type(&self) -> i32 {
        self.max_token_type
    }

    pub fn states(&self) -> &[AtnState] {
        &self.states
    }

    pub fn state(&self, state_number: usize) -> Option<&AtnState> {
        self.states.get(state_number)
    }

    pub fn state_mut(&mut self, state_number: usize) -> Option<&mut AtnState> {
        self.states.get_mut(state_number)
    }

    /// Appends a state and returns the state number assigned by insertion
    /// order.
    pub fn add_state(&mut self, state: AtnState) -> usize {
        let index = self.states.len();
        self.states.push(state);
        index
    }

    pub fn decision_to_state(&self) -> &[usize] {
        &self.decision_to_state
    }

    pub fn add_decision_state(&mut self, state_number: usize) {
        self.decision_to_state.push(state_number);
    }

    pub fn rule_to_start_state(&self) -> &[usize] {
        &self.rule_to_start_state
    }

    pub fn set_rule_to_start_state(&mut self, rule_to_start_state: Vec<usize>) {
        self.rule_to_start_state = rule_to_start_state;
    }

    pub fn rule_to_stop_state(&self) -> &[usize] {
        &self.rule_to_stop_state
    }

    pub fn set_rule_to_stop_state(&mut self, rule_to_stop_state: Vec<usize>) {
        self.rule_to_stop_state = rule_to_stop_state;
    }

    pub fn rule_to_token_type(&self) -> &[i32] {
        &self.rule_to_token_type
    }

    pub fn set_rule_to_token_type(&mut self, rule_to_token_type: Vec<i32>) {
        self.rule_to_token_type = rule_to_token_type;
    }

    pub fn mode_to_start_state(&self) -> &[usize] {
        &self.mode_to_start_state
    }

    pub fn add_mode_start_state(&mut self, state_number: usize) {
        self.mode_to_start_state.push(state_number);
    }

    pub fn lexer_actions(&self) -> &[LexerAction] {
        &self.lexer_actions
    }

    pub fn set_lexer_actions(&mut self, lexer_actions: Vec<LexerAction>) {
        self.lexer_actions = lexer_actions;
    }
}

/// A node in the ANTLR ATN graph.
///
/// Some ANTLR state subclasses carry references to paired states, such as a
/// block-start state's end state or a loop-end state's loop-back state. This
/// representation stores those links as state numbers so the graph remains easy
/// to clone and serialize in tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtnState {
    pub state_number: usize,
    pub rule_index: Option<usize>,
    pub kind: AtnStateKind,
    pub end_state: Option<usize>,
    pub loop_back_state: Option<usize>,
    pub non_greedy: bool,
    pub precedence_rule_decision: bool,
    pub left_recursive_rule: bool,
    pub transitions: Vec<Transition>,
}

impl AtnState {
    /// Creates an ATN state with no rule index and no outgoing transitions.
    pub const fn new(state_number: usize, kind: AtnStateKind) -> Self {
        Self {
            state_number,
            rule_index: None,
            kind,
            end_state: None,
            loop_back_state: None,
            non_greedy: false,
            precedence_rule_decision: false,
            left_recursive_rule: false,
            transitions: Vec::new(),
        }
    }

    #[must_use]
    pub const fn with_rule_index(mut self, rule_index: usize) -> Self {
        self.rule_index = Some(rule_index);
        self
    }

    /// Adds an outgoing transition in serialized order.
    ///
    /// Transition order matters for alternatives and lexer priority, so the
    /// runtime preserves the order emitted by ANTLR.
    pub fn add_transition(&mut self, transition: Transition) {
        self.transitions.push(transition);
    }

    pub fn is_rule_stop(&self) -> bool {
        self.kind == AtnStateKind::RuleStop
    }
}

/// Serialized ANTLR state kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtnStateKind {
    Invalid,
    Basic,
    RuleStart,
    BlockStart,
    PlusBlockStart,
    StarBlockStart,
    TokenStart,
    RuleStop,
    BlockEnd,
    StarLoopBack,
    StarLoopEntry,
    PlusLoopBack,
    LoopEnd,
}

/// Edge between two ATN states.
///
/// Epsilon-like transitions do not consume input. Matching transitions compare
/// the current input symbol against an atom, range, set, negated set, or
/// wildcard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Transition {
    Epsilon {
        target: usize,
    },
    Atom {
        target: usize,
        label: i32,
    },
    Range {
        target: usize,
        start: i32,
        stop: i32,
    },
    Set {
        target: usize,
        set: IntervalSet,
    },
    NotSet {
        target: usize,
        set: IntervalSet,
    },
    Wildcard {
        target: usize,
    },
    Rule {
        target: usize,
        rule_index: usize,
        follow_state: usize,
        precedence: i32,
    },
    Predicate {
        target: usize,
        rule_index: usize,
        pred_index: usize,
        context_dependent: bool,
    },
    Action {
        target: usize,
        rule_index: usize,
        action_index: Option<usize>,
        context_dependent: bool,
    },
    Precedence {
        target: usize,
        precedence: i32,
    },
}

impl Transition {
    /// Returns the target state number for this transition.
    pub const fn target(&self) -> usize {
        match self {
            Self::Epsilon { target }
            | Self::Atom { target, .. }
            | Self::Range { target, .. }
            | Self::Set { target, .. }
            | Self::NotSet { target, .. }
            | Self::Wildcard { target }
            | Self::Rule { target, .. }
            | Self::Predicate { target, .. }
            | Self::Action { target, .. }
            | Self::Precedence { target, .. } => *target,
        }
    }

    /// Returns whether traversing this transition consumes no input.
    pub const fn is_epsilon(&self) -> bool {
        matches!(
            self,
            Self::Epsilon { .. }
                | Self::Rule { .. }
                | Self::Predicate { .. }
                | Self::Action { .. }
                | Self::Precedence { .. }
        )
    }

    /// Tests whether this transition consumes `symbol`.
    ///
    /// `min_vocabulary` and `max_vocabulary` define the accepted symbol range
    /// for wildcard and negated-set transitions.
    pub fn matches(&self, symbol: i32, min_vocabulary: i32, max_vocabulary: i32) -> bool {
        match self {
            Self::Atom { label, .. } => *label == symbol,
            Self::Range { start, stop, .. } => (*start..=*stop).contains(&symbol),
            Self::Set { set, .. } => set.contains(symbol),
            Self::NotSet { set, .. } => {
                (min_vocabulary..=max_vocabulary).contains(&symbol) && !set.contains(symbol)
            }
            Self::Wildcard { .. } => (min_vocabulary..=max_vocabulary).contains(&symbol),
            Self::Epsilon { .. }
            | Self::Rule { .. }
            | Self::Predicate { .. }
            | Self::Action { .. }
            | Self::Precedence { .. } => false,
        }
    }
}

/// Ordered set of integer intervals used by set and negated-set transitions.
///
/// Unicode grammars can contain very large ranges, so this stores normalized
/// intervals rather than expanding every code point into a flat set.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IntervalSet {
    ranges: Vec<(i32, i32)>,
}

impl IntervalSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_range(start: i32, stop: i32) -> Self {
        let mut set = Self::new();
        set.add_range(start, stop);
        set
    }

    pub fn add(&mut self, value: i32) {
        self.add_range(value, value);
    }

    /// Adds an inclusive interval and merges it with adjacent or overlapping
    /// intervals.
    pub fn add_range(&mut self, start: i32, stop: i32) {
        let (start, stop) = if start <= stop {
            (start, stop)
        } else {
            (stop, start)
        };
        self.ranges.push((start, stop));
        self.normalize();
    }

    /// Re-sorts and coalesces interval storage after insertion.
    fn normalize(&mut self) {
        self.ranges.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(self.ranges.len());
        for (start, stop) in self.ranges.drain(..) {
            if let Some((_, last_stop)) = merged.last_mut() {
                if start <= last_stop.saturating_add(1) {
                    *last_stop = (*last_stop).max(stop);
                    continue;
                }
            }
            merged.push((start, stop));
        }
        self.ranges = merged;
    }

    /// Returns true when `value` falls inside any stored interval.
    pub fn contains(&self, value: i32) -> bool {
        // Ranges are kept sorted and coalesced by `normalize`, so the first
        // range whose `start > value` cannot contain `value` and neither can
        // any range after it. Binary searching for that boundary turns
        // membership lookup from O(n) to O(log n), which matters because
        // parser/lexer hot paths call this once per `Set`/`NotSet`/`Wildcard`
        // transition probe.
        match self.ranges.binary_search_by(|(start, _)| start.cmp(&value)) {
            Ok(_) => true,
            Err(pos) => pos > 0 && self.ranges[pos - 1].1 >= value,
        }
    }

    pub fn ranges(&self) -> &[(i32, i32)] {
        &self.ranges
    }

    pub const fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
}

/// Serialized lexer action attached to an action transition.
///
/// These actions are grammar-independent operations generated by ANTLR's lexer
/// commands (`skip`, `more`, `type`, `channel`, `pushMode`, `popMode`, and
/// `mode`). Custom embedded actions are represented but intentionally inert
/// until a generated semantic-action hook exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LexerAction {
    Channel(i32),
    Custom { rule_index: i32, action_index: i32 },
    Mode(i32),
    More,
    PopMode,
    PushMode(i32),
    Skip,
    Type(i32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_set_handles_ranges() {
        let set = IntervalSet::from_range(2, 4);
        assert!(set.contains(2));
        assert!(set.contains(3));
        assert!(set.contains(4));
        assert!(!set.contains(5));
        assert_eq!(set.ranges(), &[(2, 4)]);
    }
}
