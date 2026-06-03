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
        EMPTY_PREDICTION_CONTEXT.with(Rc::clone)
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
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_merge_call();
        if left == right {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_merge_identical();
            return left;
        }
        if let Some(cache) = cache.as_deref_mut() {
            if let Some(merged) = cache.get(&left, &right) {
                #[cfg(feature = "perf-counters")]
                crate::perf::record_context_merge_cache_hit();
                return merged;
            }
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_merge_cache_miss();
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_merge_uncached();
        let merged = merge_contexts_uncached(&left, &right, root_is_wildcard, cache.as_deref_mut());
        if let Some(cache) = cache {
            cache.insert(&left, &right, &merged);
        }
        merged
    }
}

/// Dispatches a context merge to the singleton/array helpers, faithfully
/// mirroring ANTLR's `mergeSingletons`/`mergeArrays` (Go `prediction_context.go`,
/// Java `PredictionContext.java`). The merge cache and `root_is_wildcard` are
/// threaded so recursive parent merges canonicalize identically to the
/// reference: two stack entries that share a return state are collapsed into a
/// single entry whose parent is the recursive merge of their parents, rather
/// than kept as duplicate entries.
fn merge_contexts_uncached(
    left: &Rc<PredictionContext>,
    right: &Rc<PredictionContext>,
    root_is_wildcard: bool,
    mut cache: Option<&mut PredictionContextMergeCache>,
) -> Rc<PredictionContext> {
    if left == right {
        return Rc::clone(left);
    }
    match (left.as_ref(), right.as_ref()) {
        // Two single-entry contexts (Empty counts as the `$` entry).
        (
            PredictionContext::Empty { .. } | PredictionContext::Singleton { .. },
            PredictionContext::Empty { .. } | PredictionContext::Singleton { .. },
        ) => merge_singletons(left, right, root_is_wildcard, cache.as_deref_mut()),
        (
            PredictionContext::Array {
                parents,
                return_states,
                ..
            },
            PredictionContext::Empty { .. } | PredictionContext::Singleton { .. },
        ) => {
            let (parent, return_state) = first_context_entry(right);
            let right_array = single_entry_as_array(parent, return_state);
            merge_arrays(
                parents,
                return_states,
                &right_array.0,
                &right_array.1,
                root_is_wildcard,
                cache.as_deref_mut(),
            )
        }
        (
            PredictionContext::Empty { .. } | PredictionContext::Singleton { .. },
            PredictionContext::Array {
                parents,
                return_states,
                ..
            },
        ) => {
            let (parent, return_state) = first_context_entry(left);
            let left_array = single_entry_as_array(parent, return_state);
            merge_arrays(
                &left_array.0,
                &left_array.1,
                parents,
                return_states,
                root_is_wildcard,
                cache.as_deref_mut(),
            )
        }
        (
            PredictionContext::Array {
                parents: left_parents,
                return_states: left_return_states,
                ..
            },
            PredictionContext::Array {
                parents: right_parents,
                return_states: right_return_states,
                ..
            },
        ) => merge_arrays(
            left_parents,
            left_return_states,
            right_parents,
            right_return_states,
            root_is_wildcard,
            cache,
        ),
    }
}

fn first_context_entry(context: &Rc<PredictionContext>) -> (Rc<PredictionContext>, usize) {
    match context.as_ref() {
        PredictionContext::Empty { .. } => (PredictionContext::empty(), EMPTY_RETURN_STATE),
        PredictionContext::Singleton {
            parent,
            return_state,
            ..
        } => (Rc::clone(parent), *return_state),
        PredictionContext::Array { .. } => unreachable!("array contexts have multiple entries"),
    }
}

/// Represents a single-entry context (Empty or Singleton) as a 1-element array
/// so the array merge can handle the singleton/array mixed cases uniformly,
/// mirroring how ANTLR routes those through `mergeArrays`. The `$` (empty) entry
/// is encoded as an `EMPTY_RETURN_STATE` slot whose parent is the empty context.
fn single_entry_as_array(
    parent: Rc<PredictionContext>,
    return_state: usize,
) -> (Vec<Rc<PredictionContext>>, Vec<usize>) {
    (vec![parent], vec![return_state])
}

/// Faithful port of ANTLR's `mergeSingletons`: merges two single-entry contexts,
/// recursively merging parents when the return states match (`ax + ay = a'[x,y]`
/// collapses to one entry with a merged parent).
fn merge_singletons(
    a: &Rc<PredictionContext>,
    b: &Rc<PredictionContext>,
    root_is_wildcard: bool,
    cache: Option<&mut PredictionContextMergeCache>,
) -> Rc<PredictionContext> {
    if let Some(root) = merge_root(a, b, root_is_wildcard) {
        return root;
    }
    let (a_parent, a_return_state) = first_context_entry(a);
    let (b_parent, b_return_state) = first_context_entry(b);

    if a_return_state == b_return_state {
        let parent =
            PredictionContext::merge_with_options(a_parent.clone(), b_parent.clone(), root_is_wildcard, cache);
        // ax + bx = ax, if the merged parent reduced to an existing parent.
        if parent == a_parent {
            return Rc::clone(a);
        }
        if parent == b_parent {
            return Rc::clone(b);
        }
        // ax + ay = a'[x,y]: a new singleton over the merged parent.
        return PredictionContext::singleton(parent, a_return_state);
    }

    // Return states differ. If the parents are identical we can share them.
    let single_parent = if a_parent == b_parent {
        Some(a_parent.clone())
    } else {
        None
    };
    if let Some(single_parent) = single_parent {
        let (parents, return_states) =
            sort_two_entries(single_parent.clone(), a_return_state, single_parent, b_return_state);
        return PredictionContext::array(parents, return_states);
    }
    // Parents differ and cannot merge: pack into a two-entry array.
    let (parents, return_states) =
        sort_two_entries(a_parent, a_return_state, b_parent, b_return_state);
    PredictionContext::array(parents, return_states)
}

/// Handles the `$`-root cases of a singleton merge (ANTLR `mergeRoot`).
/// Returns `Some` when one side is the empty (`$`) context; `None` otherwise.
fn merge_root(
    a: &Rc<PredictionContext>,
    b: &Rc<PredictionContext>,
    root_is_wildcard: bool,
) -> Option<Rc<PredictionContext>> {
    let a_empty = a.is_empty();
    let b_empty = b.is_empty();
    if !a_empty && !b_empty {
        return None;
    }
    if root_is_wildcard {
        // In SLL mode the empty root is a wildcard that absorbs the other side.
        if a_empty {
            return Some(PredictionContext::empty());
        }
        if b_empty {
            return Some(PredictionContext::empty());
        }
        return None;
    }
    // Full LL mode: `$` is an ordinary entry that joins the array.
    if a_empty && b_empty {
        return Some(PredictionContext::empty());
    }
    if a_empty {
        let (parent, return_state) = first_context_entry(b);
        let (parents, return_states) = sort_two_entries(
            parent,
            return_state,
            PredictionContext::empty(),
            EMPTY_RETURN_STATE,
        );
        return Some(PredictionContext::array(parents, return_states));
    }
    // b_empty
    let (parent, return_state) = first_context_entry(a);
    let (parents, return_states) = sort_two_entries(
        parent,
        return_state,
        PredictionContext::empty(),
        EMPTY_RETURN_STATE,
    );
    Some(PredictionContext::array(parents, return_states))
}

/// Builds the `(parents, return_states)` for a two-entry array sorted by
/// return state (ANTLR keeps array entries sorted by payload/return state).
fn sort_two_entries(
    first_parent: Rc<PredictionContext>,
    first_return_state: usize,
    second_parent: Rc<PredictionContext>,
    second_return_state: usize,
) -> (Vec<Rc<PredictionContext>>, Vec<usize>) {
    if first_return_state <= second_return_state {
        (
            vec![first_parent, second_parent],
            vec![first_return_state, second_return_state],
        )
    } else {
        (
            vec![second_parent, first_parent],
            vec![second_return_state, first_return_state],
        )
    }
}

/// Faithful port of ANTLR's `mergeArrays`: merges two return-state-sorted arrays,
/// recursively merging the parents of entries that share a return state and
/// collapsing them into a single entry, then sharing structurally-equal parents.
fn merge_arrays(
    left_parents: &[Rc<PredictionContext>],
    left_return_states: &[usize],
    right_parents: &[Rc<PredictionContext>],
    right_return_states: &[usize],
    root_is_wildcard: bool,
    mut cache: Option<&mut PredictionContextMergeCache>,
) -> Rc<PredictionContext> {
    let mut parents = Vec::with_capacity(left_parents.len() + right_parents.len());
    let mut return_states = Vec::with_capacity(left_return_states.len() + right_return_states.len());
    let mut left_index = 0;
    let mut right_index = 0;

    while left_index < left_parents.len() && right_index < right_parents.len() {
        let left_return_state = left_return_states[left_index];
        let right_return_state = right_return_states[right_index];
        match left_return_state.cmp(&right_return_state) {
            Ordering::Equal => {
                // Same payload (stack top): yield a single entry whose parent is
                // the recursive merge of the two parents.
                let left_parent = &left_parents[left_index];
                let right_parent = &right_parents[right_index];
                let merged_parent = if left_parent == right_parent {
                    Rc::clone(left_parent)
                } else {
                    PredictionContext::merge_with_options(
                        Rc::clone(left_parent),
                        Rc::clone(right_parent),
                        root_is_wildcard,
                        cache.as_deref_mut(),
                    )
                };
                parents.push(merged_parent);
                return_states.push(left_return_state);
                left_index += 1;
                right_index += 1;
            }
            Ordering::Less => {
                parents.push(Rc::clone(&left_parents[left_index]));
                return_states.push(left_return_state);
                left_index += 1;
            }
            Ordering::Greater => {
                parents.push(Rc::clone(&right_parents[right_index]));
                return_states.push(right_return_state);
                right_index += 1;
            }
        }
    }
    for index in left_index..left_parents.len() {
        parents.push(Rc::clone(&left_parents[index]));
        return_states.push(left_return_states[index]);
    }
    for index in right_index..right_parents.len() {
        parents.push(Rc::clone(&right_parents[index]));
        return_states.push(right_return_states[index]);
    }

    if parents.len() == 1 {
        return PredictionContext::singleton(
            parents.pop().expect("single merged parent"),
            return_states.pop().expect("single merged return state"),
        );
    }
    combine_common_parents(&mut parents);
    PredictionContext::array(parents, return_states)
}

/// Shares structurally-equal-but-distinct `Rc` parents in a merged array so
/// downstream equality/hash checks short-circuit on pointer identity (ANTLR
/// `combineCommonParents`). Behaviour-preserving: it only replaces a parent with
/// an equal one already present in the same array.
fn combine_common_parents(parents: &mut [Rc<PredictionContext>]) {
    let mut unique: Vec<Rc<PredictionContext>> = Vec::new();
    for parent in parents.iter_mut() {
        if let Some(existing) = unique.iter().find(|candidate| *candidate == parent) {
            *parent = Rc::clone(existing);
        } else {
            unique.push(Rc::clone(parent));
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

thread_local! {
    static EMPTY_PREDICTION_CONTEXT: Rc<PredictionContext> = Rc::new(PredictionContext::Empty {
        cached_hash: prediction_context_empty_hash(),
    });
}

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
        self.entries.get(&key).cloned()
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

/// Shared canonical store for prediction-context graphs retained in DFA states.
#[derive(Debug)]
pub(crate) struct PredictionContextCache {
    empty: Rc<PredictionContext>,
    entries: FxHashMap<Rc<PredictionContext>, Rc<PredictionContext>>,
}

impl PredictionContextCache {
    pub(crate) fn new() -> Self {
        Self {
            empty: PredictionContext::empty(),
            entries: FxHashMap::default(),
        }
    }

    pub(crate) fn get_cached_context(
        &mut self,
        context: &Rc<PredictionContext>,
    ) -> Rc<PredictionContext> {
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_cache_call();
        if context.is_empty() {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_cache_empty();
            return Rc::clone(&self.empty);
        }
        if let Some(existing) = self.entries.get(context) {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_cache_hit();
            return Rc::clone(existing);
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_cache_miss();
        let mut visited = FxHashMap::default();
        let cached = self.get_cached_context_inner(context, &mut visited);
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_cache_visited(visited.len());
        cached
    }

    fn get_cached_context_inner(
        &mut self,
        context: &Rc<PredictionContext>,
        visited: &mut FxHashMap<Rc<PredictionContext>, Rc<PredictionContext>>,
    ) -> Rc<PredictionContext> {
        if context.is_empty() {
            return Rc::clone(&self.empty);
        }
        if let Some(existing) = visited.get(context) {
            return Rc::clone(existing);
        }
        if let Some(existing) = self.entries.get(context) {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_cache_hit();
            let existing = Rc::clone(existing);
            visited.insert(Rc::clone(context), Rc::clone(&existing));
            return existing;
        }
        let cached = match context.as_ref() {
            PredictionContext::Empty { .. } => Rc::clone(&self.empty),
            PredictionContext::Singleton {
                parent,
                return_state,
                ..
            } => {
                let cached_parent = self.get_cached_context_inner(parent, visited);
                if Rc::ptr_eq(parent, &cached_parent) {
                    self.add(Rc::clone(context))
                } else {
                    self.add(PredictionContext::singleton(cached_parent, *return_state))
                }
            }
            PredictionContext::Array {
                parents,
                return_states,
                ..
            } => {
                let mut changed = false;
                let mut cached_parents = Vec::with_capacity(parents.len());
                for parent in parents {
                    let cached_parent = self.get_cached_context_inner(parent, visited);
                    changed |= !Rc::ptr_eq(parent, &cached_parent);
                    cached_parents.push(cached_parent);
                }
                if changed {
                    self.add(PredictionContext::array(
                        cached_parents,
                        return_states.clone(),
                    ))
                } else {
                    self.add(Rc::clone(context))
                }
            }
        };
        visited.insert(Rc::clone(context), Rc::clone(&cached));
        cached
    }

    fn add(&mut self, context: Rc<PredictionContext>) -> Rc<PredictionContext> {
        if context.is_empty() {
            return Rc::clone(&self.empty);
        }
        if let Some(existing) = self.entries.get(&context) {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_cache_hit();
            return Rc::clone(existing);
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_cache_insert();
        self.entries
            .insert(Rc::clone(&context), Rc::clone(&context));
        context
    }
}

impl Default for PredictionContextCache {
    fn default() -> Self {
        Self::new()
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
        let left_hash = prediction_context_hash(left);
        let right_hash = prediction_context_hash(right);
        if should_swap_merge_key(left, left_hash, right, right_hash) {
            return Self {
                left: Rc::clone(right),
                right: Rc::clone(left),
                left_hash: right_hash,
                right_hash: left_hash,
            };
        }
        Self {
            left: Rc::clone(left),
            right: Rc::clone(right),
            left_hash,
            right_hash,
        }
    }
}

fn should_swap_merge_key(
    left: &Rc<PredictionContext>,
    left_hash: u64,
    right: &Rc<PredictionContext>,
    right_hash: u64,
) -> bool {
    (right_hash, Rc::as_ptr(right) as usize) < (left_hash, Rc::as_ptr(left) as usize)
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

#[derive(Clone, Debug, Default)]
pub struct AtnConfigSet {
    configs: Vec<AtnConfig>,
    config_index: FxHashMap<AtnConfigKey, usize>,
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

    pub fn new_full_context(full_context: bool) -> Self {
        Self {
            configs: Vec::new(),
            config_index: FxHashMap::default(),
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
        #[cfg(feature = "perf-counters")]
        crate::perf::record_config_add_call();
        if !config.semantic_context.is_none() {
            self.has_semantic_context = true;
        }
        if config.reaches_into_outer_context > 0 {
            self.dips_into_outer_context = true;
        }
        let key = AtnConfigKey::from(&config);
        if let Some(existing_index) = self.config_index.get(&key).copied() {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_config_merge();
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
            #[cfg(feature = "perf-counters")]
            crate::perf::record_config_insert(self.configs.len());
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

    pub(crate) fn optimize_contexts(&mut self, cache: &mut PredictionContextCache) {
        assert!(!self.readonly, "cannot mutate readonly ATN config set");
        for config in &mut self.configs {
            config.context = cache.get_cached_context(&config.context);
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

impl PartialEq for AtnConfigSet {
    fn eq(&self, other: &Self) -> bool {
        self.configs == other.configs
            && self.full_context == other.full_context
            && self.has_semantic_context == other.has_semantic_context
            && self.dips_into_outer_context == other.dips_into_outer_context
    }
}

impl Eq for AtnConfigSet {}

impl Ord for AtnConfigSet {
    fn cmp(&self, other: &Self) -> Ordering {
        self.configs
            .cmp(&other.configs)
            .then_with(|| self.full_context.cmp(&other.full_context))
            .then_with(|| self.has_semantic_context.cmp(&other.has_semantic_context))
            .then_with(|| {
                self.dips_into_outer_context
                    .cmp(&other.dips_into_outer_context)
            })
    }
}

impl PartialOrd for AtnConfigSet {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

pub fn resolves_to_just_one_viable_alt(configs: &[AtnConfig]) -> Option<usize> {
    single_viable_alt(&conflicting_alt_subsets(configs))
}

fn single_viable_alt(alt_subsets: &[BTreeSet<usize>]) -> Option<usize> {
    let mut result = None;
    for alts in alt_subsets {
        let min_alt = alts.iter().next().copied()?;
        match result {
            None => result = Some(min_alt),
            Some(existing) if existing == min_alt => {}
            Some(_) => return None,
        }
    }
    result
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
    fn viable_alt_resolves_to_shared_conflict_minimum() {
        let empty = PredictionContext::empty();
        let mut set = AtnConfigSet::new_full_context(true);
        set.add(AtnConfig::new(10, 1, Rc::clone(&empty)));
        set.add(AtnConfig::new(10, 2, Rc::clone(&empty)));
        set.add(AtnConfig::new(11, 1, empty));

        assert_eq!(resolves_to_just_one_viable_alt(set.configs()), Some(1));
    }

    #[test]
    fn viable_alt_keeps_looking_for_different_conflict_minimums() {
        let empty = PredictionContext::empty();
        let mut set = AtnConfigSet::new_full_context(true);
        set.add(AtnConfig::new(10, 1, Rc::clone(&empty)));
        set.add(AtnConfig::new(10, 2, Rc::clone(&empty)));
        set.add(AtnConfig::new(11, 2, Rc::clone(&empty)));
        set.add(AtnConfig::new(11, 3, empty));

        assert_eq!(resolves_to_just_one_viable_alt(set.configs()), None);
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

    #[test]
    fn merge_arrays_linearly_preserves_order_and_deduplicates_entries() {
        let empty = PredictionContext::empty();
        let parent_one = PredictionContext::singleton(Rc::clone(&empty), 1);
        let parent_two = PredictionContext::singleton(Rc::clone(&empty), 2);
        let parent_three = PredictionContext::singleton(Rc::clone(&empty), 3);
        let left = PredictionContext::array(
            vec![Rc::clone(&parent_one), Rc::clone(&parent_three)],
            vec![10, 30],
        );
        let right =
            PredictionContext::array(vec![parent_two, Rc::clone(&parent_three)], vec![20, 30]);

        let merged = PredictionContext::merge(left, right);

        assert_eq!(merged.len(), 3);
        assert_eq!(merged.return_state(0), Some(10));
        assert_eq!(merged.parent(0), Some(parent_one));
        assert_eq!(merged.return_state(1), Some(20));
        assert_eq!(merged.return_state(2), Some(30));
        assert_eq!(merged.parent(2), Some(parent_three));
    }

    #[test]
    fn prediction_context_cache_reuses_equal_context_graphs() {
        let mut cache = PredictionContextCache::new();
        let left_parent = PredictionContext::singleton(PredictionContext::empty(), 1);
        let right_parent = PredictionContext::singleton(PredictionContext::empty(), 1);
        let left = PredictionContext::singleton(left_parent, 42);
        let right = PredictionContext::singleton(right_parent, 42);

        let cached_left = cache.get_cached_context(&left);
        let cached_right = cache.get_cached_context(&right);
        let cached_left_parent = cached_left.parent(0).expect("singleton parent");
        let cached_right_parent = cached_right.parent(0).expect("singleton parent");

        assert!(Rc::ptr_eq(&cached_left, &cached_right));
        assert!(Rc::ptr_eq(&cached_left_parent, &cached_right_parent));
    }

    #[test]
    fn config_set_optimize_contexts_canonicalizes_contexts() {
        let mut cache = PredictionContextCache::new();
        let first = PredictionContext::singleton(PredictionContext::empty(), 7);
        let second = PredictionContext::singleton(PredictionContext::empty(), 7);
        let mut set = AtnConfigSet::new();
        set.add(AtnConfig::new(1, 1, first));
        set.add(AtnConfig::new(2, 2, second));

        set.optimize_contexts(&mut cache);

        assert!(Rc::ptr_eq(&set.configs[0].context, &set.configs[1].context));
    }
}
