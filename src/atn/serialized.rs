use std::borrow::Cow;

use crate::atn::parser_atn::{
    ParserAtn, ParserAtnBuilder, ParserAtnError, ParserIntervalSetId, ParserTransitionSpec,
};
use crate::atn::{
    AtnStateKind, IntervalSet, LexerAction, LexerAtn, LexerAtnState, LexerTransition,
};
use crate::errors::AntlrError;
use crate::token::TOKEN_EOF;

pub const SERIALIZED_VERSION: i32 = 4;

/// Raw integer form of an ANTLR v4 serialized ATN.
///
/// ANTLR targets commonly embed this data as strings or integer arrays. The
/// Rust generator emits integer arrays from its compiled lexer artifact, while
/// `from_chars` supports targets that encode ATN values in string literals.
#[derive(Clone, Debug)]
pub struct SerializedAtn<'a> {
    values: Cow<'a, [i32]>,
}

impl<'a> SerializedAtn<'a> {
    /// Creates serialized ATN data from an already-decoded integer array.
    pub const fn from_i32(values: &'a [i32]) -> Self {
        Self {
            values: Cow::Borrowed(values),
        }
    }

    /// Creates serialized ATN data by widening each character to its scalar
    /// value.
    ///
    /// This is useful for ANTLR targets that store serialized ATN data in
    /// string fragments. Java-style 16-bit word decoding is not applied here;
    /// callers should pass already-decoded characters for now.
    pub fn from_chars(chars: impl IntoIterator<Item = char>) -> SerializedAtn<'static> {
        SerializedAtn {
            values: Cow::Owned(chars.into_iter().map(|ch| ch as i32).collect()),
        }
    }

    pub fn values(&self) -> &[i32] {
        &self.values
    }
}

/// Cursor-based decoder for ANTLR v4 serialized ATN data.
#[derive(Debug)]
pub struct AtnDeserializer<'a> {
    values: &'a [i32],
    cursor: usize,
}

impl<'a> AtnDeserializer<'a> {
    /// Creates a deserializer over immutable serialized ATN storage.
    pub fn new(serialized: &'a SerializedAtn<'_>) -> Self {
        Self {
            values: serialized.values(),
            cursor: 0,
        }
    }

    /// Decodes an ANTLR v4 serialized lexer ATN into the lexer graph.
    ///
    /// The layout is order-sensitive: states come first, followed by non-greedy
    /// and precedence markers, rule tables, mode starts, interval sets, edges,
    /// decisions, and lexer actions. This method keeps ANTLR's side tables as
    /// explicit vectors because lexer simulation needs them without depending
    /// on generated per-rule code. Parser input is rejected; use
    /// [`Self::deserialize_parser`] for the packed parser representation.
    pub fn deserialize(mut self) -> Result<LexerAtn, AntlrError> {
        let version = self.read("version")?;
        if version != SERIALIZED_VERSION {
            return Err(AntlrError::Unsupported(format!(
                "serialized ATN version {version}; expected {SERIALIZED_VERSION}"
            )));
        }

        match self.read("grammar type")? {
            0 => {}
            1 => {
                return Err(AntlrError::Unsupported(
                    "parser ATNs require AtnDeserializer::deserialize_parser()".to_owned(),
                ));
            }
            other => {
                return Err(AntlrError::Unsupported(format!(
                    "serialized ATN grammar type {other}"
                )));
            }
        }
        let max_token_type = self.read("max token type")?;
        let mut atn = LexerAtn::new(max_token_type);

        self.deserialize_states(&mut atn)?;
        self.deserialize_non_greedy_states(&mut atn)?;
        self.deserialize_precedence_states(&mut atn)?;
        self.deserialize_rules(&mut atn)?;
        self.deserialize_modes(&mut atn)?;
        let sets = self.deserialize_sets()?;
        self.deserialize_edges(&mut atn, &sets)?;
        self.deserialize_decisions(&mut atn)?;
        self.deserialize_lexer_actions(&mut atn)?;
        mark_precedence_decisions(&mut atn);

        Ok(atn)
    }

    /// Decodes parser metadata directly into the packed parser-only runtime
    /// representation.
    ///
    /// No lexer object graph is constructed. Temporary state and edge records
    /// live in centralized builder vectors, then become one validated
    /// index-addressed word stream.
    pub fn deserialize_parser(mut self) -> Result<ParserAtn, AntlrError> {
        self.deserialize_parser_header()?;
        let max_token_type = self.read("max token type")?;
        let mut builder = ParserAtnBuilder::new(max_token_type);
        self.deserialize_parser_states(&mut builder)?;
        self.deserialize_parser_flags(&mut builder)?;
        self.deserialize_parser_rules(&mut builder)?;
        self.deserialize_parser_modes(&builder)?;
        let sets = self.deserialize_parser_sets(&mut builder)?;
        self.deserialize_parser_edges(&mut builder, &sets)?;
        self.deserialize_parser_decisions(&mut builder)?;
        if self.cursor != self.values.len() {
            return Err(AntlrError::Unsupported(format!(
                "serialized parser ATN has {} trailing values",
                self.values.len() - self.cursor
            )));
        }
        builder.finish().map_err(|error| parser_atn_error(&error))
    }

    fn deserialize_parser_header(&mut self) -> Result<(), AntlrError> {
        let version = self.read("version")?;
        if version != SERIALIZED_VERSION {
            return Err(AntlrError::Unsupported(format!(
                "serialized ATN version {version}; expected {SERIALIZED_VERSION}"
            )));
        }
        let grammar_type = self.read("grammar type")?;
        if grammar_type != 1 {
            return Err(AntlrError::Unsupported(format!(
                "serialized parser ATN has grammar type {grammar_type}; expected 1"
            )));
        }
        Ok(())
    }

    fn deserialize_parser_states(
        &mut self,
        builder: &mut ParserAtnBuilder,
    ) -> Result<(), AntlrError> {
        let state_count = self.read_usize("state count")?;
        let mut end_states = Vec::new();
        let mut loop_back_states = Vec::new();
        for _ in 0..state_count {
            let kind = decode_state_kind(self.read("state type")?)?;
            if kind == AtnStateKind::Invalid {
                builder
                    .add_state(kind, None)
                    .map_err(|error| parser_atn_error(&error))?;
                continue;
            }
            let raw_rule_index = self.read("rule index")?;
            let rule_index = (raw_rule_index >= 0)
                .then(|| read_index(raw_rule_index, "state rule index"))
                .transpose()?;
            let state = builder
                .add_state(kind, rule_index)
                .map_err(|error| parser_atn_error(&error))?
                .index();
            match kind {
                AtnStateKind::LoopEnd => {
                    let target = self.read_usize("loop back state")?;
                    loop_back_states.push((state, target));
                }
                AtnStateKind::BlockStart
                | AtnStateKind::PlusBlockStart
                | AtnStateKind::StarBlockStart => {
                    let target = self.read_usize("block end state")?;
                    end_states.push((state, target));
                }
                _ => {}
            }
        }
        for (state, target) in end_states {
            builder
                .set_end_state(state, target)
                .map_err(|error| parser_atn_error(&error))?;
        }
        for (state, target) in loop_back_states {
            builder
                .set_loop_back_state(state, target)
                .map_err(|error| parser_atn_error(&error))?;
        }
        Ok(())
    }

    fn deserialize_parser_flags(
        &mut self,
        builder: &mut ParserAtnBuilder,
    ) -> Result<(), AntlrError> {
        let non_greedy_count = self.read_usize("non-greedy state count")?;
        for _ in 0..non_greedy_count {
            let state = self.read_usize("non-greedy state")?;
            builder
                .set_non_greedy(state)
                .map_err(|error| parser_atn_error(&error))?;
        }
        let precedence_count = self.read_usize("precedence state count")?;
        for _ in 0..precedence_count {
            let state = self.read_usize("precedence state")?;
            builder
                .set_left_recursive_rule(state)
                .map_err(|error| parser_atn_error(&error))?;
        }
        Ok(())
    }

    fn deserialize_parser_rules(
        &mut self,
        builder: &mut ParserAtnBuilder,
    ) -> Result<(), AntlrError> {
        let rule_count = self.read_usize("rule count")?;
        let mut starts = Vec::with_capacity(rule_count);
        for _ in 0..rule_count {
            starts.push(self.read_usize("rule start state")?);
        }
        let mut stops = vec![None; rule_count];
        for state in 0..builder.state_count() {
            let kind = builder
                .state_kind(state)
                .expect("state count bounds builder state lookup");
            if kind != AtnStateKind::RuleStop {
                continue;
            }
            let Some(rule_index) = builder.state_rule_index(state) else {
                continue;
            };
            if let Some(stop) = stops.get_mut(rule_index) {
                *stop = Some(state);
            }
        }
        let stops = stops
            .into_iter()
            .enumerate()
            .map(|(rule, stop)| {
                stop.ok_or_else(|| {
                    AntlrError::Unsupported(format!(
                        "serialized parser ATN has no stop state for rule {rule}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        builder
            .set_rule_to_start_state(starts)
            .map_err(|error| parser_atn_error(&error))?;
        builder
            .set_rule_to_stop_state(stops)
            .map_err(|error| parser_atn_error(&error))?;
        Ok(())
    }

    fn deserialize_parser_modes(&mut self, builder: &ParserAtnBuilder) -> Result<(), AntlrError> {
        let mode_count = self.read_usize("mode count")?;
        for _ in 0..mode_count {
            let state = self.read_usize("mode start state")?;
            if builder.state_kind(state).is_none() {
                return Err(AntlrError::Unsupported(format!(
                    "mode start state {state} outside state list"
                )));
            }
        }
        Ok(())
    }

    fn deserialize_parser_sets(
        &mut self,
        builder: &mut ParserAtnBuilder,
    ) -> Result<Vec<ParserIntervalSetId>, AntlrError> {
        let set_count = self.read_usize("set count")?;
        let mut sets = Vec::with_capacity(set_count);
        for _ in 0..set_count {
            let interval_count = self.read_usize("interval count")?;
            let contains_eof = self.read("set contains EOF")? != 0;
            let mut ranges = Vec::with_capacity(interval_count + usize::from(contains_eof));
            if contains_eof {
                ranges.push((TOKEN_EOF, TOKEN_EOF));
            }
            for _ in 0..interval_count {
                ranges.push((self.read("interval start")?, self.read("interval stop")?));
            }
            sets.push(
                builder
                    .add_interval_set(ranges)
                    .map_err(|error| parser_atn_error(&error))?,
            );
        }
        Ok(sets)
    }

    fn deserialize_parser_edges(
        &mut self,
        builder: &mut ParserAtnBuilder,
        sets: &[ParserIntervalSetId],
    ) -> Result<(), AntlrError> {
        let transition_count = self.read_usize("transition count")?;
        for _ in 0..transition_count {
            let source = self.read_usize("transition source")?;
            let target = self.read_usize("transition target")?;
            let kind = self.read("transition type")?;
            let a = self.read("transition arg 1")?;
            let b = self.read("transition arg 2")?;
            let c = self.read("transition arg 3")?;
            let transition = decode_parser_transition(target, kind, a, b, c, sets)?;
            builder
                .add_transition(source, transition)
                .map_err(|error| parser_atn_error(&error))?;
        }
        add_parser_rule_return_edges(builder)
    }

    fn deserialize_parser_decisions(
        &mut self,
        builder: &mut ParserAtnBuilder,
    ) -> Result<(), AntlrError> {
        let decision_count = self.read_usize("decision count")?;
        for _ in 0..decision_count {
            let state = self.read_usize("decision state")?;
            builder
                .add_decision_state(state)
                .map_err(|error| parser_atn_error(&error))?;
        }
        Ok(())
    }

    /// Reads all serialized ATN states and preserves state-specific paired
    /// links such as block end states and loop-back states.
    fn deserialize_states(&mut self, atn: &mut LexerAtn) -> Result<(), AntlrError> {
        let state_count = self.read_usize("state count")?;
        for state_number in 0..state_count {
            let kind = decode_state_kind(self.read("state type")?)?;
            if kind == AtnStateKind::Invalid {
                atn.add_state(LexerAtnState::new(state_number, kind));
                continue;
            }

            let rule_index = self.read("rule index")?;
            let mut state = LexerAtnState::new(state_number, kind);
            if rule_index >= 0 {
                let rule_index = usize::try_from(rule_index).map_err(|_| {
                    AntlrError::Unsupported(format!("rule index cannot be negative: {rule_index}"))
                })?;
                state = state.with_rule_index(rule_index);
            }

            match kind {
                AtnStateKind::LoopEnd => {
                    state.loop_back_state = Some(self.read_usize("loop back state")?);
                }
                AtnStateKind::BlockStart
                | AtnStateKind::PlusBlockStart
                | AtnStateKind::StarBlockStart => {
                    state.end_state = Some(self.read_usize("block end state")?);
                }
                _ => {}
            }

            atn.add_state(state);
        }
        Ok(())
    }

    /// Marks lexer and parser decision states that ANTLR encoded as
    /// non-greedy.
    fn deserialize_non_greedy_states(&mut self, atn: &mut LexerAtn) -> Result<(), AntlrError> {
        let count = self.read_usize("non-greedy state count")?;
        for _ in 0..count {
            let state_number = self.read_usize("non-greedy state")?;
            let Some(state) = atn.state_mut(state_number) else {
                return Err(AntlrError::Unsupported(format!(
                    "non-greedy state {state_number} outside state list"
                )));
            };
            state.non_greedy = true;
        }
        Ok(())
    }

    /// Marks rule-start states that ANTLR generated for left-recursive
    /// precedence rules.
    fn deserialize_precedence_states(&mut self, atn: &mut LexerAtn) -> Result<(), AntlrError> {
        let count = self.read_usize("precedence state count")?;
        for _ in 0..count {
            let state_number = self.read_usize("precedence state")?;
            let Some(state) = atn.state_mut(state_number) else {
                return Err(AntlrError::Unsupported(format!(
                    "precedence state {state_number} outside state list"
                )));
            };
            state.left_recursive_rule = true;
        }
        Ok(())
    }

    /// Decodes rule start states, lexer token types, and derived rule stop
    /// states.
    fn deserialize_rules(&mut self, atn: &mut LexerAtn) -> Result<(), AntlrError> {
        let rule_count = self.read_usize("rule count")?;
        let mut starts = Vec::with_capacity(rule_count);
        let mut token_types = Vec::new();
        for _ in 0..rule_count {
            starts.push(self.read_usize("rule start state")?);
            token_types.push(self.read("rule token type")?);
        }

        let mut stops = vec![usize::MAX; rule_count];
        for state in atn.states() {
            if state.kind == AtnStateKind::RuleStop {
                let Some(rule_index) = state.rule_index else {
                    continue;
                };
                if let Some(stop) = stops.get_mut(rule_index) {
                    *stop = state.state_number;
                }
            }
        }

        atn.set_rule_to_start_state(starts);
        atn.set_rule_to_stop_state(stops);
        atn.set_rule_to_token_type(token_types);
        Ok(())
    }

    /// Decodes lexer mode entry states.
    fn deserialize_modes(&mut self, atn: &mut LexerAtn) -> Result<(), AntlrError> {
        let mode_count = self.read_usize("mode count")?;
        for _ in 0..mode_count {
            atn.add_mode_start_state(self.read_usize("mode start state")?);
        }
        Ok(())
    }

    /// Decodes all interval sets referenced by `SET` and `NOT_SET`
    /// transitions.
    fn deserialize_sets(&mut self) -> Result<Vec<IntervalSet>, AntlrError> {
        let set_count = self.read_usize("set count")?;
        let mut sets = Vec::with_capacity(set_count);
        for _ in 0..set_count {
            let interval_count = self.read_usize("interval count")?;
            let mut set = IntervalSet::new();
            let contains_eof = self.read("set contains EOF")? != 0;
            if contains_eof {
                set.add(TOKEN_EOF);
            }
            for _ in 0..interval_count {
                let start = self.read("interval start")?;
                let stop = self.read("interval stop")?;
                set.add_range(start, stop);
            }
            sets.push(set);
        }
        Ok(sets)
    }

    /// Decodes serialized edges and appends derived rule-return epsilon edges.
    fn deserialize_edges(
        &mut self,
        atn: &mut LexerAtn,
        sets: &[IntervalSet],
    ) -> Result<(), AntlrError> {
        let transition_count = self.read_usize("transition count")?;
        for _ in 0..transition_count {
            let src = self.read_usize("transition source")?;
            let target = self.read_usize("transition target")?;
            let kind = self.read("transition type")?;
            let a = self.read("transition arg 1")?;
            let b = self.read("transition arg 2")?;
            let c = self.read("transition arg 3")?;
            let transition = decode_transition(target, kind, a, b, c, sets)?;
            let Some(state) = atn.state_mut(src) else {
                return Err(AntlrError::Unsupported(format!(
                    "transition source {src} outside state list"
                )));
            };
            state.add_transition(transition);
        }

        let mut return_edges = Vec::new();
        for state in atn.states() {
            for transition in &state.transitions {
                let LexerTransition::Rule {
                    target,
                    follow_state,
                    ..
                } = transition
                else {
                    continue;
                };
                let Some(rule_index) = atn.state(*target).and_then(|state| state.rule_index) else {
                    continue;
                };
                let Some(stop_state) = atn.rule_to_stop_state().get(rule_index).copied() else {
                    continue;
                };
                if stop_state != usize::MAX {
                    return_edges.push((stop_state, *follow_state));
                }
            }
        }
        for (stop_state, follow_state) in return_edges {
            if let Some(state) = atn.state_mut(stop_state) {
                state.add_transition(LexerTransition::Epsilon {
                    target: follow_state,
                });
            }
        }

        Ok(())
    }

    /// Decodes parser/lexer decision entry states in decision-number order.
    fn deserialize_decisions(&mut self, atn: &mut LexerAtn) -> Result<(), AntlrError> {
        let decision_count = self.read_usize("decision count")?;
        for _ in 0..decision_count {
            atn.add_decision_state(self.read_usize("decision state")?);
        }
        Ok(())
    }

    /// Decodes grammar-independent lexer actions referenced by action
    /// transitions.
    fn deserialize_lexer_actions(&mut self, atn: &mut LexerAtn) -> Result<(), AntlrError> {
        let action_count = self.read_usize("lexer action count")?;
        let mut actions = Vec::with_capacity(action_count);
        for _ in 0..action_count {
            let action_type = self.read("lexer action type")?;
            let data1 = self.read("lexer action data 1")?;
            let data2 = self.read("lexer action data 2")?;
            actions.push(decode_lexer_action(action_type, data1, data2)?);
        }
        atn.set_lexer_actions(actions);
        Ok(())
    }

    /// Reads the next integer and reports which logical field was expected if
    /// the data ends early.
    fn read(&mut self, label: &str) -> Result<i32, AntlrError> {
        let value = self.values.get(self.cursor).copied().ok_or_else(|| {
            AntlrError::Unsupported(format!("serialized ATN ended while reading {label}"))
        })?;
        self.cursor += 1;
        Ok(value)
    }

    /// Reads the next integer as a non-negative state/table count or index.
    fn read_usize(&mut self, label: &str) -> Result<usize, AntlrError> {
        let value = self.read(label)?;
        usize::try_from(value)
            .map_err(|_| AntlrError::Unsupported(format!("{label} cannot be negative: {value}")))
    }
}

/// Converts ANTLR's serialized state integer into the runtime state enum.
fn decode_state_kind(value: i32) -> Result<AtnStateKind, AntlrError> {
    let kind = match value {
        0 => AtnStateKind::Invalid,
        1 => AtnStateKind::Basic,
        2 => AtnStateKind::RuleStart,
        3 => AtnStateKind::BlockStart,
        4 => AtnStateKind::PlusBlockStart,
        5 => AtnStateKind::StarBlockStart,
        6 => AtnStateKind::TokenStart,
        7 => AtnStateKind::RuleStop,
        8 => AtnStateKind::BlockEnd,
        9 => AtnStateKind::StarLoopBack,
        10 => AtnStateKind::StarLoopEntry,
        11 => AtnStateKind::PlusLoopBack,
        12 => AtnStateKind::LoopEnd,
        other => return Err(AntlrError::Unsupported(format!("ATN state type {other}"))),
    };
    Ok(kind)
}

/// Converts one serialized edge record into a typed transition.
fn decode_transition(
    target: usize,
    kind: i32,
    a: i32,
    b: i32,
    c: i32,
    sets: &[IntervalSet],
) -> Result<LexerTransition, AntlrError> {
    let transition = match kind {
        1 => LexerTransition::Epsilon { target },
        2 => LexerTransition::Range {
            target,
            start: if c != 0 { TOKEN_EOF } else { a },
            stop: b,
        },
        3 => LexerTransition::Rule {
            target: read_index(a, "rule transition target")?,
            rule_index: read_index(b, "rule transition rule index")?,
            follow_state: target,
            precedence: c,
        },
        4 => LexerTransition::Predicate {
            target,
            rule_index: read_index(a, "predicate rule index")?,
            pred_index: read_index(b, "predicate index")?,
            context_dependent: c != 0,
        },
        5 => LexerTransition::Atom {
            target,
            label: if c != 0 { TOKEN_EOF } else { a },
        },
        6 => LexerTransition::Action {
            target,
            rule_index: read_index(a, "action rule index")?,
            action_index: usize::try_from(b).ok(),
            context_dependent: c != 0,
        },
        7 => LexerTransition::Set {
            target,
            set: sets
                .get(read_index(a, "set transition set index")?)
                .cloned()
                .ok_or_else(|| {
                    AntlrError::Unsupported(format!("set index {a} outside set list"))
                })?,
        },
        8 => LexerTransition::NotSet {
            target,
            set: sets
                .get(read_index(a, "not-set transition set index")?)
                .cloned()
                .ok_or_else(|| {
                    AntlrError::Unsupported(format!("set index {a} outside set list"))
                })?,
        },
        9 => LexerTransition::Wildcard { target },
        10 => LexerTransition::Precedence {
            target,
            precedence: a,
        },
        other => {
            return Err(AntlrError::Unsupported(format!(
                "ATN transition type {other}"
            )));
        }
    };
    Ok(transition)
}

/// Converts one serialized parser edge directly into a packed-builder record.
fn decode_parser_transition(
    target: usize,
    kind: i32,
    a: i32,
    b: i32,
    c: i32,
    sets: &[ParserIntervalSetId],
) -> Result<ParserTransitionSpec, AntlrError> {
    let transition = match kind {
        1 => ParserTransitionSpec::Epsilon { target },
        2 => ParserTransitionSpec::Range {
            target,
            start: if c != 0 { TOKEN_EOF } else { a },
            stop: b,
        },
        3 => ParserTransitionSpec::Rule {
            target: read_index(a, "rule transition target")?,
            rule_index: read_index(b, "rule transition rule index")?,
            follow_state: target,
            precedence: c,
        },
        4 => ParserTransitionSpec::Predicate {
            target,
            rule_index: read_index(a, "predicate rule index")?,
            pred_index: read_index(b, "predicate index")?,
            context_dependent: c != 0,
        },
        5 => ParserTransitionSpec::Atom {
            target,
            label: if c != 0 { TOKEN_EOF } else { a },
        },
        6 => ParserTransitionSpec::Action {
            target,
            rule_index: read_index(a, "action rule index")?,
            action_index: usize::try_from(b).ok(),
            context_dependent: c != 0,
        },
        7 => ParserTransitionSpec::Set {
            target,
            set: parser_set_id(sets, a, "set transition")?,
        },
        8 => ParserTransitionSpec::NotSet {
            target,
            set: parser_set_id(sets, a, "not-set transition")?,
        },
        9 => ParserTransitionSpec::Wildcard { target },
        10 => ParserTransitionSpec::Precedence {
            target,
            precedence: a,
        },
        other => {
            return Err(AntlrError::Unsupported(format!(
                "ATN transition type {other}"
            )));
        }
    };
    Ok(transition)
}

fn parser_set_id(
    sets: &[ParserIntervalSetId],
    value: i32,
    label: &str,
) -> Result<ParserIntervalSetId, AntlrError> {
    let index = read_index(value, label)?;
    sets.get(index).copied().ok_or_else(|| {
        AntlrError::Unsupported(format!("{label} set index {value} outside set list"))
    })
}

/// Adds ANTLR's derived epsilon returns without constructing per-state edge
/// vectors. The builder keeps the original edge order for each stop state and
/// appends these derived returns after serialized edges.
fn add_parser_rule_return_edges(builder: &mut ParserAtnBuilder) -> Result<(), AntlrError> {
    let mut return_edges = Vec::new();
    for source in 0..builder.state_count() {
        for transition in builder.transitions_from(source) {
            let ParserTransitionSpec::Rule {
                target,
                follow_state,
                ..
            } = transition
            else {
                continue;
            };
            let Some(rule_index) = builder.state_rule_index(target) else {
                continue;
            };
            let Some(stop_state) = builder.rule_stop_state(rule_index) else {
                continue;
            };
            return_edges.push((stop_state, follow_state));
        }
    }
    for (stop_state, follow_state) in return_edges {
        builder
            .add_transition(
                stop_state,
                ParserTransitionSpec::Epsilon {
                    target: follow_state,
                },
            )
            .map_err(|error| parser_atn_error(&error))?;
    }
    Ok(())
}

fn parser_atn_error(error: &ParserAtnError) -> AntlrError {
    AntlrError::Unsupported(error.to_string())
}

/// Converts ANTLR's serialized lexer action ordinal and data operands into a
/// runtime action.
fn decode_lexer_action(
    action_type: i32,
    data1: i32,
    data2: i32,
) -> Result<LexerAction, AntlrError> {
    let action = match action_type {
        0 => LexerAction::Channel(data1),
        1 => LexerAction::Custom {
            rule_index: data1,
            action_index: data2,
        },
        2 => LexerAction::Mode(data1),
        3 => LexerAction::More,
        4 => LexerAction::PopMode,
        5 => LexerAction::PushMode(data1),
        6 => LexerAction::Skip,
        7 => LexerAction::Type(data1),
        other => {
            return Err(AntlrError::Unsupported(format!(
                "lexer action type {other}"
            )));
        }
    };
    Ok(action)
}

/// Marks star-loop entry states that are parser precedence decisions.
fn mark_precedence_decisions(atn: &mut LexerAtn) {
    let mut decisions = Vec::new();
    for state in atn.states() {
        if state.kind != AtnStateKind::StarLoopEntry {
            continue;
        }
        let Some(rule_index) = state.rule_index else {
            continue;
        };
        let Some(rule_start) = atn
            .rule_to_start_state()
            .get(rule_index)
            .and_then(|state_number| atn.state(*state_number))
        else {
            continue;
        };
        if !rule_start.left_recursive_rule {
            continue;
        }
        let Some(loop_end_state) = state
            .transitions
            .last()
            .and_then(|transition| atn.state(transition.target()))
        else {
            continue;
        };
        if loop_end_state.kind != AtnStateKind::LoopEnd {
            continue;
        }
        let Some(target) = loop_end_state
            .transitions
            .first()
            .and_then(|transition| atn.state(transition.target()))
        else {
            continue;
        };
        if target.kind == AtnStateKind::RuleStop {
            decisions.push(state.state_number);
        }
    }

    for state_number in decisions {
        if let Some(state) = atn.state_mut(state_number) {
            state.precedence_rule_decision = true;
        }
    }
}

/// Converts a serialized integer operand to an index with a field-specific
/// error.
fn read_index(value: i32, label: &str) -> Result<usize, AntlrError> {
    usize::try_from(value)
        .map_err(|_| AntlrError::Unsupported(format!("{label} cannot be negative: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_small_parser_atn() {
        let serialized = SerializedAtn::from_i32(&[
            4, 1, 9, // header: version, parser, max token type
            2, // states
            2, 0, // rule start
            7, 0, // rule stop
            0, // non-greedy states
            0, // precedence states
            1, // rules
            0, // rule 0 start
            0, // modes
            0, // sets
            1, // transitions
            0, 1, 5, 42, 0, 0, // atom to state 1 with label 42
            1, // decisions
            0,
        ]);
        let atn = AtnDeserializer::new(&serialized)
            .deserialize_parser()
            .expect("artificial parser ATN should deserialize");
        assert_eq!(atn.max_token_type(), 9);
        assert_eq!(atn.states().len(), 2);
        assert_eq!(atn.rule_to_start_state().iter().collect::<Vec<_>>(), [0]);
        assert_eq!(atn.rule_to_stop_state().iter().collect::<Vec<_>>(), [1]);
        assert_eq!(atn.decision_to_state().iter().collect::<Vec<_>>(), [0]);
        assert_eq!(atn.stats().transitions, 1);
    }

    #[test]
    fn graph_deserializer_rejects_parser_input() {
        let serialized = SerializedAtn::from_i32(&[4, 1, 0]);
        let error = AtnDeserializer::new(&serialized)
            .deserialize()
            .expect_err("parser input must not create a lexer graph");
        assert!(
            error
                .to_string()
                .contains("AtnDeserializer::deserialize_parser()")
        );
    }
}
