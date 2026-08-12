// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use crate::generator::prelude::*;
use crate::semantics::{
    ActionTemplate, CoordinateDispose, PredicateTemplate, SemPatternFile, SemUnknownPolicy,
    SemanticsKind, lexer_actions_need_dispatch, lexer_custom_actions, lexer_predicate_transitions,
    lexer_typed_hook_mappings, reject_unsupported_lexer_action_templates,
    render_lexer_action_method, render_lexer_predicate_method, render_lexer_semantics_function,
    render_lexer_typed_hook_adapter, render_member_init_seeds, stack_member,
    structural_lexer_action_templates, structural_predicate_templates, uses_lexer_superclass,
};
use crate::structural::{
    embedded_body_translation_error, set_lexer_predicate_template, structural_actions,
    structural_predicates,
};

include!("render_model.rs");
include!("render.rs");
