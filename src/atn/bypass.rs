//! Rule-bypass alternatives for parse-tree pattern matching.
//!
//! ANTLR's parse-tree pattern matcher needs to interpret a hybrid token stream
//! in which an *imaginary* token can stand in for an entire parser rule (the
//! `<expr>` tag in a pattern like `x = <expr>;`). Upstream implements this by
//! re-deserializing the grammar ATN with `generateRuleBypassTransitions`
//! enabled (`ATNDeserializer`): each rule gains a **bypass alternative** so the
//! ordinary parser interpreter can match one imaginary token as if it were the
//! whole rule. See `ATNDeserializer.java` (the `isGenerateRuleBypassTransitions`
//! block) for the reference algorithm this mirrors.
//!
//! This runtime stores parser ATNs as a packed, immutable word stream rather
//! than a mutable object graph, so the transform reads the source ATN through
//! its borrowing views into an out-adjacency model, applies the rewrite there,
//! and emits a fresh [`ParserAtn`] through [`ParserAtnBuilder`]. The existing
//! ATN interpreter then runs over the result unchanged — no hot-path edits.
//!
//! ### Why `max_token_type` is left unchanged
//!
//! Upstream assigns each rule the imaginary token type `maxTokenType + i + 1`
//! but never raises `maxTokenType` itself. We keep the same invariant on
//! purpose: an [`Atom`](ParserTransitionSpec::Atom) transition matches by exact
//! label equality (no range check), so the bypass edge matches its imaginary
//! type fine, while grammar wildcards and `~x` negated sets — which the runtime
//! bounds by `min..=max_token_type` — can never accidentally match an imaginary
//! token that lives *above* the unchanged maximum.

use super::AtnStateKind;
use super::parser_atn::{
    ParserAtn, ParserAtnBuilder, ParserAtnError, ParserIntervalSetId, ParserTransitionData,
    ParserTransitionSpec,
};

impl ParserAtn {
    /// Builds a copy of this parser ATN with rule-bypass alternatives added.
    ///
    /// Every rule gains an imaginary token type (`max_token_type + rule + 1`)
    /// and a bypass block so the ATN interpreter can match that single
    /// imaginary token in place of the whole rule. `max_token_type` is
    /// unchanged (see the module docs). The returned ATN is otherwise a faithful
    /// copy: state kinds, transitions, interval sets, decisions, and
    /// rule/precedence metadata are all preserved.
    ///
    /// # Errors
    ///
    /// Returns [`ParserAtnError`] if the state/transition/token counts overflow
    /// the packed compact-index range, if a left-recursive rule's precedence
    /// prefix cannot be identified, or if the re-emitted stream fails
    /// validation.
    pub fn with_bypass_alternatives(&self) -> Result<Self, ParserAtnError> {
        BypassBuilder::new(self)?.build()
    }
}

/// Mutable working copy of a parser ATN used to apply the bypass rewrite.
struct BypassBuilder {
    max_token_type: i32,
    /// Per-state kind, parallel to state number.
    kinds: Vec<AtnStateKind>,
    /// Per-state grammar rule index (`None` for rule-agnostic states).
    rule_indices: Vec<Option<usize>>,
    /// Block-start end state, if any.
    end_states: Vec<Option<usize>>,
    /// Loop-end loop-back state, if any.
    loop_back_states: Vec<Option<usize>>,
    /// Non-greedy decision flag, preserved verbatim.
    non_greedy: Vec<bool>,
    /// Left-recursive rule flag, preserved on rule-start states.
    left_recursive: Vec<bool>,
    /// Out-adjacency: `out[source]` holds that state's transitions in order.
    out: Vec<Vec<ParserTransitionSpec>>,
    /// Interval sets copied verbatim; identities stay valid because order is
    /// preserved.
    interval_sets: Vec<Vec<(i32, i32)>>,
    decisions: Vec<usize>,
    rule_starts: Vec<usize>,
    rule_stops: Vec<usize>,
}

impl BypassBuilder {
    /// Reads the packed source ATN into the mutable model.
    fn new(atn: &ParserAtn) -> Result<Self, ParserAtnError> {
        let state_count = atn.state_count();
        let mut kinds = Vec::with_capacity(state_count);
        let mut rule_indices = Vec::with_capacity(state_count);
        let mut end_states = Vec::with_capacity(state_count);
        let mut loop_back_states = Vec::with_capacity(state_count);
        let mut non_greedy = Vec::with_capacity(state_count);
        let mut left_recursive = Vec::with_capacity(state_count);
        let mut out = Vec::with_capacity(state_count);

        for number in 0..state_count {
            let state = atn
                .state(number)
                .expect("state index below state_count is in bounds");
            kinds.push(state.kind());
            rule_indices.push(state.rule_index());
            end_states.push(state.end_state());
            loop_back_states.push(state.loop_back_state());
            non_greedy.push(state.non_greedy());
            left_recursive.push(state.left_recursive_rule());
            out.push(
                state
                    .transitions()
                    .iter()
                    .map(|transition| data_to_spec(transition.data()))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }

        let interval_sets = (0..atn.set_count())
            .map(|index| {
                atn.token_set(index)
                    .expect("set index below set_count is in bounds")
                    .ranges()
                    .collect::<Vec<_>>()
            })
            .collect();

        let decisions = atn.decision_to_state().into_iter().collect();
        let rule_starts = atn.rule_to_start_state().into_iter().collect();
        let rule_stops = atn.rule_to_stop_state().into_iter().collect();

        Ok(Self {
            max_token_type: atn.max_token_type(),
            kinds,
            rule_indices,
            end_states,
            loop_back_states,
            non_greedy,
            left_recursive,
            out,
            interval_sets,
            decisions,
            rule_starts,
            rule_stops,
        })
    }

    /// Applies the rewrite and emits the packed result.
    fn build(mut self) -> Result<ParserAtn, ParserAtnError> {
        let rule_count = self.rule_starts.len();
        let original_state_count = self.kinds.len();

        // Reserve the three new states per rule up front so their numbers are
        // known before any edge is rewired. Layout: for rule `i`,
        // `bypass_start = base`, `bypass_stop = base + 1`, `match = base + 2`.
        let new_state_base = original_state_count;
        for rule in 0..rule_count {
            let bypass_start = new_state_base + rule * 3;
            let bypass_stop = bypass_start + 1;
            let match_state = bypass_start + 2;
            // Bypass start is a decision block start whose end is the stop.
            self.push_state(
                AtnStateKind::BlockStart,
                Some(rule),
                Some(bypass_stop),
                None,
            );
            self.push_state(AtnStateKind::BlockEnd, Some(rule), None, None);
            self.push_state(AtnStateKind::Basic, None, None, None);
            debug_assert_eq!(self.out.len(), match_state + 1);
        }

        // Retarget/move plan per rule, computed against the *pre-move* graph so
        // the left-recursive exclude transition is identified correctly.
        let mut retarget: Vec<(usize, usize)> = Vec::with_capacity(rule_count);
        let mut excluded: Vec<(usize, usize)> = Vec::new();
        for rule in 0..rule_count {
            let bypass_stop = new_state_base + rule * 3 + 1;
            let (end_state, exclude) = self.rule_end_state(rule)?;
            retarget.push((end_state, bypass_stop));
            if let Some(exclude) = exclude {
                excluded.push(exclude);
            }
        }

        // Move each rule-start's transitions onto its bypass-start block.
        for rule in 0..rule_count {
            let rule_start = self.rule_starts[rule];
            let bypass_start = new_state_base + rule * 3;
            let moved = std::mem::take(&mut self.out[rule_start]);
            self.out[bypass_start] = moved;
        }

        // Retarget every edge that targeted a rule's end state onto that rule's
        // bypass stop, skipping the left-recursive exclude edge(s).
        for (source, transitions) in self.out.iter_mut().enumerate() {
            for (index, spec) in transitions.iter_mut().enumerate() {
                if excluded.contains(&(source, index)) {
                    continue;
                }
                if let Some(&(_, bypass_stop)) =
                    retarget.iter().find(|(end, _)| *end == spec.target())
                {
                    *spec = with_target(*spec, bypass_stop);
                }
            }
        }

        // Add the bypass edges last so the `bypass_stop -> end_state` link
        // (which targets an end state) is never caught by the retarget pass.
        for rule in 0..rule_count {
            let rule_start = self.rule_starts[rule];
            let bypass_start = new_state_base + rule * 3;
            let bypass_stop = bypass_start + 1;
            let match_state = bypass_start + 2;
            let (end_state, _) = self.rule_end_state(rule)?;
            let imaginary = self.imaginary_token_type(rule)?;

            self.out[rule_start].push(ParserTransitionSpec::Epsilon {
                target: bypass_start,
            });
            self.out[bypass_start].push(ParserTransitionSpec::Epsilon {
                target: match_state,
            });
            self.out[bypass_stop].push(ParserTransitionSpec::Epsilon { target: end_state });
            self.out[match_state].push(ParserTransitionSpec::Atom {
                target: bypass_stop,
                label: imaginary,
            });
            self.decisions.push(bypass_start);
        }

        self.emit()
    }

    /// Appends one new state to the model, keeping every parallel array aligned.
    fn push_state(
        &mut self,
        kind: AtnStateKind,
        rule_index: Option<usize>,
        end_state: Option<usize>,
        loop_back_state: Option<usize>,
    ) {
        self.kinds.push(kind);
        self.rule_indices.push(rule_index);
        self.end_states.push(end_state);
        self.loop_back_states.push(loop_back_state);
        self.non_greedy.push(false);
        self.left_recursive.push(false);
        self.out.push(Vec::new());
    }

    /// Returns `(end_state, exclude_edge)` for a rule.
    ///
    /// For an ordinary rule the end state is the rule stop and there is no
    /// excluded edge. For a left-recursive rule the end state is the
    /// `StarLoopEntry` that begins the precedence-climbing loop, and the excluded
    /// edge is the loop-back edge into it (which must keep pointing at the entry
    /// rather than being diverted to the bypass block). Mirrors the
    /// `isLeftRecursiveRule` branch in `ATNDeserializer`.
    fn rule_end_state(
        &self,
        rule: usize,
    ) -> Result<(usize, Option<(usize, usize)>), ParserAtnError> {
        let rule_start = self.rule_starts[rule];
        if !self.left_recursive[rule_start] {
            return Ok((self.rule_stops[rule], None));
        }

        // Find the StarLoopEntry whose last edge reaches a LoopEnd that
        // epsilon-transitions to the rule stop: that entry is the precedence
        // prefix boundary ANTLR wraps.
        for state in 0..self.kinds.len() {
            if self.rule_indices[state] != Some(rule)
                || self.kinds[state] != AtnStateKind::StarLoopEntry
            {
                continue;
            }
            let Some(last) = self.out[state].last() else {
                continue;
            };
            let loop_end = last.target();
            if self.kinds.get(loop_end).copied() != Some(AtnStateKind::LoopEnd) {
                continue;
            }
            let reaches_stop = self.out[loop_end]
                .first()
                .is_some_and(|edge| self.kinds.get(edge.target()) == Some(&AtnStateKind::RuleStop));
            if !reaches_stop {
                continue;
            }
            let Some(loop_back) = self.loop_back_states[loop_end] else {
                continue;
            };
            // The loop-back state's first edge is the excluded loop-back into
            // the entry; verify the structure before trusting it.
            if self.out[loop_back]
                .first()
                .is_some_and(|edge| edge.target() == state)
            {
                return Ok((state, Some((loop_back, 0))));
            }
        }

        Err(ParserAtnError::InvalidData(format!(
            "could not identify precedence prefix boundary for left-recursive rule {rule}"
        )))
    }

    /// Imaginary token type reserved for a rule's bypass alternative.
    fn imaginary_token_type(&self, rule: usize) -> Result<i32, ParserAtnError> {
        let overflow = || ParserAtnError::Overflow {
            field: "bypass imaginary token type",
            value: rule,
        };
        let rule = i32::try_from(rule).map_err(|_| overflow())?;
        self.max_token_type
            .checked_add(rule)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(overflow)
    }

    /// Emits the mutable model as a validated packed [`ParserAtn`].
    fn emit(self) -> Result<ParserAtn, ParserAtnError> {
        let mut builder = ParserAtnBuilder::new(self.max_token_type);

        for (index, &kind) in self.kinds.iter().enumerate() {
            builder.add_state(kind, self.rule_indices[index])?;
        }
        for (index, end_state) in self.end_states.iter().enumerate() {
            if let Some(end_state) = end_state {
                builder.set_end_state(index, *end_state)?;
            }
        }
        for (index, loop_back) in self.loop_back_states.iter().enumerate() {
            if let Some(loop_back) = loop_back {
                builder.set_loop_back_state(index, *loop_back)?;
            }
        }
        for (index, &flag) in self.non_greedy.iter().enumerate() {
            if flag {
                builder.set_non_greedy(index)?;
            }
        }
        for (index, &flag) in self.left_recursive.iter().enumerate() {
            if flag {
                builder.set_left_recursive_rule(index)?;
            }
        }
        for ranges in &self.interval_sets {
            builder.add_interval_set(ranges.iter().copied())?;
        }
        for (source, transitions) in self.out.iter().enumerate() {
            for spec in transitions {
                builder.add_transition(source, *spec)?;
            }
        }
        for &state in &self.decisions {
            builder.add_decision_state(state)?;
        }
        builder.set_rule_to_start_state(self.rule_starts)?;
        builder.set_rule_to_stop_state(self.rule_stops)?;

        builder.finish()
    }
}

/// Converts a borrowing transition view into a builder spec, translating the
/// borrowed interval set back into its stable identity.
fn data_to_spec(data: ParserTransitionData<'_>) -> Result<ParserTransitionSpec, ParserAtnError> {
    Ok(match data {
        ParserTransitionData::Epsilon { target } => ParserTransitionSpec::Epsilon { target },
        ParserTransitionData::Atom { target, label } => {
            ParserTransitionSpec::Atom { target, label }
        }
        ParserTransitionData::Range {
            target,
            start,
            stop,
        } => ParserTransitionSpec::Range {
            target,
            start,
            stop,
        },
        ParserTransitionData::Set { target, set } => ParserTransitionSpec::Set {
            target,
            set: ParserIntervalSetId::try_from(set.index())?,
        },
        ParserTransitionData::NotSet { target, set } => ParserTransitionSpec::NotSet {
            target,
            set: ParserIntervalSetId::try_from(set.index())?,
        },
        ParserTransitionData::Wildcard { target } => ParserTransitionSpec::Wildcard { target },
        ParserTransitionData::Rule {
            target,
            rule_index,
            follow_state,
            precedence,
        } => ParserTransitionSpec::Rule {
            target,
            rule_index,
            follow_state,
            precedence,
        },
        ParserTransitionData::Predicate {
            target,
            rule_index,
            pred_index,
            context_dependent,
        } => ParserTransitionSpec::Predicate {
            target,
            rule_index,
            pred_index,
            context_dependent,
        },
        ParserTransitionData::Action {
            target,
            rule_index,
            action_index,
            context_dependent,
        } => ParserTransitionSpec::Action {
            target,
            rule_index,
            action_index,
            context_dependent,
        },
        ParserTransitionData::Precedence { target, precedence } => {
            ParserTransitionSpec::Precedence { target, precedence }
        }
    })
}

/// Returns `spec` with its target redirected, preserving every other field.
const fn with_target(spec: ParserTransitionSpec, target: usize) -> ParserTransitionSpec {
    match spec {
        ParserTransitionSpec::Epsilon { .. } => ParserTransitionSpec::Epsilon { target },
        ParserTransitionSpec::Atom { label, .. } => ParserTransitionSpec::Atom { target, label },
        ParserTransitionSpec::Range { start, stop, .. } => ParserTransitionSpec::Range {
            target,
            start,
            stop,
        },
        ParserTransitionSpec::Set { set, .. } => ParserTransitionSpec::Set { target, set },
        ParserTransitionSpec::NotSet { set, .. } => ParserTransitionSpec::NotSet { target, set },
        ParserTransitionSpec::Wildcard { .. } => ParserTransitionSpec::Wildcard { target },
        ParserTransitionSpec::Rule {
            rule_index,
            follow_state,
            precedence,
            ..
        } => ParserTransitionSpec::Rule {
            target,
            rule_index,
            follow_state,
            precedence,
        },
        ParserTransitionSpec::Predicate {
            rule_index,
            pred_index,
            context_dependent,
            ..
        } => ParserTransitionSpec::Predicate {
            target,
            rule_index,
            pred_index,
            context_dependent,
        },
        ParserTransitionSpec::Action {
            rule_index,
            action_index,
            context_dependent,
            ..
        } => ParserTransitionSpec::Action {
            target,
            rule_index,
            action_index,
            context_dependent,
        },
        ParserTransitionSpec::Precedence { precedence, .. } => {
            ParserTransitionSpec::Precedence { target, precedence }
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::atn::parser_atn::ParserTransitionKind;
    use crate::token::{Token, TokenId, TokenSink, TokenSource, TokenSpec, TokenStoreError};
    use crate::{BaseParser, CommonTokenStream, NodeKind, RecognizerData, Vocabulary};

    /// A token source over a fixed list of specs, ending in EOF — the runtime
    /// analog of ANTLR's `ListTokenSource` used to feed a hybrid pattern stream.
    #[derive(Debug)]
    struct ListSource {
        specs: Vec<TokenSpec>,
        index: usize,
    }

    impl TokenSource for ListSource {
        fn next_token(&mut self, sink: &mut TokenSink<'_>) -> Result<TokenId, TokenStoreError> {
            let spec = self
                .specs
                .get(self.index)
                .cloned()
                .unwrap_or_else(|| TokenSpec::eof(self.index, self.index, 1, self.index));
            self.index += 1;
            sink.push(spec)
        }

        fn line(&self) -> usize {
            1
        }

        fn column(&self) -> usize {
            self.index
        }

        fn source_name(&self) -> &'static str {
            "bypass-test"
        }
    }

    /// Two-rule grammar `a : X b ; b : Y ;` (tokens `X=1`, `Y=2`).
    ///
    /// Deliberately plain (no loops, no left recursion) so the bypass rewrite's
    /// state growth and edge rewiring are easy to reason about.
    fn two_rule_atn() -> ParserAtn {
        let mut atn = ParserAtnBuilder::new(2);
        for (number, kind, rule) in [
            (0, AtnStateKind::RuleStart, 0),
            (1, AtnStateKind::Basic, 0),
            (2, AtnStateKind::Basic, 0),
            (3, AtnStateKind::RuleStop, 0),
            (4, AtnStateKind::RuleStart, 1),
            (5, AtnStateKind::Basic, 1),
            (6, AtnStateKind::RuleStop, 1),
        ] {
            assert_eq!(
                atn.add_state(kind, Some(rule)).expect("state").index(),
                number
            );
        }
        atn.set_rule_to_start_state(vec![0, 4]).expect("starts");
        atn.set_rule_to_stop_state(vec![3, 6]).expect("stops");
        // a : X b ;
        atn.add_transition(
            0,
            ParserTransitionSpec::Atom {
                target: 1,
                label: 1,
            },
        )
        .expect("edge");
        atn.add_transition(
            1,
            ParserTransitionSpec::Rule {
                target: 4,
                rule_index: 1,
                follow_state: 2,
                precedence: 0,
            },
        )
        .expect("edge");
        atn.add_transition(2, ParserTransitionSpec::Epsilon { target: 3 })
            .expect("edge");
        // Synthetic rule-return edge already present in a packed ATN.
        atn.add_transition(6, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("edge");
        // b : Y ;
        atn.add_transition(
            4,
            ParserTransitionSpec::Atom {
                target: 5,
                label: 2,
            },
        )
        .expect("edge");
        atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 6 })
            .expect("edge");
        atn.finish().expect("valid base ATN")
    }

    #[test]
    fn bypass_grows_three_states_per_rule_and_keeps_max_token_type() {
        let base = two_rule_atn();
        let bypass = base.with_bypass_alternatives().expect("bypass ATN");

        // Three new states per rule, appended after the originals.
        assert_eq!(
            bypass.state_count(),
            base.state_count() + 3 * base.rule_count()
        );
        // Imaginary token types live above the *unchanged* maximum.
        assert_eq!(bypass.max_token_type(), base.max_token_type());
    }

    #[test]
    fn bypass_adds_one_imaginary_atom_per_rule() {
        let base = two_rule_atn();
        let bypass = base.with_bypass_alternatives().expect("bypass ATN");

        // Rule i's imaginary token type is max_token_type + i + 1.
        let mut imaginary_atoms = Vec::new();
        for state in 0..bypass.state_count() {
            for transition in bypass.state(state).expect("state").transitions() {
                if transition.kind() == ParserTransitionKind::Atom {
                    if let ParserTransitionData::Atom { label, .. } = transition.data() {
                        if label > base.max_token_type() {
                            imaginary_atoms.push(label);
                        }
                    }
                }
            }
        }
        imaginary_atoms.sort_unstable();
        assert_eq!(imaginary_atoms, vec![3, 4]); // max(2) + {1, 2}
    }

    #[test]
    fn bypass_preserves_rule_start_and_stop_tables() {
        let base = two_rule_atn();
        let bypass = base.with_bypass_alternatives().expect("bypass ATN");

        let base_starts: Vec<_> = base.rule_to_start_state().into_iter().collect();
        let bypass_starts: Vec<_> = bypass.rule_to_start_state().into_iter().collect();
        assert_eq!(base_starts, bypass_starts);

        let base_stops: Vec<_> = base.rule_to_stop_state().into_iter().collect();
        let bypass_stops: Vec<_> = bypass.rule_to_stop_state().into_iter().collect();
        assert_eq!(base_stops, bypass_stops);
    }

    #[test]
    fn rule_start_gains_epsilon_into_bypass_block() {
        let base = two_rule_atn();
        let bypass = base.with_bypass_alternatives().expect("bypass ATN");

        // Rule 0's start state (0) should now epsilon into its bypass-start
        // block (the first appended state), added as the last outgoing edge.
        let bypass_start_for_rule_0 = base.state_count();
        let rule_start = bypass.state(0).expect("rule start");
        let epsilon_targets: Vec<_> = rule_start
            .transitions()
            .iter()
            .filter(|t| t.kind() == ParserTransitionKind::Epsilon)
            .map(super::super::parser_atn::ParserTransition::target)
            .collect();
        assert!(
            epsilon_targets.contains(&bypass_start_for_rule_0),
            "rule start must epsilon into its bypass block; got {epsilon_targets:?}"
        );
    }

    fn two_rule_recognizer_data() -> RecognizerData {
        RecognizerData::new(
            "Bypass.g4",
            Vocabulary::new(
                [None, Some("'x'"), Some("'y'")],
                [None, Some("X"), Some("Y")],
                [None::<&str>, None],
            ),
        )
        .with_rule_names(["a", "b"])
    }

    /// The load-bearing end-to-end proof: the *unchanged* ATN interpreter, run
    /// over the bypass ATN, matches a single imaginary token in place of a whole
    /// rule and renders it as a one-terminal rule subtree — exactly the shape
    /// ANTLR's `getRuleTagToken` detects.
    #[test]
    fn interpreter_matches_imaginary_token_as_whole_rule() {
        let bypass = two_rule_atn()
            .with_bypass_alternatives()
            .expect("bypass ATN");
        // Rule 1 ("b")'s imaginary token type = max_token_type(2) + 1 + 1 = 4.
        let imaginary_b = 4;
        let source = ListSource {
            specs: vec![
                TokenSpec::explicit(1, "x"),             // real X token
                TokenSpec::explicit(imaginary_b, "<b>"), // imaginary rule-b tag
            ],
            index: 0,
        };
        let mut parser =
            BaseParser::new(CommonTokenStream::new(source), two_rule_recognizer_data());

        let tree = parser
            .parse_atn_rule(&bypass, 0)
            .expect("bypass interpret of `a : X b` with imaginary b");

        let root = parser.node(tree).as_rule().expect("root is rule a");
        assert_eq!(root.rule_index(), 0);
        let children: Vec<_> = root.node().children().collect();
        assert_eq!(children.len(), 2, "a has children [X, b]");

        // First child: the real X terminal.
        let x = children[0].as_terminal().expect("first child terminal X");
        assert_eq!(x.symbol().token_type(), 1);

        // Second child: rule b rendered as a single-terminal subtree whose lone
        // leaf carries the imaginary token type. This is the `(b <b>)` shape.
        let b = children[1].as_rule().expect("second child rule b");
        assert_eq!(b.rule_index(), 1);
        let b_children: Vec<_> = b.node().children().collect();
        assert_eq!(b_children.len(), 1, "bypassed rule b has exactly one child");
        assert_eq!(b_children[0].kind(), NodeKind::Terminal);
        assert_eq!(
            b_children[0]
                .as_terminal()
                .expect("b's lone child is a terminal")
                .symbol()
                .token_type(),
            imaginary_b,
            "the lone child carries the imaginary bypass token type"
        );
    }

    /// The bypass ATN must still parse ordinary input identically: feeding the
    /// real tokens `X Y` reconstructs `(a X (b Y))` with no imaginary tokens.
    #[test]
    fn bypass_atn_still_parses_ordinary_input() {
        let bypass = two_rule_atn()
            .with_bypass_alternatives()
            .expect("bypass ATN");
        let source = ListSource {
            specs: vec![TokenSpec::explicit(1, "x"), TokenSpec::explicit(2, "y")],
            index: 0,
        };
        let mut parser =
            BaseParser::new(CommonTokenStream::new(source), two_rule_recognizer_data());

        let tree = parser
            .parse_atn_rule(&bypass, 0)
            .expect("bypass interpret of ordinary `X Y`");

        let root = parser.node(tree).as_rule().expect("root is rule a");
        let children: Vec<_> = root.node().children().collect();
        assert_eq!(children.len(), 2);
        assert_eq!(
            children[0]
                .as_terminal()
                .expect("X terminal")
                .symbol()
                .token_type(),
            1
        );
        let b = children[1].as_rule().expect("rule b");
        let y = b
            .node()
            .children()
            .next()
            .expect("b child")
            .as_terminal()
            .expect("Y terminal");
        assert_eq!(y.symbol().token_type(), 2);
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }
}
