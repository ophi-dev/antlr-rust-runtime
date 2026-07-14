//! Ahead-of-time lexer DFA compilation.
//!
//! ANTLR runtimes normally discover the lexer DFA lazily: the ATN simulator
//! computes epsilon closures per input character and caches the resulting
//! config sets. This module runs the same subset construction eagerly over
//! the entire character alphabet, once per grammar, so token matching becomes
//! one table lookup per character with no closure computation, hashing, or
//! config allocation on the hot path.
//!
//! Compilation is conservative at the edge level: a transition whose target
//! closure crosses a semantic predicate (whose outcome exists only at parse
//! time), grows an unbounded rule-call stack (recursive lexer rules such as
//! nested comments), or exceeds the state budget is compiled as an *escape*
//! edge. A token walk that reaches an escape edge is re-matched from the
//! token start by the ATN interpreter, so rare dynamic constructs never
//! poison the rest of the mode. Because the construction reuses the
//! interpreter's own closure, pruning, and accept-selection code, a compiled
//! walk that does not escape reproduces interpreter behavior exactly.

use std::collections::HashMap;
use std::hash::BuildHasherDefault;

use crate::atn::lexer::{
    LexerConfig, best_accept, epsilon_closure, lexer_action_belongs_to_accept, prune_after_accepts,
    set_config_state,
};
use crate::atn::{Atn, Transition};
use crate::int_stream::EOF;
use crate::lexer::{LexerDfaActionKey, LexerDfaConfigKey, LexerDfaKey};
use crate::prediction::PredictionFxHasher;

#[allow(clippy::disallowed_types)]
type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<PredictionFxHasher>>;

const MIN_CHAR_VALUE: i32 = 0;
const MAX_CHAR_VALUE: i32 = 0x0010_FFFF;

/// Sentinel state id meaning "no transition".
pub(super) const DEAD_STATE: u16 = u16::MAX;

/// Sentinel state id meaning "re-match this token with the ATN interpreter".
pub(super) const ESCAPE_STATE: u16 = u16::MAX - 1;

/// Per-mode state budget; targets past it compile as escape edges. The cap
/// also bounds compile time for pathological grammars.
const MAX_MODE_STATES: usize = 4096;

/// Rule-call stacks deeper than this escape to the interpreter, as a backstop
/// for grammars with extraordinarily long non-recursive fragment chains.
const MAX_STACK_DEPTH: usize = 32;

/// Configs whose surviving action trace grows past this escape to the
/// interpreter: a custom action crossed inside a loop is genuinely
/// position-dependent and cannot compile to finitely many DFA states.
const MAX_ACTION_TRACES: usize = 16;

/// Dense per-state edge row width, matching the interpreter's DFA cache rows.
const ASCII_EDGE_SYMBOLS: usize = 128;
/// [`ASCII_EDGE_SYMBOLS`] as a code point for segment arithmetic.
const ASCII_EDGE_LIMIT: i32 = 128;

/// A lexer DFA compiled ahead of time from a lexer ATN.
///
/// Build one per grammar with [`CompiledLexerDfa::compile`] (generated lexers
/// cache it in a `OnceLock` beside the deserialized ATN) and match tokens
/// through [`crate::atn::lexer::next_token_compiled`] or
/// [`crate::atn::lexer::next_token_compiled_with_hooks`].
#[derive(Clone, Debug)]
pub struct CompiledLexerDfa {
    mode_starts: Vec<Option<u16>>,
    states: Vec<CompiledLexerState>,
    ascii_rows: Vec<[u16; ASCII_EDGE_SYMBOLS]>,
    wide_rows: Vec<Box<[WideRange]>>,
    accepts: Vec<CompiledLexerAccept>,
}

/// One compiled DFA state; rows are pooled indices because many states share
/// identical edge rows (identifier continuations, string bodies, …).
#[derive(Clone, Copy, Debug)]
struct CompiledLexerState {
    ascii_row: u32,
    wide_row: u32,
    eof_target: u16,
    /// Index into `accepts`, or `u32::MAX` when the state does not accept.
    accept: u32,
}

/// Inclusive code-point range above the ASCII row mapping to one target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct WideRange {
    low: u32,
    high: u32,
    target: u16,
}

/// Accept metadata for one DFA state: the winning lexer rule plus the action
/// transitions collected on the accepted ATN path.
#[derive(Clone, Debug)]
pub(super) struct CompiledLexerAccept {
    pub(super) rule_index: usize,
    pub(super) consumed_eof: bool,
    pub(super) actions: Vec<CompiledLexerActionTrace>,
}

/// One recorded lexer action, positioned relative to the accept boundary so
/// the same DFA state serves every input offset.
#[derive(Clone, Copy, Debug)]
pub(super) struct CompiledLexerActionTrace {
    pub(super) action_index: usize,
    pub(super) rule_index: usize,
    /// Characters consumed between the action transition and the accept.
    pub(super) behind: usize,
}

impl CompiledLexerDfa {
    /// Compiles every lexer mode of `atn` that has no semantic predicates and
    /// fits the state budget; the rest stay interpreter-matched.
    pub fn compile(atn: &Atn) -> Self {
        let mut dfa = Self {
            mode_starts: Vec::new(),
            states: Vec::new(),
            ascii_rows: Vec::new(),
            wide_rows: Vec::new(),
            accepts: Vec::new(),
        };
        let mut pools = RowPools::default();
        for mode in 0..atn.mode_to_start_state().len() {
            let start = build_mode(atn, mode, &mut dfa, &mut pools);
            dfa.mode_starts.push(start);
        }
        dfa
    }

    /// True when at least one lexer mode compiled to static tables.
    pub fn has_compiled_modes(&self) -> bool {
        self.mode_starts.iter().any(Option::is_some)
    }

    /// Number of compiled DFA states across all modes (diagnostics).
    pub const fn state_count(&self) -> usize {
        self.states.len()
    }

    /// Per-mode compilation outcome (diagnostics): `true` = static tables.
    pub fn compiled_mode_flags(&self) -> Vec<bool> {
        self.mode_starts.iter().map(Option::is_some).collect()
    }

    /// Per-mode state counts (diagnostics), derived from start offsets.
    pub fn mode_state_counts(&self) -> Vec<usize> {
        let mut starts: Vec<usize> = self
            .mode_starts
            .iter()
            .flatten()
            .map(|&start| usize::from(start))
            .collect();
        starts.push(self.states.len());
        starts.windows(2).map(|pair| pair[1] - pair[0]).collect()
    }

    /// Compiled start state for `mode`, or `None` when the mode is
    /// interpreter-matched.
    pub(super) fn mode_start(&self, mode: i32) -> Option<u16> {
        let mode = usize::try_from(mode).ok()?;
        self.mode_starts.get(mode).copied().flatten()
    }

    pub(super) fn accept(&self, state: u16) -> Option<&CompiledLexerAccept> {
        self.accepts
            .get(self.states[usize::from(state)].accept as usize)
    }

    /// Transition target for a non-EOF symbol, or [`DEAD_STATE`].
    pub(super) fn char_target(&self, state: u16, symbol: i32) -> u16 {
        let compiled = &self.states[usize::from(state)];
        let code_point = symbol.cast_unsigned();
        if let Ok(ascii) = usize::try_from(symbol)
            && ascii < ASCII_EDGE_SYMBOLS
        {
            return self.ascii_rows[compiled.ascii_row as usize][ascii];
        }
        let row = &self.wide_rows[compiled.wide_row as usize];
        match row.binary_search_by(|range| range.low.cmp(&code_point)) {
            Ok(found) => row[found].target,
            Err(insert) => {
                if insert > 0 && row[insert - 1].high >= code_point {
                    row[insert - 1].target
                } else {
                    DEAD_STATE
                }
            }
        }
    }

    /// Transition target for the EOF symbol, or [`DEAD_STATE`].
    pub(super) fn eof_target(&self, state: u16) -> u16 {
        self.states[usize::from(state)].eof_target
    }

    /// Flattens the compiled DFA into a `u32` stream for embedding in
    /// generated code.
    ///
    /// The format is internal to this runtime version; [`Self::from_serialized`]
    /// rejects streams from other versions so generated lexers can fall back
    /// to [`Self::compile`].
    pub fn serialize(&self) -> Vec<u32> {
        // Exact word count: the tag, five section-length words, and each
        // section's payload (states are 4 words; ASCII rows pack 2 targets
        // per word; wide ranges and action traces are 3 words each behind
        // their per-row/per-accept length words).
        let wide_words: usize = self.wide_rows.iter().map(|row| 1 + row.len() * 3).sum();
        let accept_words: usize = self
            .accepts
            .iter()
            .map(|accept| 3 + accept.actions.len() * 3)
            .sum();
        let capacity = 6
            + self.mode_starts.len()
            + self.states.len() * 4
            + self.ascii_rows.len() * (ASCII_EDGE_SYMBOLS / 2)
            + wide_words
            + accept_words;
        let mut out = Vec::with_capacity(capacity);
        out.push(SERIALIZED_TAG);
        out.push(self.mode_starts.len() as u32);
        for start in &self.mode_starts {
            out.push(start.map_or(u32::MAX, u32::from));
        }
        out.push(self.states.len() as u32);
        for state in &self.states {
            out.push(state.ascii_row);
            out.push(state.wide_row);
            out.push(u32::from(state.eof_target));
            out.push(state.accept);
        }
        out.push(self.ascii_rows.len() as u32);
        for row in &self.ascii_rows {
            for pair in row.chunks(2) {
                out.push(u32::from(pair[0]) | (u32::from(pair[1]) << 16));
            }
        }
        out.push(self.wide_rows.len() as u32);
        for row in &self.wide_rows {
            out.push(row.len() as u32);
            for range in &**row {
                out.push(range.low);
                out.push(range.high);
                out.push(u32::from(range.target));
            }
        }
        out.push(self.accepts.len() as u32);
        for accept in &self.accepts {
            out.push(accept.rule_index as u32);
            out.push(u32::from(accept.consumed_eof));
            out.push(accept.actions.len() as u32);
            for action in &accept.actions {
                out.push(action.action_index as u32);
                out.push(action.rule_index as u32);
                out.push(action.behind as u32);
            }
        }
        debug_assert_eq!(
            out.len(),
            capacity,
            "serialized stream fills its capacity exactly"
        );
        out
    }

    /// Rebuilds a compiled DFA from [`Self::serialize`] output; `None` when
    /// the stream comes from a different runtime version or is malformed.
    pub fn from_serialized(data: &[u32]) -> Option<Self> {
        let mut reader = SerializedReader { data, position: 0 };
        if reader.next()? != SERIALIZED_TAG {
            return None;
        }
        let mode_count = reader.next_len()?;
        let mut mode_starts = Vec::with_capacity(mode_count);
        for _ in 0..mode_count {
            let word = reader.next()?;
            let start = if word == u32::MAX {
                None
            } else {
                Some(u16::try_from(word).ok()?)
            };
            mode_starts.push(start);
        }
        let states = reader.read_states()?;
        let ascii_rows = reader.read_ascii_rows()?;
        let wide_rows = reader.read_wide_rows()?;
        let accepts = reader.read_accepts()?;
        if reader.position != data.len() {
            return None;
        }
        let dfa = Self {
            mode_starts,
            states,
            ascii_rows,
            wide_rows,
            accepts,
        };
        dfa.table_indexes_are_valid().then_some(dfa)
    }

    /// Cheap structural validation so a corrupted embedded stream degrades to
    /// runtime compilation instead of an out-of-bounds panic mid-parse.
    fn table_indexes_are_valid(&self) -> bool {
        let state_ok =
            |target: u16| usize::from(target) < self.states.len() || target >= ESCAPE_STATE;
        self.mode_starts
            .iter()
            .flatten()
            .all(|&start| usize::from(start) < self.states.len())
            && self.states.iter().all(|state| {
                (state.ascii_row as usize) < self.ascii_rows.len()
                    && (state.wide_row as usize) < self.wide_rows.len()
                    && state_ok(state.eof_target)
                    && (state.accept == u32::MAX || (state.accept as usize) < self.accepts.len())
            })
            && self
                .ascii_rows
                .iter()
                .all(|row| row.iter().all(|&target| state_ok(target)))
            && self.wide_rows.iter().all(|row| {
                wide_row_is_searchable(row) && row.iter().all(|range| state_ok(range.target))
            })
    }
}

/// Wide rows must hold well-formed, sorted, disjoint ranges for
/// [`CompiledLexerDfa::char_target`]'s binary search; anything else would
/// silently misroute transitions instead of degrading to recompilation.
fn wide_row_is_searchable(row: &[WideRange]) -> bool {
    row.iter().all(|range| range.low <= range.high)
        && row.windows(2).all(|pair| pair[0].high < pair[1].low)
}

/// Version tag guarding embedded tables against serialization format drift.
const SERIALIZED_TAG: u32 = 0x4C58_4401;

/// Cursor over a serialized DFA stream.
struct SerializedReader<'a> {
    data: &'a [u32],
    position: usize,
}

impl SerializedReader<'_> {
    fn next(&mut self) -> Option<u32> {
        let value = self.data.get(self.position).copied();
        self.position += 1;
        value
    }

    fn next_u16(&mut self) -> Option<u16> {
        u16::try_from(self.next()?).ok()
    }

    fn next_len(&mut self) -> Option<usize> {
        usize::try_from(self.next()?).ok()
    }

    fn read_states(&mut self) -> Option<Vec<CompiledLexerState>> {
        let count = self.next_len()?;
        let mut states = Vec::with_capacity(count.min(self.data.len()));
        for _ in 0..count {
            states.push(CompiledLexerState {
                ascii_row: self.next()?,
                wide_row: self.next()?,
                eof_target: self.next_u16()?,
                accept: self.next()?,
            });
        }
        Some(states)
    }

    fn read_ascii_rows(&mut self) -> Option<Vec<[u16; ASCII_EDGE_SYMBOLS]>> {
        let count = self.next_len()?;
        let mut rows = Vec::with_capacity(count.min(self.data.len()));
        for _ in 0..count {
            let mut row = [DEAD_STATE; ASCII_EDGE_SYMBOLS];
            for pair in 0..ASCII_EDGE_SYMBOLS / 2 {
                let word = self.next()?;
                row[pair * 2] = (word & 0xFFFF) as u16;
                row[pair * 2 + 1] = (word >> 16) as u16;
            }
            rows.push(row);
        }
        Some(rows)
    }

    fn read_wide_rows(&mut self) -> Option<Vec<Box<[WideRange]>>> {
        let count = self.next_len()?;
        let mut rows = Vec::with_capacity(count.min(self.data.len()));
        for _ in 0..count {
            let len = self.next_len()?;
            let mut row = Vec::with_capacity(len.min(self.data.len()));
            for _ in 0..len {
                row.push(WideRange {
                    low: self.next()?,
                    high: self.next()?,
                    target: self.next_u16()?,
                });
            }
            rows.push(row.into());
        }
        Some(rows)
    }

    fn read_accepts(&mut self) -> Option<Vec<CompiledLexerAccept>> {
        let count = self.next_len()?;
        let mut accepts = Vec::with_capacity(count.min(self.data.len()));
        for _ in 0..count {
            let rule_index = self.next_len()?;
            let consumed_eof = self.next()? != 0;
            let action_count = self.next_len()?;
            let mut actions = Vec::with_capacity(action_count.min(self.data.len()));
            for _ in 0..action_count {
                actions.push(CompiledLexerActionTrace {
                    action_index: self.next_len()?,
                    rule_index: self.next_len()?,
                    behind: self.next_len()?,
                });
            }
            accepts.push(CompiledLexerAccept {
                rule_index,
                consumed_eof,
                actions,
            });
        }
        Some(accepts)
    }
}

/// Deduplicating pools for edge rows shared by many DFA states.
#[derive(Debug, Default)]
struct RowPools {
    ascii_ids: FxHashMap<[u16; ASCII_EDGE_SYMBOLS], u32>,
    wide_ids: FxHashMap<Box<[WideRange]>, u32>,
}

impl RowPools {
    fn intern_ascii(
        &mut self,
        rows: &mut Vec<[u16; ASCII_EDGE_SYMBOLS]>,
        row: [u16; ASCII_EDGE_SYMBOLS],
    ) -> u32 {
        *self.ascii_ids.entry(row).or_insert_with(|| {
            rows.push(row);
            (rows.len() - 1) as u32
        })
    }

    fn intern_wide(&mut self, rows: &mut Vec<Box<[WideRange]>>, row: Vec<WideRange>) -> u32 {
        let row: Box<[WideRange]> = row.into();
        if let Some(&id) = self.wide_ids.get(&row) {
            return id;
        }
        rows.push(row.clone());
        let id = (rows.len() - 1) as u32;
        self.wide_ids.insert(row, id);
        id
    }
}

/// In-progress subset construction for one lexer mode.
///
/// States are numbered globally (`base` + discovery order) so edges can be
/// written directly into the final table, but nothing is committed to the
/// shared [`CompiledLexerDfa`] until the whole mode succeeds.
struct ModeBuild {
    base: usize,
    ids: FxHashMap<LexerDfaKey, u16>,
    configs: Vec<Vec<LexerConfig>>,
    steps: Vec<usize>,
    accepts: Vec<Option<CompiledLexerAccept>>,
}

/// Edge rows produced by expanding one DFA state.
struct StateRows {
    /// Sorted, disjoint code-point segments with live targets.
    segments: Vec<(i32, i32, u16)>,
    eof_target: u16,
}

impl ModeBuild {
    fn new(base: usize) -> Self {
        Self {
            base,
            ids: FxHashMap::default(),
            configs: Vec::new(),
            steps: Vec::new(),
            accepts: Vec::new(),
        }
    }

    const fn len(&self) -> usize {
        self.configs.len()
    }

    /// Returns the state id for a closed, pruned config set, creating the
    /// state when the (input-offset-normalized) identity is new.
    /// [`ESCAPE_STATE`] means the state budget is exhausted and the edge must
    /// hand the token to the interpreter.
    fn intern(&mut self, atn: &Atn, configs: Vec<LexerConfig>, step: usize) -> u16 {
        let key = LexerDfaKey::new(
            configs
                .iter()
                .map(|config| relative_config_key(config, step))
                .collect(),
        );
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        let local = self.configs.len();
        let global = self.base + local;
        if local >= MAX_MODE_STATES || global >= usize::from(ESCAPE_STATE) {
            return ESCAPE_STATE;
        }
        let Ok(id) = u16::try_from(global) else {
            return ESCAPE_STATE;
        };
        self.ids.insert(key, id);
        self.accepts.push(compiled_accept(atn, &configs, step));
        self.configs.push(configs);
        self.steps.push(step);
        id
    }
}

/// Normalizes one config for DFA-state identity, measuring action positions
/// backwards from the current input offset (`step`).
///
/// This differs from the interpreter cache's token-start-relative deltas on
/// purpose: rule-final lexer commands (`skip`, `pushMode`, …) fire a fixed
/// distance before the accept, so anchoring at the read position keeps the
/// state space finite regardless of token length.
fn relative_config_key(config: &LexerConfig, step: usize) -> LexerDfaConfigKey {
    LexerDfaConfigKey::new(
        config.state,
        config.alt_rule_index,
        config.consumed_eof,
        config.passed_non_greedy,
        config.stack.clone(),
        config
            .actions
            .iter()
            .map(|action| LexerDfaActionKey {
                action_index: action.action_index,
                position_delta: step.saturating_sub(action.position),
                rule_index: action.rule_index,
            })
            .collect(),
    )
}

/// Computes the accept metadata for a DFA state from its config set, using
/// the interpreter's own rule-priority selection.
fn compiled_accept(atn: &Atn, configs: &[LexerConfig], step: usize) -> Option<CompiledLexerAccept> {
    let accept = best_accept(atn, configs)?;
    debug_assert!(
        accept.position == step,
        "every config in a lexer DFA state shares the state's input offset"
    );
    Some(CompiledLexerAccept {
        rule_index: accept.rule_index,
        consumed_eof: accept.consumed_eof,
        actions: accept
            .actions
            .iter()
            .map(|trace| CompiledLexerActionTrace {
                action_index: trace.action_index,
                rule_index: trace.rule_index,
                behind: accept.position.saturating_sub(trace.position),
            })
            .collect(),
    })
}

/// Runs subset construction for one mode; `None` leaves the whole mode to the
/// interpreter (only when its very first closure already escapes).
fn build_mode(
    atn: &Atn,
    mode: usize,
    dfa: &mut CompiledLexerDfa,
    pools: &mut RowPools,
) -> Option<u16> {
    let start_state = atn.mode_to_start_state().get(mode).copied()?;
    let mut build = ModeBuild::new(dfa.states.len());
    let start_configs = closed_configs(
        atn,
        vec![LexerConfig {
            state: start_state,
            position: 0,
            consumed_eof: false,
            alt_rule_index: None,
            passed_non_greedy: false,
            stack: Vec::new(),
            actions: Vec::new(),
        }],
    )?;
    let start_id = build.intern(atn, start_configs, 0);
    if start_id == ESCAPE_STATE {
        return None;
    }

    let mut rows = Vec::new();
    let mut cursor = 0;
    while cursor < build.len() {
        rows.push(expand_state(atn, &mut build, cursor));
        cursor += 1;
    }

    commit_mode(dfa, pools, build, rows);
    Some(start_id)
}

/// Closes and prunes a moved config set exactly like the interpreter does.
/// `None` means the closure crossed a semantic predicate (which only the
/// interpreter can evaluate) or entered a recursive lexer rule (nested
/// comments never determinize), so the edge must escape.
fn closed_configs(atn: &Atn, moved: Vec<LexerConfig>) -> Option<Vec<LexerConfig>> {
    let closure = epsilon_closure(atn, moved, &mut |_| true);
    if closure.has_semantic_context {
        return None;
    }
    if closure.configs.iter().any(has_recursive_stack) {
        return None;
    }
    let mut configs = closure.configs;
    for config in &mut configs {
        prune_dead_action_traces(atn, config);
        if config.actions.len() > MAX_ACTION_TRACES {
            return None;
        }
    }
    Some(prune_after_accepts(atn, configs))
}

/// Drops action traces the accept-time dispatcher would suppress anyway.
///
/// The interpreter keeps traces of every action transition it crosses and
/// filters them per accept with `lexer_action_belongs_to_accept`; a token
/// rule referenced from another rule leaves traces that can never fire (its
/// commands belong to itself, not the enclosing rule). Carrying them into
/// DFA-state identity would mint a fresh state per input offset — rules that
/// loop over comment/whitespace references would never determinize.
fn prune_dead_action_traces(atn: &Atn, config: &mut LexerConfig) {
    let Some(accept_rule) = config.alt_rule_index else {
        return;
    };
    config
        .actions
        .retain(|trace| lexer_action_belongs_to_accept(atn, accept_rule, trace.rule_index));
}

/// Detects lexer-rule recursion: re-entering a rule from the same call site
/// pushes the same follow state again, so a duplicated stack entry (or an
/// implausibly deep stack) marks a config a finite DFA cannot represent.
fn has_recursive_stack(config: &LexerConfig) -> bool {
    let stack = &config.stack;
    if stack.len() > MAX_STACK_DEPTH {
        return true;
    }
    stack
        .iter()
        .enumerate()
        .any(|(index, follow)| stack[..index].contains(follow))
}

/// Computes every outgoing edge of one interned DFA state.
fn expand_state(atn: &Atn, build: &mut ModeBuild, local: usize) -> StateRows {
    let configs = build.configs[local].clone();
    let step = build.steps[local];
    let entries = consuming_entries(atn, &configs);
    let eof_target = eof_move(atn, build, &configs, step, &entries);

    let entry_intervals: Vec<Vec<(i32, i32)>> = entries
        .iter()
        .map(|(_, transition)| transition_char_intervals(transition))
        .collect();
    let segments = char_segments(&entry_intervals);
    let matrix = segment_mask_matrix(&segments, &entry_intervals, entries.len());
    let words = entries.len().div_ceil(64);

    let mut rows = StateRows {
        segments: Vec::new(),
        eof_target,
    };
    // Distinct transition sets are few even when segments are many (large
    // Unicode classes fragment the alphabet), so closures run once per
    // matching-transition mask, not once per segment.
    let mut mask_targets: FxHashMap<Vec<u64>, u16> = FxHashMap::default();
    for (index, &(low, high)) in segments.iter().enumerate() {
        let mask = &matrix[index * words..(index + 1) * words];
        if mask.iter().all(|&word| word == 0) {
            continue;
        }
        let target = match mask_targets.get(mask) {
            Some(&target) => target,
            None => {
                let target = move_target(atn, build, &configs, step, &entries, mask);
                mask_targets.insert(mask.to_vec(), target);
                target
            }
        };
        if target != DEAD_STATE {
            rows.segments.push((low, high, target));
        }
    }
    rows
}

/// Lists each config's consuming transitions in the interpreter's move order.
fn consuming_entries<'a>(atn: &'a Atn, configs: &[LexerConfig]) -> Vec<(usize, &'a Transition)> {
    let mut entries = Vec::new();
    for (config_index, config) in configs.iter().enumerate() {
        let Some(state) = atn.state(config.state) else {
            continue;
        };
        for transition in &state.transitions {
            if !transition.is_epsilon() {
                entries.push((config_index, transition));
            }
        }
    }
    entries
}

/// Splits the code-point alphabet at every interval boundary, so each segment
/// is matched uniformly by every transition.
fn char_segments(entry_intervals: &[Vec<(i32, i32)>]) -> Vec<(i32, i32)> {
    let mut cuts = Vec::new();
    for intervals in entry_intervals {
        for &(low, high) in intervals {
            cuts.push(low);
            cuts.push(high + 1);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2).map(|pair| (pair[0], pair[1] - 1)).collect()
}

/// Marks, for every segment, which entries match it — one bit row per
/// segment. Sweeping each entry's intervals over the sorted segment starts
/// keeps the work proportional to interval count, not `segments × entries`.
fn segment_mask_matrix(
    segments: &[(i32, i32)],
    entry_intervals: &[Vec<(i32, i32)>],
    entry_count: usize,
) -> Vec<u64> {
    let words = entry_count.div_ceil(64);
    let mut matrix = vec![0_u64; segments.len() * words];
    for (bit, intervals) in entry_intervals.iter().enumerate() {
        for &(low, high) in intervals {
            // Interval boundaries are cut points, so the covered segments are
            // exactly those whose start lies inside the interval.
            let from = segments.partition_point(|&(start, _)| start < low);
            let to = segments.partition_point(|&(start, _)| start <= high);
            for segment in from..to {
                matrix[segment * words + bit / 64] |= 1 << (bit % 64);
            }
        }
    }
    matrix
}

/// Materializes the code-point intervals a transition consumes, clamped to
/// the valid character range (EOF is handled separately).
fn transition_char_intervals(transition: &Transition) -> Vec<(i32, i32)> {
    let mut intervals = Vec::new();
    let mut push_clamped = |low: i32, high: i32| {
        let low = low.max(MIN_CHAR_VALUE);
        let high = high.min(MAX_CHAR_VALUE);
        if low <= high {
            intervals.push((low, high));
        }
    };
    match transition {
        Transition::Atom { label, .. } => push_clamped(*label, *label),
        Transition::Range { start, stop, .. } => push_clamped(*start, *stop),
        Transition::Set { set, .. } => {
            for &(low, high) in set.ranges() {
                push_clamped(low, high);
            }
        }
        Transition::NotSet { set, .. } => {
            // `NotSet` matches the complement within the character range;
            // `IntervalSet` ranges are sorted and coalesced.
            let mut next = MIN_CHAR_VALUE;
            for &(low, high) in set.ranges() {
                if low > next {
                    push_clamped(next, low - 1);
                }
                next = next.max(high.saturating_add(1));
            }
            push_clamped(next, MAX_CHAR_VALUE);
        }
        Transition::Wildcard { .. } => push_clamped(MIN_CHAR_VALUE, MAX_CHAR_VALUE),
        _ => {}
    }
    intervals
}

/// Advances the masked entries by one character and interns the result;
/// closures that escape compile as [`ESCAPE_STATE`] edges.
fn move_target(
    atn: &Atn,
    build: &mut ModeBuild,
    configs: &[LexerConfig],
    step: usize,
    entries: &[(usize, &Transition)],
    mask: &[u64],
) -> u16 {
    let mut moved = Vec::new();
    for (bit, (config_index, transition)) in entries.iter().enumerate() {
        if mask[bit / 64] & (1 << (bit % 64)) == 0 {
            continue;
        }
        let mut advanced = configs[*config_index].clone();
        set_config_state(atn, &mut advanced, transition.target());
        advanced.position += 1;
        moved.push(advanced);
    }
    let Some(active) = closed_configs(atn, moved) else {
        return ESCAPE_STATE;
    };
    if active.is_empty() {
        return DEAD_STATE;
    }
    build.intern(atn, active, step + 1)
}

/// Advances the EOF-matching entries; EOF consumes no character, so the input
/// offset stays put and the moved configs record `consumed_eof`.
fn eof_move(
    atn: &Atn,
    build: &mut ModeBuild,
    configs: &[LexerConfig],
    step: usize,
    entries: &[(usize, &Transition)],
) -> u16 {
    let mut moved = Vec::new();
    for (config_index, transition) in entries {
        if !transition.matches(EOF, MIN_CHAR_VALUE, MAX_CHAR_VALUE) {
            continue;
        }
        let mut advanced = configs[*config_index].clone();
        set_config_state(atn, &mut advanced, transition.target());
        advanced.consumed_eof = true;
        moved.push(advanced);
    }
    if moved.is_empty() {
        return DEAD_STATE;
    }
    let Some(active) = closed_configs(atn, moved) else {
        return ESCAPE_STATE;
    };
    if active.is_empty() {
        return DEAD_STATE;
    }
    build.intern(atn, active, step)
}

/// Converts a finished mode's edge rows into pooled table entries.
fn commit_mode(
    dfa: &mut CompiledLexerDfa,
    pools: &mut RowPools,
    build: ModeBuild,
    rows: Vec<StateRows>,
) {
    for (accept, state_rows) in build.accepts.into_iter().zip(rows) {
        let accept_id = accept.map_or(u32::MAX, |accept| {
            dfa.accepts.push(accept);
            (dfa.accepts.len() - 1) as u32
        });
        let (ascii_row, wide_row) = split_rows(&state_rows.segments);
        dfa.states.push(CompiledLexerState {
            ascii_row: pools.intern_ascii(&mut dfa.ascii_rows, ascii_row),
            wide_row: pools.intern_wide(&mut dfa.wide_rows, wide_row),
            eof_target: state_rows.eof_target,
            accept: accept_id,
        });
    }
}

/// Splits sorted segments into the dense ASCII row and merged wide ranges.
fn split_rows(segments: &[(i32, i32, u16)]) -> ([u16; ASCII_EDGE_SYMBOLS], Vec<WideRange>) {
    let mut ascii = [DEAD_STATE; ASCII_EDGE_SYMBOLS];
    let mut wide: Vec<WideRange> = Vec::new();
    for &(low, high, target) in segments {
        let ascii_high = high.min(ASCII_EDGE_LIMIT - 1);
        for code_point in low..=ascii_high {
            ascii[code_point.cast_unsigned() as usize] = target;
        }
        if high >= ASCII_EDGE_LIMIT {
            let low = low.max(ASCII_EDGE_LIMIT).cast_unsigned();
            let high = high.cast_unsigned();
            if let Some(last) = wide.last_mut()
                && last.target == target
                && last.high + 1 == low
            {
                last.high = high;
                continue;
            }
            wide.push(WideRange { low, high, target });
        }
    }
    (ascii, wide)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atn::lexer::{next_token, next_token_compiled, next_token_compiled_with_hooks};
    use crate::atn::serialized::{AtnDeserializer, SerializedAtn};
    use crate::char_stream::InputStream;
    use crate::lexer::BaseLexer;
    use crate::recognizer::RecognizerData;
    use crate::token::{TOKEN_EOF, Token};
    use crate::vocabulary::Vocabulary;

    fn recognizer_data() -> RecognizerData {
        RecognizerData::new(
            "T",
            Vocabulary::new(
                [None, Some("'ab'"), Some("' '")],
                [None, Some("AB"), Some("WS")],
                [None::<&str>, None, None],
            ),
        )
    }

    /// Two-rule lexer (`AB: 'ab';` and `WS: ' ' -> skip;`), with rule 0's
    /// final epsilon optionally replaced by a semantic predicate transition.
    // `#[rustfmt::skip]`: this serialized ATN is a positional `i32` stream whose
    // meaning comes from its one-record-per-line grouping. Leaving it to rustfmt
    // explodes it to one integer per line (the cast/path elements defeat the
    // packed-list tactic) and destroys the mapping to the ANTLR ATN layout.
    #[rustfmt::skip]
    fn two_rule_atn(with_predicate: bool) -> Atn {
        let epsilon_or_predicate = if with_predicate { 4 } else { 1 };
        AtnDeserializer::new(&SerializedAtn::from_i32(&[
            4, 0, 2, // version, lexer, max token type
            9, // states
            6, -1, // 0 token start
            2, 0, // 1 rule 0 start
            1, 0, // 2
            1, 0, // 3
            7, 0, // 4 rule 0 stop
            2, 1, // 5 rule 1 start
            1, 1, // 6
            1, 1, // 7
            7, 1, // 8 rule 1 stop
            0, // non-greedy
            0, // precedence
            2, // rules
            1, 1, // rule 0 starts at 1, token type 1
            5, 2, // rule 1 starts at 5, token type 2
            1, // modes
            0, // default mode starts at 0
            0, // sets
            8, // edges
            0, 1, 1, 0, 0, 0, // start -> rule 0
            0, 5, 1, 0, 0, 0, // start -> rule 1
            1, 2, 5, 'a' as i32, 0, 0, // 'a'
            2, 3, 5, 'b' as i32, 0, 0, // 'b'
            3, 4, epsilon_or_predicate, 0, 0, 0, // epsilon or predicate to stop
            5, 6, 5, ' ' as i32, 0, 0, // ' '
            6, 7, 1, 0, 0, 0, //
            7, 8, 6, 1, 0, 0, // action 0, then stop
            1, // decisions
            0, 1, // lexer actions
            6, 0, 0, // skip
        ]))
        .deserialize()
        .expect("artificial lexer ATN should deserialize")
    }

    /// One token rule matching `[\u{100}-\u{200}]+`, exercising wide rows.
    // `#[rustfmt::skip]`: keep the one-record-per-line ATN grouping (see
    // `two_rule_atn`). Pure literals keep rustfmt's packed layout today, but a
    // single cast/path element would explode it, so pin it defensively.
    #[rustfmt::skip]
    fn wide_range_atn() -> Atn {
        AtnDeserializer::new(&SerializedAtn::from_i32(&[
            4, 0, 1, // version, lexer, max token type
            5, // states
            6, -1, // 0 token start
            2, 0, // 1 rule 0 start
            1, 0, // 2
            1, 0, // 3
            7, 0, // 4 rule 0 stop
            0, // non-greedy
            0, // precedence
            1, // rules
            1, 1, // rule 0 starts at 1, token type 1
            1, // modes
            0, // default mode starts at 0
            0, // sets
            5, // edges
            0, 1, 1, 0, 0, 0, // start -> rule 0
            1, 2, 1, 0, 0, 0, //
            2, 3, 2, 0x100, 0x200, 0, // range
            3, 2, 1, 0, 0, 0, // greedy loop continues first
            3, 4, 1, 0, 0, 0, // then exits to stop
            0, // decisions
            0, // lexer actions
        ]))
        .deserialize()
        .expect("artificial wide-range lexer ATN should deserialize")
    }

    #[test]
    fn compiled_dfa_matches_longest_token_and_skips() {
        let atn = two_rule_atn(false);
        let dfa = CompiledLexerDfa::compile(&atn);
        assert!(dfa.has_compiled_modes());
        assert!(dfa.mode_start(0).is_some());

        let mut lexer = BaseLexer::new(InputStream::new(" ab"), recognizer_data());
        let token = next_token_compiled(&mut lexer, &atn, &dfa);
        assert_eq!(token.token_type(), 1);
        assert_eq!(token.text(), "ab");
        assert_eq!(
            next_token_compiled(&mut lexer, &atn, &dfa).token_type(),
            TOKEN_EOF
        );
    }

    #[test]
    fn predicate_edge_escapes_to_the_interpreter() {
        let atn = two_rule_atn(true);
        let dfa = CompiledLexerDfa::compile(&atn);
        // The predicate sits mid-rule, so the mode still compiles; only the
        // edge that would cross it escapes to the interpreter.
        assert!(dfa.mode_start(0).is_some());

        let mut lexer = BaseLexer::new(InputStream::new(" ab"), recognizer_data());
        let token = next_token_compiled_with_hooks(
            &mut lexer,
            &atn,
            &dfa,
            |_, _| {},
            |_, _| true,
            |_, _, _| {},
        );
        assert_eq!(token.token_type(), 1);
        assert_eq!(token.text(), "ab");
    }

    #[test]
    fn compiled_dfa_walks_wide_ranges() {
        let atn = wide_range_atn();
        let dfa = CompiledLexerDfa::compile(&atn);
        assert!(dfa.mode_start(0).is_some());

        let mut lexer = BaseLexer::new(InputStream::new("ĀĂ"), recognizer_data());
        let token = next_token_compiled(&mut lexer, &atn, &dfa);
        assert_eq!(token.token_type(), 1);
        assert_eq!(token.text(), "ĀĂ");
        assert_eq!(
            next_token_compiled(&mut lexer, &atn, &dfa).token_type(),
            TOKEN_EOF
        );
    }

    #[test]
    fn compiled_dfa_reports_recognition_errors_like_the_interpreter() {
        let atn = wide_range_atn();
        let dfa = CompiledLexerDfa::compile(&atn);

        let mut compiled = BaseLexer::new(InputStream::new("zĀ"), recognizer_data());
        let mut interpreted = BaseLexer::new(InputStream::new("zĀ"), recognizer_data());
        loop {
            let compiled_token = next_token_compiled(&mut compiled, &atn, &dfa);
            let interpreted_token = next_token(&mut interpreted, &atn);
            assert_eq!(compiled_token.token_type(), interpreted_token.token_type());
            assert_eq!(compiled_token.text(), interpreted_token.text());
            if compiled_token.token_type() == TOKEN_EOF {
                break;
            }
        }
        let compiled_errors: Vec<String> = compiled
            .drain_errors()
            .into_iter()
            .map(|error| error.message)
            .collect();
        let interpreted_errors: Vec<String> = interpreted
            .drain_errors()
            .into_iter()
            .map(|error| error.message)
            .collect();
        assert_eq!(compiled_errors, vec!["token recognition error at: 'z'"]);
        assert_eq!(compiled_errors, interpreted_errors);
    }

    #[test]
    fn serialization_round_trips() {
        let atn = two_rule_atn(false);
        let dfa = CompiledLexerDfa::compile(&atn);
        let stream = dfa.serialize();

        let restored =
            CompiledLexerDfa::from_serialized(&stream).expect("stream should deserialize");
        assert_eq!(restored.serialize(), stream);

        let mut lexer = BaseLexer::new(InputStream::new(" ab"), recognizer_data());
        let token = next_token_compiled(&mut lexer, &atn, &restored);
        assert_eq!(token.token_type(), 1);
        assert_eq!(token.text(), "ab");

        // A stream from a different runtime version is rejected, not trusted.
        let mut wrong_tag = stream;
        wrong_tag[0] ^= 1;
        assert!(CompiledLexerDfa::from_serialized(&wrong_tag).is_none());
    }

    #[test]
    fn malformed_wide_rows_are_rejected() {
        let atn = wide_range_atn();
        let stream = CompiledLexerDfa::compile(&atn).serialize();

        // Invert the [0x100, 0x200] range's bounds in place; a broken wide
        // row must fail validation, not silently misroute binary searches.
        let position = stream
            .windows(2)
            .position(|pair| pair == [0x100, 0x200])
            .expect("wide-range test grammar serializes its range bounds");
        let mut inverted = stream;
        inverted.swap(position, position + 1);
        assert!(CompiledLexerDfa::from_serialized(&inverted).is_none());
    }

    #[test]
    fn force_interpreted_bypasses_compiled_tables() {
        let atn = two_rule_atn(false);
        let dfa = CompiledLexerDfa::compile(&atn);

        let mut lexer = BaseLexer::new(InputStream::new("ab"), recognizer_data());
        lexer.set_force_interpreted(true);
        let token = next_token_compiled(&mut lexer, &atn, &dfa);
        assert_eq!(token.token_type(), 1);
        // The interpreter path records the learned-DFA trace; the compiled
        // walk does not.
        assert!(!lexer.lexer_dfa_string().is_empty());
    }
}
