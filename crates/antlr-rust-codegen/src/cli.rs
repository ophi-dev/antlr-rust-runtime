use std::collections::BTreeSet;
use std::env;
use std::io::{self, Write as _};
use std::path::PathBuf;

use crate::driver::generate;
use crate::error::Error;
use crate::parser::MAX_FIXED_LOOKAHEAD_FLAG;
use crate::semantics::{
    SemPatternFile, SemUnknownPolicy, load_sem_patterns, normalize_option_hook,
};

pub(crate) fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let args = match CompilerConfig::parse_from(env::args().skip(1))? {
        CliCommand::Generate(args) => *args,
        CliCommand::Help => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{}", usage())?;
            return Ok(());
        }
        CliCommand::Version => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "antlr4-rust-gen {}", env!("CARGO_PKG_VERSION"))?;
            return Ok(());
        }
    };
    let mut stderr = io::stderr().lock();
    generate(&args, |message| writeln!(stderr, "{message}")).map_err(Error::into_io_error)?;
    Ok(())
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
    pub(crate) require_full_semantics: bool,
    pub(crate) option_hooks: BTreeSet<String>,
    pub(crate) generate_listener: bool,
    pub(crate) generate_visitor: bool,
    /// `--actions embedded`: the grammar contains real Rust action/predicate
    /// bodies (rendered through a `.test.stg`); splice them verbatim after
    /// `$`-attribute translation instead of recognizing template markup.
    pub(crate) embedded_actions: bool,
    /// `--fixed-lookahead <k>`: compile decisions whose alternatives are
    /// pairwise disjoint within `k` tokens of lookahead into static dispatch
    /// tables instead of adaptive prediction. `None` keeps the default
    /// routing (Java parity).
    pub(crate) fixed_lookahead: Option<usize>,
    /// Parser entry rules configured for reachability diagnostics and pruning.
    pub(crate) entry_rules: BTreeSet<String>,
    /// Remove parser rules unreachable from every inferred/configured entry.
    pub(crate) prune_unreachable: bool,
    /// Recognition-preserving source rewrite from issue #225.
    pub(crate) optimize_precedence_ladders: bool,
    /// Analyze the same pass on a shadow model and emit only its manifest.
    pub(crate) report_precedence_ladders: bool,
}

enum CliCommand {
    Generate(Box<CompilerConfig>),
    Help,
    Version,
}

impl CompilerConfig {
    fn parse_from(arguments: impl IntoIterator<Item = String>) -> Result<CliCommand, String> {
        let mut roots = Vec::new();
        let mut library_directories = Vec::new();
        let mut out_dir = None;
        let mut require_generated_parser = false;
        let mut allow_unsupported_lexer_actions = false;
        let mut embedded_actions = false;
        let mut sem_unknown = SemUnknownPolicy::default();
        let mut sem_patterns = SemPatternFile::default();
        let mut require_full_semantics = false;
        let mut option_hooks = BTreeSet::new();
        let mut generate_listener = true;
        let mut generate_visitor = false;
        let mut fixed_lookahead = None;
        let mut entry_rules = BTreeSet::new();
        let mut prune_unreachable = false;
        let mut optimize_precedence_ladders = false;
        let mut report_precedence_ladders = false;
        let mut positional_only = false;

        let mut iter = arguments.into_iter();
        while let Some(arg) = iter.next() {
            if positional_only {
                roots.push(PathBuf::from(arg));
                continue;
            }
            match arg.as_str() {
                "--" => positional_only = true,
                "-I" | "--lib" => {
                    library_directories.push(PathBuf::from(next_arg(&mut iter, &arg)?));
                }
                "--out-dir" => out_dir = Some(PathBuf::from(next_arg(&mut iter, "--out-dir")?)),
                "--require-generated-parser" => require_generated_parser = true,
                "--allow-unsupported-lexer-actions" => allow_unsupported_lexer_actions = true,
                "-listener" | "--listener" => generate_listener = true,
                "-no-listener" | "--no-listener" => generate_listener = false,
                "-visitor" | "--visitor" => generate_visitor = true,
                "-no-visitor" | "--no-visitor" => generate_visitor = false,
                "--sem-patterns" => {
                    sem_patterns =
                        load_sem_patterns(&PathBuf::from(next_arg(&mut iter, "--sem-patterns")?))
                            .map_err(|error| format!("failed to load --sem-patterns: {error}"))?;
                }
                "--require-full-semantics" => require_full_semantics = true,
                "--option-hook" => {
                    let value = next_arg(&mut iter, "--option-hook")?;
                    option_hooks.insert(normalize_option_hook(&value)?);
                }
                "--actions" => {
                    let value = next_arg(&mut iter, "--actions")?;
                    match value.as_str() {
                        "embedded" => embedded_actions = true,
                        "templates" => embedded_actions = false,
                        other => {
                            return Err(format!(
                                "unknown --actions mode {other:?} (expected embedded|templates)"
                            ));
                        }
                    }
                }
                "--sem-unknown" => {
                    sem_unknown =
                        SemUnknownPolicy::parse_flag(&next_arg(&mut iter, "--sem-unknown")?)?;
                }
                "--fixed-lookahead" => {
                    let value = next_arg(&mut iter, "--fixed-lookahead")?;
                    let depth = value
                        .parse::<usize>()
                        .ok()
                        .filter(|depth| (1..=MAX_FIXED_LOOKAHEAD_FLAG).contains(depth));
                    match depth {
                        Some(depth) => fixed_lookahead = Some(depth),
                        None => {
                            return Err(format!(
                                "--fixed-lookahead expects an integer between 1 and {MAX_FIXED_LOOKAHEAD_FLAG}; got {value}\n\n{}",
                                usage()
                            ));
                        }
                    }
                }
                "--entry-rule" => {
                    entry_rules.insert(next_arg(&mut iter, "--entry-rule")?);
                }
                "--prune-unreachable" => {
                    prune_unreachable = true;
                }
                "--optimize-precedence-ladders" => {
                    optimize_precedence_ladders = true;
                }
                "--report-precedence-ladders" => {
                    report_precedence_ladders = true;
                }
                "--help" | "-h" => return Ok(CliCommand::Help),
                "--version" | "-V" => return Ok(CliCommand::Version),
                other if other.starts_with("-I") && other.len() > 2 => {
                    library_directories.push(PathBuf::from(&other[2..]));
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown argument {other}\n\n{}", usage()));
                }
                root => roots.push(PathBuf::from(root)),
            }
        }

        if roots.is_empty() {
            return Err(format!(
                "at least one grammar root is required\n\n{}",
                usage()
            ));
        }
        if optimize_precedence_ladders && report_precedence_ladders {
            return Err(format!(
                "--optimize-precedence-ladders and --report-precedence-ladders are mutually exclusive\n\n{}",
                usage()
            ));
        }

        Ok(CliCommand::Generate(Box::new(Self {
            roots,
            library_directories,
            out_dir: out_dir.unwrap_or_else(|| PathBuf::from(".")),
            require_generated_parser,
            allow_unsupported_lexer_actions,
            sem_unknown,
            sem_patterns,
            require_full_semantics,
            option_hooks,
            generate_listener,
            generate_visitor,
            embedded_actions,
            fixed_lookahead,
            entry_rules,
            prune_unreachable,
            optimize_precedence_ladders,
            report_precedence_ladders,
        })))
    }
}

fn next_arg(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value\n\n{}", usage()))
}

pub(crate) fn usage() -> String {
    "\
Usage: antlr4-rust-gen [OPTIONS] ROOT.g4...

Options:
  -I, --lib DIR                    Add an import/token-vocabulary lookup directory
  --out-dir DIR                    Write generated Rust files to DIR
  --actions embedded|templates     Select real embedded actions or template metadata
  --require-generated-parser       Require generated bodies for every parser rule
  --allow-unsupported-lexer-actions
                                   Ignore unsupported lexer actions
  -listener, --listener            Generate the typed listener and walker (default)
  -no-listener, --no-listener      Do not generate the typed listener or walker
  -visitor, --visitor              Generate the typed visitor
  -no-visitor, --no-visitor        Do not generate the typed visitor (default)
  --sem-unknown error|hook|assume-true|assume-false
                                   Choose unsupported semantic predicate policy
  --sem-patterns FILE              Load semantic helper patterns
  --option-hook KEY=VALUE          Acknowledge an option implemented by caller hooks
  --require-full-semantics         Fail if any semantic coordinate or option is unsupported
  --fixed-lookahead K              Compile decisions provable within K tokens of lookahead
                                   into static dispatch tables (off by default; every
                                   remaining decision keeps adaptive prediction)
  --entry-rule NAME                Declare a parser entry rule by bare name (repeatable;
                                   applies to every generated parser defining NAME)
  --prune-unreachable              Remove parser rules unreachable from every entry rule
  --optimize-precedence-ladders    Collapse proven linear precedence ladders (changes tree/API)
  --report-precedence-ladders      Dry-run the pass and emit only optimizations.json
  -V, --version                    Print version
  -h, --help                       Print this help"
        .to_owned()
}
