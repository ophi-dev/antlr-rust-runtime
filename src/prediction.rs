use std::collections::BTreeSet;
use std::rc::Rc;

pub const EMPTY_RETURN_STATE: usize = usize::MAX;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PredictionContext {
    Empty,
    Singleton {
        parent: Rc<Self>,
        return_state: usize,
    },
    Array {
        parents: Vec<Rc<Self>>,
        return_states: Vec<usize>,
    },
}

impl PredictionContext {
    pub fn empty() -> Rc<Self> {
        Rc::new(Self::Empty)
    }

    pub fn singleton(parent: Rc<Self>, return_state: usize) -> Rc<Self> {
        if return_state == EMPTY_RETURN_STATE {
            Self::empty()
        } else {
            Rc::new(Self::Singleton {
                parent,
                return_state,
            })
        }
    }

    pub const fn len(&self) -> usize {
        match self {
            Self::Empty => 1,
            Self::Singleton { .. } => 1,
            Self::Array { return_states, .. } => return_states.len(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn return_state(&self, index: usize) -> Option<usize> {
        match self {
            Self::Empty if index == 0 => Some(EMPTY_RETURN_STATE),
            Self::Singleton { return_state, .. } if index == 0 => Some(*return_state),
            Self::Array { return_states, .. } => return_states.get(index).copied(),
            Self::Empty => None,
            Self::Singleton { .. } => None,
        }
    }

    pub fn parent(&self, index: usize) -> Option<Rc<Self>> {
        match self {
            Self::Empty => None,
            Self::Singleton { parent, .. } if index == 0 => Some(Rc::clone(parent)),
            Self::Array { parents, .. } => parents.get(index).cloned(),
            Self::Singleton { .. } => None,
        }
    }

    /// Merges two prediction contexts while preserving deterministic entry
    /// order.
    ///
    /// This is a compact baseline for parser ATN work: equal contexts are
    /// reused directly, and unequal singleton/array contexts are flattened into
    /// a deduplicated array context.
    pub fn merge(left: Rc<Self>, right: Rc<Self>) -> Rc<Self> {
        if left == right {
            return left;
        }
        let mut entries = Vec::new();
        collect_entries(&left, &mut entries);
        collect_entries(&right, &mut entries);
        drop((left, right));
        entries.sort_by_key(|(_, return_state)| *return_state);
        entries.dedup_by(|a, b| a.1 == b.1 && a.0 == b.0);
        Rc::new(Self::Array {
            parents: entries
                .iter()
                .map(|(parent, _)| Rc::clone(parent))
                .collect(),
            return_states: entries
                .iter()
                .map(|(_, return_state)| *return_state)
                .collect(),
        })
    }
}

fn collect_entries(
    context: &Rc<PredictionContext>,
    entries: &mut Vec<(Rc<PredictionContext>, usize)>,
) {
    match context.as_ref() {
        PredictionContext::Empty => entries.push((Rc::clone(context), EMPTY_RETURN_STATE)),
        PredictionContext::Singleton {
            parent,
            return_state,
        } => entries.push((Rc::clone(parent), *return_state)),
        PredictionContext::Array {
            parents,
            return_states,
        } => {
            for (parent, return_state) in parents.iter().zip(return_states) {
                entries.push((Rc::clone(parent), *return_state));
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AtnConfig {
    pub state: usize,
    pub alt: usize,
    pub context: Rc<PredictionContext>,
    pub reaches_into_outer_context: usize,
}

impl AtnConfig {
    pub const fn new(state: usize, alt: usize, context: Rc<PredictionContext>) -> Self {
        Self {
            state,
            alt,
            context,
            reaches_into_outer_context: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct AtnConfigSet {
    configs: Vec<AtnConfig>,
    config_index: BTreeSet<AtnConfig>,
    has_semantic_context: bool,
    dips_into_outer_context: bool,
    readonly: bool,
}

impl AtnConfigSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a configuration if an equivalent `(state, alt, context)` entry is
    /// not already present.
    pub fn add(&mut self, config: AtnConfig) -> bool {
        assert!(!self.readonly, "cannot mutate readonly ATN config set");
        if self.config_index.insert(config.clone()) {
            if config.reaches_into_outer_context > 0 {
                self.dips_into_outer_context = true;
            }
            self.configs.push(config);
            true
        } else {
            false
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

    pub const fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
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
}
