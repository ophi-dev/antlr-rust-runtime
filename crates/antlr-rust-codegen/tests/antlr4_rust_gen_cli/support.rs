pub(super) use std::collections::BTreeSet;
pub(super) use std::ffi::OsStr;
pub(super) use std::fs;
use std::io::Write as _;
pub(super) use std::path::{Path, PathBuf};
pub(super) use std::process::{Command, Output, Stdio};
pub(super) use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn run_antlr4_rust_gen(args: &[impl AsRef<OsStr>]) -> Output {
    cli_command(env!("CARGO_BIN_EXE_antlr4-rust-gen"))
        .args(args)
        .output()
        .expect("antlr4-rust-gen should run")
}

pub(super) fn run_antlr4_rust_testrig(args: &[impl AsRef<OsStr>]) -> Output {
    cli_command(env!("CARGO_BIN_EXE_antlr4-rust-testrig"))
        .args(args)
        .output()
        .expect("antlr4-rust-testrig should run")
}

pub(super) fn run_antlr4_rust_testrig_with_stdin(
    args: &[impl AsRef<OsStr>],
    input: &[u8],
) -> Output {
    let mut child = cli_command(env!("CARGO_BIN_EXE_antlr4-rust-testrig"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("antlr4-rust-testrig should start");
    child
        .stdin
        .take()
        .expect("testrig stdin should be piped")
        .write_all(input)
        .expect("testrig input should be writable");
    child
        .wait_with_output()
        .expect("antlr4-rust-testrig should finish")
}

pub(super) fn cli_command(binary: &str) -> Command {
    let mut command = Command::new(binary);
    command
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("CLICOLOR_FORCE", "0")
        .env("NO_GRAPHICS", "0")
        .env("COLUMNS", "4096");
    command
}

pub(super) fn assert_generated_modules_compile(temp_dir: &Path, modules: &[&str]) {
    assert_generated_project(temp_dir, modules, "");
}

pub(super) fn assert_generated_project(temp_dir: &Path, modules: &[&str], test_source: &str) {
    let output = run_generated_project(temp_dir, modules, test_source);
    assert!(
        output.status.success(),
        "generated project failed\nstdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
}

pub(super) fn run_generated_project(
    temp_dir: &Path,
    modules: &[&str],
    test_source: &str,
) -> Output {
    let project = temp_dir.join("compile-generated");
    let source = project.join("src");
    let uses_insta = test_source.contains("insta");
    let dev_dependencies = if uses_insta {
        // Keep this exact pin aligned with Cargo.lock so offline temp crates reuse
        // the workspace's cached Insta version.
        "\n[dev-dependencies]\ninsta = { version = \"=1.48.0\", default-features = false }\n"
    } else {
        ""
    };
    fs::create_dir_all(&source).expect("generated-module check should be writable");
    fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\n\
             name = \"compile-generated\"\n\
             version = \"0.0.0\"\n\
             edition = \"2024\"\n\
             \n\
             [dependencies]\n\
             antlr-rust-runtime = {{ path = {:?} }}\n\
             {dev_dependencies}",
            runtime_crate_root(),
        ),
    )
    .expect("generated-module manifest should be writable");
    let declarations = modules
        .iter()
        .map(|module| {
            let module_name = module.strip_suffix(".rs").unwrap_or(module);
            format!("#[path = {module:?}]\nmod {module_name};")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        source.join("lib.rs"),
        format!("{declarations}\n{test_source}"),
    )
    .expect("generated-module crate root should be writable");
    for module in modules {
        fs::copy(temp_dir.join("generated").join(module), source.join(module))
            .expect("generated module should be copied into the check crate");
    }
    let generated_support = temp_dir.join("generated").join("antlr-rust-support");
    if generated_support.is_dir() {
        copy_directory(&generated_support, &source.join("antlr-rust-support"))
            .expect("generated Rust support should be copied into the check crate");
    }

    Command::new(env!("CARGO"))
        .args([
            if test_source.is_empty() {
                "check"
            } else {
                "test"
            },
            "--quiet",
            "--offline",
            "--manifest-path",
            project
                .join("Cargo.toml")
                .to_str()
                .expect("temporary path should be UTF-8"),
        ])
        .env("CARGO_TARGET_DIR", project.join("target"))
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .expect("cargo check should run")
}

fn copy_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

pub(super) fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("process output should be UTF-8")
}

pub(super) fn normalize_cli_snapshot(value: &str) -> String {
    let mut normalized = value
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    if value.ends_with('\n') {
        normalized.push('\n');
    }
    normalized
}

pub(super) fn replace_miette_path(value: &str, path: &str, replacement: &str) -> String {
    let Some(first) = path.chars().next() else {
        return value.to_owned();
    };
    let mut first_buffer = [0; 4];
    let first: &str = first.encode_utf8(&mut first_buffer);
    let mut normalized = value.to_owned();
    let mut search_from = 0;
    while let Some(relative_start) = normalized[search_from..].find(first) {
        let start = search_from + relative_start;
        let Some(end) = wrapped_path_end(&normalized, start, path) else {
            search_from = start + first.len();
            continue;
        };
        normalized.replace_range(start..end, replacement);
        search_from = start + replacement.len();
    }
    normalized
}

fn wrapped_path_end(value: &str, start: usize, path: &str) -> Option<usize> {
    const CONTINUATION: &str = "\n  │ ";
    let mut cursor = start;
    for expected in path.chars() {
        while value.get(cursor..)?.starts_with(CONTINUATION) {
            cursor += CONTINUATION.len();
        }
        let actual = value.get(cursor..)?.chars().next()?;
        if actual != expected {
            return None;
        }
        cursor += actual.len_utf8();
    }
    Some(cursor)
}

#[test]
fn miette_path_normalization_handles_message_wrapping() {
    let path = "/tmp/antlr-rust-gen-long/Example.g4";
    let rendered = "  × /tmp/antlr-rust-gen-\n  │ long/Example.g4:4:6: invalid input";
    assert_eq!(
        replace_miette_path(rendered, path, "$GRAMMAR"),
        "  × $GRAMMAR:4:6: invalid input"
    );
}

pub(super) fn normalize_current_package_version(value: &str) -> String {
    const PLACEHOLDER: &str = "<generator-version>";
    let version = env!("CARGO_PKG_VERSION");
    let mut normalized = value.to_owned();
    let mut search_from = 0;
    while let Some(relative_start) = normalized[search_from..].find(version) {
        let start = search_from + relative_start;
        normalized.replace_range(start..start + version.len(), PLACEHOLDER);
        search_from = start + PLACEHOLDER.len();
    }
    normalized
}

pub(super) fn generated_parser_api(source: &str) -> Vec<String> {
    let mut api = BTreeSet::new();
    if source.contains("antlr4_runtime::__antlr4_rust_parser_facade!") {
        api.extend(
            [
                "add_error_listener",
                "add_parse_listener",
                "clear_dfa",
                "compile_parse_tree_pattern",
                "into_parsed_file",
                "into_token_store",
                "into_token_stream",
                "metadata",
                "node",
                "parse_tree_storage",
                "parser_dfa_stats",
                "prediction_context_stats",
                "remove_error_listeners",
                "remove_parse_listeners",
                "reset",
                "set_token_stream",
                "token_store",
                "token_stream",
                "token_stream_mut",
            ]
            .map(|name| format!("fn {name}")),
        );
    }
    let mut context_methods = false;
    let mut context_accessors = false;
    for line in source.lines().map(str::trim) {
        if line == "methods: {" {
            context_methods = true;
            continue;
        }
        if context_methods {
            if line == "}" {
                context_methods = false;
                continue;
            }
            let name = line
                .split_once(':')
                .map(|(_, name)| name.trim_end_matches(',').trim())
                .unwrap_or_default();
            if !name.is_empty() {
                api.insert(format!("fn {name}"));
            }
            continue;
        }
        if line == "antlr4_runtime::__antlr4_rust_context_accessors! {" {
            context_accessors = true;
            continue;
        }
        if context_accessors {
            // The first trimmed `}` is the inner `{view_name} {` close; the
            // invocation's outer `}` then falls through to the generic prefix
            // scan below, where it matches nothing.
            if line == "}" {
                context_accessors = false;
                continue;
            }
            let declaration = ["rule ", "token ", "label_rule ", "label_token "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix));
            if let Some(rest) = declaration
                && let Some((name, _)) = rest.split_once(':')
            {
                api.insert(format!("fn {}", name.trim()));
            }
            continue;
        }
        for (prefix, kind) in [
            ("pub const fn ", "fn"),
            ("pub const ", "const"),
            ("pub enum ", "enum"),
            ("pub fn ", "fn"),
            ("pub static ", "static"),
            ("pub struct ", "struct"),
            ("pub trait ", "trait"),
            ("pub type ", "type"),
        ] {
            let Some(rest) = line.strip_prefix(prefix) else {
                continue;
            };
            let name = rest
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                api.insert(format!("{kind} {name}"));
            }
            break;
        }

        let Some(rest) = line.strip_prefix("fn ") else {
            continue;
        };
        let name = rest
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .next()
            .unwrap_or_default();
        if name.starts_with("enter_") || name.starts_with("exit_") || name.starts_with("visit_") {
            api.insert(format!("callback {name}"));
        }
    }
    api.into_iter().collect()
}

/// Lines of `haystack` containing `needle`, numbered, capped so a failure
/// message stays readable when the subject is a large generated file.
pub(super) fn matching_lines(haystack: &str, needle: &str) -> String {
    const LIMIT: usize = 20;
    let hits = haystack
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(index, line)| format!("  {}: {}", index + 1, line.trim()))
        .take(LIMIT)
        .collect::<Vec<_>>();
    hits.join("\n")
}

pub(super) fn temporary_directory(label: &str) -> TempDirectory {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "antlr4-rust-gen-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary directory should be writable");
    TempDirectory(path)
}

pub(super) struct TempDirectory(PathBuf);

impl TempDirectory {
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

pub(super) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("codegen package should live below the workspace root")
        .to_path_buf()
}

pub(super) fn runtime_crate_root() -> PathBuf {
    workspace_root().join("crates/antlr-rust-runtime")
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
