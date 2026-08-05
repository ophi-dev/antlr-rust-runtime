mod identity;
mod prompt;
mod python;
mod stage;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::artifact::GeneratedArtifacts;
use crate::config::CompilerConfig;
use crate::error::Error;
use crate::json::to_pretty_json;
use crate::rust_output::replace_all;
use identity::{BundleIdentity, TrustStore, identify};
use prompt::TrustDecision;

pub(crate) use python::run_transform_child;

const TRANSFORM_PATH: &str = "Rust/transformGrammar.py";

#[derive(Clone, Debug, Default)]
pub(crate) struct RustSupportOptions {
    pub(crate) enabled: bool,
    pub(crate) trusted_fingerprints: BTreeSet<String>,
    pub(crate) interactive: bool,
    pub(crate) child_executable: Option<PathBuf>,
    pub(crate) trust_store_path: Option<PathBuf>,
    pub(crate) transform_timeout: Duration,
}

impl RustSupportOptions {
    pub(crate) fn disabled() -> Self {
        Self::default()
    }
}

pub(crate) fn parse_trust_fingerprint(value: &str) -> Result<String, String> {
    let digest = value.strip_prefix("sha256:").unwrap_or(value);
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("expected sha256:<64 lowercase hexadecimal digits>".to_owned());
    }
    Ok(format!("sha256:{digest}"))
}

#[derive(Debug)]
struct SupportFile {
    source: PathBuf,
    module_name: String,
}

#[derive(Debug)]
struct PreparedBundle {
    _temporary: tempfile::TempDir,
    original_directory: PathBuf,
    staged_directory: PathBuf,
    identity: BundleIdentity,
    generated_module: String,
    artifact_directory: PathBuf,
    support_files: Vec<SupportFile>,
    transformed_grammars: Vec<PathBuf>,
}

#[derive(Serialize)]
struct RustSupportManifest<'a> {
    version: u8,
    bundles: Vec<RustSupportBundleManifest<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RustSupportBundleManifest<'a> {
    source: String,
    revision: Option<&'a str>,
    fingerprint: &'a str,
    transform: &'static str,
    rust_modules: Vec<&'a str>,
    transformed_grammars: Vec<&'a str>,
}

#[derive(Debug)]
pub(crate) struct PreparedRustSupport {
    roots: Vec<PathBuf>,
    library_directories: Vec<PathBuf>,
    bundles: Vec<PreparedBundle>,
}

impl PreparedRustSupport {
    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(crate) fn library_directories(&self) -> &[PathBuf] {
        &self.library_directories
    }

    pub(crate) const fn has_bundles(&self) -> bool {
        !self.bundles.is_empty()
    }

    pub(crate) fn all_roots_supported(&self) -> bool {
        self.has_bundles() && self.roots.iter().all(|root| self.supports_source(root))
    }

    pub(crate) fn supports_source(&self, source: &Path) -> bool {
        self.bundles
            .iter()
            .any(|bundle| source.starts_with(&bundle.staged_directory))
    }

    pub(crate) fn original_path(&self, path: &Path) -> PathBuf {
        self.bundles
            .iter()
            .find_map(|bundle| {
                path.strip_prefix(&bundle.staged_directory)
                    .ok()
                    .map(|relative| bundle.original_directory.join(relative))
            })
            .unwrap_or_else(|| path.to_path_buf())
    }

    pub(crate) fn decorate_rendered_module(&self, module: &mut String) {
        for bundle in &self.bundles {
            let reference = format!("super::{}::", bundle.generated_module);
            if !module.contains(&reference) {
                continue;
            }
            let module_path = portable_generated_path(&bundle.artifact_directory.join("mod.rs"));
            let declaration = format!(
                "#[doc(hidden)]\n#[path = {path:?}]\npub mod {module_name};\n\
                 #[allow(unused_imports)]\npub use self::{module_name} as rust_support;\n\n",
                path = module_path,
                module_name = bundle.generated_module,
            );
            module.insert_str(0, &declaration);
        }
    }

    pub(crate) fn add_artifacts(&self, artifacts: &mut GeneratedArtifacts) -> io::Result<()> {
        for bundle in &self.bundles {
            if !bundle.support_files.is_empty() {
                let mut aggregator = String::from(
                    "#![allow(warnings, missing_docs, clippy::all, clippy::pedantic, \
                     clippy::nursery)]\n",
                );
                for support in &bundle.support_files {
                    let _ = writeln!(aggregator, "pub mod {};", support.module_name);
                    artifacts.insert(
                        bundle
                            .artifact_directory
                            .join(support.source.file_name().expect("support file has a name")),
                        fs::read(&support.source)?,
                    )?;
                }
                artifacts.insert(bundle.artifact_directory.join("mod.rs"), aggregator)?;
            }
            for grammar in &bundle.transformed_grammars {
                artifacts.insert(
                    bundle
                        .artifact_directory
                        .join("transformed")
                        .join(grammar.file_name().expect("grammar has a file name")),
                    fs::read(grammar)?,
                )?;
            }
        }
        if !self.bundles.is_empty() {
            artifacts.insert("rust-support.json", self.render_manifest()?)?;
        }
        Ok(())
    }

    pub(crate) fn additional_inputs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.bundles.iter().flat_map(|bundle| {
            let script = bundle.original_directory.join(TRANSFORM_PATH);
            std::iter::once(script)
                .chain(
                    bundle
                        .support_files
                        .iter()
                        .filter_map(|support| support.source.file_name())
                        .map(|name| bundle.original_directory.join("Rust").join(name))
                        .filter(|path| path.is_file()),
                )
                .chain(
                    bundle
                        .transformed_grammars
                        .iter()
                        .filter_map(|grammar| grammar.file_name())
                        .map(|name| bundle.original_directory.join(name))
                        .filter(|path| path.is_file()),
                )
        })
    }

    fn render_manifest(&self) -> io::Result<String> {
        let bundles = self
            .bundles
            .iter()
            .map(|bundle| {
                Ok(RustSupportBundleManifest {
                    source: bundle.identity.source_label(),
                    revision: bundle.identity.revision.as_deref(),
                    fingerprint: &bundle.identity.fingerprint,
                    transform: TRANSFORM_PATH,
                    rust_modules: file_names(
                        bundle
                            .support_files
                            .iter()
                            .map(|support| support.source.as_path()),
                    )?,
                    transformed_grammars: file_names(
                        bundle.transformed_grammars.iter().map(PathBuf::as_path),
                    )?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(to_pretty_json(&RustSupportManifest {
            version: 1,
            bundles,
        }))
    }
}

pub(crate) fn prepare(config: &CompilerConfig) -> Result<PreparedRustSupport, Error> {
    if !config.rust_support.enabled {
        return Ok(PreparedRustSupport {
            roots: config.roots.clone(),
            library_directories: config.library_directories.clone(),
            bundles: Vec::new(),
        });
    }

    let candidates = discover(&config.roots);
    if candidates.is_empty() {
        return Ok(PreparedRustSupport {
            roots: config.roots.clone(),
            library_directories: config.library_directories.clone(),
            bundles: Vec::new(),
        });
    }
    let executable = config
        .rust_support
        .child_executable
        .as_deref()
        .ok_or_else(|| {
            Error::configuration(
                "trusted Rust target support currently requires the antlr4-rust-gen CLI",
            )
        })?;
    let mut trust_store = None;
    let mut roots = config.roots.clone();
    let mut bundles = Vec::with_capacity(candidates.len());

    for (directory, root_indexes) in candidates {
        let support_directory = directory.join("Rust");
        let (temporary, staged_directory) =
            stage::copy_grammar_directory(&directory).map_err(Error::generation)?;
        let staged_support_directory = staged_directory.join("Rust");
        let ships_rust_modules =
            !stage::top_level_files_with_extension(&staged_support_directory, "rs")
                .map_err(Error::generation)?
                .is_empty();
        if ships_rust_modules {
            stage::prepare_support_output_directory(&staged_directory)
                .map_err(Error::generation)?;
        }
        let identity = identify(&directory, &support_directory, &staged_support_directory)
            .map_err(Error::generation)?;
        ensure_trusted(&config.rust_support, &identity, &mut trust_store)?;
        stage::execute_transform(
            executable,
            &staged_directory,
            config.rust_support.transform_timeout,
        )
        .map_err(Error::generation)?;

        let support_paths = stage::top_level_files_with_extension(&staged_support_directory, "rs")
            .map_err(Error::generation)?;
        let artifact_id = identity.artifact_id();
        let generated_module = format!(
            "__antlr_rust_support_{}",
            replace_all(&artifact_id, "-", "_")
        );
        rewrite_support_paths(&staged_directory, &support_paths, &generated_module)
            .map_err(Error::generation)?;
        let transformed_grammars = stage::top_level_files_with_extension(&staged_directory, "g4")
            .map_err(Error::generation)?;
        for root_index in root_indexes {
            let relative = canonical_relative_root(&config.roots[root_index], &directory)
                .map_err(Error::generation)?;
            let transformed = staged_directory.join(relative);
            if !transformed.is_file() {
                return Err(Error::configuration(format!(
                    "Rust target transform did not preserve grammar root {}",
                    config.roots[root_index].display()
                )));
            }
            roots[root_index] = transformed;
        }
        let support_files = support_paths
            .into_iter()
            .map(|source| {
                let module_name = source
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .expect("validated Rust support module name")
                    .to_owned();
                SupportFile {
                    source,
                    module_name,
                }
            })
            .collect();
        bundles.push(PreparedBundle {
            _temporary: temporary,
            original_directory: directory,
            staged_directory,
            identity,
            generated_module,
            artifact_directory: PathBuf::from("antlr-rust-support").join(artifact_id),
            support_files,
            transformed_grammars,
        });
    }

    let library_directories = config
        .library_directories
        .iter()
        .map(|library| remap_library_directory(library, &bundles))
        .collect();
    Ok(PreparedRustSupport {
        roots,
        library_directories,
        bundles,
    })
}

fn discover(roots: &[PathBuf]) -> BTreeMap<PathBuf, Vec<usize>> {
    let mut candidates = BTreeMap::<PathBuf, Vec<usize>>::new();
    for (index, root) in roots.iter().enumerate() {
        let Ok(canonical) = fs::canonicalize(root) else {
            continue;
        };
        let Some(directory) = canonical.parent() else {
            continue;
        };
        if directory.join(TRANSFORM_PATH).is_file() {
            candidates
                .entry(directory.to_path_buf())
                .or_default()
                .push(index);
        }
    }
    candidates
}

fn ensure_trusted(
    options: &RustSupportOptions,
    identity: &BundleIdentity,
    trust_store: &mut Option<TrustStore>,
) -> Result<(), Error> {
    if options.trusted_fingerprints.contains(&identity.fingerprint) {
        return Ok(());
    }
    if trust_store.is_none() {
        *trust_store =
            Some(TrustStore::load(options.trust_store_path.clone()).map_err(Error::generation)?);
    }
    let trust_store = trust_store
        .as_mut()
        .expect("trust store is loaded before persisted decisions are checked");
    if trust_store.trusts_revision(identity) || trust_store.trusts_repository(identity) {
        return Ok(());
    }
    if !options.interactive {
        return Err(Error::configuration(format!(
            "Rust target support is not trusted\n\nsource: {}\nfingerprint: {}\n\nrerun with \
             --trust-rust-support {}",
            identity.source_label(),
            identity.fingerprint,
            identity.fingerprint
        )));
    }
    match prompt::ask(identity).map_err(Error::generation)? {
        TrustDecision::Once => Ok(()),
        TrustDecision::Revision => trust_store
            .trust_revision(identity)
            .map_err(Error::generation),
        TrustDecision::Repository => trust_store
            .trust_repository(identity)
            .map_err(Error::generation),
        TrustDecision::Abort => Err(Error::configuration(
            "Rust target support execution was not approved",
        )),
    }
}

fn canonical_relative_root(root: &Path, directory: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(root)?;
    canonical
        .strip_prefix(directory)
        .map(Path::to_path_buf)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "grammar root {} is outside its support directory {}",
                    canonical.display(),
                    directory.display()
                ),
            )
        })
}

fn rewrite_support_paths(
    staged_directory: &Path,
    support_paths: &[PathBuf],
    generated_module: &str,
) -> io::Result<()> {
    let modules = support_paths
        .iter()
        .map(|path| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .filter(|name| valid_rust_identifier(name))
                .map(str::to_owned)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid Rust support module name: {}", path.display()),
                    )
                })
        })
        .collect::<io::Result<Vec<_>>>()?;
    for grammar in stage::top_level_files_with_extension(staged_directory, "g4")? {
        let mut source = fs::read_to_string(&grammar)?;
        let original = source.clone();
        for module in &modules {
            source = replace_all(
                &source,
                &format!("crate::{module}"),
                &format!("super::{generated_module}::{module}"),
            );
        }
        if source != original {
            fs::write(grammar, source)?;
        }
    }
    for support in support_paths {
        let mut source = fs::read_to_string(support)?;
        let original = source.clone();
        for module in &modules {
            source = replace_all(
                &source,
                &format!("crate::{module}"),
                &format!("super::{module}"),
            );
        }
        if source != original {
            fs::write(support, source)?;
        }
    }
    Ok(())
}

fn valid_rust_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn portable_generated_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|character| {
            if character == std::path::MAIN_SEPARATOR {
                '/'
            } else {
                character
            }
        })
        .collect()
}

fn remap_library_directory(library: &Path, bundles: &[PreparedBundle]) -> PathBuf {
    let Ok(canonical) = fs::canonicalize(library) else {
        return library.to_path_buf();
    };
    for bundle in bundles {
        if let Ok(relative) = canonical.strip_prefix(&bundle.original_directory) {
            return bundle.staged_directory.join(relative);
        }
    }
    library.to_path_buf()
}

fn file_names<'a>(paths: impl Iterator<Item = &'a Path>) -> io::Result<Vec<&'a str>> {
    paths
        .map(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Rust support output path is not UTF-8: {}", path.display()),
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_trust_fingerprint;

    #[test]
    fn trust_fingerprint_parser_normalizes_the_prefix() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_trust_fingerprint(&digest).expect("digest should parse"),
            format!("sha256:{digest}")
        );
        assert!(parse_trust_fingerprint(&"A".repeat(64)).is_err());
        assert!(parse_trust_fingerprint("sha256:abc").is_err());
    }
}
