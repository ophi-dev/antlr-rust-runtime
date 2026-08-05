use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::ValueEnum;

use crate::artifact::Generation;
use crate::config::CompilerConfig;
use crate::driver;
use crate::error::Error;
use crate::parser::MAX_FIXED_LOOKAHEAD_FLAG;
use crate::rust_support::RustSupportOptions;
use crate::semantics::{
    SemPatternFile, SemUnknownPolicy, load_sem_patterns, normalize_option_hook,
};

/// How grammar action and predicate bodies are interpreted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum ActionMode {
    #[default]
    Templates,
    Embedded,
}

/// Runtime policy for semantic coordinates that cannot be translated.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum UnknownSemanticPolicy {
    #[default]
    AssumeTrue,
    AssumeFalse,
    Hook,
    Error,
}

impl UnknownSemanticPolicy {
    pub(crate) const fn into_internal(self) -> SemUnknownPolicy {
        match self {
            Self::AssumeTrue => SemUnknownPolicy::AssumeTrue,
            Self::AssumeFalse => SemUnknownPolicy::AssumeFalse,
            Self::Hook => SemUnknownPolicy::Hook,
            Self::Error => SemUnknownPolicy::Error,
        }
    }
}

/// Configures grammar compilation and Rust source generation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct Builder {
    roots: Vec<PathBuf>,
    library_directories: Vec<PathBuf>,
    output_directory: Option<PathBuf>,
    require_generated_parser: bool,
    allow_unsupported_lexer_actions: bool,
    semantic_policy: UnknownSemanticPolicy,
    semantic_patterns: Option<PathBuf>,
    require_full_semantics: bool,
    option_hooks: BTreeSet<String>,
    generate_listener: bool,
    generate_visitor: bool,
    action_mode: ActionMode,
    fixed_lookahead: Option<usize>,
    entry_rules: BTreeSet<String>,
    prune_unreachable: bool,
    optimize_precedence_ladders: bool,
    report_precedence_ladders: bool,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            library_directories: Vec::new(),
            output_directory: None,
            require_generated_parser: false,
            allow_unsupported_lexer_actions: false,
            semantic_policy: UnknownSemanticPolicy::default(),
            semantic_patterns: None,
            require_full_semantics: false,
            option_hooks: BTreeSet::new(),
            generate_listener: true,
            generate_visitor: false,
            action_mode: ActionMode::default(),
            fixed_lookahead: None,
            entry_rules: BTreeSet::new(),
            prune_unreachable: false,
            optimize_precedence_ladders: false,
            report_precedence_ladders: false,
        }
    }
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grammar(mut self, path: impl AsRef<Path>) -> Self {
        self.roots.push(path.as_ref().to_path_buf());
        self
    }

    pub fn grammars(mut self, paths: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
        self.roots
            .extend(paths.into_iter().map(|path| path.as_ref().to_path_buf()));
        self
    }

    pub fn library_directory(mut self, path: impl AsRef<Path>) -> Self {
        self.library_directories.push(path.as_ref().to_path_buf());
        self
    }

    pub fn out_dir(mut self, path: impl AsRef<Path>) -> Self {
        self.output_directory = Some(path.as_ref().to_path_buf());
        self
    }

    pub const fn action_mode(mut self, mode: ActionMode) -> Self {
        self.action_mode = mode;
        self
    }

    pub const fn require_generated_parser(mut self, enabled: bool) -> Self {
        self.require_generated_parser = enabled;
        self
    }

    pub const fn allow_unsupported_lexer_actions(mut self, enabled: bool) -> Self {
        self.allow_unsupported_lexer_actions = enabled;
        self
    }

    pub const fn unknown_semantics(mut self, policy: UnknownSemanticPolicy) -> Self {
        self.semantic_policy = policy;
        self
    }

    pub fn semantic_patterns(mut self, path: impl AsRef<Path>) -> Self {
        self.semantic_patterns = Some(path.as_ref().to_path_buf());
        self
    }

    pub const fn require_full_semantics(mut self, enabled: bool) -> Self {
        self.require_full_semantics = enabled;
        self
    }

    pub fn option_hook(mut self, assignment: impl Into<String>) -> Self {
        self.option_hooks.insert(assignment.into());
        self
    }

    pub const fn generate_listener(mut self, enabled: bool) -> Self {
        self.generate_listener = enabled;
        self
    }

    pub const fn generate_visitor(mut self, enabled: bool) -> Self {
        self.generate_visitor = enabled;
        self
    }

    pub const fn fixed_lookahead(mut self, depth: usize) -> Self {
        self.fixed_lookahead = Some(depth);
        self
    }

    pub fn entry_rule(mut self, name: impl Into<String>) -> Self {
        self.entry_rules.insert(name.into());
        self
    }

    pub const fn prune_unreachable(mut self, enabled: bool) -> Self {
        self.prune_unreachable = enabled;
        self
    }

    pub const fn optimize_precedence_ladders(mut self, enabled: bool) -> Self {
        self.optimize_precedence_ladders = enabled;
        self
    }

    pub const fn report_precedence_ladders(mut self, enabled: bool) -> Self {
        self.report_precedence_ladders = enabled;
        self
    }

    pub fn generate(self) -> Result<Generation, Error> {
        let config = self.into_config()?;
        driver::generate(&config, |_| Ok(()))
    }

    fn into_config(self) -> Result<CompilerConfig, Error> {
        if self.roots.is_empty() {
            return Err(Error::configuration(
                "at least one grammar root is required",
            ));
        }
        let output_directory = self
            .output_directory
            .ok_or_else(|| Error::configuration("an output directory is required"))?;
        if self.optimize_precedence_ladders && self.report_precedence_ladders {
            return Err(Error::configuration(
                "precedence-ladder optimization and report-only mode are mutually exclusive",
            ));
        }
        if self
            .fixed_lookahead
            .is_some_and(|depth| !(1..=usize::from(MAX_FIXED_LOOKAHEAD_FLAG)).contains(&depth))
        {
            return Err(Error::configuration(
                "fixed lookahead must be between 1 and 8",
            ));
        }
        let semantic_patterns_path = self.semantic_patterns;
        let semantic_patterns = semantic_patterns_path
            .as_deref()
            .map_or_else(|| Ok(SemPatternFile::default()), load_sem_patterns)
            .map_err(Error::generation)?;
        let mut option_hooks = BTreeSet::new();
        for assignment in self.option_hooks {
            option_hooks.insert(normalize_option_hook(&assignment).map_err(Error::configuration)?);
        }
        Ok(CompilerConfig {
            roots: self.roots,
            library_directories: self.library_directories,
            out_dir: output_directory,
            require_generated_parser: self.require_generated_parser,
            allow_unsupported_lexer_actions: self.allow_unsupported_lexer_actions,
            sem_unknown: self.semantic_policy.into_internal(),
            sem_patterns: semantic_patterns,
            sem_patterns_path: semantic_patterns_path,
            require_full_semantics: self.require_full_semantics,
            option_hooks,
            generate_listener: self.generate_listener,
            generate_visitor: self.generate_visitor,
            embedded_actions: self.action_mode == ActionMode::Embedded,
            fixed_lookahead: self.fixed_lookahead,
            entry_rules: self.entry_rules,
            prune_unreachable: self.prune_unreachable,
            optimize_precedence_ladders: self.optimize_precedence_ladders,
            report_precedence_ladders: self.report_precedence_ladders,
            test_rig: None,
            rust_support: RustSupportOptions::disabled(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Builder;

    #[test]
    fn new_matches_default() {
        assert_eq!(Builder::new(), Builder::default());
    }
}
