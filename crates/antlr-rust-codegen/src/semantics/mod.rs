use crate::cli::usage;
use crate::generator::prelude::*;
use crate::parser::{
    DecisionReportRow, DecisionTierReport, LexerTypedHookKind, LexerTypedHookMapping,
    ParserTypedHookKind, TypedHookMapping, build_structural_portable_local_data,
};
use crate::structural::{
    structural_actions, structural_embedded_model, structural_line_column, structural_predicates,
    structural_rule_calls,
};

pub(crate) mod stack_member;
pub(crate) mod template_syntax;

use template_syntax as templates;
use templates::{
    matching_template_close, parse_template_string, split_template_arguments,
    template_sequence_bodies,
};

include!("model.rs");
include!("patterns.rs");
include!("inventory.rs");
include!("manifest.rs");
include!("templates.rs");
include!("hooks.rs");
include!("semir.rs");
