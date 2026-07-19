//! Packed, index-addressed parser ATN storage.
//!
//! Parser ATNs are immutable after generation/deserialization. Keeping their
//! states and transitions in one validated word stream avoids the allocation
//! and pointer-chasing costs of an owned object graph while still exposing
//! borrowing semantic views to the simulator and diagnostics.

// These accessors are scalar address calculations used throughout the parser's
// innermost transition loops. Cross-crate generated parsers need them inlined.
#![allow(clippy::inline_always)]

use std::borrow::Cow;
use std::fmt;
use std::iter::FusedIterator;

use crate::token::TOKEN_EOF;

use super::AtnStateKind;

const PARSER_ATN_MAGIC: u32 = 0x5041_544e;
const PARSER_ATN_FORMAT_VERSION: u32 = 2;
const PARSER_ATN_MIN_FORMAT_VERSION: u32 = 1;
const PARSER_ATN_MAX_FORMAT_VERSION: u32 = 2;
const PARSER_ATN_BYTE_ORDER: u32 = 0x0102_0304;

const LEGACY_HEADER_WORDS: usize = 26;
const HEADER_WORDS: usize = 29;
const STATE_WORDS: usize = 7;
const TRANSITION_WORDS: usize = 5;
const LEGACY_SET_WORDS: usize = 2;
const SET_WORDS: usize = 5;
const PACKED_U64_WORDS: usize = 2;

const INLINE_TOKEN_SET_WORDS: usize = 2;
const INLINE_TOKEN_SET_MAX_SLOT: usize = INLINE_TOKEN_SET_WORDS * u64::BITS as usize - 1;
const MAX_DENSE_TOKEN_SET_BYTES: usize = 64 * 1024;
const MAX_DENSE_TOKEN_SET_WORDS: usize = MAX_DENSE_TOKEN_SET_BYTES / size_of::<u64>();
const DENSE_TOKEN_SET_COST_MULTIPLIER: usize = 2;
const DENSE_TOKEN_SET_MIN_DENSITY_DENOMINATOR: u64 = 8;

const NO_INDEX: u32 = u32::MAX;

const FLAG_NON_GREEDY: u32 = 1 << 0;
const FLAG_PRECEDENCE_DECISION: u32 = 1 << 1;
const FLAG_LEFT_RECURSIVE_RULE: u32 = 1 << 2;
const FLAG_EPSILON_ONLY: u32 = 1 << 3;
const FLAG_RULE_STOP: u32 = 1 << 4;
const FLAG_HAS_CONSUMING: u32 = 1 << 5;
const FLAG_HAS_SEMANTIC: u32 = 1 << 6;
const STATE_FLAGS: u32 = FLAG_NON_GREEDY
    | FLAG_PRECEDENCE_DECISION
    | FLAG_LEFT_RECURSIVE_RULE
    | FLAG_EPSILON_ONLY
    | FLAG_RULE_STOP
    | FLAG_HAS_CONSUMING
    | FLAG_HAS_SEMANTIC;

const HEADER_MAGIC: usize = 0;
const HEADER_VERSION: usize = 1;
const HEADER_BYTE_ORDER: usize = 2;
const HEADER_SIZE: usize = 3;
const HEADER_MAX_TOKEN_TYPE: usize = 4;
const HEADER_STATE_COUNT: usize = 5;
const HEADER_TRANSITION_COUNT: usize = 6;
const HEADER_SET_COUNT: usize = 7;
const HEADER_INTERVAL_COUNT: usize = 8;
const HEADER_DECISION_COUNT: usize = 9;
const HEADER_RULE_COUNT: usize = 10;
const HEADER_STATES_OFFSET: usize = 11;
const HEADER_TRANSITIONS_OFFSET: usize = 13;
const HEADER_SETS_OFFSET: usize = 15;
const HEADER_INTERVALS_OFFSET: usize = 17;
const HEADER_DECISIONS_OFFSET: usize = 19;
const HEADER_RULE_STARTS_OFFSET: usize = 21;
const HEADER_RULE_STOPS_OFFSET: usize = 23;
const HEADER_TOTAL_LEN: usize = 25;
const HEADER_TOKEN_BIT_WORD_COUNT: usize = 26;
const HEADER_TOKEN_BITS_OFFSET: usize = 27;

/// Checked compact identity for one parser ATN state.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AtnStateId(u32);

impl AtnStateId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    const fn raw(self) -> u32 {
        self.0
    }
}

impl TryFrom<usize> for AtnStateId {
    type Error = ParserAtnError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        compact_id("parser ATN state", value).map(Self)
    }
}

/// Checked compact identity for one parser ATN transition.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TransitionId(u32);

impl TransitionId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<usize> for TransitionId {
    type Error = ParserAtnError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        compact_id("parser ATN transition", value).map(Self)
    }
}

/// Checked compact identity for one shared interval set.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ParserIntervalSetId(u32);

impl ParserIntervalSetId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    const fn raw(self) -> u32 {
        self.0
    }
}

impl TryFrom<usize> for ParserIntervalSetId {
    type Error = ParserAtnError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        compact_id("parser ATN interval set", value).map(Self)
    }
}

/// Membership representation selected for one immutable parser token set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ParserTokenSetKind {
    /// Sorted, coalesced inclusive ranges searched by interval boundary.
    Intervals = 0,
    /// Two packed words covering EOF and token types `1..=127`.
    Inline128 = 1,
    /// A bounded packed word slice for a larger, cost-effective token domain.
    Dense = 2,
}

/// Failure while reading, validating, or constructing packed parser ATN data.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ParserAtnError {
    #[error(
        "generated parser ATN format version {found} is unsupported; \
         this runtime requires generator/runtime format {minimum}..={maximum}"
    )]
    UnsupportedVersion {
        found: u32,
        minimum: u32,
        maximum: u32,
    },
    #[error("invalid packed parser ATN: {0}")]
    InvalidData(String),
    #[error("{field} count/index {value} exceeds the compact u32 range")]
    Overflow { field: &'static str, value: usize },
}

/// Storage and shape measurements for one packed parser ATN.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParserAtnStats {
    pub states: usize,
    pub transitions: usize,
    pub interval_sets: usize,
    pub interval_ranges: usize,
    pub inline_token_sets: usize,
    pub dense_token_sets: usize,
    pub interval_token_sets: usize,
    pub token_bitset_bytes: usize,
    pub decisions: usize,
    pub rules: usize,
    pub packed_bytes: usize,
}

/// Immutable packed parser ATN.
///
/// Generated parsers borrow a static word stream directly. Deserialization of
/// ordinary ANTLR v4 integer metadata produces the same layout in one owned
/// allocation.
pub struct ParserAtn {
    words: Cow<'static, [u32]>,
    words_address: usize,
    layout: ParserAtnLayout,
}

impl Clone for ParserAtn {
    fn clone(&self) -> Self {
        let words = self.words.clone();
        let words_address = words.as_ptr() as usize;
        Self {
            words,
            words_address,
            layout: self.layout,
        }
    }
}

impl PartialEq for ParserAtn {
    fn eq(&self, other: &Self) -> bool {
        self.words == other.words && self.layout == other.layout
    }
}

impl Eq for ParserAtn {}

impl fmt::Debug for ParserAtn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParserAtn")
            .field("max_token_type", &self.max_token_type())
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl ParserAtn {
    /// Validates and borrows generator-emitted packed data without allocating.
    pub fn from_static(words: &'static [u32]) -> Result<Self, ParserAtnError> {
        let layout = validate_packed(words)?;
        let atn = Self {
            words: Cow::Borrowed(words),
            words_address: words.as_ptr() as usize,
            layout,
        };
        #[cfg(feature = "perf-counters")]
        atn.record_token_set_inventory();
        Ok(atn)
    }

    /// Validates one owned packed stream.
    pub fn from_owned(words: Vec<u32>) -> Result<Self, ParserAtnError> {
        let layout = validate_packed(&words)?;
        let words: Cow<'static, [u32]> = Cow::Owned(words);
        let words_address = words.as_ptr() as usize;
        let atn = Self {
            words,
            words_address,
            layout,
        };
        #[cfg(feature = "perf-counters")]
        atn.record_token_set_inventory();
        Ok(atn)
    }

    /// Canonical generator/runtime format version carried by this ATN.
    pub fn format_version(&self) -> u32 {
        self.words[HEADER_VERSION]
    }

    #[inline(always)]
    pub const fn max_token_type(&self) -> i32 {
        self.layout.max_token_type
    }

    pub const fn state_count(&self) -> usize {
        self.layout.state_count
    }

    pub const fn transition_count(&self) -> usize {
        self.layout.transition_count
    }

    pub const fn decision_count(&self) -> usize {
        self.layout.decisions.len
    }

    pub const fn rule_count(&self) -> usize {
        self.layout.rule_starts.len
    }

    #[inline(always)]
    pub fn state(&self, state_number: usize) -> Option<ParserAtnState<'_>> {
        (state_number < self.state_count())
            .then(|| ParserAtnState::new(self, AtnStateId(state_number as u32)))
    }

    #[inline(always)]
    pub fn state_by_id(&self, id: AtnStateId) -> Option<ParserAtnState<'_>> {
        (id.index() < self.state_count()).then(|| ParserAtnState::new(self, id))
    }

    pub const fn states(&self) -> ParserAtnStates<'_> {
        ParserAtnStates {
            atn: self,
            next: 0,
            end: self.state_count(),
        }
    }

    #[inline(always)]
    pub fn transition(&self, id: TransitionId) -> Option<ParserTransition<'_>> {
        (id.index() < self.transition_count()).then(|| ParserTransition::new(self, id))
    }

    pub const fn decision_to_state(&self) -> ParserStateIdTable<'_> {
        ParserStateIdTable::new(self, self.layout.decisions)
    }

    pub const fn rule_to_start_state(&self) -> ParserStateIdTable<'_> {
        ParserStateIdTable::new(self, self.layout.rule_starts)
    }

    pub const fn rule_to_stop_state(&self) -> ParserStateIdTable<'_> {
        ParserStateIdTable::new(self, self.layout.rule_stops)
    }

    /// Returns the exact generator-emitted representation.
    pub fn packed_words(&self) -> &[u32] {
        &self.words
    }

    /// Returns one immutable parser token set by its packed metadata index.
    ///
    /// Generated rule bodies use this to share the same adaptive membership
    /// representation as ATN prediction instead of embedding a second set.
    #[inline(always)]
    pub fn token_set(&self, index: usize) -> Option<ParserIntervalSet<'_>> {
        (index < self.set_count()).then(|| self.interval_set(ParserIntervalSetId(index as u32)))
    }

    /// Stable backing-storage address used by thread-local grammar caches.
    pub(crate) fn storage_identity(&self) -> (usize, usize) {
        (self.words.as_ptr() as usize, self.words.len())
    }

    pub fn stats(&self) -> ParserAtnStats {
        let mut inline_token_sets = 0;
        let mut dense_token_sets = 0;
        let mut interval_token_sets = 0;
        let mut token_bitset_bytes = 0;
        for index in 0..self.set_count() {
            let set = self.interval_set(ParserIntervalSetId(index as u32));
            match set.kind() {
                ParserTokenSetKind::Inline128 => inline_token_sets += 1,
                ParserTokenSetKind::Dense => dense_token_sets += 1,
                ParserTokenSetKind::Intervals => interval_token_sets += 1,
            }
            token_bitset_bytes += set.bit_len * size_of::<u64>();
        }
        ParserAtnStats {
            states: self.state_count(),
            transitions: self.transition_count(),
            interval_sets: self.set_count(),
            interval_ranges: self.layout.intervals.len / 2,
            inline_token_sets,
            dense_token_sets,
            interval_token_sets,
            token_bitset_bytes,
            decisions: self.decision_count(),
            rules: self.rule_count(),
            packed_bytes: self.words.len() * size_of::<u32>(),
        }
    }

    const fn set_count(&self) -> usize {
        self.layout.sets.len / self.layout.set_words
    }

    #[inline(always)]
    fn word(&self, section: Section, record: usize, field: usize, width: usize) -> u32 {
        self.packed_word(section.offset + record * width + field)
    }

    #[inline(always)]
    fn interval_set(&self, id: ParserIntervalSetId) -> ParserIntervalSet<'_> {
        let width = self.layout.set_words;
        let start = self.word(self.layout.sets, id.index(), 0, width) as usize;
        let len = self.word(self.layout.sets, id.index(), 1, width) as usize;
        let (kind, bit_start, bit_len) = if self.layout.format_version == 1 {
            (ParserTokenSetKind::Intervals, 0, 0)
        } else {
            (
                decode_token_set_kind(self.word(self.layout.sets, id.index(), 2, width))
                    .expect("packed parser token-set kind was validated"),
                self.word(self.layout.sets, id.index(), 3, width) as usize,
                self.word(self.layout.sets, id.index(), 4, width) as usize,
            )
        };
        ParserIntervalSet {
            atn: self,
            id,
            start,
            len,
            kind,
            bit_start,
            bit_len,
        }
    }

    #[inline(always)]
    fn token_bit_word(&self, index: usize) -> u64 {
        let offset = self.layout.token_bits.offset + index * PACKED_U64_WORDS;
        u64::from(self.packed_word(offset)) | (u64::from(self.packed_word(offset + 1)) << u32::BITS)
    }

    #[cfg(feature = "perf-counters")]
    fn record_token_set_inventory(&self) {
        for index in 0..self.set_count() {
            let set = self.interval_set(ParserIntervalSetId(index as u32));
            crate::perf::record_parser_token_set_selection(
                set.kind(),
                set.bit_len * size_of::<u64>(),
            );
        }
    }

    #[inline(always)]
    fn packed_address(&self, index: usize) -> usize {
        debug_assert!(index < self.words.len());
        self.words_address + index * size_of::<u32>()
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    fn packed_word(&self, index: usize) -> u32 {
        debug_assert!(index < self.words.len());
        // `words_address` is captured after the final backing allocation is in
        // place. Parser ATNs are immutable, and every view index/range is
        // validated before construction, so the allocation remains live and
        // the read stays in bounds for the lifetime of `self`.
        unsafe { *((self.words_address as *const u32).add(index)) }
    }
}

/// Borrowing semantic view over one parser ATN state.
#[derive(Clone, Copy)]
pub struct ParserAtnState<'a> {
    atn: &'a ParserAtn,
    record_address: usize,
}

impl fmt::Debug for ParserAtnState<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParserAtnState")
            .field("id", &self.id())
            .field("kind", &self.kind())
            .field("rule_index", &self.rule_index())
            .field("transition_count", &self.transitions().len())
            .finish()
    }
}

impl<'a> ParserAtnState<'a> {
    #[inline(always)]
    fn new(atn: &'a ParserAtn, id: AtnStateId) -> Self {
        Self {
            atn,
            record_address: atn.packed_address(atn.layout.states.offset + id.index() * STATE_WORDS),
        }
    }

    pub const fn id(self) -> AtnStateId {
        let word = (self.record_address - self.atn.words_address) / size_of::<u32>();
        AtnStateId(((word - self.atn.layout.states.offset) / STATE_WORDS) as u32)
    }

    pub const fn state_number(self) -> usize {
        self.id().index()
    }

    #[inline(always)]
    pub fn kind(self) -> AtnStateKind {
        decode_state_kind(self.word(0)).expect("packed parser ATN state kind was validated")
    }

    #[inline(always)]
    pub fn rule_index(self) -> Option<usize> {
        unpack_index(self.word(1))
    }

    #[inline(always)]
    pub fn end_state(self) -> Option<usize> {
        unpack_index(self.word(5))
    }

    #[inline(always)]
    pub fn loop_back_state(self) -> Option<usize> {
        unpack_index(self.word(6))
    }

    #[inline(always)]
    pub fn non_greedy(self) -> bool {
        self.flags() & FLAG_NON_GREEDY != 0
    }

    #[inline(always)]
    pub fn precedence_rule_decision(self) -> bool {
        self.flags() & FLAG_PRECEDENCE_DECISION != 0
    }

    #[inline(always)]
    pub fn left_recursive_rule(self) -> bool {
        self.flags() & FLAG_LEFT_RECURSIVE_RULE != 0
    }

    #[inline]
    pub fn is_rule_stop(self) -> bool {
        self.flags() & FLAG_RULE_STOP != 0
    }

    #[inline]
    pub fn epsilon_only(self) -> bool {
        self.flags() & FLAG_EPSILON_ONLY != 0
    }

    #[inline]
    pub fn has_consuming_transition(self) -> bool {
        self.flags() & FLAG_HAS_CONSUMING != 0
    }

    #[inline]
    pub fn has_semantic_transition(self) -> bool {
        self.flags() & FLAG_HAS_SEMANTIC != 0
    }

    #[inline(always)]
    pub fn transitions(self) -> ParserTransitions<'a> {
        let start = self.word(3) as usize;
        ParserTransitions {
            atn: self.atn,
            record_address: self
                .atn
                .packed_address(self.atn.layout.transitions.offset + start * TRANSITION_WORDS),
            len: self.word(4) as usize,
        }
    }

    #[inline(always)]
    fn flags(self) -> u32 {
        self.word(2)
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    fn word(self, field: usize) -> u32 {
        debug_assert!(field < STATE_WORDS);
        // The record address comes from the immutable validated state
        // section, and `field` is constrained to the fixed record width.
        unsafe { *((self.record_address as *const u32).add(field)) }
    }
}

/// Borrowing range of transitions owned by the ATN's shared transition pool.
#[derive(Clone, Copy, Debug)]
pub struct ParserTransitions<'a> {
    atn: &'a ParserAtn,
    record_address: usize,
    len: usize,
}

impl<'a> ParserTransitions<'a> {
    pub const fn len(self) -> usize {
        self.len
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn get(self, index: usize) -> Option<ParserTransition<'a>> {
        (index < self.len).then(|| ParserTransition {
            atn: self.atn,
            record_address: self.record_address + index * TRANSITION_WORDS * size_of::<u32>(),
        })
    }

    #[inline(always)]
    pub fn first(self) -> Option<ParserTransition<'a>> {
        self.get(0)
    }

    #[inline]
    pub fn last(self) -> Option<ParserTransition<'a>> {
        self.len.checked_sub(1).and_then(|index| self.get(index))
    }

    pub const fn iter(self) -> ParserTransitionIter<'a> {
        ParserTransitionIter {
            atn: self.atn,
            next_record_address: self.record_address,
            remaining: self.len,
        }
    }
}

impl<'a> IntoIterator for ParserTransitions<'a> {
    type Item = ParserTransition<'a>;
    type IntoIter = ParserTransitionIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a ParserTransitions<'a> {
    type Item = ParserTransition<'a>;
    type IntoIter = ParserTransitionIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over one state's contiguous transition range.
#[derive(Clone, Debug)]
pub struct ParserTransitionIter<'a> {
    atn: &'a ParserAtn,
    next_record_address: usize,
    remaining: usize,
}

impl<'a> Iterator for ParserTransitionIter<'a> {
    type Item = ParserTransition<'a>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let transition = ParserTransition {
            atn: self.atn,
            record_address: self.next_record_address,
        };
        self.next_record_address += TRANSITION_WORDS * size_of::<u32>();
        self.remaining -= 1;
        Some(transition)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for ParserTransitionIter<'_> {}
impl FusedIterator for ParserTransitionIter<'_> {}

/// Borrowing semantic view over one parser ATN transition.
#[derive(Clone, Copy)]
pub struct ParserTransition<'a> {
    atn: &'a ParserAtn,
    record_address: usize,
}

impl fmt::Debug for ParserTransition<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.data().fmt(formatter)
    }
}

impl<'a> ParserTransition<'a> {
    #[inline(always)]
    fn new(atn: &'a ParserAtn, id: TransitionId) -> Self {
        Self {
            atn,
            record_address: atn
                .packed_address(atn.layout.transitions.offset + id.index() * TRANSITION_WORDS),
        }
    }

    #[inline(always)]
    pub const fn id(self) -> TransitionId {
        let word = (self.record_address - self.atn.words_address) / size_of::<u32>();
        TransitionId(((word - self.atn.layout.transitions.offset) / TRANSITION_WORDS) as u32)
    }

    #[inline(always)]
    pub fn target_id(self) -> AtnStateId {
        AtnStateId(self.word(1))
    }

    #[inline(always)]
    pub fn target(self) -> usize {
        self.target_id().index()
    }

    #[inline(always)]
    pub fn kind(self) -> ParserTransitionKind {
        decode_transition_kind(self.word(0))
            .expect("packed parser ATN transition kind was validated")
    }

    #[inline(always)]
    pub fn is_epsilon(self) -> bool {
        matches!(
            self.kind(),
            ParserTransitionKind::Epsilon
                | ParserTransitionKind::Rule
                | ParserTransitionKind::Predicate
                | ParserTransitionKind::Action
                | ParserTransitionKind::Precedence
        )
    }

    #[inline(always)]
    pub fn is_action(self) -> bool {
        self.kind() == ParserTransitionKind::Action
    }

    #[inline(always)]
    pub fn matches(self, symbol: i32, min_vocabulary: i32, max_vocabulary: i32) -> bool {
        self.matches_kind(self.kind(), symbol, min_vocabulary, max_vocabulary)
    }

    #[inline(always)]
    pub(crate) fn matches_kind(
        self,
        kind: ParserTransitionKind,
        symbol: i32,
        min_vocabulary: i32,
        max_vocabulary: i32,
    ) -> bool {
        match kind {
            ParserTransitionKind::Atom => unpack_i32(self.arg0()) == symbol,
            ParserTransitionKind::Range => {
                (unpack_i32(self.arg0())..=unpack_i32(self.arg1())).contains(&symbol)
            }
            ParserTransitionKind::Set => self
                .atn
                .interval_set(ParserIntervalSetId(self.arg0()))
                .contains(symbol),
            ParserTransitionKind::NotSet => {
                (min_vocabulary..=max_vocabulary).contains(&symbol)
                    && !self
                        .atn
                        .interval_set(ParserIntervalSetId(self.arg0()))
                        .contains(symbol)
            }
            ParserTransitionKind::Wildcard => (min_vocabulary..=max_vocabulary).contains(&symbol),
            ParserTransitionKind::Epsilon
            | ParserTransitionKind::Rule
            | ParserTransitionKind::Predicate
            | ParserTransitionKind::Action
            | ParserTransitionKind::Precedence => false,
        }
    }

    #[inline(always)]
    pub(crate) fn arg0(self) -> u32 {
        self.word(2)
    }

    #[inline(always)]
    pub(crate) fn arg1(self) -> u32 {
        self.word(3)
    }

    #[inline(always)]
    pub(crate) fn arg2(self) -> u32 {
        self.word(4)
    }

    #[inline(always)]
    pub fn data(self) -> ParserTransitionData<'a> {
        match decode_transition_kind(self.word(0))
            .expect("packed parser ATN transition kind was validated")
        {
            ParserTransitionKind::Epsilon => ParserTransitionData::Epsilon {
                target: self.word(1) as usize,
            },
            ParserTransitionKind::Atom => ParserTransitionData::Atom {
                target: self.word(1) as usize,
                label: unpack_i32(self.word(2)),
            },
            ParserTransitionKind::Range => ParserTransitionData::Range {
                target: self.word(1) as usize,
                start: unpack_i32(self.word(2)),
                stop: unpack_i32(self.word(3)),
            },
            ParserTransitionKind::Set => ParserTransitionData::Set {
                target: self.word(1) as usize,
                set: self.atn.interval_set(ParserIntervalSetId(self.word(2))),
            },
            ParserTransitionKind::NotSet => ParserTransitionData::NotSet {
                target: self.word(1) as usize,
                set: self.atn.interval_set(ParserIntervalSetId(self.word(2))),
            },
            ParserTransitionKind::Wildcard => ParserTransitionData::Wildcard {
                target: self.word(1) as usize,
            },
            ParserTransitionKind::Rule => ParserTransitionData::Rule {
                target: self.word(1) as usize,
                rule_index: self.word(2) as usize,
                follow_state: self.word(3) as usize,
                precedence: unpack_i32(self.word(4)),
            },
            ParserTransitionKind::Predicate => ParserTransitionData::Predicate {
                target: self.word(1) as usize,
                rule_index: self.word(2) as usize,
                pred_index: self.word(3) as usize,
                context_dependent: self.word(4) != 0,
            },
            ParserTransitionKind::Action => ParserTransitionData::Action {
                target: self.word(1) as usize,
                rule_index: self.word(2) as usize,
                action_index: unpack_index(self.word(3)),
                context_dependent: self.word(4) != 0,
            },
            ParserTransitionKind::Precedence => ParserTransitionData::Precedence {
                target: self.word(1) as usize,
                precedence: unpack_i32(self.word(2)),
            },
        }
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    fn word(self, field: usize) -> u32 {
        debug_assert!(field < TRANSITION_WORDS);
        // The record address comes from the immutable validated transition
        // section, and `field` is constrained to the fixed record width.
        unsafe { *((self.record_address as *const u32).add(field)) }
    }
}

/// Fixed transition tag stored in the packed transition table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ParserTransitionKind {
    Epsilon = 1,
    Range = 2,
    Rule = 3,
    Predicate = 4,
    Atom = 5,
    Action = 6,
    Set = 7,
    NotSet = 8,
    Wildcard = 9,
    Precedence = 10,
}

/// Borrowing semantic payload for a packed parser transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParserTransitionData<'a> {
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
        set: ParserIntervalSet<'a>,
    },
    NotSet {
        target: usize,
        set: ParserIntervalSet<'a>,
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

impl ParserTransitionData<'_> {
    pub const fn target(self) -> usize {
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
            | Self::Precedence { target, .. } => target,
        }
    }

    pub const fn is_epsilon(self) -> bool {
        matches!(
            self,
            Self::Epsilon { .. }
                | Self::Rule { .. }
                | Self::Predicate { .. }
                | Self::Action { .. }
                | Self::Precedence { .. }
        )
    }

    pub const fn is_action(self) -> bool {
        matches!(self, Self::Action { .. })
    }

    pub fn matches(self, symbol: i32, min_vocabulary: i32, max_vocabulary: i32) -> bool {
        match self {
            Self::Atom { label, .. } => label == symbol,
            Self::Range { start, stop, .. } => (start..=stop).contains(&symbol),
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

/// Borrowing view over one interval set in the shared interval pool.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ParserIntervalSet<'a> {
    atn: &'a ParserAtn,
    id: ParserIntervalSetId,
    start: usize,
    len: usize,
    kind: ParserTokenSetKind,
    bit_start: usize,
    bit_len: usize,
}

impl fmt::Debug for ParserIntervalSet<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.ranges()).finish()
    }
}

impl<'a> ParserIntervalSet<'a> {
    /// Stable index of this set in the packed parser metadata.
    pub const fn index(self) -> usize {
        self.id.index()
    }

    /// Membership representation selected when the packed ATN was built.
    pub const fn kind(self) -> ParserTokenSetKind {
        self.kind
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn contains(self, value: i32) -> bool {
        let hit = match self.kind {
            ParserTokenSetKind::Inline128 | ParserTokenSetKind::Dense => {
                self.contains_bitset(value)
            }
            ParserTokenSetKind::Intervals => self.contains_intervals(value),
        };
        #[cfg(feature = "perf-counters")]
        crate::perf::record_parser_token_set_probe(self.kind, hit);
        hit
    }

    #[inline(always)]
    fn contains_bitset(self, value: i32) -> bool {
        let Some(slot) = token_set_slot(value) else {
            return false;
        };
        let word = slot / u64::BITS as usize;
        word < self.bit_len
            && self.atn.token_bit_word(self.bit_start + word)
                & (1_u64 << (slot % u64::BITS as usize))
                != 0
    }

    #[inline(always)]
    fn contains_intervals(self, value: i32) -> bool {
        let mut low = 0;
        let mut high = self.len;
        while low < high {
            let middle = low + (high - low) / 2;
            if self.range_start(middle) <= value {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low > 0 && self.range_stop(low - 1) >= value
    }

    pub const fn ranges(self) -> ParserIntervalRanges<'a> {
        ParserIntervalRanges { set: self, next: 0 }
    }

    #[inline(always)]
    fn range(self, index: usize) -> (i32, i32) {
        (self.range_start(index), self.range_stop(index))
    }

    #[inline(always)]
    fn range_start(self, index: usize) -> i32 {
        let word = self.atn.layout.intervals.offset + (self.start + index) * 2;
        unpack_i32(self.atn.packed_word(word))
    }

    #[inline(always)]
    fn range_stop(self, index: usize) -> i32 {
        let word = self.atn.layout.intervals.offset + (self.start + index) * 2 + 1;
        unpack_i32(self.atn.packed_word(word))
    }
}

/// Iterator over inclusive ranges in one shared parser interval set.
#[derive(Clone, Debug)]
pub struct ParserIntervalRanges<'a> {
    set: ParserIntervalSet<'a>,
    next: usize,
}

impl Iterator for ParserIntervalRanges<'_> {
    type Item = (i32, i32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.set.len {
            return None;
        }
        let range = self.set.range(self.next);
        self.next += 1;
        Some(range)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.set.len.saturating_sub(self.next);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ParserIntervalRanges<'_> {}
impl FusedIterator for ParserIntervalRanges<'_> {}

/// Borrowing compact-ID side table with checked `usize` accessors.
#[derive(Clone, Copy, Debug)]
pub struct ParserStateIdTable<'a> {
    atn: &'a ParserAtn,
    section: Section,
}

impl<'a> ParserStateIdTable<'a> {
    const fn new(atn: &'a ParserAtn, section: Section) -> Self {
        Self { atn, section }
    }

    pub const fn len(self) -> usize {
        self.section.len
    }

    pub const fn is_empty(self) -> bool {
        self.section.len == 0
    }

    #[inline(always)]
    pub fn get(self, index: usize) -> Option<usize> {
        (index < self.len()).then(|| self.atn.packed_word(self.section.offset + index) as usize)
    }

    pub fn get_id(self, index: usize) -> Option<AtnStateId> {
        self.get(index).map(|value| {
            AtnStateId::try_from(value).expect("validated side-table state fits compact ID")
        })
    }

    pub const fn iter(self) -> ParserStateIdIter<'a> {
        ParserStateIdIter {
            table: self,
            next: 0,
        }
    }
}

impl<'a> IntoIterator for ParserStateIdTable<'a> {
    type Item = usize;
    type IntoIter = ParserStateIdIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over checked state indices in a parser ATN side table.
#[derive(Clone, Debug)]
pub struct ParserStateIdIter<'a> {
    table: ParserStateIdTable<'a>,
    next: usize,
}

impl Iterator for ParserStateIdIter<'_> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let value = self.table.get(self.next)?;
        self.next += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.table.len().saturating_sub(self.next);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ParserStateIdIter<'_> {}
impl FusedIterator for ParserStateIdIter<'_> {}

/// Iterator over every state in deterministic state-number order.
#[derive(Clone, Debug)]
pub struct ParserAtnStates<'a> {
    atn: &'a ParserAtn,
    next: usize,
    end: usize,
}

impl<'a> Iterator for ParserAtnStates<'a> {
    type Item = ParserAtnState<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.end {
            return None;
        }
        let state = self.atn.state(self.next);
        self.next += 1;
        state
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end.saturating_sub(self.next);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ParserAtnStates<'_> {}
impl FusedIterator for ParserAtnStates<'_> {}

/// Centralized construction API for packed parser ATNs.
///
/// States never own transition collections; edges are grouped into contiguous
/// ranges only when the final packed stream is emitted.
#[derive(Debug)]
pub struct ParserAtnBuilder {
    max_token_type: i32,
    states: Vec<StateBuild>,
    transitions: Vec<TransitionBuild>,
    interval_sets: Vec<TokenSetBuild>,
    interval_ranges: Vec<(i32, i32)>,
    token_bit_words: Vec<u64>,
    decisions: Vec<AtnStateId>,
    rule_starts: Vec<AtnStateId>,
    rule_stops: Vec<AtnStateId>,
}

impl ParserAtnBuilder {
    pub const fn new(max_token_type: i32) -> Self {
        Self {
            max_token_type,
            states: Vec::new(),
            transitions: Vec::new(),
            interval_sets: Vec::new(),
            interval_ranges: Vec::new(),
            token_bit_words: Vec::new(),
            decisions: Vec::new(),
            rule_starts: Vec::new(),
            rule_stops: Vec::new(),
        }
    }

    pub fn add_state(
        &mut self,
        kind: AtnStateKind,
        rule_index: Option<usize>,
    ) -> Result<AtnStateId, ParserAtnError> {
        let id = AtnStateId::try_from(self.states.len())?;
        let rule_index = pack_optional_index("parser ATN rule", rule_index)?;
        self.states.push(StateBuild {
            kind,
            rule_index,
            flags: u32::from(kind == AtnStateKind::RuleStop) * FLAG_RULE_STOP,
            end_state: NO_INDEX,
            loop_back_state: NO_INDEX,
        });
        Ok(id)
    }

    pub fn set_end_state(&mut self, state: usize, end_state: usize) -> Result<(), ParserAtnError> {
        let end_state = self.checked_state(end_state, "block end state")?;
        self.state_mut(state, "block start state")?.end_state = end_state.raw();
        Ok(())
    }

    pub fn set_loop_back_state(
        &mut self,
        state: usize,
        loop_back_state: usize,
    ) -> Result<(), ParserAtnError> {
        let loop_back_state = self.checked_state(loop_back_state, "loop back state")?;
        self.state_mut(state, "loop end state")?.loop_back_state = loop_back_state.raw();
        Ok(())
    }

    pub fn set_non_greedy(&mut self, state: usize) -> Result<(), ParserAtnError> {
        self.state_mut(state, "non-greedy state")?.flags |= FLAG_NON_GREEDY;
        Ok(())
    }

    pub fn set_left_recursive_rule(&mut self, state: usize) -> Result<(), ParserAtnError> {
        self.state_mut(state, "precedence rule state")?.flags |= FLAG_LEFT_RECURSIVE_RULE;
        Ok(())
    }

    pub fn set_precedence_rule_decision(&mut self, state: usize) -> Result<(), ParserAtnError> {
        self.state_mut(state, "precedence decision state")?.flags |= FLAG_PRECEDENCE_DECISION;
        Ok(())
    }

    pub fn add_interval_set(
        &mut self,
        ranges: impl IntoIterator<Item = (i32, i32)>,
    ) -> Result<ParserIntervalSetId, ParserAtnError> {
        let id = ParserIntervalSetId::try_from(self.interval_sets.len())?;
        let normalized = normalize_ranges(ranges);
        let interval_start = compact_id("parser ATN interval start", self.interval_ranges.len())?;
        let interval_len = compact_id("parser ATN interval count", normalized.len())?;
        let prepared = prepare_token_set(&normalized);
        let bit_start = compact_id("parser token-set bit start", self.token_bit_words.len())?;
        let bit_len = compact_id("parser token-set bit count", prepared.words.len())?;
        self.interval_ranges.extend(normalized);
        self.token_bit_words.extend(prepared.words);
        self.interval_sets.push(TokenSetBuild {
            interval_start,
            interval_len,
            kind: prepared.kind,
            bit_start,
            bit_len,
        });
        Ok(id)
    }

    pub fn add_transition(
        &mut self,
        source: usize,
        transition: ParserTransitionSpec,
    ) -> Result<TransitionId, ParserAtnError> {
        let source = self.checked_state(source, "transition source")?;
        let record = self.transition_record(source, transition)?;
        let id = TransitionId::try_from(self.transitions.len())?;
        self.transitions.push(record);
        Ok(id)
    }

    pub fn set_rule_to_start_state(&mut self, states: Vec<usize>) -> Result<(), ParserAtnError> {
        self.rule_starts = self.checked_states(states, "rule start state")?;
        Ok(())
    }

    pub fn set_rule_to_stop_state(&mut self, states: Vec<usize>) -> Result<(), ParserAtnError> {
        self.rule_stops = self.checked_states(states, "rule stop state")?;
        Ok(())
    }

    pub fn add_decision_state(&mut self, state: usize) -> Result<(), ParserAtnError> {
        let state = self.checked_state(state, "decision state")?;
        self.decisions.push(state);
        Ok(())
    }

    pub fn state_kind(&self, state: usize) -> Option<AtnStateKind> {
        self.states.get(state).map(|record| record.kind)
    }

    pub const fn state_count(&self) -> usize {
        self.states.len()
    }

    pub fn state_rule_index(&self, state: usize) -> Option<usize> {
        self.states
            .get(state)
            .and_then(|record| unpack_index(record.rule_index))
    }

    pub fn rule_stop_state(&self, rule: usize) -> Option<usize> {
        self.rule_stops.get(rule).copied().map(AtnStateId::index)
    }

    pub fn transitions_from(
        &self,
        source: usize,
    ) -> impl DoubleEndedIterator<Item = ParserTransitionSpec> + '_ {
        self.transitions
            .iter()
            .filter(move |transition| transition.source.index() == source)
            .map(TransitionBuild::spec)
    }

    pub fn finish(mut self) -> Result<ParserAtn, ParserAtnError> {
        self.mark_precedence_decisions();
        self.transitions.sort_by_key(|transition| transition.source);
        let transition_ranges = self.transition_ranges()?;
        self.precompute_state_flags(&transition_ranges);
        let words = self.encode(&transition_ranges)?;
        ParserAtn::from_owned(words)
    }

    fn state_mut(&mut self, state: usize, label: &str) -> Result<&mut StateBuild, ParserAtnError> {
        self.states.get_mut(state).ok_or_else(|| {
            ParserAtnError::InvalidData(format!("{label} {state} outside state list"))
        })
    }

    fn checked_state(&self, state: usize, label: &str) -> Result<AtnStateId, ParserAtnError> {
        let id = AtnStateId::try_from(state)?;
        if state >= self.states.len() {
            return Err(ParserAtnError::InvalidData(format!(
                "{label} {state} outside state list"
            )));
        }
        Ok(id)
    }

    fn checked_states(
        &self,
        states: Vec<usize>,
        label: &str,
    ) -> Result<Vec<AtnStateId>, ParserAtnError> {
        states
            .into_iter()
            .map(|state| self.checked_state(state, label))
            .collect()
    }

    fn transition_record(
        &self,
        source: AtnStateId,
        spec: ParserTransitionSpec,
    ) -> Result<TransitionBuild, ParserAtnError> {
        let target = self.checked_state(spec.target(), "transition target")?;
        let (kind, arg0, arg1, arg2) = match spec {
            ParserTransitionSpec::Epsilon { .. } => (ParserTransitionKind::Epsilon, 0, 0, 0),
            ParserTransitionSpec::Atom { label, .. } => {
                (ParserTransitionKind::Atom, pack_i32(label), 0, 0)
            }
            ParserTransitionSpec::Range { start, stop, .. } => (
                ParserTransitionKind::Range,
                pack_i32(start),
                pack_i32(stop),
                0,
            ),
            ParserTransitionSpec::Set { set, .. } => {
                self.checked_set(set)?;
                (ParserTransitionKind::Set, set.raw(), 0, 0)
            }
            ParserTransitionSpec::NotSet { set, .. } => {
                self.checked_set(set)?;
                (ParserTransitionKind::NotSet, set.raw(), 0, 0)
            }
            ParserTransitionSpec::Wildcard { .. } => (ParserTransitionKind::Wildcard, 0, 0, 0),
            ParserTransitionSpec::Rule {
                rule_index,
                follow_state,
                precedence,
                ..
            } => (
                ParserTransitionKind::Rule,
                compact_id("rule transition rule", rule_index)?,
                self.checked_state(follow_state, "rule follow state")?.raw(),
                pack_i32(precedence),
            ),
            ParserTransitionSpec::Predicate {
                rule_index,
                pred_index,
                context_dependent,
                ..
            } => (
                ParserTransitionKind::Predicate,
                compact_id("predicate rule", rule_index)?,
                compact_id("predicate index", pred_index)?,
                u32::from(context_dependent),
            ),
            ParserTransitionSpec::Action {
                rule_index,
                action_index,
                context_dependent,
                ..
            } => (
                ParserTransitionKind::Action,
                compact_id("action rule", rule_index)?,
                pack_optional_index("action", action_index)?,
                u32::from(context_dependent),
            ),
            ParserTransitionSpec::Precedence { precedence, .. } => {
                (ParserTransitionKind::Precedence, pack_i32(precedence), 0, 0)
            }
        };
        Ok(TransitionBuild {
            source,
            kind,
            target,
            arg0,
            arg1,
            arg2,
        })
    }

    fn checked_set(&self, set: ParserIntervalSetId) -> Result<(), ParserAtnError> {
        if set.index() >= self.interval_sets.len() {
            return Err(ParserAtnError::InvalidData(format!(
                "interval set {} outside set list",
                set.index()
            )));
        }
        Ok(())
    }

    fn transition_ranges(&self) -> Result<Vec<(u32, u32)>, ParserAtnError> {
        let mut ranges = vec![(0, 0); self.states.len()];
        let mut cursor = 0;
        for (state, range) in ranges.iter_mut().enumerate() {
            let start = cursor;
            while cursor < self.transitions.len()
                && self.transitions[cursor].source.index() == state
            {
                cursor += 1;
            }
            *range = (
                compact_id("state transition start", start)?,
                compact_id("state transition count", cursor - start)?,
            );
        }
        Ok(ranges)
    }

    fn precompute_state_flags(&mut self, ranges: &[(u32, u32)]) {
        for (state, &(start, len)) in self.states.iter_mut().zip(ranges) {
            let transitions = &self.transitions[start as usize..start as usize + len as usize];
            if !transitions.is_empty()
                && transitions
                    .iter()
                    .all(|transition| transition.kind.is_epsilon())
            {
                state.flags |= FLAG_EPSILON_ONLY;
            }
            if transitions
                .iter()
                .any(|transition| transition.kind.is_consuming())
            {
                state.flags |= FLAG_HAS_CONSUMING;
            }
            if transitions
                .iter()
                .any(|transition| transition.kind.is_semantic())
            {
                state.flags |= FLAG_HAS_SEMANTIC;
            }
        }
    }

    fn mark_precedence_decisions(&mut self) {
        let candidates = (0..self.states.len())
            .filter(|&state| self.is_precedence_decision(state))
            .collect::<Vec<_>>();
        for state in candidates {
            self.states[state].flags |= FLAG_PRECEDENCE_DECISION;
        }
    }

    fn is_precedence_decision(&self, state: usize) -> bool {
        let record = &self.states[state];
        if record.kind != AtnStateKind::StarLoopEntry {
            return false;
        }
        let Some(rule_index) = unpack_index(record.rule_index) else {
            return false;
        };
        let Some(rule_start) = self.rule_starts.get(rule_index) else {
            return false;
        };
        if self.states[rule_start.index()].flags & FLAG_LEFT_RECURSIVE_RULE == 0 {
            return false;
        }
        let Some(loop_end) = self.transitions_from(state).next_back() else {
            return false;
        };
        let loop_end = loop_end.target();
        if self.state_kind(loop_end) != Some(AtnStateKind::LoopEnd) {
            return false;
        }
        self.transitions_from(loop_end)
            .next()
            .and_then(|transition| self.state_kind(transition.target()))
            == Some(AtnStateKind::RuleStop)
    }

    fn encode(&self, transition_ranges: &[(u32, u32)]) -> Result<Vec<u32>, ParserAtnError> {
        let layout = EncodedLayout::new(self)?;
        let mut words = vec![0; layout.total_len];
        self.encode_header(&mut words, layout)?;
        self.encode_states(&mut words, layout.states, transition_ranges);
        self.encode_transitions(&mut words, layout.transitions);
        self.encode_sets(&mut words, layout.sets);
        self.encode_intervals(&mut words, layout.intervals);
        self.encode_token_bits(&mut words, layout.token_bits);
        encode_ids(&mut words, layout.decisions, &self.decisions);
        encode_ids(&mut words, layout.rule_starts, &self.rule_starts);
        encode_ids(&mut words, layout.rule_stops, &self.rule_stops);
        Ok(words)
    }

    fn encode_header(
        &self,
        words: &mut [u32],
        layout: EncodedLayout,
    ) -> Result<(), ParserAtnError> {
        words[HEADER_MAGIC] = PARSER_ATN_MAGIC;
        words[HEADER_VERSION] = PARSER_ATN_FORMAT_VERSION;
        words[HEADER_BYTE_ORDER] = PARSER_ATN_BYTE_ORDER;
        words[HEADER_SIZE] = compact_id("parser ATN header size", HEADER_WORDS)?;
        words[HEADER_MAX_TOKEN_TYPE] = pack_i32(self.max_token_type);
        words[HEADER_STATE_COUNT] = compact_id("parser ATN state count", self.states.len())?;
        words[HEADER_TRANSITION_COUNT] =
            compact_id("parser ATN transition count", self.transitions.len())?;
        words[HEADER_SET_COUNT] =
            compact_id("parser ATN interval-set count", self.interval_sets.len())?;
        words[HEADER_INTERVAL_COUNT] =
            compact_id("parser ATN interval count", self.interval_ranges.len())?;
        words[HEADER_DECISION_COUNT] =
            compact_id("parser ATN decision count", self.decisions.len())?;
        words[HEADER_RULE_COUNT] = compact_id("parser ATN rule count", self.rule_starts.len())?;
        write_section(words, HEADER_STATES_OFFSET, layout.states)?;
        write_section(words, HEADER_TRANSITIONS_OFFSET, layout.transitions)?;
        write_section(words, HEADER_SETS_OFFSET, layout.sets)?;
        write_section(words, HEADER_INTERVALS_OFFSET, layout.intervals)?;
        words[HEADER_TOKEN_BIT_WORD_COUNT] = compact_id(
            "parser token-set bit word count",
            self.token_bit_words.len(),
        )?;
        write_section(words, HEADER_TOKEN_BITS_OFFSET, layout.token_bits)?;
        write_section(words, HEADER_DECISIONS_OFFSET, layout.decisions)?;
        write_section(words, HEADER_RULE_STARTS_OFFSET, layout.rule_starts)?;
        write_section(words, HEADER_RULE_STOPS_OFFSET, layout.rule_stops)?;
        words[HEADER_TOTAL_LEN] = compact_id("packed parser ATN word", layout.total_len)?;
        Ok(())
    }

    fn encode_states(&self, words: &mut [u32], section: Section, transition_ranges: &[(u32, u32)]) {
        for (index, (state, &(start, len))) in self.states.iter().zip(transition_ranges).enumerate()
        {
            let base = section.offset + index * STATE_WORDS;
            words[base] = state_kind_word(state.kind);
            words[base + 1] = state.rule_index;
            words[base + 2] = state.flags;
            words[base + 3] = start;
            words[base + 4] = len;
            words[base + 5] = state.end_state;
            words[base + 6] = state.loop_back_state;
        }
    }

    fn encode_transitions(&self, words: &mut [u32], section: Section) {
        for (index, transition) in self.transitions.iter().enumerate() {
            let base = section.offset + index * TRANSITION_WORDS;
            words[base] = transition.kind as u32;
            words[base + 1] = transition.target.raw();
            words[base + 2] = transition.arg0;
            words[base + 3] = transition.arg1;
            words[base + 4] = transition.arg2;
        }
    }

    fn encode_sets(&self, words: &mut [u32], section: Section) {
        for (index, set) in self.interval_sets.iter().enumerate() {
            let base = section.offset + index * SET_WORDS;
            words[base] = set.interval_start;
            words[base + 1] = set.interval_len;
            words[base + 2] = set.kind as u32;
            words[base + 3] = set.bit_start;
            words[base + 4] = set.bit_len;
        }
    }

    fn encode_intervals(&self, words: &mut [u32], section: Section) {
        for (index, &(start, stop)) in self.interval_ranges.iter().enumerate() {
            let base = section.offset + index * 2;
            words[base] = pack_i32(start);
            words[base + 1] = pack_i32(stop);
        }
    }

    fn encode_token_bits(&self, words: &mut [u32], section: Section) {
        for (index, &bits) in self.token_bit_words.iter().enumerate() {
            let base = section.offset + index * PACKED_U64_WORDS;
            words[base] = bits as u32;
            words[base + 1] = (bits >> u32::BITS) as u32;
        }
    }
}

/// Transient semantic transition accepted by [`ParserAtnBuilder`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParserTransitionSpec {
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
        set: ParserIntervalSetId,
    },
    NotSet {
        target: usize,
        set: ParserIntervalSetId,
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

impl ParserTransitionSpec {
    pub const fn target(self) -> usize {
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
            | Self::Precedence { target, .. } => target,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParserAtnLayout {
    format_version: u32,
    max_token_type: i32,
    state_count: usize,
    transition_count: usize,
    set_words: usize,
    states: Section,
    transitions: Section,
    sets: Section,
    intervals: Section,
    token_bits: Section,
    decisions: Section,
    rule_starts: Section,
    rule_stops: Section,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Section {
    offset: usize,
    len: usize,
}

#[derive(Clone, Copy, Debug)]
struct EncodedLayout {
    states: Section,
    transitions: Section,
    sets: Section,
    intervals: Section,
    token_bits: Section,
    decisions: Section,
    rule_starts: Section,
    rule_stops: Section,
    total_len: usize,
}

impl EncodedLayout {
    fn new(builder: &ParserAtnBuilder) -> Result<Self, ParserAtnError> {
        let mut cursor = HEADER_WORDS;
        let states = next_section(&mut cursor, builder.states.len(), STATE_WORDS, "states")?;
        let transitions = next_section(
            &mut cursor,
            builder.transitions.len(),
            TRANSITION_WORDS,
            "transitions",
        )?;
        let sets = next_section(
            &mut cursor,
            builder.interval_sets.len(),
            SET_WORDS,
            "interval sets",
        )?;
        let intervals = next_section(
            &mut cursor,
            builder.interval_ranges.len(),
            2,
            "interval ranges",
        )?;
        let token_bits = next_section(
            &mut cursor,
            builder.token_bit_words.len(),
            PACKED_U64_WORDS,
            "token-set bits",
        )?;
        let decisions = next_section(&mut cursor, builder.decisions.len(), 1, "decisions")?;
        let rule_starts = next_section(&mut cursor, builder.rule_starts.len(), 1, "rule starts")?;
        let rule_stops = next_section(&mut cursor, builder.rule_stops.len(), 1, "rule stops")?;
        compact_id("packed parser ATN word", cursor)?;
        Ok(Self {
            states,
            transitions,
            sets,
            intervals,
            token_bits,
            decisions,
            rule_starts,
            rule_stops,
            total_len: cursor,
        })
    }
}

#[derive(Clone, Debug)]
struct StateBuild {
    kind: AtnStateKind,
    rule_index: u32,
    flags: u32,
    end_state: u32,
    loop_back_state: u32,
}

#[derive(Clone, Debug)]
struct TokenSetBuild {
    interval_start: u32,
    interval_len: u32,
    kind: ParserTokenSetKind,
    bit_start: u32,
    bit_len: u32,
}

#[derive(Debug)]
struct PreparedTokenSet {
    kind: ParserTokenSetKind,
    words: Vec<u64>,
}

#[derive(Clone, Debug)]
struct TransitionBuild {
    source: AtnStateId,
    kind: ParserTransitionKind,
    target: AtnStateId,
    arg0: u32,
    arg1: u32,
    arg2: u32,
}

impl TransitionBuild {
    const fn spec(&self) -> ParserTransitionSpec {
        let target = self.target.index();
        match self.kind {
            ParserTransitionKind::Epsilon => ParserTransitionSpec::Epsilon { target },
            ParserTransitionKind::Atom => ParserTransitionSpec::Atom {
                target,
                label: unpack_i32(self.arg0),
            },
            ParserTransitionKind::Range => ParserTransitionSpec::Range {
                target,
                start: unpack_i32(self.arg0),
                stop: unpack_i32(self.arg1),
            },
            ParserTransitionKind::Set => ParserTransitionSpec::Set {
                target,
                set: ParserIntervalSetId(self.arg0),
            },
            ParserTransitionKind::NotSet => ParserTransitionSpec::NotSet {
                target,
                set: ParserIntervalSetId(self.arg0),
            },
            ParserTransitionKind::Wildcard => ParserTransitionSpec::Wildcard { target },
            ParserTransitionKind::Rule => ParserTransitionSpec::Rule {
                target,
                rule_index: self.arg0 as usize,
                follow_state: self.arg1 as usize,
                precedence: unpack_i32(self.arg2),
            },
            ParserTransitionKind::Predicate => ParserTransitionSpec::Predicate {
                target,
                rule_index: self.arg0 as usize,
                pred_index: self.arg1 as usize,
                context_dependent: self.arg2 != 0,
            },
            ParserTransitionKind::Action => ParserTransitionSpec::Action {
                target,
                rule_index: self.arg0 as usize,
                action_index: unpack_index(self.arg1),
                context_dependent: self.arg2 != 0,
            },
            ParserTransitionKind::Precedence => ParserTransitionSpec::Precedence {
                target,
                precedence: unpack_i32(self.arg0),
            },
        }
    }
}

impl ParserTransitionKind {
    const fn is_epsilon(self) -> bool {
        matches!(
            self,
            Self::Epsilon | Self::Rule | Self::Predicate | Self::Action | Self::Precedence
        )
    }

    const fn is_consuming(self) -> bool {
        matches!(
            self,
            Self::Atom | Self::Range | Self::Set | Self::NotSet | Self::Wildcard
        )
    }

    const fn is_semantic(self) -> bool {
        matches!(self, Self::Predicate | Self::Action | Self::Precedence)
    }
}

fn validate_packed(words: &[u32]) -> Result<ParserAtnLayout, ParserAtnError> {
    validate_header(words)?;
    let layout = read_layout(words)?;
    validate_sections(words, layout)?;
    validate_states(words, layout)?;
    validate_transitions(words, layout)?;
    validate_state_flags(words, layout)?;
    validate_sets(words, layout)?;
    validate_side_tables(words, layout)?;
    Ok(layout)
}

fn validate_header(words: &[u32]) -> Result<(), ParserAtnError> {
    if words.len() < LEGACY_HEADER_WORDS {
        return Err(ParserAtnError::InvalidData(format!(
            "header has {} words; expected at least {LEGACY_HEADER_WORDS}",
            words.len()
        )));
    }
    if words[HEADER_MAGIC] != PARSER_ATN_MAGIC {
        return Err(ParserAtnError::InvalidData(format!(
            "magic 0x{:08x}; expected 0x{PARSER_ATN_MAGIC:08x}",
            words[HEADER_MAGIC]
        )));
    }
    let version = words[HEADER_VERSION];
    if !(PARSER_ATN_MIN_FORMAT_VERSION..=PARSER_ATN_MAX_FORMAT_VERSION).contains(&version) {
        return Err(ParserAtnError::UnsupportedVersion {
            found: version,
            minimum: PARSER_ATN_MIN_FORMAT_VERSION,
            maximum: PARSER_ATN_MAX_FORMAT_VERSION,
        });
    }
    let header_words = if version == 1 {
        LEGACY_HEADER_WORDS
    } else {
        HEADER_WORDS
    };
    if words.len() < header_words {
        return Err(ParserAtnError::InvalidData(format!(
            "format {version} header has {} words; expected at least {header_words}",
            words.len()
        )));
    }
    if words[HEADER_BYTE_ORDER] != PARSER_ATN_BYTE_ORDER {
        return Err(ParserAtnError::InvalidData(format!(
            "byte-order marker 0x{:08x}; expected 0x{PARSER_ATN_BYTE_ORDER:08x}",
            words[HEADER_BYTE_ORDER]
        )));
    }
    if words[HEADER_SIZE] as usize != header_words {
        return Err(ParserAtnError::InvalidData(format!(
            "format {version} header length {}; expected {header_words}",
            words[HEADER_SIZE],
        )));
    }
    if words[HEADER_TOTAL_LEN] as usize != words.len() {
        return Err(ParserAtnError::InvalidData(format!(
            "declared total length {} does not match {} words",
            words[HEADER_TOTAL_LEN],
            words.len()
        )));
    }
    Ok(())
}

fn read_layout(words: &[u32]) -> Result<ParserAtnLayout, ParserAtnError> {
    let format_version = words[HEADER_VERSION];
    let set_words = if format_version == 1 {
        LEGACY_SET_WORDS
    } else {
        SET_WORDS
    };
    let states = read_section(words, HEADER_STATES_OFFSET)?;
    let transitions = read_section(words, HEADER_TRANSITIONS_OFFSET)?;
    let sets = read_section(words, HEADER_SETS_OFFSET)?;
    let intervals = read_section(words, HEADER_INTERVALS_OFFSET)?;
    let token_bits = if format_version == 1 {
        Section {
            offset: intervals.offset + intervals.len,
            len: 0,
        }
    } else {
        read_section(words, HEADER_TOKEN_BITS_OFFSET)?
    };
    let decisions = read_section(words, HEADER_DECISIONS_OFFSET)?;
    let rule_starts = read_section(words, HEADER_RULE_STARTS_OFFSET)?;
    let rule_stops = read_section(words, HEADER_RULE_STOPS_OFFSET)?;
    let state_count = words[HEADER_STATE_COUNT] as usize;
    let transition_count = words[HEADER_TRANSITION_COUNT] as usize;
    expect_section_len("states", states, state_count, STATE_WORDS)?;
    expect_section_len(
        "transitions",
        transitions,
        transition_count,
        TRANSITION_WORDS,
    )?;
    expect_section_len(
        "interval sets",
        sets,
        words[HEADER_SET_COUNT] as usize,
        set_words,
    )?;
    expect_section_len(
        "intervals",
        intervals,
        words[HEADER_INTERVAL_COUNT] as usize,
        2,
    )?;
    if format_version != 1 {
        expect_section_len(
            "token-set bits",
            token_bits,
            words[HEADER_TOKEN_BIT_WORD_COUNT] as usize,
            PACKED_U64_WORDS,
        )?;
    }
    expect_section_len(
        "decisions",
        decisions,
        words[HEADER_DECISION_COUNT] as usize,
        1,
    )?;
    expect_section_len(
        "rule starts",
        rule_starts,
        words[HEADER_RULE_COUNT] as usize,
        1,
    )?;
    expect_section_len(
        "rule stops",
        rule_stops,
        words[HEADER_RULE_COUNT] as usize,
        1,
    )?;
    Ok(ParserAtnLayout {
        format_version,
        max_token_type: unpack_i32(words[HEADER_MAX_TOKEN_TYPE]),
        state_count,
        transition_count,
        set_words,
        states,
        transitions,
        sets,
        intervals,
        token_bits,
        decisions,
        rule_starts,
        rule_stops,
    })
}

fn validate_sections(words: &[u32], layout: ParserAtnLayout) -> Result<(), ParserAtnError> {
    let sections = [
        ("states", layout.states),
        ("transitions", layout.transitions),
        ("sets", layout.sets),
        ("intervals", layout.intervals),
        ("token-set bits", layout.token_bits),
        ("decisions", layout.decisions),
        ("rule starts", layout.rule_starts),
        ("rule stops", layout.rule_stops),
    ];
    let mut expected_offset = if layout.format_version == 1 {
        LEGACY_HEADER_WORDS
    } else {
        HEADER_WORDS
    };
    for (name, section) in sections {
        if section.offset != expected_offset {
            return Err(ParserAtnError::InvalidData(format!(
                "{name} section starts at {}, expected {expected_offset}",
                section.offset
            )));
        }
        expected_offset = section_end(section, words.len(), name)?;
    }
    if expected_offset != words.len() {
        return Err(ParserAtnError::InvalidData(format!(
            "sections end at {expected_offset}, stream ends at {}",
            words.len()
        )));
    }
    Ok(())
}

fn validate_states(words: &[u32], layout: ParserAtnLayout) -> Result<(), ParserAtnError> {
    let mut transition_cursor = 0;
    for state in 0..layout.state_count {
        let base = layout.states.offset + state * STATE_WORDS;
        decode_state_kind(words[base])?;
        let flags = words[base + 2];
        if flags & !STATE_FLAGS != 0 {
            return Err(ParserAtnError::InvalidData(format!(
                "state {state} has unknown flags 0x{:x}",
                flags & !STATE_FLAGS
            )));
        }
        validate_optional_index(words[base + 1], layout.rule_starts.len, "state rule index")?;
        let transition_start = words[base + 3] as usize;
        if transition_start != transition_cursor {
            return Err(ParserAtnError::InvalidData(format!(
                "state {state} transition range starts at {transition_start}, expected {transition_cursor}"
            )));
        }
        validate_range(
            words[base + 3],
            words[base + 4],
            layout.transition_count,
            "state transition",
        )?;
        transition_cursor += words[base + 4] as usize;
        validate_optional_index(words[base + 5], layout.state_count, "block end state")?;
        validate_optional_index(words[base + 6], layout.state_count, "loop back state")?;
    }
    if transition_cursor != layout.transition_count {
        return Err(ParserAtnError::InvalidData(format!(
            "state transition ranges cover {transition_cursor} transitions; expected {}",
            layout.transition_count
        )));
    }
    Ok(())
}

fn validate_transitions(words: &[u32], layout: ParserAtnLayout) -> Result<(), ParserAtnError> {
    for transition in 0..layout.transition_count {
        let base = layout.transitions.offset + transition * TRANSITION_WORDS;
        let kind = decode_transition_kind(words[base])?;
        validate_index(words[base + 1], layout.state_count, "transition target")?;
        match kind {
            ParserTransitionKind::Range => {
                let start = unpack_i32(words[base + 2]);
                let stop = unpack_i32(words[base + 3]);
                if start > stop {
                    return Err(ParserAtnError::InvalidData(format!(
                        "transition {transition} range starts at {start} after stop {stop}"
                    )));
                }
            }
            ParserTransitionKind::Set | ParserTransitionKind::NotSet => {
                validate_index(
                    words[base + 2],
                    layout.sets.len / layout.set_words,
                    "interval set",
                )?;
            }
            ParserTransitionKind::Rule => {
                validate_index(words[base + 2], layout.rule_starts.len, "rule index")?;
                validate_index(words[base + 3], layout.state_count, "rule follow state")?;
            }
            ParserTransitionKind::Predicate => {
                validate_index(words[base + 2], layout.rule_starts.len, "predicate rule")?;
                validate_bool(words[base + 4], "predicate context-dependent flag")?;
            }
            ParserTransitionKind::Action => {
                validate_index(words[base + 2], layout.rule_starts.len, "action rule")?;
                validate_bool(words[base + 4], "action context-dependent flag")?;
            }
            ParserTransitionKind::Epsilon
            | ParserTransitionKind::Atom
            | ParserTransitionKind::Wildcard
            | ParserTransitionKind::Precedence => {}
        }
    }
    Ok(())
}

fn validate_state_flags(words: &[u32], layout: ParserAtnLayout) -> Result<(), ParserAtnError> {
    for state in 0..layout.state_count {
        let base = layout.states.offset + state * STATE_WORDS;
        let kind = decode_state_kind(words[base])?;
        let start = words[base + 3] as usize;
        let len = words[base + 4] as usize;
        let mut all_epsilon = len != 0;
        let mut has_consuming = false;
        let mut has_semantic = false;
        for transition in start..start + len {
            let base = layout.transitions.offset + transition * TRANSITION_WORDS;
            let kind = decode_transition_kind(words[base])
                .expect("packed parser transition kind was already validated");
            all_epsilon &= kind.is_epsilon();
            has_consuming |= kind.is_consuming();
            has_semantic |= kind.is_semantic();
        }
        let mut expected = u32::from(kind == AtnStateKind::RuleStop) * FLAG_RULE_STOP;
        expected |= u32::from(all_epsilon) * FLAG_EPSILON_ONLY;
        expected |= u32::from(has_consuming) * FLAG_HAS_CONSUMING;
        expected |= u32::from(has_semantic) * FLAG_HAS_SEMANTIC;
        let derived = words[base + 2]
            & (FLAG_EPSILON_ONLY | FLAG_RULE_STOP | FLAG_HAS_CONSUMING | FLAG_HAS_SEMANTIC);
        if derived != expected {
            return Err(ParserAtnError::InvalidData(format!(
                "state {state} has inconsistent precomputed flags 0x{derived:x}; expected 0x{expected:x}"
            )));
        }
    }
    Ok(())
}

fn validate_sets(words: &[u32], layout: ParserAtnLayout) -> Result<(), ParserAtnError> {
    let set_count = layout.sets.len / layout.set_words;
    let mut bit_cursor = 0;
    for set in 0..set_count {
        let base = layout.sets.offset + set * layout.set_words;
        validate_range(
            words[base],
            words[base + 1],
            layout.intervals.len / 2,
            "interval set",
        )?;
        let start = words[base] as usize;
        let len = words[base + 1] as usize;
        let mut previous_stop: Option<i32> = None;
        for interval in start..start + len {
            let interval_base = layout.intervals.offset + interval * 2;
            let range_start = unpack_i32(words[interval_base]);
            let range_stop = unpack_i32(words[interval_base + 1]);
            if range_start > range_stop {
                return Err(ParserAtnError::InvalidData(format!(
                    "interval {interval} starts at {range_start} after stop {range_stop}"
                )));
            }
            if previous_stop.is_some_and(|stop| range_start <= stop.saturating_add(1)) {
                return Err(ParserAtnError::InvalidData(format!(
                    "interval set {set} is not sorted and coalesced"
                )));
            }
            previous_stop = Some(range_stop);
        }
        if layout.format_version == 1 {
            continue;
        }
        let kind = decode_token_set_kind(words[base + 2])?;
        let bit_start = words[base + 3];
        let bit_len = words[base + 4];
        if bit_start as usize != bit_cursor {
            return Err(ParserAtnError::InvalidData(format!(
                "parser token set {set} bit range starts at {bit_start}, expected {bit_cursor}"
            )));
        }
        validate_range(
            bit_start,
            bit_len,
            layout.token_bits.len / PACKED_U64_WORDS,
            "parser token-set bits",
        )?;
        let (expected_kind, expected_bit_len) =
            token_set_shape((start..start + len).map(|interval| {
                let interval_base = layout.intervals.offset + interval * 2;
                (
                    unpack_i32(words[interval_base]),
                    unpack_i32(words[interval_base + 1]),
                )
            }));
        if kind != expected_kind || bit_len as usize != expected_bit_len {
            return Err(ParserAtnError::InvalidData(format!(
                "parser token set {set} uses {kind:?} with {bit_len} words; \
                 expected {expected_kind:?} with {expected_bit_len} words"
            )));
        }
        for bit_word in 0..expected_bit_len {
            let expected = expected_token_set_word(
                (start..start + len).map(|interval| {
                    let interval_base = layout.intervals.offset + interval * 2;
                    (
                        unpack_i32(words[interval_base]),
                        unpack_i32(words[interval_base + 1]),
                    )
                }),
                bit_word,
            );
            let actual = packed_u64(words, layout.token_bits, bit_cursor + bit_word);
            if actual != expected {
                return Err(ParserAtnError::InvalidData(format!(
                    "parser token set {set} bit word {bit_word} is 0x{actual:016x}; \
                     expected 0x{expected:016x}"
                )));
            }
        }
        bit_cursor += expected_bit_len;
    }
    if layout.format_version != 1 && bit_cursor != layout.token_bits.len / PACKED_U64_WORDS {
        return Err(ParserAtnError::InvalidData(format!(
            "parser token sets cover {bit_cursor} bit words; expected {}",
            layout.token_bits.len / PACKED_U64_WORDS
        )));
    }
    Ok(())
}

fn validate_side_tables(words: &[u32], layout: ParserAtnLayout) -> Result<(), ParserAtnError> {
    for (name, section) in [
        ("decision state", layout.decisions),
        ("rule start state", layout.rule_starts),
        ("rule stop state", layout.rule_stops),
    ] {
        for &state in &words[section.offset..section.offset + section.len] {
            validate_index(state, layout.state_count, name)?;
        }
    }
    Ok(())
}

#[inline(always)]
fn decode_state_kind(value: u32) -> Result<AtnStateKind, ParserAtnError> {
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
        other => {
            return Err(ParserAtnError::InvalidData(format!(
                "parser ATN state kind {other}"
            )));
        }
    };
    Ok(kind)
}

#[inline(always)]
fn decode_transition_kind(value: u32) -> Result<ParserTransitionKind, ParserAtnError> {
    let kind = match value {
        1 => ParserTransitionKind::Epsilon,
        2 => ParserTransitionKind::Range,
        3 => ParserTransitionKind::Rule,
        4 => ParserTransitionKind::Predicate,
        5 => ParserTransitionKind::Atom,
        6 => ParserTransitionKind::Action,
        7 => ParserTransitionKind::Set,
        8 => ParserTransitionKind::NotSet,
        9 => ParserTransitionKind::Wildcard,
        10 => ParserTransitionKind::Precedence,
        other => {
            return Err(ParserAtnError::InvalidData(format!(
                "parser ATN transition kind {other}"
            )));
        }
    };
    Ok(kind)
}

fn decode_token_set_kind(value: u32) -> Result<ParserTokenSetKind, ParserAtnError> {
    match value {
        0 => Ok(ParserTokenSetKind::Intervals),
        1 => Ok(ParserTokenSetKind::Inline128),
        2 => Ok(ParserTokenSetKind::Dense),
        other => Err(ParserAtnError::InvalidData(format!(
            "parser token-set kind {other}"
        ))),
    }
}

const fn state_kind_word(kind: AtnStateKind) -> u32 {
    match kind {
        AtnStateKind::Invalid => 0,
        AtnStateKind::Basic => 1,
        AtnStateKind::RuleStart => 2,
        AtnStateKind::BlockStart => 3,
        AtnStateKind::PlusBlockStart => 4,
        AtnStateKind::StarBlockStart => 5,
        AtnStateKind::TokenStart => 6,
        AtnStateKind::RuleStop => 7,
        AtnStateKind::BlockEnd => 8,
        AtnStateKind::StarLoopBack => 9,
        AtnStateKind::StarLoopEntry => 10,
        AtnStateKind::PlusLoopBack => 11,
        AtnStateKind::LoopEnd => 12,
    }
}

fn compact_id(field: &'static str, value: usize) -> Result<u32, ParserAtnError> {
    u32::try_from(value).map_err(|_| ParserAtnError::Overflow { field, value })
}

fn pack_optional_index(field: &'static str, value: Option<usize>) -> Result<u32, ParserAtnError> {
    match value {
        Some(value) => {
            let compact = compact_id(field, value)?;
            if compact == NO_INDEX {
                return Err(ParserAtnError::Overflow { field, value });
            }
            Ok(compact)
        }
        None => Ok(NO_INDEX),
    }
}

const fn unpack_index(value: u32) -> Option<usize> {
    if value == NO_INDEX {
        None
    } else {
        Some(value as usize)
    }
}

const fn pack_i32(value: i32) -> u32 {
    u32::from_le_bytes(value.to_le_bytes())
}

const fn unpack_i32(value: u32) -> i32 {
    i32::from_le_bytes(value.to_le_bytes())
}

fn normalize_ranges(ranges: impl IntoIterator<Item = (i32, i32)>) -> Vec<(i32, i32)> {
    let mut ranges = ranges
        .into_iter()
        .map(|(start, stop)| {
            if start <= stop {
                (start, stop)
            } else {
                (stop, start)
            }
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    let mut normalized: Vec<(i32, i32)> = Vec::with_capacity(ranges.len());
    for (start, stop) in ranges {
        if let Some((_, previous_stop)) = normalized.last_mut()
            && start <= previous_stop.saturating_add(1)
        {
            *previous_stop = (*previous_stop).max(stop);
            continue;
        }
        normalized.push((start, stop));
    }
    normalized
}

/// Selects token-set storage without allocating from an unchecked maximum.
///
/// Every compatible set at or below token slot 127 uses two inline words.
/// Larger sets use dense words only when the payload is at most 64 KiB and
/// either no larger than interval storage, or at most twice that storage while
/// covering at least one eighth of the indexed domain. Sparse, malformed, and
/// very large domains retain normalized interval lookup.
fn token_set_shape(ranges: impl IntoIterator<Item = (i32, i32)>) -> (ParserTokenSetKind, usize) {
    let mut compatible = true;
    let mut max_slot = 0;
    let mut represented = 0_u64;
    let mut range_count = 0_usize;
    for (start, stop) in ranges {
        range_count += 1;
        represented = represented.saturating_add(
            u64::try_from(i64::from(stop) - i64::from(start) + 1).unwrap_or(u64::MAX),
        );
        if start == TOKEN_EOF && stop == TOKEN_EOF {
            continue;
        }
        if start < 1 {
            compatible = false;
            continue;
        }
        let stop = usize::try_from(stop).expect("positive i32 token type fits usize");
        max_slot = max_slot.max(stop);
    }
    if !compatible {
        return (ParserTokenSetKind::Intervals, 0);
    }
    if max_slot <= INLINE_TOKEN_SET_MAX_SLOT {
        return (ParserTokenSetKind::Inline128, INLINE_TOKEN_SET_WORDS);
    }
    let word_len = max_slot / u64::BITS as usize + 1;
    let Some(dense_bytes) = word_len.checked_mul(size_of::<u64>()) else {
        return (ParserTokenSetKind::Intervals, 0);
    };
    let interval_bytes = range_count.saturating_mul(size_of::<(i32, i32)>());
    let dense_enough = represented.saturating_mul(DENSE_TOKEN_SET_MIN_DENSITY_DENOMINATOR)
        >= u64::try_from(max_slot)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
    let cost_effective = dense_bytes <= interval_bytes
        || (dense_bytes <= interval_bytes.saturating_mul(DENSE_TOKEN_SET_COST_MULTIPLIER)
            && dense_enough);
    if word_len <= MAX_DENSE_TOKEN_SET_WORDS && cost_effective {
        (ParserTokenSetKind::Dense, word_len)
    } else {
        (ParserTokenSetKind::Intervals, 0)
    }
}

fn prepare_token_set(ranges: &[(i32, i32)]) -> PreparedTokenSet {
    let (kind, word_len) = token_set_shape(ranges.iter().copied());
    let mut words = vec![0; word_len];
    for &(start, stop) in ranges {
        insert_token_set_range(&mut words, start, stop);
    }
    PreparedTokenSet { kind, words }
}

fn insert_token_set_range(words: &mut [u64], start: i32, stop: i32) {
    if words.is_empty() {
        return;
    }
    if start == TOKEN_EOF && stop == TOKEN_EOF {
        words[0] |= 1;
        return;
    }
    debug_assert!(start >= 1 && stop >= start);
    let start = usize::try_from(start).expect("positive i32 token type fits usize");
    let stop = usize::try_from(stop).expect("positive i32 token type fits usize");
    let start_word = start / u64::BITS as usize;
    let stop_word = stop / u64::BITS as usize;
    if start_word == stop_word {
        words[start_word] |= token_word_mask(start % u64::BITS as usize, stop % u64::BITS as usize);
        return;
    }
    words[start_word] |= !0_u64 << (start % u64::BITS as usize);
    words[(start_word + 1)..stop_word].fill(!0);
    words[stop_word] |= !0_u64 >> (u64::BITS as usize - 1 - stop % u64::BITS as usize);
}

fn expected_token_set_word(ranges: impl IntoIterator<Item = (i32, i32)>, word_index: usize) -> u64 {
    let word_start = word_index * u64::BITS as usize;
    let word_stop = word_start + u64::BITS as usize - 1;
    let mut expected = 0;
    for (start, stop) in ranges {
        if start == TOKEN_EOF && stop == TOKEN_EOF {
            if word_index == 0 {
                expected |= 1;
            }
            continue;
        }
        let start = usize::try_from(start).expect("positive i32 token type fits usize");
        let stop = usize::try_from(stop).expect("positive i32 token type fits usize");
        if stop < word_start || start > word_stop {
            continue;
        }
        expected |= token_word_mask(
            start.max(word_start) - word_start,
            stop.min(word_stop) - word_start,
        );
    }
    expected
}

const fn token_word_mask(start: usize, stop: usize) -> u64 {
    (!0_u64 << start) & (!0_u64 >> (u64::BITS as usize - 1 - stop))
}

fn packed_u64(words: &[u32], section: Section, index: usize) -> u64 {
    let offset = section.offset + index * PACKED_U64_WORDS;
    u64::from(words[offset]) | (u64::from(words[offset + 1]) << u32::BITS)
}

fn token_set_slot(value: i32) -> Option<usize> {
    if value == TOKEN_EOF {
        Some(0)
    } else if value > 0 {
        usize::try_from(value).ok()
    } else {
        None
    }
}

fn next_section(
    cursor: &mut usize,
    count: usize,
    width: usize,
    name: &str,
) -> Result<Section, ParserAtnError> {
    let len = count.checked_mul(width).ok_or_else(|| {
        ParserAtnError::InvalidData(format!("{name} section length overflows usize"))
    })?;
    let section = Section {
        offset: *cursor,
        len,
    };
    *cursor = cursor.checked_add(len).ok_or_else(|| {
        ParserAtnError::InvalidData(format!("{name} section end overflows usize"))
    })?;
    Ok(section)
}

fn write_section(
    words: &mut [u32],
    header_offset: usize,
    section: Section,
) -> Result<(), ParserAtnError> {
    words[header_offset] = compact_id("parser ATN section offset", section.offset)?;
    words[header_offset + 1] = compact_id("parser ATN section length", section.len)?;
    Ok(())
}

fn encode_ids(words: &mut [u32], section: Section, ids: &[AtnStateId]) {
    for (target, id) in words[section.offset..section.offset + section.len]
        .iter_mut()
        .zip(ids)
    {
        *target = id.raw();
    }
}

fn read_section(words: &[u32], header_offset: usize) -> Result<Section, ParserAtnError> {
    let offset = words[header_offset] as usize;
    let len = words[header_offset + 1] as usize;
    section_end(Section { offset, len }, words.len(), "declared")?;
    Ok(Section { offset, len })
}

fn section_end(section: Section, total: usize, name: &str) -> Result<usize, ParserAtnError> {
    let end = section.offset.checked_add(section.len).ok_or_else(|| {
        ParserAtnError::InvalidData(format!("{name} section offset arithmetic overflow"))
    })?;
    if end > total {
        return Err(ParserAtnError::InvalidData(format!(
            "{name} section {0}..{end} exceeds stream length {total}",
            section.offset
        )));
    }
    Ok(end)
}

fn expect_section_len(
    name: &str,
    section: Section,
    count: usize,
    width: usize,
) -> Result<(), ParserAtnError> {
    let expected = count.checked_mul(width).ok_or_else(|| {
        ParserAtnError::InvalidData(format!("{name} count/width multiplication overflow"))
    })?;
    if section.len != expected {
        return Err(ParserAtnError::InvalidData(format!(
            "{name} section has {} words; expected {expected}",
            section.len
        )));
    }
    Ok(())
}

fn validate_index(value: u32, count: usize, name: &str) -> Result<(), ParserAtnError> {
    if value as usize >= count {
        return Err(ParserAtnError::InvalidData(format!(
            "{name} {value} outside 0..{count}"
        )));
    }
    Ok(())
}

fn validate_optional_index(value: u32, count: usize, name: &str) -> Result<(), ParserAtnError> {
    if value == NO_INDEX {
        return Ok(());
    }
    validate_index(value, count, name)
}

fn validate_bool(value: u32, name: &str) -> Result<(), ParserAtnError> {
    if value > 1 {
        return Err(ParserAtnError::InvalidData(format!(
            "{name} is {value}; expected 0 or 1"
        )));
    }
    Ok(())
}

fn validate_range(start: u32, len: u32, count: usize, name: &str) -> Result<(), ParserAtnError> {
    let start = start as usize;
    let len = len as usize;
    let end = start
        .checked_add(len)
        .ok_or_else(|| ParserAtnError::InvalidData(format!("{name} range arithmetic overflow")))?;
    if end > count {
        return Err(ParserAtnError::InvalidData(format!(
            "{name} range {start}..{end} exceeds count {count}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_atn() -> ParserAtn {
        let mut builder = ParserAtnBuilder::new(9);
        builder
            .add_state(AtnStateKind::RuleStart, Some(0))
            .expect("rule start");
        builder
            .add_state(AtnStateKind::RuleStop, Some(0))
            .expect("rule stop");
        builder
            .set_rule_to_start_state(vec![0])
            .expect("rule starts");
        builder.set_rule_to_stop_state(vec![1]).expect("rule stops");
        builder.add_decision_state(0).expect("decision");
        builder
            .add_transition(
                0,
                ParserTransitionSpec::Atom {
                    target: 1,
                    label: 7,
                },
            )
            .expect("transition");
        builder.finish().expect("packed parser ATN")
    }

    fn token_set_atn(max_token_type: i32, ranges: &[(i32, i32)]) -> ParserAtn {
        let mut builder = ParserAtnBuilder::new(max_token_type);
        builder
            .add_interval_set(ranges.iter().copied())
            .expect("token set");
        builder.finish().expect("packed parser ATN")
    }

    fn legacy_words(atn: &ParserAtn) -> Vec<u32> {
        let source = atn.packed_words();
        let source_layout = atn.layout;
        let set_count = source_layout.sets.len / source_layout.set_words;
        let mut cursor = LEGACY_HEADER_WORDS;
        let states = next_section(
            &mut cursor,
            source_layout.state_count,
            STATE_WORDS,
            "states",
        )
        .expect("legacy states");
        let transitions = next_section(
            &mut cursor,
            source_layout.transition_count,
            TRANSITION_WORDS,
            "transitions",
        )
        .expect("legacy transitions");
        let sets =
            next_section(&mut cursor, set_count, LEGACY_SET_WORDS, "sets").expect("legacy sets");
        let intervals = next_section(&mut cursor, source_layout.intervals.len / 2, 2, "intervals")
            .expect("legacy intervals");
        let decisions = next_section(&mut cursor, source_layout.decisions.len, 1, "decisions")
            .expect("legacy decisions");
        let rule_starts =
            next_section(&mut cursor, source_layout.rule_starts.len, 1, "rule starts")
                .expect("legacy rule starts");
        let rule_stops = next_section(&mut cursor, source_layout.rule_stops.len, 1, "rule stops")
            .expect("legacy rule stops");
        let mut words = vec![0; cursor];
        words[..=HEADER_RULE_COUNT].copy_from_slice(&source[..=HEADER_RULE_COUNT]);
        words[HEADER_VERSION] = 1;
        words[HEADER_SIZE] = LEGACY_HEADER_WORDS as u32;
        write_section(&mut words, HEADER_STATES_OFFSET, states).expect("states header");
        write_section(&mut words, HEADER_TRANSITIONS_OFFSET, transitions)
            .expect("transitions header");
        write_section(&mut words, HEADER_SETS_OFFSET, sets).expect("sets header");
        write_section(&mut words, HEADER_INTERVALS_OFFSET, intervals).expect("intervals header");
        write_section(&mut words, HEADER_DECISIONS_OFFSET, decisions).expect("decisions header");
        write_section(&mut words, HEADER_RULE_STARTS_OFFSET, rule_starts)
            .expect("rule starts header");
        write_section(&mut words, HEADER_RULE_STOPS_OFFSET, rule_stops).expect("rule stops header");
        words[HEADER_TOTAL_LEN] = cursor as u32;
        for (target, section) in [
            (states, source_layout.states),
            (transitions, source_layout.transitions),
            (intervals, source_layout.intervals),
            (decisions, source_layout.decisions),
            (rule_starts, source_layout.rule_starts),
            (rule_stops, source_layout.rule_stops),
        ] {
            words[target.offset..target.offset + target.len]
                .copy_from_slice(&source[section.offset..section.offset + section.len]);
        }
        for set in 0..set_count {
            let source_base = source_layout.sets.offset + set * source_layout.set_words;
            let target_base = sets.offset + set * LEGACY_SET_WORDS;
            words[target_base..target_base + LEGACY_SET_WORDS]
                .copy_from_slice(&source[source_base..source_base + LEGACY_SET_WORDS]);
        }
        words
    }

    #[test]
    fn packed_views_preserve_state_and_transition_semantics() {
        let atn = sample_atn();
        let start = atn.state(0).expect("start");
        assert_eq!(start.kind(), AtnStateKind::RuleStart);
        assert_eq!(start.rule_index(), Some(0));
        assert!(start.has_consuming_transition());
        let transition = start.transitions().first().expect("transition");
        assert_eq!(
            transition.data(),
            ParserTransitionData::Atom {
                target: 1,
                label: 7
            }
        );
        assert!(transition.matches(7, 1, 9));
        assert!(!transition.matches(8, 1, 9));
        assert_eq!(atn.rule_to_stop_state().get(0), Some(1));
    }

    #[test]
    fn static_format_is_allocation_free_and_version_checked() {
        let atn = sample_atn();
        let words = Box::leak(atn.packed_words().to_vec().into_boxed_slice());
        let borrowed = ParserAtn::from_static(words).expect("static packed ATN");
        assert!(matches!(borrowed.words, Cow::Borrowed(_)));

        let mut wrong_version = words.to_vec();
        wrong_version[HEADER_VERSION] = PARSER_ATN_FORMAT_VERSION + 1;
        assert_eq!(
            ParserAtn::from_owned(wrong_version),
            Err(ParserAtnError::UnsupportedVersion {
                found: 3,
                minimum: 1,
                maximum: 2,
            })
        );
    }

    #[test]
    fn legacy_interval_format_remains_readable() {
        let current = token_set_atn(200, &[(TOKEN_EOF, TOKEN_EOF), (2, 8), (150, 150)]);
        let legacy = ParserAtn::from_owned(legacy_words(&current)).expect("legacy packed ATN");
        let set = legacy.token_set(0).expect("legacy token set");

        assert_eq!(legacy.format_version(), 1);
        assert_eq!(set.kind(), ParserTokenSetKind::Intervals);
        assert_eq!(
            set.ranges().collect::<Vec<_>>(),
            [(TOKEN_EOF, TOKEN_EOF), (2, 8), (150, 150)]
        );
        assert!(set.contains(TOKEN_EOF));
        assert!(set.contains(6));
        assert!(set.contains(150));
        assert!(!set.contains(149));
    }

    #[test]
    fn adaptive_token_sets_cover_boundaries_and_safe_fallbacks() {
        let inline = token_set_atn(127, &[(TOKEN_EOF, TOKEN_EOF), (1, 1), (63, 64), (127, 127)]);
        let inline = inline.token_set(0).expect("inline set");
        assert_eq!(inline.kind(), ParserTokenSetKind::Inline128);
        for token in [TOKEN_EOF, 1, 63, 64, 127] {
            assert!(inline.contains(token), "missing token {token}");
        }
        for token in [-2, 0, 2, 62, 65, 126, 128] {
            assert!(!inline.contains(token), "unexpected token {token}");
        }

        let singleton_atn = token_set_atn(127, &[(42, 42)]);
        let singleton = singleton_atn.token_set(0).expect("singleton set");
        assert_eq!(singleton.kind(), ParserTokenSetKind::Inline128);
        assert!(singleton.contains(42));
        assert!(!singleton.contains(41));
        assert!(!singleton.contains(43));

        let dense_ranges = (1..=512)
            .step_by(2)
            .map(|token| (token, token))
            .collect::<Vec<_>>();
        let dense_atn = token_set_atn(512, &dense_ranges);
        let dense = dense_atn.token_set(0).expect("dense set");
        assert_eq!(dense.kind(), ParserTokenSetKind::Dense);
        assert!(dense.contains(511));
        assert!(!dense.contains(512));

        let at_cap_max =
            i32::try_from(MAX_DENSE_TOKEN_SET_WORDS * u64::BITS as usize - 1).expect("test bound");
        assert_eq!(
            token_set_shape((1..=at_cap_max).step_by(2).map(|token| (token, token))),
            (ParserTokenSetKind::Dense, MAX_DENSE_TOKEN_SET_WORDS)
        );
        let over_cap_max = at_cap_max + 1;
        assert_eq!(
            token_set_shape(
                (1..=over_cap_max)
                    .step_by(2)
                    .map(|token| (token, token))
                    .chain([(over_cap_max, over_cap_max)])
            ),
            (ParserTokenSetKind::Intervals, 0)
        );

        for ranges in [
            vec![(1, 1), (1_000_000, 1_000_000)],
            vec![(1, 1), (i32::MAX, i32::MAX)],
            vec![(-2, -2), (1, 4)],
            vec![(0, 4)],
        ] {
            let atn = token_set_atn(i32::MAX, &ranges);
            let set = atn.token_set(0).expect("interval set");
            assert_eq!(set.kind(), ParserTokenSetKind::Intervals, "{ranges:?}");
            assert_eq!(atn.stats().token_bitset_bytes, 0);
            for &(start, stop) in &ranges {
                assert!(set.contains(start));
                assert!(set.contains(stop));
            }
        }

        let empty_atn = token_set_atn(0, &[]);
        let empty = empty_atn.token_set(0).expect("empty set");
        assert_eq!(empty.kind(), ParserTokenSetKind::Inline128);
        assert!(empty.is_empty());
        assert!(!empty.contains(TOKEN_EOF));
        assert!(!empty.contains(1));
    }

    #[test]
    fn adaptive_membership_matches_randomized_normalized_intervals() {
        let mut random = 0x9e37_79b9_7f4a_7c15_u64;
        for case in 0..256 {
            let range_count = (next_random(&mut random) % 24) as usize;
            let mut ranges = Vec::with_capacity(range_count);
            for _ in 0..range_count {
                let start = (next_random(&mut random) % 2_100) as i32 - 4;
                let width = (next_random(&mut random) % 24) as i32;
                ranges.push((start, start.saturating_add(width)));
            }
            if case % 17 == 0 {
                ranges.push((TOKEN_EOF, TOKEN_EOF));
            }
            if case % 29 == 0 {
                ranges.push((i32::MAX, i32::MAX));
            }
            let normalized = normalize_ranges(ranges);
            let atn = token_set_atn(i32::MAX, &normalized);
            let set = atn.token_set(0).expect("randomized set");
            for token in [TOKEN_EOF, -3, 0, 1, 63, 64, 127, 128, 2_048, i32::MAX] {
                let expected = normalized
                    .iter()
                    .any(|(start, stop)| (*start..=*stop).contains(&token));
                assert_eq!(
                    set.contains(token),
                    expected,
                    "case {case}, token {token}, kind {:?}, ranges {normalized:?}",
                    set.kind()
                );
            }
            for _ in 0..64 {
                let token = (next_random(&mut random) % 2_200) as i32 - 16;
                let expected = normalized
                    .iter()
                    .any(|(start, stop)| (*start..=*stop).contains(&token));
                assert_eq!(set.contains(token), expected, "case {case}, token {token}");
            }
        }
    }

    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn header_encoding_rejects_values_outside_u32() {
        let builder = ParserAtnBuilder::new(0);
        let section = Section {
            offset: HEADER_WORDS,
            len: 0,
        };
        let mut layout = EncodedLayout {
            states: section,
            transitions: section,
            sets: section,
            intervals: section,
            token_bits: section,
            decisions: section,
            rule_starts: section,
            rule_stops: section,
            total_len: usize::MAX,
        };
        let mut words = [0; HEADER_WORDS];

        assert_eq!(
            builder.encode_header(&mut words, layout),
            Err(ParserAtnError::Overflow {
                field: "packed parser ATN word",
                value: usize::MAX,
            })
        );

        layout.states.offset = usize::MAX;
        layout.total_len = HEADER_WORDS;
        assert_eq!(
            builder.encode_header(&mut words, layout),
            Err(ParserAtnError::Overflow {
                field: "parser ATN section offset",
                value: usize::MAX,
            })
        );
    }

    #[test]
    fn rejects_invalid_header_and_section_layout() {
        let atn = sample_atn();
        let cases = [
            (HEADER_MAGIC, 0, "magic"),
            (HEADER_BYTE_ORDER, 0x0403_0201, "byte-order marker"),
            (HEADER_SIZE, 0, "header length"),
            (HEADER_STATES_OFFSET, 0, "states section starts"),
            (HEADER_STATES_OFFSET + 1, 0, "states section has 0 words"),
            (HEADER_TOTAL_LEN, 0, "declared total length"),
        ];
        for (word, value, expected) in cases {
            let mut words = atn.packed_words().to_vec();
            words[word] = value;
            let error = ParserAtn::from_owned(words).expect_err("invalid format must fail");
            assert!(
                error.to_string().contains(expected),
                "{error} did not contain {expected:?}"
            );
        }
    }

    #[test]
    fn rejects_non_contiguous_state_transition_ranges() {
        let atn = sample_atn();
        let mut words = atn.packed_words().to_vec();
        let second_state = atn.layout.states.offset + STATE_WORDS;
        words[second_state + 3] = 0;
        let error = ParserAtn::from_owned(words).expect_err("overlapping ranges must fail");
        assert!(error.to_string().contains("transition range starts"));
    }

    #[test]
    fn interval_sets_share_one_range_pool() {
        let mut builder = ParserAtnBuilder::new(20);
        builder
            .add_state(AtnStateKind::RuleStart, Some(0))
            .expect("start");
        builder
            .add_state(AtnStateKind::RuleStop, Some(0))
            .expect("stop");
        builder
            .set_rule_to_start_state(vec![0])
            .expect("rule starts");
        builder.set_rule_to_stop_state(vec![1]).expect("rule stops");
        let set = builder
            .add_interval_set([(2, 4), (4, 8), (10, 10)])
            .expect("set");
        builder
            .add_transition(0, ParserTransitionSpec::Set { target: 1, set })
            .expect("set transition");
        let atn = builder.finish().expect("ATN");
        let transition = atn
            .state(0)
            .expect("start")
            .transitions()
            .first()
            .expect("transition");
        let ParserTransitionData::Set { set, .. } = transition.data() else {
            panic!("expected set transition");
        };
        assert_eq!(set.ranges().collect::<Vec<_>>(), vec![(2, 8), (10, 10)]);
        assert!(set.contains(7));
        assert!(!set.contains(9));
        assert_eq!(atn.stats().interval_ranges, 2);
    }

    #[test]
    fn rejects_out_of_range_transition_target() {
        let atn = sample_atn();
        let mut words = atn.packed_words().to_vec();
        let target = atn.layout.transitions.offset + 1;
        words[target] = 99;
        assert!(matches!(
            ParserAtn::from_owned(words),
            Err(ParserAtnError::InvalidData(message))
                if message.contains("transition target")
        ));
    }

    #[test]
    fn not_set_membership_preserves_vocabulary_bounds() {
        let mut builder = ParserAtnBuilder::new(5);
        builder
            .add_state(AtnStateKind::RuleStart, Some(0))
            .expect("start");
        builder
            .add_state(AtnStateKind::RuleStop, Some(0))
            .expect("stop");
        builder
            .set_rule_to_start_state(vec![0])
            .expect("rule starts");
        builder.set_rule_to_stop_state(vec![1]).expect("rule stops");
        let excluded = builder.add_interval_set([(2, 4)]).expect("excluded set");
        builder
            .add_transition(
                0,
                ParserTransitionSpec::NotSet {
                    target: 1,
                    set: excluded,
                },
            )
            .expect("not-set transition");
        let atn = builder.finish().expect("ATN");
        let transition = atn
            .state(0)
            .expect("start")
            .transitions()
            .first()
            .expect("transition");

        assert!(transition.matches(1, 1, 5));
        assert!(!transition.matches(2, 1, 5));
        assert!(!transition.matches(4, 1, 5));
        assert!(transition.matches(5, 1, 5));
        assert!(!transition.matches(TOKEN_EOF, 1, 5));
        assert!(!transition.matches(0, 1, 5));
        assert!(!transition.matches(6, 1, 5));
    }

    #[test]
    fn rejects_inconsistent_adaptive_token_set_bits() {
        let atn = token_set_atn(127, &[(1, 3), (63, 64), (127, 127)]);
        let mut words = atn.packed_words().to_vec();
        words[atn.layout.token_bits.offset] ^= 1 << 1;
        let error = ParserAtn::from_owned(words).expect_err("corrupted token bits must fail");
        assert!(error.to_string().contains("bit word"), "{error}");

        let mut words = atn.packed_words().to_vec();
        words[atn.layout.sets.offset + 2] = 99;
        let error = ParserAtn::from_owned(words).expect_err("unknown token-set kind must fail");
        assert!(error.to_string().contains("token-set kind"), "{error}");
    }

    #[cfg(feature = "perf-counters")]
    #[test]
    fn token_set_counters_report_selection_and_probes() {
        crate::perf::reset();
        let before = crate::perf::parser_token_set_snapshot();
        let inline_atn = token_set_atn(10, &[(1, 4)]);
        let dense_ranges = (1..=256)
            .step_by(2)
            .map(|token| (token, token))
            .collect::<Vec<_>>();
        let dense_atn = token_set_atn(256, &dense_ranges);
        let interval_atn = token_set_atn(i32::MAX, &[(1, 1), (i32::MAX, i32::MAX)]);
        let inline = inline_atn.token_set(0).expect("inline");
        let dense = dense_atn.token_set(0).expect("dense");
        let intervals = interval_atn.token_set(0).expect("intervals");

        assert!(inline.contains(2));
        assert!(!inline.contains(9));
        assert!(dense.contains(255));
        assert!(!dense.contains(256));
        assert!(intervals.contains(i32::MAX));
        assert!(!intervals.contains(2));

        let after = crate::perf::parser_token_set_snapshot();
        assert!(after[0] > before[0], "{before:?} -> {after:?}");
        assert!(after[1] > before[1], "{before:?} -> {after:?}");
        assert!(after[2] > before[2], "{before:?} -> {after:?}");
        assert_eq!(after[5] - before[5], 1);
        assert_eq!(after[6] - before[6], 1);
        assert_eq!(after[7] - before[7], 1);
        assert_eq!(after[8] - before[8], 1);
        assert_eq!(after[9] - before[9], 1);
        assert_eq!(after[10] - before[10], 1);
        assert_eq!(after[11] - before[11], 4);
        assert_eq!(after[12] - before[12], 2);
    }

    #[test]
    fn eof_interval_is_preserved_as_signed_data() {
        let mut builder = ParserAtnBuilder::new(3);
        builder
            .add_state(AtnStateKind::RuleStart, Some(0))
            .expect("start");
        builder
            .add_state(AtnStateKind::RuleStop, Some(0))
            .expect("stop");
        builder
            .set_rule_to_start_state(vec![0])
            .expect("rule starts");
        builder.set_rule_to_stop_state(vec![1]).expect("rule stops");
        let set = builder
            .add_interval_set([(TOKEN_EOF, TOKEN_EOF)])
            .expect("set");
        builder
            .add_transition(0, ParserTransitionSpec::Set { target: 1, set })
            .expect("transition");
        let atn = builder.finish().expect("ATN");
        let transition = atn
            .state(0)
            .expect("start")
            .transitions()
            .first()
            .expect("transition");
        assert!(transition.matches(TOKEN_EOF, 1, 3));
    }
}
