use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::rc::Rc;

pub const EMPTY_RETURN_STATE: usize = usize::MAX;

/// Lightweight `FxHash`-style hasher.
///
/// Used by `BaseLexer`'s DFA-trace map and the `epsilon_closure` `seen`
/// set to avoid the `SipHash` overhead of `std::collections::HashMap`'s
/// default hasher on the hot lexer path.
#[derive(Debug, Default)]
pub struct PredictionFxHasher {
    hash: u64,
}

const FX_ROT: u32 = 5;
const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl Hasher for PredictionFxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut bytes = bytes;
        while bytes.len() >= 8 {
            let (head, rest) = bytes.split_at(8);
            let word = u64::from_le_bytes(head.try_into().expect("8-byte chunk"));
            self.hash = (self.hash.rotate_left(FX_ROT) ^ word).wrapping_mul(FX_SEED);
            bytes = rest;
        }
        for &b in bytes {
            self.hash = (self.hash.rotate_left(FX_ROT) ^ u64::from(b)).wrapping_mul(FX_SEED);
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.hash = (self.hash.rotate_left(FX_ROT) ^ u64::from(value)).wrapping_mul(FX_SEED);
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.hash = (self.hash.rotate_left(FX_ROT) ^ u64::from(value)).wrapping_mul(FX_SEED);
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.hash = (self.hash.rotate_left(FX_ROT) ^ value).wrapping_mul(FX_SEED);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.hash = (self.hash.rotate_left(FX_ROT) ^ value as u64).wrapping_mul(FX_SEED);
    }

    #[inline]
    fn write_i32(&mut self, value: i32) {
        self.write_u32(i32::cast_unsigned(value));
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<PredictionFxHasher>>;

#[derive(Clone, Debug)]
pub enum PredictionContext {
    Empty {
        cached_hash: u64,
    },
    Singleton {
        parent: Rc<Self>,
        return_state: usize,
        cached_hash: u64,
    },
    Array {
        parents: Vec<Rc<Self>>,
        return_states: Vec<usize>,
        cached_hash: u64,
    },
}

impl PredictionContext {
    pub fn empty() -> Rc<Self> {
        Rc::new(Self::Empty {
            cached_hash: prediction_context_empty_hash(),
        })
    }

    pub fn singleton(parent: Rc<Self>, return_state: usize) -> Rc<Self> {
        if return_state == EMPTY_RETURN_STATE {
            Self::empty()
        } else {
            Rc::new(Self::Singleton {
                cached_hash: prediction_context_singleton_hash(&parent, return_state),
                parent,
                return_state,
            })
        }
    }

    fn array(parents: Vec<Rc<Self>>, return_states: Vec<usize>) -> Rc<Self> {
        Rc::new(Self::Array {
            cached_hash: prediction_context_array_hash(&parents, &return_states),
            parents,
            return_states,
        })
    }

    pub const fn cached_hash(&self) -> u64 {
        match self {
            Self::Empty { cached_hash }
            | Self::Singleton { cached_hash, .. }
            | Self::Array { cached_hash, .. } => *cached_hash,
        }
    }

    pub const fn len(&self) -> usize {
        match self {
            Self::Empty { .. } => 1,
            Self::Singleton { .. } => 1,
            Self::Array { return_states, .. } => return_states.len(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty { .. })
    }

    pub fn return_state(&self, index: usize) -> Option<usize> {
        match self {
            Self::Empty { .. } if index == 0 => Some(EMPTY_RETURN_STATE),
            Self::Singleton { return_state, .. } if index == 0 => Some(*return_state),
            Self::Array { return_states, .. } => return_states.get(index).copied(),
            Self::Empty { .. } => None,
            Self::Singleton { .. } => None,
        }
    }

    pub fn parent(&self, index: usize) -> Option<Rc<Self>> {
        match self {
            Self::Empty { .. } => None,
            Self::Singleton { parent, .. } if index == 0 => Some(Rc::clone(parent)),
            Self::Array { parents, .. } => parents.get(index).cloned(),
            Self::Singleton { .. } => None,
        }
    }

    pub fn has_empty_path(&self) -> bool {
        match self {
            Self::Empty { .. } => true,
            Self::Singleton { return_state, .. } => *return_state == EMPTY_RETURN_STATE,
            Self::Array { return_states, .. } => return_states.contains(&EMPTY_RETURN_STATE),
        }
    }

    pub fn merge(left: Rc<Self>, right: Rc<Self>) -> Rc<Self> {
        Self::merge_with_options(left, right, false, None)
    }

    /// Merges two prediction contexts using ANTLR's SLL/LL root semantics.
    ///
    /// In SLL mode the empty root is a wildcard: `$ + x = $`. In full LL mode
    /// it is an ordinary array entry: `$ + x = [$, x]`. The optional merge
    /// cache is intentionally per prediction operation so large conflict-heavy
    /// parses can drop the cache immediately after `adaptive_predict`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn merge_with_options(
        left: Rc<Self>,
        right: Rc<Self>,
        root_is_wildcard: bool,
        mut cache: Option<&mut PredictionContextMergeCache>,
    ) -> Rc<Self> {
        if left == right {
            return left;
        }
        if let Some(cache) = cache.as_deref_mut() {
            if let Some(merged) = cache.get(&left, &right) {
                return merged;
            }
        }
        let merged = if root_is_wildcard && (left.is_empty() || right.is_empty()) {
            Self::empty()
        } else {
            merge_contexts_uncached(Rc::clone(&left), Rc::clone(&right))
        };
        if let Some(cache) = cache {
            cache.insert(&left, &right, &merged);
        }
        merged
    }
}

fn merge_contexts_uncached(
    left: Rc<PredictionContext>,
    right: Rc<PredictionContext>,
) -> Rc<PredictionContext> {
    if left == right {
        return left;
    }
    match (left.as_ref(), right.as_ref()) {
        (PredictionContext::Empty { .. }, PredictionContext::Empty { .. }) => {
            PredictionContext::empty()
        }
        _ => {
            let mut entries = Vec::new();
            collect_entries(&left, &mut entries);
            collect_entries(&right, &mut entries);
            drop((left, right));
            entries.sort_by_key(|(parent, return_state)| (*return_state, parent.cached_hash()));
            let mut deduplicated: Vec<(Rc<PredictionContext>, usize)> =
                Vec::with_capacity(entries.len());
            for (parent, return_state) in entries {
                let parent_hash = parent.cached_hash();
                let mut duplicate = false;
                for (existing_parent, existing_return_state) in deduplicated.iter().rev() {
                    if *existing_return_state != return_state
                        || existing_parent.cached_hash() != parent_hash
                    {
                        break;
                    }
                    if existing_parent == &parent {
                        duplicate = true;
                        break;
                    }
                }
                if !duplicate {
                    deduplicated.push((parent, return_state));
                }
            }
            let mut entries = deduplicated;
            if entries.len() == 1 {
                let (parent, return_state) = entries.remove(0);
                return PredictionContext::singleton(parent, return_state);
            }
            let parents = entries
                .iter()
                .map(|(parent, _)| Rc::clone(parent))
                .collect();
            let return_states = entries
                .iter()
                .map(|(_, return_state)| *return_state)
                .collect();
            PredictionContext::array(parents, return_states)
        }
    }
}

impl PartialEq for PredictionContext {
    fn eq(&self, other: &Self) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        if self.cached_hash() != other.cached_hash() {
            return false;
        }
        match (self, other) {
            (Self::Empty { .. }, Self::Empty { .. }) => true,
            (
                Self::Singleton {
                    parent,
                    return_state,
                    ..
                },
                Self::Singleton {
                    parent: other_parent,
                    return_state: other_return_state,
                    ..
                },
            ) => return_state == other_return_state && parent == other_parent,
            (
                Self::Array {
                    parents,
                    return_states,
                    ..
                },
                Self::Array {
                    parents: other_parents,
                    return_states: other_return_states,
                    ..
                },
            ) => return_states == other_return_states && parents == other_parents,
            _ => false,
        }
    }
}

impl Eq for PredictionContext {}

impl Hash for PredictionContext {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.cached_hash());
    }
}

impl Ord for PredictionContext {
    fn cmp(&self, other: &Self) -> Ordering {
        if std::ptr::eq(self, other) {
            return Ordering::Equal;
        }
        self.cached_hash()
            .cmp(&other.cached_hash())
            .then_with(|| prediction_context_variant(self).cmp(&prediction_context_variant(other)))
            .then_with(|| match (self, other) {
                (Self::Empty { .. }, Self::Empty { .. }) => Ordering::Equal,
                (
                    Self::Singleton {
                        parent,
                        return_state,
                        ..
                    },
                    Self::Singleton {
                        parent: other_parent,
                        return_state: other_return_state,
                        ..
                    },
                ) => return_state
                    .cmp(other_return_state)
                    .then_with(|| parent.cmp(other_parent)),
                (
                    Self::Array {
                        parents,
                        return_states,
                        ..
                    },
                    Self::Array {
                        parents: other_parents,
                        return_states: other_return_states,
                        ..
                    },
                ) => return_states
                    .cmp(other_return_states)
                    .then_with(|| parents.cmp(other_parents)),
                _ => Ordering::Equal,
            })
    }
}

impl PartialOrd for PredictionContext {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const fn prediction_context_variant(context: &PredictionContext) -> u8 {
    match context {
        PredictionContext::Empty { .. } => 0,
        PredictionContext::Singleton { .. } => 1,
        PredictionContext::Array { .. } => 2,
    }
}

fn prediction_context_empty_hash() -> u64 {
    let mut hasher = PredictionFxHasher::default();
    hasher.write_u8(0);
    hasher.finish()
}

fn prediction_context_singleton_hash(parent: &Rc<PredictionContext>, return_state: usize) -> u64 {
    let mut hasher = PredictionFxHasher::default();
    hasher.write_u8(1);
    hasher.write_u64(parent.cached_hash());
    hasher.write_usize(return_state);
    hasher.finish()
}

fn prediction_context_array_hash(
    parents: &[Rc<PredictionContext>],
    return_states: &[usize],
) -> u64 {
    let mut hasher = PredictionFxHasher::default();
    hasher.write_u8(2);
    hasher.write_usize(parents.len());
    for parent in parents {
        hasher.write_u64(parent.cached_hash());
    }
    hasher.write_usize(return_states.len());
    for return_state in return_states {
        hasher.write_usize(*return_state);
    }
    hasher.finish()
}

/// Per-prediction memo for graph-structured stack merges.
#[derive(Debug, Default)]
pub struct PredictionContextMergeCache {
    entries: FxHashMap<PredictionContextMergeKey, Rc<PredictionContext>>,
}

impl PredictionContextMergeCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(
        &self,
        left: &Rc<PredictionContext>,
        right: &Rc<PredictionContext>,
    ) -> Option<Rc<PredictionContext>> {
        let key = PredictionContextMergeKey::new(left, right);
        self.entries
            .get(&key)
            .or_else(|| self.entries.get(&key.reversed()))
            .cloned()
    }

    fn insert(
        &mut self,
        left: &Rc<PredictionContext>,
        right: &Rc<PredictionContext>,
        merged: &Rc<PredictionContext>,
    ) {
        self.entries.insert(
            PredictionContextMergeKey::new(left, right),
            Rc::clone(merged),
        );
    }
}

#[derive(Clone, Debug)]
struct PredictionContextMergeKey {
    left: Rc<PredictionContext>,
    right: Rc<PredictionContext>,
    left_hash: u64,
    right_hash: u64,
}

impl PredictionContextMergeKey {
    fn new(left: &Rc<PredictionContext>, right: &Rc<PredictionContext>) -> Self {
        Self {
            left: Rc::clone(left),
            right: Rc::clone(right),
            left_hash: prediction_context_hash(left),
            right_hash: prediction_context_hash(right),
        }
    }

    fn reversed(&self) -> Self {
        Self {
            left: Rc::clone(&self.right),
            right: Rc::clone(&self.left),
            left_hash: self.right_hash,
            right_hash: self.left_hash,
        }
    }
}

impl PartialEq for PredictionContextMergeKey {
    fn eq(&self, other: &Self) -> bool {
        self.left_hash == other.left_hash
            && self.right_hash == other.right_hash
            && self.left == other.left
            && self.right == other.right
    }
}

impl Eq for PredictionContextMergeKey {}

impl Hash for PredictionContextMergeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.left_hash);
        state.write_u64(self.right_hash);
    }
}

fn prediction_context_hash(context: &Rc<PredictionContext>) -> u64 {
    context.cached_hash()
}

fn collect_entries(
    context: &Rc<PredictionContext>,
    entries: &mut Vec<(Rc<PredictionContext>, usize)>,
) {
    match context.as_ref() {
        PredictionContext::Empty { .. } => {
            entries.push((Rc::clone(context), EMPTY_RETURN_STATE));
        }
        PredictionContext::Singleton {
            parent,
            return_state,
            ..
        } => entries.push((Rc::clone(parent), *return_state)),
        PredictionContext::Array {
            parents,
            return_states,
            ..
        } => {
            for (parent, return_state) in parents.iter().zip(return_states) {
                entries.push((Rc::clone(parent), *return_state));
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SemanticContext {
    None,
    Predicate {
        rule_index: usize,
        pred_index: usize,
        context_dependent: bool,
    },
    Precedence {
        precedence: i32,
    },
    And(Vec<Self>),
    Or(Vec<Self>),
}

impl SemanticContext {
    pub const fn none() -> Self {
        Self::None
    }

    pub fn and(left: Self, right: Self) -> Self {
        combine_semantic_context(left, right, true)
    }

    pub fn or(left: Self, right: Self) -> Self {
        combine_semantic_context(left, right, false)
    }

    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

fn combine_semantic_context(
    left: SemanticContext,
    right: SemanticContext,
    and: bool,
) -> SemanticContext {
    if left == right {
        return left;
    }
    if left.is_none() {
        return right;
    }
    if right.is_none() {
        return left;
    }
    let mut entries = Vec::new();
    for context in [left, right] {
        match (and, context) {
            (true, SemanticContext::And(children)) | (false, SemanticContext::Or(children)) => {
                entries.extend(children);
            }
            (_, other) => entries.push(other),
        }
    }
    entries.sort();
    entries.dedup();
    if and {
        SemanticContext::And(entries)
    } else {
        SemanticContext::Or(entries)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AtnConfig {
    pub state: usize,
    pub alt: usize,
    pub context: Rc<PredictionContext>,
    pub semantic_context: SemanticContext,
    pub reaches_into_outer_context: usize,
    pub precedence_filter_suppressed: bool,
}

impl AtnConfig {
    pub const fn new(state: usize, alt: usize, context: Rc<PredictionContext>) -> Self {
        Self {
            state,
            alt,
            context,
            semantic_context: SemanticContext::None,
            reaches_into_outer_context: 0,
            precedence_filter_suppressed: false,
        }
    }

    #[must_use]
    pub fn with_semantic_context(mut self, semantic_context: SemanticContext) -> Self {
        self.semantic_context = semantic_context;
        self
    }

    #[must_use]
    pub const fn with_reaches_into_outer_context(mut self, reaches: usize) -> Self {
        self.reaches_into_outer_context = reaches;
        self
    }
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct AtnConfigSet {
    configs: Vec<AtnConfig>,
    config_index: BTreeMap<AtnConfigKey, usize>,
    full_context: bool,
    unique_alt: Option<usize>,
    conflicting_alts: BTreeSet<usize>,
    has_semantic_context: bool,
    dips_into_outer_context: bool,
    readonly: bool,
}

impl AtnConfigSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn new_full_context(full_context: bool) -> Self {
        Self {
            configs: Vec::new(),
            config_index: BTreeMap::new(),
            full_context,
            unique_alt: None,
            conflicting_alts: BTreeSet::new(),
            has_semantic_context: false,
            dips_into_outer_context: false,
            readonly: false,
        }
    }

    pub fn add(&mut self, config: AtnConfig) -> bool {
        self.add_with_merge_cache(config, None)
    }

    /// Adds a configuration, merging prediction contexts for equivalent
    /// `(state, alt, semantic-context)` keys.
    pub fn add_with_merge_cache(
        &mut self,
        config: AtnConfig,
        cache: Option<&mut PredictionContextMergeCache>,
    ) -> bool {
        assert!(!self.readonly, "cannot mutate readonly ATN config set");
        if !config.semantic_context.is_none() {
            self.has_semantic_context = true;
        }
        if config.reaches_into_outer_context > 0 {
            self.dips_into_outer_context = true;
        }
        let key = AtnConfigKey::from(&config);
        if let Some(existing_index) = self.config_index.get(&key).copied() {
            let root_is_wildcard = !self.full_context;
            let existing = &mut self.configs[existing_index];
            existing.context = PredictionContext::merge_with_options(
                Rc::clone(&existing.context),
                config.context,
                root_is_wildcard,
                cache,
            );
            existing.reaches_into_outer_context = existing
                .reaches_into_outer_context
                .max(config.reaches_into_outer_context);
            existing.precedence_filter_suppressed |= config.precedence_filter_suppressed;
            false
        } else {
            let index = self.configs.len();
            self.config_index.insert(key, index);
            self.configs.push(config);
            self.unique_alt = None;
            self.conflicting_alts.clear();
            true
        }
    }

    pub fn configs(&self) -> &[AtnConfig] {
        &self.configs
    }

    pub const fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    pub const fn len(&self) -> usize {
        self.configs.len()
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
        if readonly {
            self.config_index.clear();
        }
    }

    pub const fn is_readonly(&self) -> bool {
        self.readonly
    }

    pub const fn full_context(&self) -> bool {
        self.full_context
    }

    pub const fn has_semantic_context(&self) -> bool {
        self.has_semantic_context
    }

    pub const fn set_has_semantic_context(&mut self, value: bool) {
        self.has_semantic_context = value;
    }

    pub const fn dips_into_outer_context(&self) -> bool {
        self.dips_into_outer_context
    }

    pub fn unique_alt(&mut self) -> Option<usize> {
        if self.unique_alt.is_none() {
            self.unique_alt = unique_alt(self.configs());
        }
        self.unique_alt
    }

    pub fn alts(&self) -> BTreeSet<usize> {
        self.configs.iter().map(|config| config.alt).collect()
    }

    pub fn conflicting_alt_subsets(&self) -> Vec<BTreeSet<usize>> {
        conflicting_alt_subsets(self.configs())
    }

    pub fn conflicting_alts(&mut self) -> BTreeSet<usize> {
        if self.conflicting_alts.is_empty() {
            self.conflicting_alts = self
                .conflicting_alt_subsets()
                .into_iter()
                .filter(|alts| alts.len() > 1)
                .flatten()
                .collect();
        }
        self.conflicting_alts.clone()
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AtnConfigKey {
    state: usize,
    alt: usize,
    semantic_context: SemanticContext,
}

impl From<&AtnConfig> for AtnConfigKey {
    fn from(config: &AtnConfig) -> Self {
        Self {
            state: config.state,
            alt: config.alt,
            semantic_context: config.semantic_context.clone(),
        }
    }
}

pub fn unique_alt(configs: &[AtnConfig]) -> Option<usize> {
    let mut alt = None;
    for config in configs {
        match alt {
            None => alt = Some(config.alt),
            Some(existing) if existing == config.alt => {}
            Some(_) => return None,
        }
    }
    alt
}

pub fn conflicting_alt_subsets(configs: &[AtnConfig]) -> Vec<BTreeSet<usize>> {
    let mut by_state_context = BTreeMap::<(usize, Rc<PredictionContext>), BTreeSet<usize>>::new();
    for config in configs {
        by_state_context
            .entry((config.state, Rc::clone(&config.context)))
            .or_default()
            .insert(config.alt);
    }
    by_state_context.into_values().collect()
}

pub fn has_sll_conflict_terminating_prediction(
    configs: &AtnConfigSet,
    is_rule_stop_state: impl Fn(usize) -> bool,
) -> bool {
    if configs
        .configs()
        .iter()
        .all(|config| is_rule_stop_state(config.state))
    {
        return true;
    }
    let alt_subsets = configs.conflicting_alt_subsets();
    alt_subsets.iter().any(|alts| alts.len() > 1)
        && !has_state_associated_with_one_alt(configs.configs())
}

fn has_state_associated_with_one_alt(configs: &[AtnConfig]) -> bool {
    let mut by_state = BTreeMap::<usize, BTreeSet<usize>>::new();
    for config in configs {
        by_state.entry(config.state).or_default().insert(config.alt);
    }
    by_state.values().any(|alts| alts.len() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_set_deduplicates_configs() {
        let empty = PredictionContext::empty();
        let mut set = AtnConfigSet::new();
        assert!(set.add(AtnConfig::new(1, 1, Rc::clone(&empty))));
        assert!(!set.add(AtnConfig::new(1, 1, Rc::clone(&empty))));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn sll_conflict_does_not_stop_for_empty_contexts_alone() {
        let empty = PredictionContext::empty();
        let mut set = AtnConfigSet::new();
        set.add(AtnConfig::new(1, 1, Rc::clone(&empty)));
        set.add(AtnConfig::new(2, 2, empty));

        assert!(!has_sll_conflict_terminating_prediction(&set, |_| false));
    }

    #[test]
    fn sll_conflict_stops_when_all_configs_reached_rule_stop() {
        let empty = PredictionContext::empty();
        let mut set = AtnConfigSet::new();
        set.add(AtnConfig::new(10, 1, Rc::clone(&empty)));
        set.add(AtnConfig::new(11, 2, empty));

        assert!(has_sll_conflict_terminating_prediction(&set, |state| {
            matches!(state, 10 | 11)
        }));
    }

    #[test]
    fn singleton_context_reports_parent_and_return_state() {
        let empty = PredictionContext::empty();
        let context = PredictionContext::singleton(Rc::clone(&empty), 42);
        assert_eq!(context.return_state(0), Some(42));
        assert_eq!(context.parent(0), Some(empty));
    }

    #[test]
    fn merge_with_empty_preserves_non_empty_return_state() {
        let empty = PredictionContext::empty();
        let singleton = PredictionContext::singleton(Rc::clone(&empty), 42);

        let merged = PredictionContext::merge(Rc::clone(&singleton), Rc::clone(&empty));

        assert_eq!(merged.len(), 2);
        assert_eq!(merged.return_state(0), Some(42));
        assert_eq!(merged.parent(0), Some(empty.clone()));
        assert_eq!(merged.return_state(1), Some(EMPTY_RETURN_STATE));
        assert_eq!(merged.parent(1), Some(empty));
    }

    #[test]
    fn merge_deduplicates_entries_with_same_parent_and_return_state() {
        let empty = PredictionContext::empty();
        let parent_one = PredictionContext::singleton(Rc::clone(&empty), 1);
        let parent_two = PredictionContext::singleton(Rc::clone(&empty), 2);
        let left = PredictionContext::array(vec![Rc::clone(&parent_one), parent_two], vec![42, 42]);
        let right = PredictionContext::singleton(Rc::clone(&parent_one), 42);

        let merged = PredictionContext::merge(left, right);

        assert_eq!(merged.len(), 2);
    }
}
