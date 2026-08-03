use crate::grammar::model::{Block, ElementKind, GrammarUnit};

/// Deterministic structural counts captured around an optimization pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StructuralMetrics {
    pub(crate) rules: usize,
    pub(crate) alternatives: usize,
    pub(crate) elements: usize,
}

pub(crate) fn integrated_grammar_metrics(units: &[GrammarUnit]) -> StructuralMetrics {
    let mut metrics = StructuralMetrics::default();
    for unit in units {
        metrics.rules += unit.rules.len();
        for rule in &unit.rules {
            accumulate_block(&rule.block, &mut metrics);
        }
    }
    metrics
}

fn accumulate_block(block: &Block, metrics: &mut StructuralMetrics) {
    metrics.alternatives += block.alternatives.len();
    for alternative in &block.alternatives {
        metrics.elements += alternative.elements.len();
        for element in &alternative.elements {
            if let ElementKind::Block(nested) = &element.kind {
                accumulate_block(nested, metrics);
            }
        }
    }
}
