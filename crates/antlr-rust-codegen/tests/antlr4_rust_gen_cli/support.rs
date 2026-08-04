pub(super) use std::collections::BTreeSet;
pub(super) use std::ffi::OsStr;
pub(super) use std::fs;
pub(super) use std::path::{Path, PathBuf};
pub(super) use std::process::{Command, Output};
pub(super) use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn run_antlr4_rust_gen(args: &[impl AsRef<OsStr>]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_antlr4-rust-gen"))
        .args(args)
        .output()
        .expect("antlr4-rust-gen should run")
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

pub(super) fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("process output should be UTF-8")
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
    let mut context_methods = false;
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
