use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::mem::size_of;

pub const EMPTY_RETURN_STATE: usize = usize::MAX;
const COMPACT_EMPTY_RETURN_STATE: u32 = u32::MAX;

/// Lightweight `FxHash`-style hasher used on prediction hot paths.
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
        for &byte in bytes {
            self.hash = (self.hash.rotate_left(FX_ROT) ^ u64::from(byte)).wrapping_mul(FX_SEED);
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

/// Store-local identity for one canonical prediction-context graph node.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContextId(u32);

pub const EMPTY_CONTEXT: ContextId = ContextId(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContextTag {
    Empty,
    Singleton,
    Array,
}

#[derive(Clone, Copy, Debug)]
struct ContextRecord {
    tag: ContextTag,
    cached_hash: u64,
    parent_or_start: u32,
    return_state_or_len: u32,
}

/// Allocation and interning totals for one prediction-context arena.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PredictionContextStats {
    pub contexts_created: usize,
    pub singleton_contexts: usize,
    pub array_contexts: usize,
    pub array_entries: usize,
    pub interner_hits: usize,
    pub pooled_bytes: usize,
    /// Element storage implied by retained capacities, excluding allocator and
    /// hash-table control metadata.
    pub retained_bytes: usize,
    pub context_capacity: usize,
    pub array_parent_capacity: usize,
    pub array_return_state_capacity: usize,
    pub interner_capacity: usize,
    pub workspace_merge_cache_entries: usize,
    pub workspace_merge_cache_capacity: usize,
    pub workspace_entry_capacity: usize,
    pub outer_context_cache_hits: usize,
    pub outer_context_cache_misses: usize,
}

/// Canonical compact storage paired with one learned parser DFA store.
#[derive(Debug)]
pub(crate) struct ContextArena {
    records: Vec<ContextRecord>,
    array_parents: Vec<ContextId>,
    array_return_states: Vec<u32>,
    interner_heads: FxHashMap<u64, ContextId>,
    interner_next: Vec<Option<ContextId>>,
    interner_hits: usize,
    #[cfg(debug_assertions)]
    generation: u64,
}

#[cfg(debug_assertions)]
fn next_context_arena_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
    NEXT_GENERATION.fetch_add(1, AtomicOrdering::Relaxed)
}

impl ContextArena {
    pub(crate) fn new() -> Self {
        let empty = ContextRecord {
            tag: ContextTag::Empty,
            cached_hash: prediction_context_empty_hash(),
            parent_or_start: 0,
            return_state_or_len: 0,
        };
        let mut interner_heads = FxHashMap::default();
        interner_heads.insert(empty.cached_hash, EMPTY_CONTEXT);
        Self {
            records: vec![empty],
            array_parents: Vec::new(),
            array_return_states: Vec::new(),
            interner_heads,
            interner_next: vec![None],
            interner_hits: 0,
            #[cfg(debug_assertions)]
            generation: next_context_arena_generation(),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn stats(&self) -> PredictionContextStats {
        let mut singleton_contexts = 0;
        let mut array_contexts = 0;
        for record in &self.records {
            match record.tag {
                ContextTag::Empty => {}
                ContextTag::Singleton => singleton_contexts += 1,
                ContextTag::Array => array_contexts += 1,
            }
        }
        PredictionContextStats {
            contexts_created: self.records.len(),
            singleton_contexts,
            array_contexts,
            array_entries: self.array_parents.len(),
            interner_hits: self.interner_hits,
            pooled_bytes: self.records.len() * size_of::<ContextRecord>()
                + self.array_parents.len() * size_of::<ContextId>()
                + self.array_return_states.len() * size_of::<u32>()
                + self.interner_next.len() * size_of::<Option<ContextId>>(),
            retained_bytes: self.records.capacity() * size_of::<ContextRecord>()
                + self.array_parents.capacity() * size_of::<ContextId>()
                + self.array_return_states.capacity() * size_of::<u32>()
                + self.interner_heads.capacity() * size_of::<(u64, ContextId)>()
                + self.interner_next.capacity() * size_of::<Option<ContextId>>(),
            context_capacity: self.records.capacity(),
            array_parent_capacity: self.array_parents.capacity(),
            array_return_state_capacity: self.array_return_states.capacity(),
            interner_capacity: self.interner_heads.capacity(),
            workspace_merge_cache_entries: 0,
            workspace_merge_cache_capacity: 0,
            workspace_entry_capacity: 0,
            outer_context_cache_hits: 0,
            outer_context_cache_misses: 0,
        }
    }

    pub(crate) fn singleton(&mut self, parent: ContextId, return_state: usize) -> ContextId {
        self.assert_valid(parent);
        if return_state == EMPTY_RETURN_STATE {
            return EMPTY_CONTEXT;
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_cache_call();
        let return_state =
            u32::try_from(return_state).expect("prediction return state must fit in u32");
        let cached_hash = prediction_context_singleton_hash(self.cached_hash(parent), return_state);
        if let Some(existing) = self.find_interned(cached_hash, |record| {
            record.tag == ContextTag::Singleton
                && record.parent_or_start == parent.0
                && record.return_state_or_len == return_state
        }) {
            self.interner_hits = self.interner_hits.saturating_add(1);
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_cache_hit();
            return existing;
        }
        #[cfg(feature = "perf-counters")]
        {
            crate::perf::record_context_cache_miss();
            crate::perf::record_context_cache_insert();
        }
        self.push_record(ContextRecord {
            tag: ContextTag::Singleton,
            cached_hash,
            parent_or_start: parent.0,
            return_state_or_len: return_state,
        })
    }

    fn intern_entries(&mut self, entries: &[(ContextId, u32)]) -> ContextId {
        match entries {
            [] => EMPTY_CONTEXT,
            [(parent, return_state)] => {
                if *return_state == COMPACT_EMPTY_RETURN_STATE {
                    EMPTY_CONTEXT
                } else {
                    self.singleton(
                        *parent,
                        usize::try_from(*return_state).expect("u32 return state fits in usize"),
                    )
                }
            }
            _ => {
                debug_assert!(
                    entries
                        .windows(2)
                        .all(|pair| { compare_entries(pair[0], pair[1]) == Ordering::Less })
                );
                #[cfg(feature = "perf-counters")]
                crate::perf::record_context_cache_call();
                let cached_hash = prediction_context_array_hash(self, entries);
                if let Some(existing) = self.find_interned(cached_hash, |record| {
                    if record.tag != ContextTag::Array
                        || usize::try_from(record.return_state_or_len).ok() != Some(entries.len())
                    {
                        return false;
                    }
                    let start =
                        usize::try_from(record.parent_or_start).expect("u32 pool index fits usize");
                    let end = start + entries.len();
                    self.array_parents[start..end]
                        .iter()
                        .copied()
                        .zip(self.array_return_states[start..end].iter().copied())
                        .eq(entries.iter().copied())
                }) {
                    self.interner_hits = self.interner_hits.saturating_add(1);
                    #[cfg(feature = "perf-counters")]
                    crate::perf::record_context_cache_hit();
                    return existing;
                }
                #[cfg(feature = "perf-counters")]
                {
                    crate::perf::record_context_cache_miss();
                    crate::perf::record_context_cache_insert();
                }
                let start = u32::try_from(self.array_parents.len())
                    .expect("prediction-context parent pool must fit in u32");
                let len = u32::try_from(entries.len())
                    .expect("prediction-context array length must fit in u32");
                self.array_parents
                    .extend(entries.iter().map(|(parent, _)| *parent));
                self.array_return_states
                    .extend(entries.iter().map(|(_, return_state)| *return_state));
                self.push_record(ContextRecord {
                    tag: ContextTag::Array,
                    cached_hash,
                    parent_or_start: start,
                    return_state_or_len: len,
                })
            }
        }
    }

    fn find_interned(
        &self,
        cached_hash: u64,
        matches: impl Fn(&ContextRecord) -> bool,
    ) -> Option<ContextId> {
        let mut candidate = self.interner_heads.get(&cached_hash).copied();
        while let Some(id) = candidate {
            let index = usize::try_from(id.0).expect("u32 context ID fits in usize");
            let record = &self.records[index];
            if matches(record) {
                return Some(id);
            }
            candidate = self.interner_next[index];
        }
        None
    }

    fn push_record(&mut self, record: ContextRecord) -> ContextId {
        let id = ContextId(
            u32::try_from(self.records.len()).expect("prediction-context arena must fit in u32"),
        );
        let previous = self.interner_heads.insert(record.cached_hash, id);
        self.records.push(record);
        self.interner_next.push(previous);
        id
    }

    pub(crate) fn merge(
        &mut self,
        left: ContextId,
        right: ContextId,
        root_is_wildcard: bool,
        workspace: &mut PredictionWorkspace,
    ) -> ContextId {
        self.assert_valid(left);
        self.assert_valid(right);
        #[cfg(feature = "perf-counters")]
        crate::perf::record_context_merge_call();
        if left == right {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_merge_identical();
            return left;
        }
        let key = MergeKey::new(left, right, root_is_wildcard);
        if let Some(merged) = workspace.merge_cache.get(&key).copied() {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_context_merge_cache_hit();
            return merged;
        }
        #[cfg(feature = "perf-counters")]
        {
            crate::perf::record_context_merge_cache_miss();
            crate::perf::record_context_merge_uncached();
        }
        let merged = if root_is_wildcard && (left == EMPTY_CONTEXT || right == EMPTY_CONTEXT) {
            EMPTY_CONTEXT
        } else {
            self.merge_uncached(left, right, workspace)
        };
        workspace.merge_cache.insert(key, merged);
        merged
    }

    fn merge_uncached(
        &mut self,
        left: ContextId,
        right: ContextId,
        workspace: &mut PredictionWorkspace,
    ) -> ContextId {
        match (self.tag(left), self.tag(right)) {
            (ContextTag::Array, ContextTag::Array) => self.merge_arrays(left, right, workspace),
            (ContextTag::Array, _) => {
                let entry = self.first_entry(right);
                self.merge_array_with_entry(left, entry, false, workspace)
            }
            (_, ContextTag::Array) => {
                let entry = self.first_entry(left);
                self.merge_array_with_entry(right, entry, true, workspace)
            }
            _ => self.merge_two_entries(self.first_entry(left), self.first_entry(right), workspace),
        }
    }

    fn merge_two_entries(
        &mut self,
        left: (ContextId, u32),
        right: (ContextId, u32),
        workspace: &mut PredictionWorkspace,
    ) -> ContextId {
        if left == right {
            return self.intern_entries(std::slice::from_ref(&left));
        }
        workspace.entries.clear();
        if compare_entries(right, left) == Ordering::Less {
            workspace.entries.extend([right, left]);
        } else {
            workspace.entries.extend([left, right]);
        }
        self.intern_entries(&workspace.entries)
    }

    fn merge_array_with_entry(
        &mut self,
        array: ContextId,
        entry: (ContextId, u32),
        entry_on_left: bool,
        workspace: &mut PredictionWorkspace,
    ) -> ContextId {
        let array_len = self.len(array);
        let mut insert_index = array_len;
        for index in 0..array_len {
            let current = self.entry(array, index).expect("array entry in range");
            let ordering = compare_entries(entry, current);
            if ordering == Ordering::Equal {
                return array;
            }
            let should_insert = if entry_on_left {
                ordering != Ordering::Greater
            } else {
                ordering == Ordering::Less
            };
            if should_insert {
                insert_index = index;
                break;
            }
        }

        workspace.entries.clear();
        for index in 0..insert_index {
            workspace
                .entries
                .push(self.entry(array, index).expect("array entry in range"));
        }
        workspace.entries.push(entry);
        for index in insert_index..array_len {
            workspace
                .entries
                .push(self.entry(array, index).expect("array entry in range"));
        }
        self.intern_entries(&workspace.entries)
    }

    fn merge_arrays(
        &mut self,
        left: ContextId,
        right: ContextId,
        workspace: &mut PredictionWorkspace,
    ) -> ContextId {
        workspace.entries.clear();
        let mut left_index = 0;
        let mut right_index = 0;
        while left_index < self.len(left) && right_index < self.len(right) {
            let left_entry = self.entry(left, left_index).expect("array entry in range");
            let right_entry = self
                .entry(right, right_index)
                .expect("array entry in range");
            match compare_entries(left_entry, right_entry) {
                Ordering::Less => {
                    workspace.entries.push(left_entry);
                    left_index += 1;
                }
                Ordering::Greater => {
                    workspace.entries.push(right_entry);
                    right_index += 1;
                }
                Ordering::Equal => {
                    workspace.entries.push(left_entry);
                    left_index += 1;
                    right_index += 1;
                }
            }
        }
        while left_index < self.len(left) {
            workspace
                .entries
                .push(self.entry(left, left_index).expect("array entry in range"));
            left_index += 1;
        }
        while right_index < self.len(right) {
            workspace.entries.push(
                self.entry(right, right_index)
                    .expect("array entry in range"),
            );
            right_index += 1;
        }
        self.intern_entries(&workspace.entries)
    }

    pub(crate) fn len(&self, context: ContextId) -> usize {
        let record = self.record(context);
        match record.tag {
            ContextTag::Empty | ContextTag::Singleton => 1,
            ContextTag::Array => usize::try_from(record.return_state_or_len)
                .expect("u32 context length fits in usize"),
        }
    }

    pub(crate) fn is_empty(&self, context: ContextId) -> bool {
        self.assert_valid(context);
        context == EMPTY_CONTEXT
    }

    pub(crate) fn has_empty_path(&self, context: ContextId) -> bool {
        if context == EMPTY_CONTEXT {
            return true;
        }
        let record = self.record(context);
        match record.tag {
            ContextTag::Empty => true,
            ContextTag::Singleton => false,
            ContextTag::Array => {
                let len = usize::try_from(record.return_state_or_len)
                    .expect("u32 context length fits in usize");
                let start = usize::try_from(record.parent_or_start)
                    .expect("u32 context pool index fits in usize");
                self.array_return_states[start + len - 1] == COMPACT_EMPTY_RETURN_STATE
            }
        }
    }

    pub(crate) fn return_state(&self, context: ContextId, index: usize) -> Option<usize> {
        let (_, return_state) = self.entry(context, index)?;
        Some(expand_return_state(return_state))
    }

    pub(crate) fn parent(&self, context: ContextId, index: usize) -> Option<ContextId> {
        if context == EMPTY_CONTEXT {
            self.assert_valid(context);
            return None;
        }
        self.entry(context, index).map(|(parent, _)| parent)
    }

    fn first_entry(&self, context: ContextId) -> (ContextId, u32) {
        self.entry(context, 0)
            .expect("empty and singleton contexts have one logical entry")
    }

    fn entry(&self, context: ContextId, index: usize) -> Option<(ContextId, u32)> {
        let record = self.record(context);
        match record.tag {
            ContextTag::Empty if index == 0 => Some((EMPTY_CONTEXT, COMPACT_EMPTY_RETURN_STATE)),
            ContextTag::Singleton if index == 0 => Some((
                ContextId(record.parent_or_start),
                record.return_state_or_len,
            )),
            ContextTag::Array => {
                let len = usize::try_from(record.return_state_or_len).ok()?;
                if index >= len {
                    return None;
                }
                let start = usize::try_from(record.parent_or_start).ok()?;
                Some((
                    self.array_parents[start + index],
                    self.array_return_states[start + index],
                ))
            }
            ContextTag::Empty | ContextTag::Singleton => None,
        }
    }

    fn tag(&self, context: ContextId) -> ContextTag {
        self.record(context).tag
    }

    fn cached_hash(&self, context: ContextId) -> u64 {
        self.record(context).cached_hash
    }

    fn record(&self, context: ContextId) -> &ContextRecord {
        self.assert_valid(context);
        &self.records[usize::try_from(context.0).expect("u32 context ID fits in usize")]
    }

    pub(crate) fn assert_valid(&self, context: ContextId) {
        assert!(
            usize::try_from(context.0).is_ok_and(|index| index < self.records.len()),
            "prediction ContextId does not belong to this store"
        );
    }

    pub(crate) fn import_all(
        &mut self,
        source: &Self,
        workspace: &mut PredictionWorkspace,
    ) -> Vec<ContextId> {
        let mut remap = Vec::with_capacity(source.records.len());
        remap.push(EMPTY_CONTEXT);
        for source_index in 1..source.records.len() {
            let source_id = ContextId(
                u32::try_from(source_index).expect("source prediction-context ID fits in u32"),
            );
            let imported = match source.tag(source_id) {
                ContextTag::Empty => EMPTY_CONTEXT,
                ContextTag::Singleton => {
                    let (parent, return_state) = source.first_entry(source_id);
                    let parent_index =
                        usize::try_from(parent.0).expect("u32 context ID fits usize");
                    assert!(
                        parent_index < remap.len(),
                        "prediction contexts must reference earlier arena records"
                    );
                    self.singleton(remap[parent_index], expand_return_state(return_state))
                }
                ContextTag::Array => {
                    workspace.entries.clear();
                    for entry_index in 0..source.len(source_id) {
                        let (parent, return_state) = source
                            .entry(source_id, entry_index)
                            .expect("source array entry in range");
                        let parent_index =
                            usize::try_from(parent.0).expect("u32 context ID fits usize");
                        assert!(
                            parent_index < remap.len(),
                            "prediction contexts must reference earlier arena records"
                        );
                        workspace.entries.push((remap[parent_index], return_state));
                    }
                    workspace
                        .entries
                        .sort_unstable_by(|left, right| compare_entries(*left, *right));
                    workspace.entries.dedup();
                    self.intern_entries(&workspace.entries)
                }
            };
            remap.push(imported);
        }
        remap
    }
}

impl Default for ContextArena {
    fn default() -> Self {
        Self::new()
    }
}

fn compare_entries(left: (ContextId, u32), right: (ContextId, u32)) -> Ordering {
    left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
}

fn expand_return_state(return_state: u32) -> usize {
    if return_state == COMPACT_EMPTY_RETURN_STATE {
        EMPTY_RETURN_STATE
    } else {
        usize::try_from(return_state).expect("u32 return state fits in usize")
    }
}

fn prediction_context_empty_hash() -> u64 {
    let mut hasher = PredictionFxHasher::default();
    hasher.write_u8(0);
    hasher.finish()
}

fn prediction_context_singleton_hash(parent_hash: u64, return_state: u32) -> u64 {
    let mut hasher = PredictionFxHasher::default();
    hasher.write_u8(1);
    hasher.write_u64(parent_hash);
    hasher.write_u32(return_state);
    hasher.finish()
}

fn prediction_context_array_hash(arena: &ContextArena, entries: &[(ContextId, u32)]) -> u64 {
    let mut hasher = PredictionFxHasher::default();
    hasher.write_u8(2);
    hasher.write_usize(entries.len());
    for (parent, _) in entries {
        hasher.write_u64(arena.cached_hash(*parent));
    }
    hasher.write_usize(entries.len());
    for (_, return_state) in entries {
        hasher.write_u32(*return_state);
    }
    hasher.finish()
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MergeKey {
    left: ContextId,
    right: ContextId,
    root_is_wildcard: bool,
}

impl MergeKey {
    fn new(left: ContextId, right: ContextId, root_is_wildcard: bool) -> Self {
        let (left, right) = if right < left {
            (right, left)
        } else {
            (left, right)
        };
        Self {
            left,
            right,
            root_is_wildcard,
        }
    }
}

const MAX_RETAINED_MERGE_CACHE_ENTRIES: usize = 65_536;
const MAX_RETAINED_CONTEXT_ENTRIES: usize = 16_384;

/// Reusable per-prediction merge cache and temporary compact entry storage.
#[derive(Debug, Default)]
pub(crate) struct PredictionWorkspace {
    merge_cache: FxHashMap<MergeKey, ContextId>,
    entries: Vec<(ContextId, u32)>,
}

impl PredictionWorkspace {
    pub(crate) fn reset(&mut self) {
        if self.merge_cache.capacity() > MAX_RETAINED_MERGE_CACHE_ENTRIES {
            self.merge_cache = FxHashMap::default();
        } else {
            self.merge_cache.clear();
        }
        if self.entries.capacity() > MAX_RETAINED_CONTEXT_ENTRIES {
            self.entries = Vec::new();
        } else {
            self.entries.clear();
        }
    }

    pub(crate) fn merge_cache_capacity(&self) -> usize {
        self.merge_cache.capacity()
    }

    pub(crate) fn merge_cache_len(&self) -> usize {
        self.merge_cache.len()
    }

    pub(crate) const fn entry_capacity(&self) -> usize {
        self.entries.capacity()
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.merge_cache.capacity() * size_of::<(MergeKey, ContextId)>()
            + self.entries.capacity() * size_of::<(ContextId, u32)>()
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
pub(crate) struct AtnConfig {
    pub(crate) state: usize,
    pub(crate) alt: usize,
    pub(crate) context: ContextId,
    pub(crate) semantic_context: SemanticContext,
    pub(crate) reaches_into_outer_context: usize,
    pub(crate) precedence_filter_suppressed: bool,
    #[cfg(debug_assertions)]
    context_generation: u64,
}

impl AtnConfig {
    pub(crate) fn new(state: usize, alt: usize, context: ContextId, arena: &ContextArena) -> Self {
        arena.assert_valid(context);
        Self {
            state,
            alt,
            context,
            semantic_context: SemanticContext::None,
            reaches_into_outer_context: 0,
            precedence_filter_suppressed: false,
            #[cfg(debug_assertions)]
            context_generation: arena.generation(),
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn with_semantic_context(mut self, semantic_context: SemanticContext) -> Self {
        self.semantic_context = semantic_context;
        self
    }

    pub(crate) fn set_context(&mut self, context: ContextId, arena: &ContextArena) {
        arena.assert_valid(context);
        self.context = context;
        #[cfg(debug_assertions)]
        {
            self.context_generation = arena.generation();
        }
    }

    pub(crate) fn moved_to(&self, state: usize, context: ContextId, arena: &ContextArena) -> Self {
        let mut moved = Self::new(state, self.alt, context, arena);
        moved.semantic_context = self.semantic_context.clone();
        moved.reaches_into_outer_context = self.reaches_into_outer_context;
        moved.precedence_filter_suppressed = self.precedence_filter_suppressed;
        moved
    }

    pub(crate) fn assert_store(&self, arena: &ContextArena) {
        arena.assert_valid(self.context);
        #[cfg(debug_assertions)]
        assert_eq!(
            self.context_generation,
            arena.generation(),
            "ATN config carries a ContextId from another prediction store"
        );
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AtnConfigSet {
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
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn new_full_context(full_context: bool) -> Self {
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

    /// Adds a configuration, merging contexts for equivalent config keys.
    pub(crate) fn add(
        &mut self,
        config: AtnConfig,
        arena: &mut ContextArena,
        workspace: &mut PredictionWorkspace,
    ) -> bool {
        assert!(!self.readonly, "cannot mutate readonly ATN config set");
        config.assert_store(arena);
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
            let existing = &mut self.configs[existing_index];
            existing.assert_store(arena);
            existing.context = arena.merge(
                existing.context,
                config.context,
                !self.full_context,
                workspace,
            );
            existing.reaches_into_outer_context = existing
                .reaches_into_outer_context
                .max(config.reaches_into_outer_context);
            existing.precedence_filter_suppressed |= config.precedence_filter_suppressed;
            self.conflicting_alts.clear();
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

    pub(crate) fn configs(&self) -> &[AtnConfig] {
        &self.configs
    }

    pub(crate) fn into_configs(self) -> Vec<AtnConfig> {
        self.configs
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    pub(crate) const fn len(&self) -> usize {
        self.configs.len()
    }

    pub(crate) fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
        if readonly {
            self.config_index = FxHashMap::default();
            self.conflicting_alts.clear();
        }
    }

    pub(crate) const fn full_context(&self) -> bool {
        self.full_context
    }

    pub(crate) const fn has_semantic_context(&self) -> bool {
        self.has_semantic_context
    }

    pub(crate) fn unique_alt(&mut self) -> Option<usize> {
        if self.unique_alt.is_none() {
            self.unique_alt = unique_alt(self.configs());
        }
        self.unique_alt
    }

    pub(crate) fn alts(&self) -> BTreeSet<usize> {
        self.configs.iter().map(|config| config.alt).collect()
    }

    pub(crate) fn conflicting_alt_subsets(&self) -> Vec<BTreeSet<usize>> {
        conflicting_alt_subsets(self.configs())
    }

    pub(crate) fn conflicting_alts(&mut self) -> BTreeSet<usize> {
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

    pub(crate) fn remap_contexts(&mut self, remap: &[ContextId], arena: &ContextArena) {
        for config in &mut self.configs {
            let index = usize::try_from(config.context.0).expect("u32 context ID fits usize");
            config.set_context(
                *remap
                    .get(index)
                    .expect("every imported context ID has a remap"),
                arena,
            );
        }
        self.config_index.clear();
        if !self.readonly {
            for (index, config) in self.configs.iter().enumerate() {
                self.config_index.insert(AtnConfigKey::from(config), index);
            }
        }
    }

    pub(crate) fn fingerprint(&self) -> u64 {
        let mut hasher = PredictionFxHasher::default();
        self.configs.hash(&mut hasher);
        self.full_context.hash(&mut hasher);
        self.has_semantic_context.hash(&mut hasher);
        self.dips_into_outer_context.hash(&mut hasher);
        hasher.finish()
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.configs.capacity() * size_of::<AtnConfig>()
            + self.config_index.capacity() * size_of::<(AtnConfigKey, usize)>()
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

pub(crate) fn unique_alt(configs: &[AtnConfig]) -> Option<usize> {
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

pub(crate) fn conflicting_alt_subsets(configs: &[AtnConfig]) -> Vec<BTreeSet<usize>> {
    let mut by_state_context = FxHashMap::<(usize, ContextId), BTreeSet<usize>>::default();
    for config in configs {
        by_state_context
            .entry((config.state, config.context))
            .or_default()
            .insert(config.alt);
    }
    by_state_context.into_values().collect()
}

pub(crate) fn all_subsets_conflict(alt_subsets: &[BTreeSet<usize>]) -> bool {
    alt_subsets.iter().all(|alts| alts.len() > 1)
}

pub(crate) fn all_subsets_equal(alt_subsets: &[BTreeSet<usize>]) -> bool {
    let mut subsets = alt_subsets.iter();
    let Some(first) = subsets.next() else {
        return true;
    };
    subsets.all(|alts| alts == first)
}

pub(crate) fn single_viable_alt(alt_subsets: &[BTreeSet<usize>]) -> Option<usize> {
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

pub(crate) fn has_sll_conflict_terminating_prediction(
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
    fn arena_interns_singletons_without_per_context_objects() {
        let mut arena = ContextArena::new();
        let first = arena.singleton(EMPTY_CONTEXT, 7);
        let second = arena.singleton(EMPTY_CONTEXT, 7);

        assert_eq!(first, second);
        assert_eq!(arena.stats().singleton_contexts, 1);
        assert_eq!(arena.stats().interner_hits, 1);
    }

    #[test]
    fn array_interner_verifies_payload_after_hash_collision() {
        let mut arena = ContextArena::new();
        let first_parent = arena.singleton(EMPTY_CONTEXT, 1);
        let second_parent = arena.singleton(EMPTY_CONTEXT, 2);
        let expected = [(first_parent, 10), (second_parent, 20)];
        let colliding = [(second_parent, 10), (first_parent, 20)];
        let cached_hash = prediction_context_array_hash(&arena, &expected);
        let start = u32::try_from(arena.array_parents.len()).expect("pool index fits u32");
        arena
            .array_parents
            .extend(colliding.iter().map(|(parent, _)| *parent));
        arena
            .array_return_states
            .extend(colliding.iter().map(|(_, return_state)| *return_state));
        let collision = arena.push_record(ContextRecord {
            tag: ContextTag::Array,
            cached_hash,
            parent_or_start: start,
            return_state_or_len: 2,
        });

        let interned = arena.intern_entries(&expected);

        assert_ne!(interned, collision);
        assert_eq!(arena.entry(interned, 0), Some(expected[0]));
        assert_eq!(arena.entry(interned, 1), Some(expected[1]));
    }

    #[test]
    fn merge_with_empty_preserves_full_context_empty_path() {
        let mut arena = ContextArena::new();
        let mut workspace = PredictionWorkspace::default();
        let singleton = arena.singleton(EMPTY_CONTEXT, 42);

        let merged = arena.merge(singleton, EMPTY_CONTEXT, false, &mut workspace);

        assert_eq!(arena.len(merged), 2);
        assert_eq!(arena.return_state(merged, 0), Some(42));
        assert_eq!(arena.parent(merged, 0), Some(EMPTY_CONTEXT));
        assert_eq!(arena.return_state(merged, 1), Some(EMPTY_RETURN_STATE));
        assert!(arena.has_empty_path(merged));
    }

    #[test]
    fn wildcard_merge_collapses_to_empty() {
        let mut arena = ContextArena::new();
        let mut workspace = PredictionWorkspace::default();
        let singleton = arena.singleton(EMPTY_CONTEXT, 42);

        assert_eq!(
            arena.merge(singleton, EMPTY_CONTEXT, true, &mut workspace),
            EMPTY_CONTEXT
        );
    }

    #[test]
    fn merge_is_order_independent() {
        let mut arena = ContextArena::new();
        let mut workspace = PredictionWorkspace::default();
        let left_parent = arena.singleton(EMPTY_CONTEXT, 100);
        let right_parent = arena.singleton(EMPTY_CONTEXT, 200);
        let left = arena.singleton(left_parent, 7);
        let right = arena.singleton(right_parent, 7);

        let left_right = arena.merge(left, right, false, &mut workspace);
        workspace.reset();
        let right_left = arena.merge(right, left, false, &mut workspace);

        assert_eq!(left_right, right_left);
        assert_eq!(arena.len(left_right), 2);
    }

    #[test]
    fn import_remaps_contexts_into_destination_arena() {
        let mut source = ContextArena::new();
        let parent = source.singleton(EMPTY_CONTEXT, 3);
        let child = source.singleton(parent, 9);
        let mut destination = ContextArena::new();
        let mut workspace = PredictionWorkspace::default();

        let remap = destination.import_all(&source, &mut workspace);
        let imported = remap[usize::try_from(child.0).expect("context ID fits usize")];

        assert_eq!(destination.return_state(imported, 0), Some(9));
        let imported_parent = destination.parent(imported, 0).expect("parent");
        assert_eq!(destination.return_state(imported_parent, 0), Some(3));
    }

    #[test]
    fn config_set_merges_context_ids() {
        let mut arena = ContextArena::new();
        let mut workspace = PredictionWorkspace::default();
        let left = arena.singleton(EMPTY_CONTEXT, 1);
        let right = arena.singleton(EMPTY_CONTEXT, 2);
        let mut set = AtnConfigSet::new_full_context(true);

        assert!(set.add(
            AtnConfig::new(1, 1, left, &arena),
            &mut arena,
            &mut workspace
        ));
        assert!(!set.add(
            AtnConfig::new(1, 1, right, &arena),
            &mut arena,
            &mut workspace
        ));
        assert_eq!(set.len(), 1);
        assert_eq!(arena.len(set.configs()[0].context), 2);
    }

    #[test]
    fn workspace_drops_pathological_capacity() {
        let mut workspace = PredictionWorkspace::default();
        workspace
            .merge_cache
            .reserve(MAX_RETAINED_MERGE_CACHE_ENTRIES.saturating_mul(2));
        workspace
            .entries
            .reserve(MAX_RETAINED_CONTEXT_ENTRIES.saturating_mul(2));
        workspace.reset();

        assert!(workspace.merge_cache.capacity() <= MAX_RETAINED_MERGE_CACHE_ENTRIES);
        assert!(workspace.entries.capacity() <= MAX_RETAINED_CONTEXT_ENTRIES);
    }
}
