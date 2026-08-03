use crate::generator::prelude::*;
use crate::semantics::{
    RuleArgTemplate, SemPatternFile, SemUnknownPolicy, SemanticHelperCall, SemanticsKind,
    can_generate_parser_predicate, empty_parser_action_states, likely_parser_entry_rule_indices,
    parser_action_assume_overridden, parser_action_state_coordinates, parser_action_states,
    parser_predicate_transitions, parser_typed_hook_mappings, render_member_init_seeds,
    render_parser_action_method, render_parser_rustdoc, render_parser_semantics_function,
    render_typed_hook_adapter, stack_member, structural_parameterized_parser_rules,
    structural_parser_rule_args, structural_predicate_templates, synthetic_parser_action_states,
    uses_alt_number_contexts, uses_structural_context_alt_numbers,
};
use crate::structural::{
    choice_child_cardinalities, embedded_body_translation_error, structural_actions,
    structural_embedded_model, structural_line_column, structural_predicates,
    structural_rule_calls,
};

mod decision {
    use super::*;
    include!("decision.rs");
}
mod ir {
    use super::*;
    include!("ir/mod.rs");
    include!("ir/lower.rs");
    include!("ir/optimize.rs");
}
mod render {
    use super::*;
    include!("render/rules.rs");
    include!("render/decisions.rs");
    include!("render/loops.rs");
    include!("render/fallback.rs");
    include!("render_model.rs");
    include!("render/mod.rs");
}
mod routing {
    use super::*;
    include!("routing.rs");
}
mod surface {
    use super::*;
    include!("surface/model.rs");
    include!("surface/support_abi.rs");
    include!("surface/names.rs");
    include!("surface/accessors.rs");
    include!("surface/traversal.rs");
    include!("surface/contexts.rs");
    include!("surface/facade.rs");
}

pub(crate) use decision::*;
pub(crate) use ir::*;
pub(crate) use render::*;
pub(crate) use routing::*;
pub(crate) use surface::*;
