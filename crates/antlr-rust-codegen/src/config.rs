// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::rust_support::RustSupportOptions;
use crate::semantics::{SemPatternFile, SemUnknownPolicy};

#[derive(Debug)]
pub(crate) struct TestRigConfig {
    pub(crate) start_rule: String,
}

#[derive(Debug)]
pub(crate) struct CompilerConfig {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) library_directories: Vec<PathBuf>,
    pub(crate) out_dir: PathBuf,
    pub(crate) require_generated_parser: bool,
    pub(crate) allow_unsupported_lexer_actions: bool,
    pub(crate) sem_unknown: SemUnknownPolicy,
    pub(crate) sem_patterns: SemPatternFile,
    pub(crate) sem_patterns_path: Option<PathBuf>,
    pub(crate) require_full_semantics: bool,
    pub(crate) option_hooks: BTreeSet<String>,
    pub(crate) generate_listener: bool,
    pub(crate) generate_visitor: bool,
    /// The grammar contains real Rust action/predicate bodies rendered through
    /// a `.test.stg`, rather than template metadata.
    pub(crate) embedded_actions: bool,
    /// Compile decisions whose alternatives are pairwise disjoint within this
    /// token depth into static dispatch tables.
    pub(crate) fixed_lookahead: Option<usize>,
    /// Parser entry rules configured for reachability diagnostics and pruning.
    pub(crate) entry_rules: BTreeSet<String>,
    /// Remove parser rules unreachable from every inferred/configured entry.
    pub(crate) prune_unreachable: bool,
    /// Inline trivial pure parser rules into their call sites (issue #130).
    pub(crate) inline_trivial_rules: bool,
    /// Analyze trivial-rule inlining on a shadow model and emit only its manifest.
    pub(crate) report_trivial_rules: bool,
    /// Recognition-preserving source rewrite from issue #225.
    pub(crate) optimize_precedence_ladders: bool,
    /// Analyze the same pass on a shadow model and emit only its manifest.
    pub(crate) report_precedence_ladders: bool,
    /// Emit a temporary-crate entry point for `antlr4-rust-testrig`.
    pub(crate) test_rig: Option<TestRigConfig>,
    /// Trusted sibling `Rust/transformGrammar.py` execution.
    pub(crate) rust_support: RustSupportOptions,
}
