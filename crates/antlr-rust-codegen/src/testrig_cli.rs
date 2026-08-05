use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use miette::{Context as _, IntoDiagnostic as _};

use crate::builder::{ActionMode, UnknownSemanticPolicy};
use crate::config::{CompilerConfig, TestRigConfig};
use crate::driver::generate;
use crate::parser::MAX_FIXED_LOOKAHEAD_FLAG;
use crate::rust_support::RustSupportOptions;
use crate::semantics::{SemPatternFile, load_sem_patterns, normalize_option_hook};

const TARGET_DIR_ENV: &str = "ANTLR4_RUST_TESTRIG_TARGET_DIR";
const RUNTIME_PATH_ENV: &str = "ANTLR4_RUST_TESTRIG_RUNTIME";
static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn run_cli() -> miette::Result<ExitCode> {
    let args = CliArgs::parse();
    let project = TemporaryProject::new()
        .into_diagnostic()
        .wrap_err("failed to create temporary TestRig project")?;
    let config = args.compiler_config(&project.source_directory())?;
    let mut stderr = io::stderr().lock();
    generate(&config, |message| writeln!(stderr, "{message}"))
        .map_err(crate::cli_report::codegen_error)?;
    project
        .write_manifest()
        .into_diagnostic()
        .wrap_err("failed to write temporary TestRig manifest")?;
    let status = args
        .run_generated(&project)
        .into_diagnostic()
        .wrap_err("failed to execute temporary TestRig runner")?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[derive(Debug, Parser)]
#[command(
    name = "antlr4-rust-testrig",
    version = clap::crate_version!(),
    about = "Generate and run an ANTLR Rust recognizer against test inputs"
)]
struct CliArgs {
    /// Combined, parser, or lexer grammar to run.
    #[arg(value_name = "GRAMMAR[.g4]")]
    grammar: PathBuf,

    /// Parser start rule, or `tokens` for lexer-only execution.
    #[arg(value_name = "START_RULE")]
    start_rule: String,

    /// UTF-8 input files. Reads stdin when omitted.
    #[arg(value_name = "INPUT")]
    input_files: Vec<PathBuf>,

    /// Lexer grammar paired with a split parser grammar.
    #[arg(long, value_name = "LEXER[.g4]")]
    lexer_grammar: Option<PathBuf>,

    /// Add an import/token-vocabulary lookup directory.
    #[arg(short = 'I', long = "lib", value_name = "DIR")]
    library_directories: Vec<PathBuf>,

    /// Print every buffered token, including hidden-channel tokens and EOF.
    #[arg(long)]
    tokens: bool,

    /// Print the parse tree.
    #[arg(long)]
    tree: bool,

    /// Print parser rule enter/exit events.
    #[arg(long)]
    trace: bool,

    /// Report exact-ambiguity prediction diagnostics.
    #[arg(long, conflicts_with = "sll")]
    diagnostics: bool,

    /// Use SLL prediction mode instead of LL.
    #[arg(long)]
    sll: bool,

    /// Ignore unsupported lexer actions.
    #[arg(long)]
    allow_unsupported_lexer_actions: bool,

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
}

impl CliArgs {
    fn compiler_config(&self, output_directory: &Path) -> miette::Result<CompilerConfig> {
        let mut roots = Vec::with_capacity(usize::from(self.lexer_grammar.is_some()) + 1);
        if let Some(lexer) = &self.lexer_grammar {
            roots.push(grammar_path(lexer));
        }
        roots.push(grammar_path(&self.grammar));

        let sem_patterns_path = self.sem_patterns.clone();
        let sem_patterns = match sem_patterns_path.as_deref() {
            Some(path) => load_sem_patterns(path)
                .into_diagnostic()
                .wrap_err("failed to load --sem-patterns")?,
            None => SemPatternFile::default(),
        };
        Ok(CompilerConfig {
            roots,
            library_directories: self.library_directories.clone(),
            out_dir: output_directory.to_path_buf(),
            require_generated_parser: false,
            allow_unsupported_lexer_actions: self.allow_unsupported_lexer_actions,
            sem_unknown: self.sem_unknown.into_internal(),
            sem_patterns,
            sem_patterns_path,
            require_full_semantics: self.require_full_semantics,
            option_hooks: self.option_hooks.iter().cloned().collect::<BTreeSet<_>>(),
            generate_listener: false,
            generate_visitor: false,
            embedded_actions: self.actions == ActionMode::Embedded,
            fixed_lookahead: self.fixed_lookahead.map(usize::from),
            entry_rules: BTreeSet::new(),
            prune_unreachable: false,
            optimize_precedence_ladders: false,
            report_precedence_ladders: false,
            test_rig: Some(TestRigConfig {
                start_rule: self.start_rule.clone(),
            }),
            rust_support: RustSupportOptions::disabled(),
        })
    }

    fn run_generated(
        &self,
        project: &TemporaryProject,
    ) -> Result<std::process::ExitStatus, io::Error> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        let mut command = Command::new(cargo);
        command
            .arg("run")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(project.manifest())
            .arg("--target-dir")
            .arg(target_directory(project))
            .arg("--");
        for (enabled, flag) in [
            (self.tokens, "--tokens"),
            (self.tree, "--tree"),
            (self.trace, "--trace"),
            (self.diagnostics, "--diagnostics"),
            (self.sll, "--sll"),
        ] {
            if enabled {
                command.arg(flag);
            }
        }
        command.arg("--").args(&self.input_files);
        command.env("CARGO_TERM_COLOR", "never").status()
    }
}

fn grammar_path(path: &Path) -> PathBuf {
    if path.extension().is_none() {
        path.with_extension("g4")
    } else {
        path.to_path_buf()
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

#[derive(Debug)]
struct TemporaryProject {
    root: PathBuf,
    package_name: String,
}

impl TemporaryProject {
    fn new() -> io::Result<Self> {
        loop {
            let identifier = NEXT_PROJECT_ID.fetch_add(1, Ordering::Relaxed);
            let package_name = format!(
                "antlr4-rust-testrig-runner-{}-{identifier}",
                std::process::id()
            );
            let root = std::env::temp_dir().join(&package_name);
            match create_private_directory(&root) {
                Ok(()) => {
                    fs::create_dir(root.join("src"))?;
                    return Ok(Self { root, package_name });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn source_directory(&self) -> PathBuf {
        self.root.join("src")
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    fn write_manifest(&self) -> io::Result<()> {
        let runtime = runtime_dependency()?;
        fs::write(
            self.manifest(),
            format!(
                "[workspace]\n\n\
                 [package]\n\
                 name = \"{}\"\n\
                 version = \"0.0.0\"\n\
                 edition = \"2024\"\n\
                 publish = false\n\n\
                 [[bin]]\n\
                 name = \"{}\"\n\
                 path = \"src/{}\"\n\n\
                 [dependencies]\n\
                 {runtime}\
                 miette = {{ version = \"=7.6.0\", default-features = false, features = [\"fancy\"] }}\n",
                self.package_name,
                self.package_name,
                crate::test_rig::MAIN_PATH,
            ),
        )
    }
}

impl Drop for TemporaryProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn runtime_dependency() -> io::Result<String> {
    if let Some(configured) = std::env::var_os(RUNTIME_PATH_ENV) {
        return runtime_path_dependency(&PathBuf::from(configured));
    }

    let sibling = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|crates| crates.join("antlr-rust-runtime"));
    if let Some(sibling) = sibling
        && sibling.join("Cargo.toml").is_file()
    {
        return runtime_path_dependency(&sibling);
    }

    Ok(format!(
        "antlr-rust-runtime = \"={}\"\n",
        env!("CARGO_PKG_VERSION")
    ))
}

fn runtime_path_dependency(path: &Path) -> io::Result<String> {
    let absolute = fs::canonicalize(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot resolve runtime path from {RUNTIME_PATH_ENV}: {}: {error}",
                path.display()
            ),
        )
    })?;
    Ok(format!(
        "antlr-rust-runtime = {{ path = {} }}\n",
        toml_string(&absolute)?
    ))
}

fn toml_string(path: &Path) -> io::Result<String> {
    let value = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("runtime path is not valid UTF-8: {}", path.display()),
        )
    })?;
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "runtime path contains an unsupported control character",
                ));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    Ok(escaped)
}

fn target_directory(project: &TemporaryProject) -> PathBuf {
    select_target_directory(
        std::env::var_os(TARGET_DIR_ENV).map(PathBuf::from),
        user_cache_directory(),
        &project.root,
    )
}

fn select_target_directory(
    configured: Option<PathBuf>,
    user_cache: Option<PathBuf>,
    project_root: &Path,
) -> PathBuf {
    configured.unwrap_or_else(|| {
        user_cache.map_or_else(
            || project_root.join("target"),
            |cache| {
                cache
                    .join("antlr4-rust-testrig")
                    .join(env!("CARGO_PKG_VERSION"))
                    .join("target")
            },
        )
    })
}

fn user_cache_directory() -> Option<PathBuf> {
    if let Some(cache) = std::env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(cache));
    }
    if cfg!(target_os = "windows")
        && let Some(cache) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(cache));
    }
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))?;
    let home = PathBuf::from(home);
    Some(if cfg!(target_os = "macos") {
        home.join("Library").join("Caches")
    } else {
        home.join(".cache")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_grammar_extension_only_when_absent() {
        assert_eq!(grammar_path(Path::new("Demo")), PathBuf::from("Demo.g4"));
        assert_eq!(grammar_path(Path::new("Demo.g4")), PathBuf::from("Demo.g4"));
    }

    #[test]
    fn toml_path_escaping_handles_quotes_and_backslashes() {
        let path = Path::new("a\\b\"c");
        assert_eq!(
            toml_string(path).expect("path should escape"),
            r#""a\\b\"c""#
        );
    }

    #[test]
    fn target_directory_prefers_override_then_user_cache_then_project() {
        let configured = PathBuf::from("configured");
        let cache = PathBuf::from("cache");
        let project = Path::new("project");
        assert_eq!(
            select_target_directory(Some(configured.clone()), Some(cache.clone()), project),
            configured
        );
        assert_eq!(
            select_target_directory(None, Some(cache), project),
            PathBuf::from("cache")
                .join("antlr4-rust-testrig")
                .join(env!("CARGO_PKG_VERSION"))
                .join("target")
        );
        assert_eq!(
            select_target_directory(None, None, project),
            project.join("target")
        );
    }

    #[cfg(unix)]
    #[test]
    fn temporary_project_directory_is_private() {
        use std::os::unix::fs::PermissionsExt as _;

        let project = TemporaryProject::new().expect("temporary project should be created");
        let mode = fs::metadata(&project.root)
            .expect("temporary project should have metadata")
            .permissions()
            .mode();

        assert_eq!(mode & 0o077, 0);
    }
}
