use std::io::{self, Write as _};
use std::path::PathBuf;

use clap::{ArgAction, Parser};

use crate::builder::{ActionMode, UnknownSemanticPolicy};
use crate::config::CompilerConfig;
use crate::driver::generate;
use crate::error::Error;
use crate::parser::MAX_FIXED_LOOKAHEAD_FLAG;
use crate::semantics::{SemPatternFile, load_sem_patterns, normalize_option_hook};

pub(crate) fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse().into_config()?;
    let mut stderr = io::stderr().lock();
    generate(&args, |message| writeln!(stderr, "{message}")).map_err(Error::into_io_error)?;
    Ok(())
}

#[derive(Debug, Parser)]
#[command(
    name = "antlr4-rust-gen",
    version = clap::crate_version!(),
    about = clap::crate_description!(),
    args_override_self = true
)]
struct CliArgs {
    /// ANTLR grammar roots to compile.
    #[arg(value_name = "ROOT.g4", required = true)]
    roots: Vec<PathBuf>,

    /// Add an import/token-vocabulary lookup directory.
    #[arg(short = 'I', long = "lib", value_name = "DIR")]
    library_directories: Vec<PathBuf>,

    /// Write generated Rust files to this directory.
    #[arg(long, value_name = "DIR", default_value = ".")]
    out_dir: PathBuf,

    /// Require generated bodies for every parser rule.
    #[arg(long)]
    require_generated_parser: bool,

    /// Ignore unsupported lexer actions.
    #[arg(long)]
    allow_unsupported_lexer_actions: bool,

    /// Generate the typed listener and walker (default).
    #[arg(
        long,
        action = ArgAction::SetTrue,
        overrides_with = "no_listener"
    )]
    listener: bool,

    /// Do not generate the typed listener or walker.
    #[arg(
        long,
        action = ArgAction::SetTrue,
        overrides_with = "listener"
    )]
    no_listener: bool,

    /// Generate the typed visitor.
    #[arg(
        long,
        action = ArgAction::SetTrue,
        overrides_with = "no_visitor"
    )]
    visitor: bool,

    /// Do not generate the typed visitor (default).
    #[arg(
        long,
        action = ArgAction::SetTrue,
        overrides_with = "visitor"
    )]
    no_visitor: bool,

    /// Choose how grammar action and predicate bodies are interpreted.
    #[arg(long, value_enum, default_value = "templates")]
    actions: ActionMode,

    /// Choose the unsupported semantic predicate policy.
    #[arg(long, value_enum, default_value = "assume-true")]
    sem_unknown: UnknownSemanticPolicy,

    /// Load semantic helper patterns.
    #[arg(long, value_name = "FILE")]
    sem_patterns: Option<PathBuf>,

    /// Acknowledge an option implemented by caller hooks.
    #[arg(
        long = "option-hook",
        value_name = "KEY=VALUE",
        value_parser = normalize_option_hook,
        allow_hyphen_values = true
    )]
    option_hooks: Vec<String>,

    /// Fail if any semantic coordinate or option is unsupported.
    #[arg(long)]
    require_full_semantics: bool,

    /// Compile decisions provable within K tokens into static dispatch tables.
    #[arg(
        long,
        value_name = "K",
        value_parser = clap::value_parser!(u8)
            .range(1..=i64::from(MAX_FIXED_LOOKAHEAD_FLAG))
    )]
    fixed_lookahead: Option<u8>,

    /// Declare a parser entry rule by bare name (repeatable).
    #[arg(long = "entry-rule", value_name = "NAME")]
    entry_rules: Vec<String>,

    /// Remove parser rules unreachable from every entry rule.
    #[arg(long)]
    prune_unreachable: bool,

    /// Collapse proven linear precedence ladders (changes tree/API).
    #[arg(long, conflicts_with = "report_precedence_ladders")]
    optimize_precedence_ladders: bool,

    /// Dry-run precedence-ladder analysis and emit only optimizations.json.
    #[arg(long, conflicts_with = "optimize_precedence_ladders")]
    report_precedence_ladders: bool,
}

impl CliArgs {
    fn into_config(self) -> Result<CompilerConfig, Box<dyn std::error::Error>> {
        let sem_patterns_path = self.sem_patterns;
        let sem_patterns = sem_patterns_path
            .as_deref()
            .map_or_else(|| Ok(SemPatternFile::default()), load_sem_patterns)
            .map_err(|error| format!("failed to load --sem-patterns: {error}"))?;
        Ok(CompilerConfig {
            roots: self.roots,
            library_directories: self.library_directories,
            out_dir: self.out_dir,
            require_generated_parser: self.require_generated_parser,
            allow_unsupported_lexer_actions: self.allow_unsupported_lexer_actions,
            sem_unknown: self.sem_unknown.into_internal(),
            sem_patterns,
            sem_patterns_path,
            require_full_semantics: self.require_full_semantics,
            option_hooks: self.option_hooks.into_iter().collect(),
            generate_listener: self.listener || !self.no_listener,
            generate_visitor: self.visitor && !self.no_visitor,
            embedded_actions: self.actions == ActionMode::Embedded,
            fixed_lookahead: self.fixed_lookahead.map(usize::from),
            entry_rules: self.entry_rules.into_iter().collect(),
            prune_unreachable: self.prune_unreachable,
            optimize_precedence_ladders: self.optimize_precedence_ladders,
            report_precedence_ladders: self.report_precedence_ladders,
            test_rig: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::CliArgs;
    use crate::semantics::SemUnknownPolicy;

    #[test]
    fn derives_typed_defaults() {
        let args =
            CliArgs::try_parse_from(["antlr4-rust-gen", "T.g4"]).expect("minimal CLI should parse");
        let config = args
            .into_config()
            .expect("minimal CLI should produce compiler configuration");

        assert!(!config.embedded_actions);
        assert_eq!(config.sem_unknown, SemUnknownPolicy::AssumeTrue);
        assert!(config.generate_listener);
        assert!(!config.generate_visitor);
    }

    #[test]
    fn paired_generation_flags_use_the_last_value() {
        let args = CliArgs::try_parse_from([
            "antlr4-rust-gen",
            "--no-listener",
            "--listener",
            "--visitor",
            "--no-visitor",
            "T.g4",
        ])
        .expect("paired flags should override each other");
        let config = args
            .into_config()
            .expect("paired flags should produce compiler configuration");

        assert!(config.generate_listener);
        assert!(!config.generate_visitor);
    }
}
