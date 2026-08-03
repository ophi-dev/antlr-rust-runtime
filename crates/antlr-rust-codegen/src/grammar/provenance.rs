use std::collections::BTreeMap;

use super::frontend::{SourceSpan, SyntaxId};
use super::model::{
    AlternativeId, BuildStateId, BuildTransitionId, ElementId, GrammarId, ModelNodeId, RuleId,
    TransformId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MandatoryTransform {
    ImportIntegration,
    BlockSetReduction,
    CombinedSplit,
    ImplicitLiteralRule,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LeftRecursionRole {
    Primary,
    Operator,
    PrecedencePredicate,
    RecursiveCall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntheticReason {
    RuleBoundary,
    BlockBoundary,
    LoopBoundary,
    EntryEof,
    RuleFollow,
    LexerModeStart,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Origin {
    Authored {
        syntax: SyntaxId,
        span: SourceSpan,
    },
    Imported {
        edge: u32,
        original: ModelNodeId,
    },
    ImplicitLexer {
        combined: GrammarId,
        original: ModelNodeId,
    },
    MandatoryTransform {
        kind: MandatoryTransform,
        inputs: Box<[ModelNodeId]>,
    },
    OptionalTransform {
        pass: TransformId,
        inputs: Box<[ModelNodeId]>,
    },
    LeftRecursion {
        rule: RuleId,
        original_alt: AlternativeId,
        role: LeftRecursionRole,
    },
    Synthetic {
        reason: SyntheticReason,
        owner: ModelNodeId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Tombstone {
    pub(crate) phase: &'static str,
    pub(crate) reason: &'static str,
    pub(crate) replacements: Box<[ModelNodeId]>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProvenanceIndex {
    model_origins: BTreeMap<ModelNodeId, Vec<Origin>>,
    syntax_models: BTreeMap<SyntaxId, Vec<ModelNodeId>>,
    state_origins: BTreeMap<BuildStateId, Vec<Origin>>,
    transition_origins: BTreeMap<BuildTransitionId, Vec<Origin>>,
    tombstones: BTreeMap<SyntaxId, Tombstone>,
}

impl ProvenanceIndex {
    pub(crate) fn record_model(
        &mut self,
        id: ModelNodeId,
        origins: impl IntoIterator<Item = Origin>,
    ) {
        let entry = self.model_origins.entry(id).or_default();
        for origin in origins {
            if let Origin::Authored { syntax, .. } = &origin {
                push_unique(self.syntax_models.entry(*syntax).or_default(), id);
            }
            if !entry.contains(&origin) {
                entry.push(origin);
            }
        }
    }

    pub(crate) fn record_state(
        &mut self,
        id: BuildStateId,
        origins: impl IntoIterator<Item = Origin>,
    ) {
        match self.state_origins.entry(id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(origins.into_iter().collect());
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                extend_unique(entry.into_mut(), origins);
            }
        }
    }

    pub(crate) fn record_transition(
        &mut self,
        id: BuildTransitionId,
        origins: impl IntoIterator<Item = Origin>,
    ) {
        match self.transition_origins.entry(id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(origins.into_iter().collect());
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                extend_unique(entry.into_mut(), origins);
            }
        }
    }

    pub(crate) fn tombstone(&mut self, syntax: SyntaxId, tombstone: Tombstone) {
        self.tombstones.insert(syntax, tombstone);
    }

    pub(crate) fn origins(&self, id: ModelNodeId) -> &[Origin] {
        self.model_origins.get(&id).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn state_origins(&self, id: BuildStateId) -> &[Origin] {
        self.state_origins.get(&id).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn transition_origins(&self, id: BuildTransitionId) -> &[Origin] {
        self.transition_origins.get(&id).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn models_for_syntax(&self, syntax: SyntaxId) -> &[ModelNodeId] {
        self.syntax_models.get(&syntax).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn validate_authored_coverage(
        &self,
        authored: impl IntoIterator<Item = SyntaxId>,
    ) -> Result<(), SyntaxId> {
        authored
            .into_iter()
            .find(|syntax| {
                let survives = self
                    .syntax_models
                    .get(syntax)
                    .is_some_and(|models| !models.is_empty());
                !survives && !self.tombstones.contains_key(syntax)
            })
            .map_or(Ok(()), Err)
    }
}

fn extend_unique<T: Eq>(target: &mut Vec<T>, values: impl IntoIterator<Item = T>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn push_unique<T: Eq>(target: &mut Vec<T>, value: T) {
    if !target.contains(&value) {
        target.push(value);
    }
}

pub(crate) const fn element_node(id: ElementId) -> ModelNodeId {
    ModelNodeId::Element(id)
}
